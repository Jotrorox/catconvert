//! Tauri shell for CatConvert.
//!
//! Commands here are thin wrappers over `catconvert-core`: they convert
//! arguments, call the core, and forward results or events to the webview.
//! No conversion logic belongs in this crate.

use std::sync::mpsc;

use catconvert_core::{Job, JobId, MediaInfo};
use tauri::{AppHandle, Emitter};

/// Event name for [`catconvert_core::JobProgress`] payloads.
const JOB_PROGRESS_EVENT: &str = "job-progress";

/// Probe a file and return its media info.
#[tauri::command]
fn probe_file(path: String) -> Result<MediaInfo, String> {
    catconvert_core::probe(path).map_err(|err| err.to_string())
}

/// Start a (stubbed) conversion job and stream progress as `job-progress` events.
#[tauri::command]
fn start_job(app: AppHandle, path: String, target_format: String) -> JobId {
    let (tx, rx) = mpsc::channel();
    let handle = catconvert_core::submit_job(Job::new(path, target_format), tx);
    let id = handle.id;

    // Forward core events to the webview until the job's sender is dropped.
    std::thread::spawn(move || {
        for event in rx {
            if let Err(err) = app.emit(JOB_PROGRESS_EVENT, &event) {
                eprintln!("failed to emit {JOB_PROGRESS_EVENT}: {err}");
                break;
            }
        }
        handle.join();
    });

    id
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![probe_file, start_job])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
