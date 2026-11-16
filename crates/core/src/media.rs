use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Broad media category, guessed from the file extension for now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Unknown,
}

impl MediaKind {
    /// Guess the kind from a file extension (case-insensitive, without the dot).
    #[must_use]
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "heic" | "heif" | "tif" | "tiff"
            | "bmp" | "jxl" => Self::Image,
            "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "wmv" | "mpg" | "mpeg" => Self::Video,
            "mp3" | "m4a" | "aac" | "flac" | "wav" | "ogg" | "opus" | "aiff" | "wma" => Self::Audio,
            _ => Self::Unknown,
        }
    }
}

/// Basic information about a media file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    /// Path as given by the caller.
    pub path: PathBuf,
    /// Final path component, lossily converted to UTF-8.
    pub file_name: String,
    /// Lower-cased extension without the leading dot, if any.
    pub extension: Option<String>,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Category guessed from the extension.
    pub kind: MediaKind,
}

/// Inspect a file and return its [`MediaInfo`].
///
/// Placeholder implementation: reads only filesystem metadata (size) and the
/// extension. It does not open or decode the file.
///
/// # Errors
///
/// Returns [`Error::Io`] if the metadata cannot be read and
/// [`Error::NotAFile`] if the path is a directory or other non-file.
pub fn probe(path: impl AsRef<Path>) -> Result<MediaInfo> {
    let path = path.as_ref();
    let metadata = std::fs::metadata(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(Error::NotAFile {
            path: path.to_path_buf(),
        });
    }

    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase());
    let kind = extension
        .as_deref()
        .map_or(MediaKind::Unknown, MediaKind::from_extension);
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();

    Ok(MediaInfo {
        path: path.to_path_buf(),
        file_name,
        extension,
        size_bytes: metadata.len(),
        kind,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn temp_file(name: &str, contents: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join(name);
        let mut file = std::fs::File::create(&path).expect("create temp file");
        file.write_all(contents).expect("write temp file");
        (dir, path)
    }

    #[test]
    fn probe_reads_size_and_extension() {
        let (_dir, path) = temp_file("Holiday.JPG", &[0u8; 1234]);
        let info = probe(&path).expect("probe succeeds");

        assert_eq!(info.path, path);
        assert_eq!(info.file_name, "Holiday.JPG");
        assert_eq!(info.extension.as_deref(), Some("jpg"));
        assert_eq!(info.size_bytes, 1234);
        assert_eq!(info.kind, MediaKind::Image);
    }

    #[test]
    fn probe_handles_missing_extension() {
        let (_dir, path) = temp_file("README", b"hello");
        let info = probe(&path).expect("probe succeeds");

        assert_eq!(info.extension, None);
        assert_eq!(info.kind, MediaKind::Unknown);
        assert_eq!(info.size_bytes, 5);
    }

    #[test]
    fn probe_missing_file_is_io_error() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let err = probe(dir.path().join("nope.mp4")).expect_err("must fail");
        assert!(matches!(err, Error::Io { .. }), "got {err:?}");
    }

    #[test]
    fn probe_directory_is_rejected() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let err = probe(dir.path()).expect_err("must fail");
        assert!(matches!(err, Error::NotAFile { .. }), "got {err:?}");
    }

    #[test]
    fn media_kind_from_extension() {
        assert_eq!(MediaKind::from_extension("MOV"), MediaKind::Video);
        assert_eq!(MediaKind::from_extension("flac"), MediaKind::Audio);
        assert_eq!(MediaKind::from_extension("webp"), MediaKind::Image);
        assert_eq!(MediaKind::from_extension("txt"), MediaKind::Unknown);
    }

    #[test]
    fn media_info_serializes_camel_case() {
        let (_dir, path) = temp_file("song.mp3", b"abc");
        let json = serde_json::to_value(probe(&path).expect("probe succeeds")).expect("serialize");

        assert_eq!(json["fileName"], "song.mp3");
        assert_eq!(json["sizeBytes"], 3);
        assert_eq!(json["kind"], "audio");
    }
}
