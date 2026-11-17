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
//!
//! The editor, Queue and History play sound straight from disk through the
//! asset protocol. Its scope starts empty: [`playback_source`] adds each file
//! as it is played, so the webview can't read anything else.
//!
//! Results go where the conversion's [`catconvert_core::Destination`] says:
//! next to the original, into the output folder chosen in Settings, or in
//! the original's place. The webview never writes files itself (there is no
//! fs plugin), so the output folder needs no file-system scope: the core
//! writes there, [`prepare_output_folder`] creates and checks it, and
//! playback and "Show in folder" work per file as above.
//!
//! Files can also come in from outside the window ("Open with", the Finder
//! Quick Action, "Send to"); see [`incoming`] and [`quick_action`].

mod incoming;
mod quick_action;

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use catconvert_core::{
    ConvertOptions, Error, Ffmpeg, JobId, JobTicket, JxlSupport, MediaInfo, OutputFormat,
    QualitySummary, Queue, SavedFrame, SequenceOptions, VideoCodec,
};

/// Folder in Downloads suggested when "Save to a folder" is first chosen.
const OUTPUT_FOLDER_NAME: &str = "CatConvert";
use serde::Serialize;
use tauri::ipc::Response;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::incoming::Incoming;
use crate::quick_action::QuickActionStatus;

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
    /// Video codecs ffmpeg can write, for MP4, MKV and WebM's codec choice.
    video_codecs: Vec<VideoCodec>,
    /// ffmpeg decodes AV1 video and AVIF pictures (it has dav1d or libaom).
    reads_av1: bool,
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
        video_codecs: [VideoCodec::H264, VideoCodec::Vp9, VideoCodec::Av1]
            .into_iter()
            .filter(|&codec| {
                ffmpeg
                    .as_ref()
                    .is_some_and(|f| f.missing_codec(codec).is_none())
            })
            .collect(),
        reads_av1: ffmpeg.as_ref().is_some_and(Ffmpeg::reads_av1),
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

/// One frame as JPEG bytes (an `ArrayBuffer` on the JS side), tone-mapped
/// when `hdr` is set.
#[tauri::command]
async fn preview_frame(
    state: State<'_, AppState>,
    path: String,
    at: f64,
    height: u32,
    hdr: bool,
) -> Result<Response, String> {
    let ffmpeg = state.queue()?.ffmpeg().clone();
    let bytes =
        blocking(move || ffmpeg.frame(path.as_ref(), at, height.clamp(16, 1080), hdr)).await?;
    Ok(Response::new(bytes))
}

