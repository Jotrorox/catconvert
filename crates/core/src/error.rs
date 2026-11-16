use std::path::PathBuf;

/// Errors returned by the core API.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{path} is not a regular file")]
    NotAFile { path: PathBuf },
}

/// Convenience alias used throughout the core API.
pub type Result<T, E = Error> = std::result::Result<T, E>;
