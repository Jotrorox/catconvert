//! "Convert with CatConvert" in Finder's Quick Actions menu (macOS only).
//!
//! The Quick Action is an Automator workflow in `~/Library/Services` that runs
//! `open -b <bundle id> <files>`, so the files arrive like "Open with" does,
//! as `RunEvent::Opened`. An app can't add a Quick Action by being installed
//! (that takes an app extension), so Settings offers to add and remove it.
//! The templates live in `quick-action/`.

use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Bundle name in `~/Library/Services`; Finder shows the menu title from `Info.plist`.
const WORKFLOW: &str = "Convert with CatConvert.workflow";
const INFO_PLIST: &str = include_str!("../quick-action/Info.plist");
const DOCUMENT: &str = include_str!("../quick-action/document.wflow");
const BUNDLE_ID_PLACEHOLDER: &str = "__BUNDLE_ID__";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickActionStatus {
    /// Only macOS has Quick Actions.
    pub supported: bool,
    pub installed: bool,
}

/// Where the workflow goes for the user whose home is `home`.
pub fn workflow_path(home: &Path) -> PathBuf {
    home.join("Library").join("Services").join(WORKFLOW)
}

pub fn status(workflow: &Path) -> QuickActionStatus {
    QuickActionStatus {
        supported: cfg!(target_os = "macos"),
        installed: cfg!(target_os = "macos") && workflow.join("Contents/document.wflow").is_file(),
    }
}

/// Write the workflow, opening files with the app whose identifier is `bundle_id`.
pub fn install(workflow: &Path, bundle_id: &str) -> io::Result<()> {
    if !cfg!(target_os = "macos") {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Quick Actions are a macOS feature",
        ));
    }
    write_workflow(workflow, bundle_id)?;
    refresh_services();
    Ok(())
}

pub fn remove(workflow: &Path) -> io::Result<()> {
    delete_workflow(workflow)?;
    refresh_services();
    Ok(())
}

fn write_workflow(workflow: &Path, bundle_id: &str) -> io::Result<()> {
    // The identifier lands in a plist string and a shell command unquoted.
    if bundle_id.is_empty()
        || !bundle_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unexpected bundle identifier {bundle_id:?}"),
        ));
    }
    let contents = workflow.join("Contents");
    std::fs::create_dir_all(&contents)?;
    std::fs::write(contents.join("Info.plist"), INFO_PLIST)?;
    std::fs::write(
        contents.join("document.wflow"),
        DOCUMENT.replace(BUNDLE_ID_PLACEHOLDER, bundle_id),
    )
}

fn delete_workflow(workflow: &Path) -> io::Result<()> {
    match std::fs::remove_dir_all(workflow) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

/// Tell macOS the Services menu changed, so the action shows up (or goes away)
/// without logging out. Best effort: it appears eventually either way.
fn refresh_services() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("/System/Library/CoreServices/pbs")
            .arg("-update")
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_workflow_for_the_bundle_id() {
        let home = tempfile::tempdir().unwrap();
        let workflow = workflow_path(home.path());
        write_workflow(&workflow, "dev.jotrorox.catconvert").unwrap();

        let document = std::fs::read_to_string(workflow.join("Contents/document.wflow")).unwrap();
        assert!(document.contains(r#"open -b dev.jotrorox.catconvert "$@""#));
        assert!(!document.contains(BUNDLE_ID_PLACEHOLDER));
        assert!(workflow.join("Contents/Info.plist").is_file());
        assert_eq!(status(&workflow).installed, cfg!(target_os = "macos"));

        delete_workflow(&workflow).unwrap();
        assert!(!workflow.exists());
        assert!(!status(&workflow).installed);
        // Removing twice is fine.
        delete_workflow(&workflow).unwrap();
    }

    #[test]
    fn refuses_odd_bundle_ids() {
        let home = tempfile::tempdir().unwrap();
        let workflow = workflow_path(home.path());
        assert!(write_workflow(&workflow, "dev.cat; rm -rf ~").is_err());
        assert!(write_workflow(&workflow, "").is_err());
        assert!(!workflow.exists());
    }

    /// Finder ignores a workflow whose plists don't parse.
    #[cfg(target_os = "macos")]
    #[test]
    fn templates_are_valid_plists() {
        let home = tempfile::tempdir().unwrap();
        let workflow = workflow_path(home.path());
        write_workflow(&workflow, "dev.jotrorox.catconvert").unwrap();
        let lint = std::process::Command::new("plutil")
            .arg("-lint")
            .arg(workflow.join("Contents/Info.plist"))
            .arg(workflow.join("Contents/document.wflow"))
            .output()
            .unwrap();
        assert!(
            lint.status.success(),
            "{}",
            String::from_utf8_lossy(&lint.stdout)
        );
    }
}
