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
use std::time::Duration;

use catconvert_core::{
    ConvertOptions, DownloadKind, GpuCheck, MediaInfo, UnpinnedDownload, Upscaler, UpscalerStatus,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::AppState;

/// Event name for [`DownloadProgress`] payloads.
const DOWNLOAD_EVENT: &str = "upscaler-download";

/// Emit progress at most every this many bytes.
const PROGRESS_STEP: u64 = 256 * 1024;

/// How long a server may take to answer, and to send the next piece of a
/// file, before the download gives up instead of waiting on a dead connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Longest `.sha256` file taken for one: a hash, two spaces and a file name.
const MAX_CHECKSUM_BYTES: usize = 1024;

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
    /// This build downloads the upscaler without a SHA-256 of its own to
    /// check it against (see `catconvert_core::unpinned_upscaler_download`).
    unpinned: bool,
}

/// What a running download is busy with.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum Stage {
    /// Fetching the files, which can be cancelled.
    Downloading,
    /// Checking, unpacking and converting what was fetched.
    Installing,
    /// Trying the upscaler on the graphics card.
    Testing,
}

/// How far a download has got, over all files.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    stage: Stage,
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
        unpinned: catconvert_core::unpinned_upscaler_download().is_some(),
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
///
/// A build from source has no SHA-256 of the upscaler to check it against
/// (see `catconvert_core::can_download_upscaler`), so it fetches the one in
/// its version's release and checks it against the checksum published there
/// (`catconvert_core::unpinned_upscaler_download`). The models don't wait for
/// that to work: they are installed even if the upscaler can't be had.
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
    let mut wanted: Vec<Wanted> = catconvert_core::upscaler_downloads()
        .into_iter()
        .filter(|d| {
            status.missing.contains(&d.kind)
                || (d.kind == DownloadKind::Upscaler && status.outdated)
        })
        .map(|d| Wanted {
            url: d.url,
            file_name: d.file_name,
            size: d.size,
            optional: false,
        })
        .collect();
    // Last, so that the models are in whatever becomes of it.
    let unpinned =
        catconvert_core::unpinned_upscaler_download().filter(|_| status.binary.is_none());
    if let Some(pack) = &unpinned {
        wanted.push(Wanted {
            url: pack.url.clone(),
            file_name: pack.file_name.clone(),
            size: None,
            optional: true,
        });
    }
    if wanted.is_empty() {
        return info(&app, &upscaler);
    }

    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let downloads = tempfile::Builder::new()
        .prefix(".download-")
        .tempdir_in(&dir)
        .map_err(|err| err.to_string())?;
    let client = client()?;
    let progress = fetch(&app, &client, &wanted, downloads.path(), &upscaler.cancel).await?;
    let mut files = progress.files;
    // What kept the upscaler of a build from source out, told once the rest is in.
    let mut problem = progress.missed.map(|err| match err {
        FetchError::NotFound => format!(
            "the release of CatConvert {} has no upscaler to download. Build one with `mise run \
             upscaler` and install it from a file",
            env!("CARGO_PKG_VERSION")
        ),
        err => err.message(unpinned.as_ref().map_or("the upscaler", |p| &p.file_name)),
    });
    if let Some(pack) = &unpinned {
        let path = downloads.path().join(&pack.file_name);
        if files.contains(&path)
            && let Err(err) = check_unpinned(&client, pack, &path).await
        {
            files.retain(|file| file != &path);
            problem = Some(err);
        }
    }
    let staged = |stage| {
        let _ = app.emit(
            DOWNLOAD_EVENT,
            DownloadProgress {
                stage,
                received: progress.received,
                total: Some(progress.received),
            },
        );
    };

    if !files.is_empty() {
        staged(Stage::Installing);
        let target = dir.clone();
        tauri::async_runtime::spawn_blocking(move || {
            catconvert_core::install_upscaler(&target, &files)
        })
        .await
        .map_err(|err| err.to_string())?
        .map_err(|err| err.to_string())?;
    }
    drop(downloads);
    relocate(&state, &dir);
    staged(Stage::Testing);
    check_gpu(&state, &upscaler).await?;
    match problem {
        Some(problem) => Err(problem),
        None => info(&app, &upscaler),
    }
}

