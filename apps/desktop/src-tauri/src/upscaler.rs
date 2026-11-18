//! The optional AI upscaler (see `catconvert_core::Upscaler`): Settings →
//! Upscaling downloads, installs from files or removes it, and tries it on
//! the graphics card.
//!
//! Downloading only fetches: the upscaler from this version's GitHub release
//! and the models from the official Real-ESRGAN release. Nothing is sent, and
//! the core checks every file against the SHA-256 compiled into the app
//! before installing it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use catconvert_core::{
    ConvertOptions, Download, DownloadKind, GpuCheck, MediaInfo, Upscaler, UpscalerStatus,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::AppState;

/// Event name for [`DownloadProgress`] payloads.
const DOWNLOAD_EVENT: &str = "upscaler-download";

/// Emit progress at most every this many bytes.
const PROGRESS_STEP: u64 = 256 * 1024;

/// What the app knows about the upscaler beyond the files on disk.
#[derive(Default)]
pub(crate) struct UpscalerState {
    /// The last test run on the graphics card, if any since it was installed.
    gpu: Mutex<Option<GpuCheck>>,
    /// A download or install is running.
    busy: AtomicBool,
    /// Raised to stop the running download.
    cancel: Arc<AtomicBool>,
}

impl UpscalerState {
    fn gpu(&self) -> Option<GpuCheck> {
        self.gpu
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set_gpu(&self, check: Option<GpuCheck>) {
        *self.gpu.lock().unwrap_or_else(PoisonError::into_inner) = check;
    }
}

/// Clears [`UpscalerState::busy`] however the work ends.
struct Busy<'a>(&'a AtomicBool);

impl<'a> Busy<'a> {
    fn start(flag: &'a AtomicBool) -> Result<Self, String> {
        if flag.swap(true, Ordering::SeqCst) {
            return Err("AI upscaling is already being installed".to_owned());
        }
        Ok(Self(flag))
    }
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// The state of AI upscaling for Settings and the editor.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpscalerInfo {
    #[serde(flatten)]
    status: UpscalerStatus,
    /// The last test run on the graphics card.
    gpu: Option<GpuCheck>,
    /// A download or install is running.
    busy: bool,
}

/// How far a download has got, over all files.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    received: u64,
    /// `None` until every file's size is known.
    total: Option<u64>,
}

/// Where the upscaler is installed: the folder the core and the CLI look in,
/// or the app's data folder when no home folder is known.
pub(crate) fn install_dir(app: &AppHandle) -> Result<PathBuf, String> {
    catconvert_core::upscaler_dir().map(Ok).unwrap_or_else(|| {
        app.path()
            .app_data_dir()
            .map(|dir| dir.join("upscaler"))
            .map_err(|err| err.to_string())
    })
}

fn info(app: &AppHandle, upscaler: &UpscalerState) -> Result<UpscalerInfo, String> {
    Ok(UpscalerInfo {
        status: catconvert_core::upscaler_status(&install_dir(app)?),
        gpu: upscaler.gpu(),
        busy: upscaler.busy.load(Ordering::SeqCst),
    })
}

/// Point the conversion backend at what is installed now.
fn relocate(state: &AppState, dir: &Path) {
    if let Some(ffmpeg) = state.ffmpeg() {
        ffmpeg.set_upscaler(Upscaler::locate(Some(dir)));
    }
}

/// Try the upscaler on the graphics card and remember the result.
async fn check_gpu(state: &AppState, upscaler: &UpscalerState) -> Result<(), String> {
    let check = match state.ffmpeg() {
        Some(ffmpeg) if ffmpeg.upscaler().is_some() => Some(
            tauri::async_runtime::spawn_blocking(move || ffmpeg.check_upscaler())
                .await
                .map_err(|err| err.to_string())?,
        ),
        _ => None,
    };
    upscaler.set_gpu(check);
    Ok(())
}

/// What is installed, and how the last test on the graphics card went.
#[tauri::command]
pub(crate) fn upscaler_status(
    app: AppHandle,
    upscaler: State<'_, UpscalerState>,
) -> Result<UpscalerInfo, String> {
    info(&app, &upscaler)
}

/// Try the upscaler on the graphics card (takes a second or two).
#[tauri::command]
pub(crate) async fn check_upscaler_gpu(
    app: AppHandle,
    state: State<'_, AppState>,
    upscaler: State<'_, UpscalerState>,
) -> Result<UpscalerInfo, String> {
    check_gpu(&state, &upscaler).await?;
    info(&app, &upscaler)
}

