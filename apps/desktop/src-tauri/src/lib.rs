//! Tauri shell for CatConvert.
//!
//! Commands here are thin wrappers over `catconvert-core`: they convert
//! arguments, call the core, and forward results or events to the webview.
//! No conversion logic belongs in this crate.
//!
//! Anything that starts ffmpeg runs on the blocking thread pool, never on the
//! main thread, so the window stays responsive.
//!
//! Release builds ship ffmpeg, ffprobe and libjxl's tools as Tauri sidecars
//! (see `tauri.media-tools.conf.json`); without them the app falls back to an
//! installed ffmpeg.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use catconvert_core::{
    ConvertOptions, Error, Ffmpeg, JobId, JobTicket, JxlSupport, MediaInfo, OutputFormat, Queue,
    SequenceOptions,
};
use serde::Serialize;
use tauri::ipc::Response;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, State};

/// Event name for [`catconvert_core::JobProgress`] payloads.
const JOB_PROGRESS_EVENT: &str = "job-progress";

/// Bundled resource listing the licenses of the sidecar tools.
const LICENSES_RESOURCE: &str = "THIRD-PARTY-LICENSES.txt";

/// The conversion backend, or why there is none.
enum Backend {
    Ready(Arc<Queue>),
    Missing(String),
}

struct AppState {
    backend: RwLock<Backend>,
}

impl AppState {
    fn queue(&self) -> Result<Arc<Queue>, String> {
        match &*self.backend.read().unwrap_or_else(|e| e.into_inner()) {
            Backend::Ready(queue) => Ok(Arc::clone(queue)),
            Backend::Missing(reason) => Err(reason.clone()),
        }
    }

    fn ffmpeg(&self) -> Option<Ffmpeg> {
        self.queue().ok().map(|queue| queue.ffmpeg().clone())
    }
}

/// Where `bundle.externalBin` puts the sidecars: next to the executable
/// (`Contents/MacOS` on macOS, the install folder on Windows, `usr/bin` in the
/// Linux packages). Development builds have none there.
fn sidecar_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

fn start_backend(app: &AppHandle) -> Backend {
    match Ffmpeg::locate_with(sidecar_dir().as_deref()) {
        Ok(ffmpeg) => {
            let app = app.clone();
            Backend::Ready(Arc::new(Queue::new(ffmpeg, move |event| {
                if let Err(err) = app.emit(JOB_PROGRESS_EVENT, &event) {
                    eprintln!("failed to emit {JOB_PROGRESS_EVENT}: {err}");
                }
            })))
        }
        Err(err) => Backend::Missing(err.to_string()),
    }
}

