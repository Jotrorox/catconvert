//! # catconvert-ffi
//!
//! UniFFI exports over [`catconvert_core`] for future native shells
//! (Swift on iOS/macOS, Kotlin on Android). Types are mirrored here instead of
//! deriving UniFFI traits in the core, so the core stays free of FFI concerns.
//!
//! Bindings are not generated yet. When the shells arrive, generate them with
//! `uniffi-bindgen` (same version as the `uniffi` crate) from the built library.

uniffi::setup_scaffolding!();

/// FFI mirror of [`catconvert_core::MediaKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Unknown,
}

impl From<catconvert_core::MediaKind> for MediaKind {
    fn from(kind: catconvert_core::MediaKind) -> Self {
        match kind {
            catconvert_core::MediaKind::Image => Self::Image,
            catconvert_core::MediaKind::Video => Self::Video,
            catconvert_core::MediaKind::Audio => Self::Audio,
            catconvert_core::MediaKind::Unknown => Self::Unknown,
        }
    }
}

/// FFI mirror of [`catconvert_core::MediaInfo`].
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MediaInfo {
    pub path: String,
    pub file_name: String,
    pub extension: Option<String>,
    pub size_bytes: u64,
    pub kind: MediaKind,
}

impl From<catconvert_core::MediaInfo> for MediaInfo {
    fn from(info: catconvert_core::MediaInfo) -> Self {
        Self {
            path: info.path.to_string_lossy().into_owned(),
            file_name: info.file_name,
            extension: info.extension,
            size_bytes: info.size_bytes,
            kind: info.kind.into(),
        }
    }
}

/// Errors surfaced to Swift/Kotlin as exceptions.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ProbeError {
    #[error("{message}")]
    Io { message: String },
    #[error("{message}")]
    NotAFile { message: String },
    #[error("{message}")]
    Other { message: String },
}

impl From<catconvert_core::Error> for ProbeError {
    fn from(err: catconvert_core::Error) -> Self {
        let message = err.to_string();
        match err {
            catconvert_core::Error::Io { .. } => Self::Io { message },
            catconvert_core::Error::NotAFile { .. } => Self::NotAFile { message },
            _ => Self::Other { message },
        }
    }
}

/// Inspect a file. See [`catconvert_core::probe`].
///
/// # Errors
///
/// Returns [`ProbeError`] if the file cannot be read or is not a regular file.
#[uniffi::export]
pub fn probe(path: String) -> Result<MediaInfo, ProbeError> {
    Ok(catconvert_core::probe(path)?.into())
}

/// Version of the native library, handy for diagnostics in the shells.
#[uniffi::export]
#[must_use]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_maps_core_result() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("clip.mkv");
        std::fs::write(&path, [0u8; 10]).expect("write file");

        let info = probe(path.to_string_lossy().into_owned()).expect("probe succeeds");
        assert_eq!(info.file_name, "clip.mkv");
        assert_eq!(info.size_bytes, 10);
        assert_eq!(info.kind, MediaKind::Video);
    }

    #[test]
    fn probe_maps_errors() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let err = probe(dir.path().to_string_lossy().into_owned()).expect_err("must fail");
        assert!(matches!(err, ProbeError::NotAFile { .. }), "got {err:?}");
    }
}
