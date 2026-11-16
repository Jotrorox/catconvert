//! Tauri shell for CatConvert.
//!
//! Commands here are thin wrappers over `catconvert-core`: they convert
//! arguments, call the core, and forward results or events to the webview.
//! No conversion logic belongs in this crate.
//!
//! Anything that starts ffmpeg runs on the blocking thread pool, never on the
//! main thread, so the window stays responsive.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use catconvert_core::{
    ConvertOptions, Error, Ffmpeg, JobId, JobTicket, MediaInfo, OutputFormat, Queue,
};
use serde::Serialize;
use tauri::ipc::Response;
use tauri::{AppHandle, Emitter, Manager, State};

/// Event name for [`catconvert_core::JobProgress`] payloads.
const JOB_PROGRESS_EVENT: &str = "job-progress";

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

fn start_backend(app: &AppHandle) -> Backend {
    match Ffmpeg::locate() {
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
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FfmpegStatus {
    found: bool,
    version: Option<String>,
    path: Option<PathBuf>,
    error: Option<String>,
    formats: Vec<FormatSupport>,
}

fn status_of(state: &AppState) -> FfmpegStatus {
    let ffmpeg = state.ffmpeg();
    let formats = OutputFormat::ALL
        .iter()
        .map(|&format| FormatSupport {
            format,
            available: ffmpeg.as_ref().is_some_and(|f| f.supports(format)),
        })
        .collect();
    FfmpegStatus {
        found: ffmpeg.is_some(),
        version: ffmpeg.as_ref().map(|f| f.version().to_owned()),
        path: ffmpeg.as_ref().map(|f| f.ffmpeg_path().to_path_buf()),
        error: state.queue().err(),
        formats,
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

/// The ffmpeg command a conversion would run, for the "show command" panel.
#[tauri::command]
fn preview_command(info: MediaInfo, options: ConvertOptions) -> Result<String, String> {
    let output = catconvert_core::output_path(&info.path, options.format, |_| false);
    catconvert_core::plan(&info, &options, &output)
        .map(|plan| plan.command_line())
        .map_err(|err| err.to_string())
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
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
            start_job,
            cancel_job,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