/// Download what the install still lacks (or an update of the upscaler),
/// install it, and try it on the graphics card. Progress arrives as
/// `upscaler-download` events.
#[tauri::command]
pub(crate) async fn download_upscaler(
    app: AppHandle,
    state: State<'_, AppState>,
    upscaler: State<'_, UpscalerState>,
) -> Result<UpscalerInfo, String> {
    let _busy = Busy::start(&upscaler.busy)?;
    upscaler.cancel.store(false, Ordering::SeqCst);
    let dir = install_dir(&app)?;
    let status = catconvert_core::upscaler_status(&dir);
    let wanted: Vec<Download> = catconvert_core::upscaler_downloads()
        .into_iter()
        .filter(|d| {
            status.missing.contains(&d.kind)
                || (d.kind == DownloadKind::Upscaler && status.outdated)
        })
        .collect();
    if status.missing.contains(&DownloadKind::Upscaler)
        && !wanted.iter().any(|d| d.kind == DownloadKind::Upscaler)
    {
        return Err(
            "this build of CatConvert can't download the upscaler: use \"Install from file…\""
                .to_owned(),
        );
    }
    if wanted.is_empty() {
        return info(&app, &upscaler);
    }

    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let downloads = tempfile::Builder::new()
        .prefix(".download-")
        .tempdir_in(&dir)
        .map_err(|err| err.to_string())?;
    let files = fetch(&app, &wanted, downloads.path(), &upscaler.cancel).await?;

    let target = dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        catconvert_core::install_upscaler(&target, &files)
    })
    .await
    .map_err(|err| err.to_string())?
    .map_err(|err| err.to_string())?;
    drop(downloads);
    relocate(&state, &dir);
    check_gpu(&state, &upscaler).await?;
    info(&app, &upscaler)
}

/// Stop a running download.
#[tauri::command]
pub(crate) fn cancel_upscaler_download(upscaler: State<'_, UpscalerState>) {
    upscaler.cancel.store(true, Ordering::SeqCst);
}

/// Install from files the user downloaded (see
/// `catconvert_core::install_upscaler`), then try it on the graphics card.
#[tauri::command]
pub(crate) async fn install_upscaler_files(
    app: AppHandle,
    state: State<'_, AppState>,
    upscaler: State<'_, UpscalerState>,
    paths: Vec<PathBuf>,
) -> Result<UpscalerInfo, String> {
    let _busy = Busy::start(&upscaler.busy)?;
    let dir = install_dir(&app)?;
    let target = dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        catconvert_core::install_upscaler(&target, &paths)
    })
    .await
    .map_err(|err| err.to_string())?
    .map_err(|err| err.to_string())?;
    relocate(&state, &dir);
    check_gpu(&state, &upscaler).await?;
    info(&app, &upscaler)
}

/// Remove the upscaler and its models.
#[tauri::command]
pub(crate) async fn remove_upscaler(
    app: AppHandle,
    state: State<'_, AppState>,
    upscaler: State<'_, UpscalerState>,
) -> Result<UpscalerInfo, String> {
    let _busy = Busy::start(&upscaler.busy)?;
    let dir = install_dir(&app)?;
    let target = dir.clone();
    tauri::async_runtime::spawn_blocking(move || catconvert_core::uninstall_upscaler(&target))
        .await
        .map_err(|err| err.to_string())?
        .map_err(|err| err.to_string())?;
    relocate(&state, &dir);
    upscaler.set_gpu(None);
    info(&app, &upscaler)
}

/// How long the AI pass of a conversion roughly takes, in seconds, from a
/// fast to a slow graphics card; `None` when it uses no AI upscaling.
#[tauri::command]
pub(crate) fn ai_estimate(info: MediaInfo, options: ConvertOptions) -> Option<(f64, f64)> {
    catconvert_core::ai_estimate(&info, &options)
}

/// Download `wanted` into `dir`, reporting progress, and return the files.
async fn fetch(
    app: &AppHandle,
    wanted: &[Download],
    dir: &Path,
    cancel: &AtomicBool,
) -> Result<Vec<PathBuf>, String> {
    // The same TLS setup as the updater: rustls with ring.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder()
        .user_agent(concat!("CatConvert/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|err| err.to_string())?;
    let mut sizes: Vec<Option<u64>> = wanted.iter().map(|d| d.size).collect();
    let mut received = 0_u64;
    let mut files = Vec::new();
    for (index, download) in wanted.iter().enumerate() {
        let failed = |err: &dyn std::fmt::Display| {
            format!("couldn't download {}: {err}", download.file_name)
        };
        let mut response = client
            .get(&download.url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|err| failed(&err))?;
        if let Some(length) = response.content_length() {
            sizes[index] = Some(length);
        }
        let path = dir.join(&download.file_name);
        let mut file = std::fs::File::create(&path).map_err(|err| failed(&err))?;
        let mut reported = received;
        while let Some(chunk) = response.chunk().await.map_err(|err| failed(&err))? {
            if cancel.load(Ordering::SeqCst) {
                return Err("the download was cancelled".to_owned());
            }
            std::io::Write::write_all(&mut file, &chunk).map_err(|err| failed(&err))?;
            received += chunk.len() as u64;
            if received - reported >= PROGRESS_STEP {
                reported = received;
                let total = sizes.iter().copied().sum::<Option<u64>>();
                let _ = app.emit(DOWNLOAD_EVENT, DownloadProgress { received, total });
            }
        }
        file.sync_all().map_err(|err| failed(&err))?;
        files.push(path);
    }
    let total = sizes.iter().copied().sum::<Option<u64>>();
    let _ = app.emit(DOWNLOAD_EVENT, DownloadProgress { received, total });
    Ok(files)
}
