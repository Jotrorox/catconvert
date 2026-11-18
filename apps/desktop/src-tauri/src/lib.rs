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
//! the original's place. The editor can refine that with an [`OutputTarget`]:
//! another folder, a file name or a name pattern. The webview never writes
//! files itself (there is no fs plugin), so neither folder needs a
//! file-system scope: the core writes there, [`prepare_output_folder`]
//! creates and checks the Settings folder, and playback and "Show in folder"
//! work per file as above.
//!
//! The editor's before/after comparison shows samples that
//! [`catconvert_core::Samples`] renders into a private temp folder; that
//! folder joins the asset protocol's scope the first time (see
//! [`compare_sample`]).
//!
//! Files can also come in from outside the window ("Open with", the Finder
//! Quick Action, "Send to"); see [`incoming`] and [`quick_action`].
//!
//! The window starts hidden and opens in the light or dark theme chosen in
//! Settings; see [`theme`].

mod incoming;
mod quick_action;
mod theme;

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use catconvert_core::{
    ConvertOptions, Destination, Error, Ffmpeg, JobId, JobTicket, JxlSupport, MediaInfo, Naming,
    OutputFormat, OutputPreview, OutputTarget, QualitySummary, Queue, Sample, SampleSide, Samples,
    SavedFrame, SequenceOptions, VideoCodec,
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
    compare: Compare,
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
    /// Video codecs ffmpeg can write, for the codec choice of MP4, MOV, MKV
    /// and WebM.
    video_codecs: Vec<VideoCodec>,
    /// H.265 can be encoded in hardware (VideoToolbox on Apple silicon).
    hevc_hardware: bool,
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
        video_codecs: [
            VideoCodec::H264,
            VideoCodec::H265,
            VideoCodec::Vp9,
            VideoCodec::Av1,
        ]
        .into_iter()
        .filter(|&codec| {
            ffmpeg
                .as_ref()
                .is_some_and(|f| f.missing_codec(codec).is_none())
        })
        .collect(),
        hevc_hardware: ffmpeg
            .as_ref()
            .is_some_and(Ffmpeg::encodes_hevc_in_hardware),
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

/// Save the frame at `at` seconds as a PNG, by default next to the video,
/// right away.
#[tauri::command]
async fn save_frame(
    state: State<'_, AppState>,
    path: String,
    options: ConvertOptions,
    at: f64,
    target: Option<OutputTarget>,
) -> Result<SavedFrame, String> {
    let queue = state.queue()?;
    let target = target.unwrap_or_default();
    blocking(move || queue.save_frame(&path, &options, at, &target)).await
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

/// Before/after samples for the editor's comparison, in a temp folder the
/// webview may read once the first sample is made.
#[derive(Default)]
struct Compare {
    samples: Arc<Samples>,
    allowed: OnceLock<()>,
}

impl Compare {
    /// Let the webview load samples through the asset protocol.
    fn allow(&self, app: &AppHandle) -> Result<(), String> {
        if self.allowed.get().is_none() {
            let root = self.samples.root().map_err(|err| err.to_string())?;
            app.asset_protocol_scope()
                .allow_directory(&root, true)
                .map_err(|err| err.to_string())?;
            let _ = self.allowed.set(());
        }
        Ok(())
    }
}

/// One half of the editor's before/after comparison: the original, or what
/// `options` make of it, at the frame `at` seconds in, with the short side
/// at most `limit` pixels (`None`: full size). The picture is loaded from
/// the returned path through the asset protocol.
#[tauri::command]
async fn compare_sample(
    app: AppHandle,
    state: State<'_, AppState>,
    info: MediaInfo,
    options: ConvertOptions,
    side: SampleSide,
    at: f64,
    limit: Option<u32>,
) -> Result<Sample, String> {
    let ffmpeg = state.queue()?.ffmpeg().clone();
    state.compare.allow(&app)?;
    let samples = Arc::clone(&state.compare.samples);
    blocking(move || samples.render(&ffmpeg, &info, &options, side, at, limit)).await
}

/// A finished file's frame at `at` seconds as a picture the webview can
/// show, to compare it with its original. `hdr` tone-maps HDR video.
#[tauri::command]
async fn compare_file(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    at: f64,
    limit: Option<u32>,
    hdr: bool,
) -> Result<Sample, String> {
    let ffmpeg = state.queue()?.ffmpeg().clone();
    state.compare.allow(&app)?;
    let samples = Arc::clone(&state.compare.samples);
    blocking(move || samples.render_file(&ffmpeg, Path::new(&path), at, limit, hdr)).await
}

/// The command(s) a conversion would run, for the "show command" panel.
#[tauri::command]
fn preview_command(
    state: State<'_, AppState>,
    info: MediaInfo,
    options: ConvertOptions,
    target: Option<OutputTarget>,
) -> Result<String, String> {
    preview_plan(&state, &info, &options, &target.unwrap_or_default())
        .map(|plan| plan.command_line())
        .map_err(|err| err.to_string())
}

/// The audio bitrate and video CRF (or exact bitrate) a conversion would
/// use, for the line under the quality slider.
#[tauri::command]
fn quality_summary(
    state: State<'_, AppState>,
    info: MediaInfo,
    options: ConvertOptions,
) -> QualitySummary {
    let support = state
        .ffmpeg()
        .map(|ffmpeg| ffmpeg.support())
        .unwrap_or_default();
    catconvert_core::quality_summary(&info, &options, support)
}

/// The plan a conversion of `info` with `options` would run, for previews.
/// The output name skips names queued jobs will write, as submitting does.
fn preview_plan(
    state: &AppState,
    info: &MediaInfo,
    options: &ConvertOptions,
    target: &OutputTarget,
) -> Result<catconvert_core::Plan, Error> {
    match state.queue() {
        Ok(queue) => queue.preview(info, options, target),
        Err(_) => {
            let naming = Naming::conversion(info, options);
            let (to, _) =
                catconvert_core::resolve_output(&naming, &options.destination, target, |_| false)?;
            catconvert_core::plan(info, options, &to.written, JxlSupport::default())
        }
    }
}

/// Where a result named by `naming` would be saved with `target`, skipping
/// names queued jobs will write.
fn output_of(
    state: &AppState,
    naming: &Naming,
    destination: &Destination,
    target: &OutputTarget,
) -> Result<OutputPreview, String> {
    match state.queue() {
        Ok(queue) => queue.output_preview(naming, destination, target),
        Err(_) => catconvert_core::resolve_output(naming, destination, target, |_| false).map(
            |(to, exists)| OutputPreview {
                path: to.output,
                exists,
            },
        ),
    }
    .map_err(|err| err.to_string())
}

/// Where converting `info` with `options` would save to, and whether the
/// chosen name is taken by a file it could replace. Errors say what is wrong
/// with the chosen name or folder.
#[tauri::command]
fn output_preview(
    state: State<'_, AppState>,
    info: MediaInfo,
    options: ConvertOptions,
    target: OutputTarget,
) -> Result<OutputPreview, String> {
    output_of(
        &state,
        &Naming::conversion(&info, &options),
        &options.destination,
        &target,
    )
}

/// Like [`output_preview`], for the animation made from `pictures`.
#[tauri::command]
fn sequence_output_preview(
    state: State<'_, AppState>,
    pictures: Vec<MediaInfo>,
    options: ConvertOptions,
    target: OutputTarget,
) -> Result<OutputPreview, String> {
    output_of(
        &state,
        &catconvert_core::sequence_naming(&pictures, &options),
        &options.destination,
        &target,
    )
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
    target: Option<OutputTarget>,
) -> Result<String, String> {
    let support = state.ffmpeg().map(|f| f.support()).unwrap_or_default();
    preview_plan(&state, &info, &options, &target.unwrap_or_default())
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
    target: Option<OutputTarget>,
) -> Result<String, String> {
    preview_plan(&state, &info, &options, &target.unwrap_or_default())
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
    target: Option<OutputTarget>,
) -> Result<JobTicket, String> {
    let queue = state.queue()?;
    let target = target.unwrap_or_default();
    blocking(move || queue.submit_custom(&path, &options, &target, &command)).await
}

/// The command(s) making one animation from `pictures` would run.
#[tauri::command]
fn preview_sequence_command(
    state: State<'_, AppState>,
    pictures: Vec<MediaInfo>,
    options: ConvertOptions,
    sequence: SequenceOptions,
    target: Option<OutputTarget>,
) -> Result<String, String> {
    let support = state.ffmpeg().map(|f| f.support()).unwrap_or_default();
    let naming = catconvert_core::sequence_naming(&pictures, &options);
    let output = output_of(
        &state,
        &naming,
        &options.destination,
        &target.unwrap_or_default(),
    )?
    .path;
    catconvert_core::plan_sequence(&pictures, &options, &sequence, &output, support)
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
    target: Option<OutputTarget>,
) -> Result<JobTicket, String> {
    let queue = state.queue()?;
    let target = target.unwrap_or_default();
    blocking(move || queue.submit_sequence(&paths, &options, &sequence, &target)).await
}

/// Queue a conversion; progress arrives as `job-progress` events.
#[tauri::command]
async fn start_job(
    state: State<'_, AppState>,
    path: String,
    options: ConvertOptions,
    target: Option<OutputTarget>,
) -> Result<JobTicket, String> {
    let queue = state.queue()?;
    let target = target.unwrap_or_default();
    blocking(move || queue.submit(&path, &options, &target)).await
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
        .on_window_event(theme::on_window_event)
        .setup(|app| {
            theme::setup(app.handle());
            let backend = start_backend(app.handle());
            app.manage(AppState {
                backend: RwLock::new(backend),
                compare: Compare::default(),
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
            compare_sample,
            compare_file,
            waveform,
            playback_source,
            loudness,
            preview_command,
            quality_summary,
            output_preview,
            sequence_output_preview,
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
            theme::set_window_theme,
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