/// Save the frame at `at` seconds as a PNG next to the video, right away.
#[tauri::command]
async fn save_frame(
    state: State<'_, AppState>,
    path: String,
    options: ConvertOptions,
    at: f64,
) -> Result<SavedFrame, String> {
    let queue = state.queue()?;
    blocking(move || queue.save_frame(&path, &options, at)).await
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

/// Let the webview play `path` through the asset protocol and return the file
/// to load: `path` itself, or with `decode` a WAV copy of its sound for
/// formats the webview can't play (see [`Ffmpeg::listenable`]).
#[tauri::command]
async fn playback_source(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    decode: bool,
) -> Result<PathBuf, String> {
    let source = if decode {
        let ffmpeg = state.queue()?.ffmpeg().clone();
        blocking(move || ffmpeg.listenable(path.as_ref())).await?
    } else {
        let path = PathBuf::from(path);
        if !path.is_file() {
            return Err(format!("{} is no longer there", path.display()));
        }
        path
    };
    app.asset_protocol_scope()
        .allow_file(&source)
        .map_err(|err| err.to_string())?;
    Ok(source)
}

/// Integrated loudness of the sound in LUFS (`None` if silent), so the
/// editor can preview loudness normalisation.
#[tauri::command]
async fn loudness(state: State<'_, AppState>, path: String) -> Result<Option<f64>, String> {
    let ffmpeg = state.queue()?.ffmpeg().clone();
    blocking(move || ffmpeg.loudness(path.as_ref())).await
}

/// The command(s) a conversion would run, for the "show command" panel.
#[tauri::command]
fn preview_command(
    state: State<'_, AppState>,
    info: MediaInfo,
    options: ConvertOptions,
) -> Result<String, String> {
    preview_plan(&state, &info, &options)
        .map(|plan| plan.command_line())
        .map_err(|err| err.to_string())
}

/// The audio bitrate and video CRF (or exact bitrate) a conversion would
/// use, for the line under the quality slider.
#[tauri::command]
fn quality_summary(info: MediaInfo, options: ConvertOptions) -> QualitySummary {
    catconvert_core::quality_summary(&info, &options)
}

/// The plan a conversion of `info` with `options` would run, for previews.
/// The output name skips names queued jobs will write, as submitting does.
fn preview_plan(
    state: &AppState,
    info: &MediaInfo,
    options: &ConvertOptions,
) -> Result<catconvert_core::Plan, Error> {
    match state.queue() {
        Ok(queue) => queue.preview(info, options),
        Err(_) => {
            let target = catconvert_core::output_target(
                &info.path,
                options.format,
                &options.destination,
                |_| false,
            )?;
            catconvert_core::plan(info, options, &target.written, JxlSupport::default())
        }
    }
}

/// Check a command edited in the editor without running it. Returns the
/// command as it would run (with a recipe's placeholders filled in), or why
/// it would be refused.
#[tauri::command]
fn check_command(
    state: State<'_, AppState>,
    info: MediaInfo,
    options: ConvertOptions,
    command: String,
) -> Result<String, String> {
    let support = state.ffmpeg().map(|f| f.support()).unwrap_or_default();
    preview_plan(&state, &info, &options)
        .and_then(|plan| catconvert_core::custom_plan(&info, &plan, &command, support))
        .map(|plan| plan.command_line())
        .map_err(|err| err.to_string())
}

/// A recipe's saved command with this file's paths filled in, for the editor.
#[tauri::command]
fn fill_command(
    state: State<'_, AppState>,
    info: MediaInfo,
    options: ConvertOptions,
    template: String,
) -> Result<String, String> {
    preview_plan(&state, &info, &options)
        .and_then(|plan| catconvert_core::fill_command(&template, &info.path, &plan.output))
        .map_err(|err| err.to_string())
}

/// An edited command with the file's paths swapped for placeholders, to save
/// in a recipe or run for other files.
#[tauri::command]
fn command_template(path: String, command: String) -> Result<String, String> {
    catconvert_core::command_template(Path::new(&path), &command).map_err(|err| err.to_string())
}

/// Queue a conversion running `command`, edited by hand, instead of the
/// generated one. Refused commands come back as an error, unqueued.
#[tauri::command]
async fn start_custom_job(
    state: State<'_, AppState>,
    path: String,
    options: ConvertOptions,
    command: String,
) -> Result<JobTicket, String> {
    let queue = state.queue()?;
    blocking(move || queue.submit_custom(&path, &options, &command)).await
}

/// The command(s) making one animation from `pictures` would run.
#[tauri::command]
fn preview_sequence_command(
    state: State<'_, AppState>,
    pictures: Vec<MediaInfo>,
    options: ConvertOptions,
    sequence: SequenceOptions,
) -> Result<String, String> {
    let support = state.ffmpeg().map(|f| f.support()).unwrap_or_default();
    catconvert_core::sequence_target(&pictures, options.format, &options.destination, |_| false)
        .and_then(|target| {
            catconvert_core::plan_sequence(&pictures, &options, &sequence, &target.written, support)
        })
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

/// Where Settings suggests saving converted files: `CatConvert` in the
/// Downloads folder (or the home folder, if there is no Downloads).
#[tauri::command]
fn default_output_folder(app: AppHandle) -> Result<PathBuf, String> {
    let path = app.path();
    path.download_dir()
        .or_else(|_| path.home_dir())
        .map(|dir| dir.join(OUTPUT_FOLDER_NAME))
        .map_err(|err| err.to_string())
}

/// Create the output folder chosen in Settings if need be and check that
/// files can be written there. Returns the folder.
#[tauri::command]
async fn prepare_output_folder(path: String) -> Result<PathBuf, String> {
    tauri::async_runtime::spawn_blocking(move || check_output_folder(Path::new(&path)))
        .await
        .map_err(|err| err.to_string())?
}

fn check_output_folder(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{} isn't a full path", path.display()));
    }
    std::fs::create_dir_all(path)
        .map_err(|err| format!("couldn't create {}: {err}", path.display()))?;
    let probe = path.join(".catconvert-write-test");
    std::fs::write(&probe, b"")
        .map_err(|err| format!("can't save files in {}: {err}", path.display()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(path.to_path_buf())
}

/// Files opened from outside the app before the front end was listening.
/// Later ones arrive as `open-files` events.
#[tauri::command]
fn take_opened_files(incoming: State<'_, Incoming>) -> Vec<PathBuf> {
    incoming.take()
}

fn quick_action_path(app: &AppHandle) -> Result<PathBuf, String> {
    let home = app.path().home_dir().map_err(|err| err.to_string())?;
    Ok(quick_action::workflow_path(&home))
}

/// Whether "Convert with CatConvert" is in Finder's Quick Actions menu.
#[tauri::command]
fn quick_action_status(app: AppHandle) -> Result<QuickActionStatus, String> {
    Ok(quick_action::status(&quick_action_path(&app)?))
}

/// Add or remove the Finder Quick Action.
#[tauri::command]
async fn set_quick_action(app: AppHandle, installed: bool) -> Result<QuickActionStatus, String> {
    let workflow = quick_action_path(&app)?;
    let bundle_id = app.config().identifier.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if installed {
            quick_action::install(&workflow, &bundle_id)?;
        } else {
            quick_action::remove(&workflow)?;
        }
        Ok::<_, std::io::Error>(quick_action::status(&workflow))
    })
    .await
    .map_err(|err| err.to_string())?
    .map_err(|err| err.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // First, so a second launch hands its files to the running app and exits
    // before anything else starts.
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            incoming::show_main_window(app);
            let files = incoming::files_in_args(args, Path::new(&cwd));
            app.state::<Incoming>().receive(app, files);
        }))
        .plugin(tauri_plugin_drag::init());

    builder
        .manage(Incoming::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let backend = start_backend(app.handle());
            app.manage(AppState {
                backend: RwLock::new(backend),
            });
            // Windows' "Open with" and "Send to" and Linux file managers pass
            // the files as arguments.
            if let Ok(cwd) = std::env::current_dir() {
                let files = incoming::files_in_args(std::env::args_os(), &cwd);
                app.state::<Incoming>().receive(app.handle(), files);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ffmpeg_status,
            relocate_ffmpeg,
            probe_file,
            preview_frame,
            waveform,
            playback_source,
            loudness,
            preview_command,
            quality_summary,
            check_command,
            fill_command,
            command_template,
            start_custom_job,
            save_frame,
            preview_sequence_command,
            start_job,
            start_sequence,
            cancel_job,
            relaunch,
            missing_files,
            default_output_folder,
            prepare_output_folder,
            third_party_licenses,
            take_opened_files,
            quick_action_status,
            set_quick_action,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // "Open with", a drop on the Dock icon, or the Quick Action.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = event {
                incoming::show_main_window(app);
                app.state::<Incoming>()
                    .receive(app, incoming::files_in_urls(&urls));
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_folders_are_created_and_checked() {
        let dir = tempfile::tempdir().expect("temp dir");
        let folder = dir.path().join("Converted").join("Nested");
        assert_eq!(check_output_folder(&folder), Ok(folder.clone()));
        assert!(folder.is_dir());
        assert!(!folder.join(".catconvert-write-test").exists());

        assert!(check_output_folder(Path::new("relative")).is_err());
        let file = dir.path().join("a file");
        std::fs::write(&file, b"x").expect("write");
        assert!(check_output_folder(&file).is_err());
    }
}