/// Run blocking core work off the main thread.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|err| err.to_string())?
        .map_err(|err| err.to_string())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FormatSupport {
    format: OutputFormat,
    available: bool,
    /// Pictures or a video can become an animation in this format.
    animated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FfmpegStatus {
    found: bool,
    /// The ffmpeg shipped with the app, not an installed one.
    bundled: bool,
    version: Option<String>,
    path: Option<PathBuf>,
    error: Option<String>,
    formats: Vec<FormatSupport>,
    /// Version of libjxl's tools, if `cjxl` was found.
    libjxl: Option<String>,
    jxl: JxlSupport,
}

fn status_of(state: &AppState) -> FfmpegStatus {
    let ffmpeg = state.ffmpeg();
    let formats = OutputFormat::ALL
        .iter()
        .map(|&format| FormatSupport {
            format,
            available: ffmpeg.as_ref().is_some_and(|f| f.supports(format)),
            animated: ffmpeg
                .as_ref()
                .is_some_and(|f| f.supports_animation(format)),
        })
        .collect();
    FfmpegStatus {
        found: ffmpeg.is_some(),
        bundled: ffmpeg.as_ref().is_some_and(Ffmpeg::is_bundled),
        version: ffmpeg.as_ref().map(|f| f.version().to_owned()),
        path: ffmpeg.as_ref().map(|f| f.ffmpeg_path().to_path_buf()),
        error: state.queue().err(),
        formats,
        libjxl: ffmpeg.as_ref().and_then(|f| f.jxl_tools().version.clone()),
        jxl: ffmpeg.as_ref().map(Ffmpeg::jxl_support).unwrap_or_default(),
    }
}

/// Which ffmpeg is in use and which output formats it can write.
#[tauri::command]
fn ffmpeg_status(state: State<'_, AppState>) -> FfmpegStatus {
    status_of(&state)
}

/// Look for ffmpeg again (after the user installed it). Queued jobs of the
/// previous queue still finish.
#[tauri::command]
async fn relocate_ffmpeg(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<FfmpegStatus, String> {
    let handle = app.clone();
    let backend = tauri::async_runtime::spawn_blocking(move || start_backend(&handle))
        .await
        .map_err(|err| err.to_string())?;
    *state.backend.write().unwrap_or_else(|e| e.into_inner()) = backend;
    Ok(status_of(&state))
}

/// License texts of the bundled tools, or `None` in builds without them.
#[tauri::command]
async fn third_party_licenses(app: AppHandle) -> Option<String> {
    let path = app
        .path()
        .resolve(LICENSES_RESOURCE, BaseDirectory::Resource)
        .ok()?;
    tauri::async_runtime::spawn_blocking(move || std::fs::read_to_string(path).ok())
        .await
        .ok()
        .flatten()
}

/// Probe a file: ffprobe details when ffmpeg is available, size and
/// extension otherwise.
#[tauri::command]
async fn probe_file(state: State<'_, AppState>, path: String) -> Result<MediaInfo, String> {
    let ffmpeg = state.ffmpeg();
    blocking(move || match ffmpeg {
        Some(ffmpeg) => ffmpeg.probe(&path),
        None => catconvert_core::probe(&path),
    })
    .await
}

/// One frame as JPEG bytes (an `ArrayBuffer` on the JS side).
#[tauri::command]
async fn preview_frame(
    state: State<'_, AppState>,
    path: String,
    at: f64,
    height: u32,
) -> Result<Response, String> {
    let ffmpeg = state.queue()?.ffmpeg().clone();
    let bytes = blocking(move || ffmpeg.frame(path.as_ref(), at, height.clamp(16, 1080))).await?;
    Ok(Response::new(bytes))
}

/// The audio waveform as PNG bytes.
#[tauri::command]
async fn waveform(
    state: State<'_, AppState>,
    path: String,
    width: u32,
    height: u32,
    color: String,
) -> Result<Response, String> {
    let ffmpeg = state.queue()?.ffmpeg().clone();
    let bytes = blocking(move || ffmpeg.waveform(path.as_ref(), width, height, &color)).await?;
    Ok(Response::new(bytes))
}

/// The command(s) a conversion would run, for the "show command" panel.
#[tauri::command]
fn preview_command(
    state: State<'_, AppState>,
    info: MediaInfo,
    options: ConvertOptions,
) -> Result<String, String> {
    let jxl = state.ffmpeg().map(|f| f.jxl_support()).unwrap_or_default();
    let output = catconvert_core::output_path(&info.path, options.format, |_| false);
    catconvert_core::plan(&info, &options, &output, jxl)
        .map(|plan| plan.command_line())
        .map_err(|err| err.to_string())
}

/// The command(s) making one animation from `pictures` would run.
#[tauri::command]
fn preview_sequence_command(
    state: State<'_, AppState>,
    pictures: Vec<MediaInfo>,
    options: ConvertOptions,
    sequence: SequenceOptions,
) -> Result<String, String> {
    let jxl = state.ffmpeg().map(|f| f.jxl_support()).unwrap_or_default();
    let output = catconvert_core::sequence_output_path(&pictures, options.format, |_| false);
    catconvert_core::plan_sequence(&pictures, &options, &sequence, &output, jxl)
        .map(|plan| plan.command_line())
        .map_err(|err| err.to_string())
}

/// Queue one animation made from `paths`, in this order.
#[tauri::command]
async fn start_sequence(
    state: State<'_, AppState>,
    paths: Vec<String>,
    options: ConvertOptions,
    sequence: SequenceOptions,
) -> Result<JobTicket, String> {
    let queue = state.queue()?;
    blocking(move || queue.submit_sequence(&paths, &options, &sequence)).await
}

/// Queue a conversion; progress arrives as `job-progress` events.
#[tauri::command]
async fn start_job(
    state: State<'_, AppState>,
    path: String,
    options: ConvertOptions,
) -> Result<JobTicket, String> {
    let queue = state.queue()?;
    blocking(move || queue.submit(&path, &options)).await
}

/// Cancel a queued or running job. Returns `false` if it already finished.
#[tauri::command]
fn cancel_job(state: State<'_, AppState>, id: JobId) -> bool {
    state.queue().is_ok_and(|queue| queue.cancel(id))
}

/// Restart the app, e.g. after an update was installed.
#[tauri::command]
fn relaunch(app: AppHandle) {
    app.restart();
}

/// The paths among `paths` that no longer point at a file (moved or deleted),
/// so History can say so instead of failing to reveal them.
#[tauri::command]
async fn missing_files(paths: Vec<String>) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        paths
            .into_iter()
            .filter(|path| !std::path::Path::new(path).is_file())
            .collect()
    })
    .await
    .map_err(|err| err.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let backend = start_backend(app.handle());
            app.manage(AppState {
                backend: RwLock::new(backend),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ffmpeg_status,
            relocate_ffmpeg,
            probe_file,
            preview_frame,
            waveform,
            preview_command,
            preview_sequence_command,
            start_job,
            start_sequence,
            cancel_job,
            relaunch,
            missing_files,
            third_party_licenses,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