/// Check the upscaler a build from source downloaded against the checksum
/// published next to it.
async fn check_unpinned(
    client: &reqwest::Client,
    pack: &UnpinnedDownload,
    file: &Path,
) -> Result<(), String> {
    let name = format!("{}.sha256", pack.file_name);
    let checksum = client
        .get(&pack.checksum_url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|err| FetchError::from(&err).message(&name))?
        .bytes()
        .await
        .map_err(|err| FetchError::from(&err).message(&name))?;
    // A checksum is a line of text; anything long is something else.
    let text = std::str::from_utf8(&checksum)
        .ok()
        .filter(|text| text.len() <= MAX_CHECKSUM_BYTES)
        .unwrap_or_default();
    match catconvert_core::upscaler_matches_checksum(file, text) {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!(
            "{} doesn't match the checksum published with it, so it wasn't installed. Try again",
            pack.file_name
        )),
        Err(err) => Err(err.to_string()),
    }
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

/// A file to download.
struct Wanted {
    url: String,
    /// The name to save it under.
    file_name: String,
    /// Size in bytes, where known in advance.
    size: Option<u64>,
    /// The rest is worth installing without it.
    optional: bool,
}

/// Why a file didn't arrive.
enum FetchError {
    /// The user stopped the download.
    Cancelled,
    /// The server has no such file.
    NotFound,
    /// The server didn't answer, or stopped answering.
    NoAnswer,
    /// Anything else, in the words of what failed.
    Other(String),
}

impl FetchError {
    /// What happened to `file`, in words that say what to do about it.
    fn message(&self, file: &str) -> String {
        match self {
            Self::Cancelled => "the download was cancelled".to_owned(),
            Self::NotFound => format!("couldn't download {file}: the server doesn't have it"),
            Self::NoAnswer => format!(
                "couldn't download {file}: the server didn't answer. Check your internet \
                 connection, then try again"
            ),
            Self::Other(reason) => format!("couldn't download {file}: {reason}"),
        }
    }
}

impl From<&reqwest::Error> for FetchError {
    fn from(err: &reqwest::Error) -> Self {
        if err.is_connect() || err.is_timeout() {
            Self::NoAnswer
        } else if err.status() == Some(reqwest::StatusCode::NOT_FOUND) {
            Self::NotFound
        } else if let Some(status) = err.status() {
            Self::Other(format!("the server answered {status}"))
        } else {
            Self::Other(err.to_string())
        }
    }
}

impl From<std::io::Error> for FetchError {
    fn from(err: std::io::Error) -> Self {
        Self::Other(err.to_string())
    }
}

/// What [`fetch`] brought in.
struct Fetched {
    files: Vec<PathBuf>,
    /// Bytes over all files.
    received: u64,
    /// Why an optional file is not among them.
    missed: Option<FetchError>,
}

/// The same TLS setup as the updater: rustls with ring.
fn client() -> Result<reqwest::Client, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .user_agent(concat!("CatConvert/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .map_err(|err| err.to_string())
}

/// Download `wanted` into `dir`, reporting progress, and return the files.
/// Only a file that isn't optional fails the lot, and so does cancelling.
async fn fetch(
    app: &AppHandle,
    client: &reqwest::Client,
    wanted: &[Wanted],
    dir: &Path,
    cancel: &AtomicBool,
) -> Result<Fetched, String> {
    let mut sizes: Vec<Option<u64>> = wanted.iter().map(|d| d.size).collect();
    let mut received = 0_u64;
    let mut files = Vec::new();
    let mut missed = None;
    let report = |received, sizes: &[Option<u64>]| {
        let total = sizes.iter().copied().sum::<Option<u64>>();
        let _ = app.emit(
            DOWNLOAD_EVENT,
            DownloadProgress {
                stage: Stage::Downloading,
                received,
                total,
            },
        );
    };
    for (index, download) in wanted.iter().enumerate() {
        let path = dir.join(&download.file_name);
        let before = received;
        let fetched: Result<(), FetchError> = async {
            let mut response = client
                .get(&download.url)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(|err| FetchError::from(&err))?;
            if let Some(length) = response.content_length() {
                sizes[index] = Some(length);
            }
            let mut file = std::fs::File::create(&path)?;
            let mut reported = received;
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|err| FetchError::from(&err))?
            {
                if cancel.load(Ordering::SeqCst) {
                    return Err(FetchError::Cancelled);
                }
                std::io::Write::write_all(&mut file, &chunk)?;
                received += chunk.len() as u64;
                if received - reported >= PROGRESS_STEP {
                    reported = received;
                    report(received, &sizes);
                }
            }
            file.sync_all()?;
            Ok(())
        }
        .await;
        match fetched {
            Ok(()) => files.push(path),
            Err(err) if download.optional && !matches!(err, FetchError::Cancelled) => {
                // Its bytes don't count towards what gets installed.
                received = before;
                sizes[index] = Some(0);
                missed = Some(err);
            }
            Err(err) => return Err(err.message(&download.file_name)),
        }
    }
    report(received, &sizes);
    Ok(Fetched {
        files,
        received,
        missed,
    })
}
