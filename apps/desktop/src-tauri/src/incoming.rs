//! Files handed to the app from outside: "Open with", the Finder Quick Action,
//! Windows' "Send to", or paths on the command line.
//!
//! macOS delivers them as `RunEvent::Opened`. Windows and Linux start the
//! program with the paths as arguments; when CatConvert is already running,
//! the single-instance plugin passes that second launch's arguments to the
//! first one, and the second process exits.
//!
//! Files can arrive before the webview listens (they usually do at launch),
//! so they wait here until the front end calls `take_opened_files`. From then
//! on they go out right away as `open-files` events.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};

/// Event name for a list of absolute file paths to open.
pub const OPEN_FILES_EVENT: &str = "open-files";

#[derive(Debug, Default)]
pub struct Incoming(Mutex<Inbox>);

#[derive(Debug, Default)]
struct Inbox {
    /// The front end has taken the backlog and listens for events.
    listening: bool,
    waiting: Vec<PathBuf>,
}

impl Incoming {
    /// Hand files to the front end now, or once it listens.
    pub fn receive(&self, app: &AppHandle, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        // Emitting under the lock keeps `take` from slipping in between.
        let mut inbox = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if inbox.listening {
            if let Err(err) = app.emit(OPEN_FILES_EVENT, &paths) {
                eprintln!("failed to emit {OPEN_FILES_EVENT}: {err}");
            }
        } else {
            inbox.waiting.extend(paths);
        }
    }

    /// Files that arrived before the front end listened. Later files are
    /// emitted as events, so call this after subscribing.
    pub fn take(&self) -> Vec<PathBuf> {
        let mut inbox = self.0.lock().unwrap_or_else(|e| e.into_inner());
        inbox.listening = true;
        std::mem::take(&mut inbox.waiting)
    }
}

/// The files named in a command line, whose first argument is the program.
/// Relative paths are resolved against `cwd`, the directory the command ran
/// in. Flags and anything that isn't an existing file are skipped.
pub fn files_in_args<I>(args: I, cwd: &Path) -> Vec<PathBuf>
where
    I: IntoIterator,
    I::Item: Into<OsString>,
{
    args.into_iter()
        .skip(1)
        .map(Into::into)
        .filter(|arg| !arg.to_string_lossy().starts_with('-'))
        .map(|arg| cwd.join(arg))
        .filter(|path| path.is_file())
        .collect()
}

/// The local files among opened URLs (macOS hands them over as `file://` URLs).
#[cfg(target_os = "macos")]
pub fn files_in_urls(urls: &[tauri::Url]) -> Vec<PathBuf> {
    urls.iter()
        .filter_map(|url| url.to_file_path().ok())
        .filter(|path| path.is_file())
        .collect()
}

/// Bring the window forward, e.g. when another launch handed us files.
pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_existing_files_and_resolves_relative_paths() {
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("clip.mov");
        let photo = dir.path().join("cat photo.jpg");
        std::fs::write(&clip, b"").unwrap();
        std::fs::write(&photo, b"").unwrap();

        let args = [
            OsString::from("catconvert"),
            clip.clone().into_os_string(),
            OsString::from("cat photo.jpg"),
        ];
        assert_eq!(files_in_args(args, dir.path()), vec![clip, photo]);
    }

    #[test]
    fn skips_the_program_flags_folders_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("catconvert");
        std::fs::write(&program, b"").unwrap();

        let args = [
            program.into_os_string(),
            OsString::from("-psn_0_12345"),
            OsString::from("--verbose"),
            dir.path().as_os_str().to_owned(),
            OsString::from("gone.mp4"),
        ];
        assert!(files_in_args(args, dir.path()).is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn opened_urls_become_paths() {
        let dir = tempfile::tempdir().unwrap();
        let song = dir.path().join("song one.flac");
        std::fs::write(&song, b"").unwrap();

        let urls = [
            tauri::Url::from_file_path(&song).unwrap(),
            tauri::Url::parse("https://example.com/song.flac").unwrap(),
            tauri::Url::from_file_path(dir.path().join("gone.flac")).unwrap(),
        ];
        assert_eq!(files_in_urls(&urls), vec![song]);
    }
}
