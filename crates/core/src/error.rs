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

    #[error("ffmpeg was not found; install it or set CATCONVERT_FFMPEG to its path")]
    FfmpegNotFound,

    #[error("could not start {program}: {source}")]
    Spawn {
        program: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// ffmpeg or ffprobe ran but failed. `message` is the most useful line of its output.
    #[error("{message}")]
    Ffmpeg { message: String },

    /// The requested conversion does not make sense for this input or this ffmpeg build.
    #[error("{message}")]
    Unsupported { message: String },
}

impl Error {
    pub(crate) fn unsupported(message: impl Into<String>) -> Self {
        Self::Unsupported {
            message: message.into(),
        }
    }
}

/// Convenience alias used throughout the core API.
pub type Result<T, E = Error> = std::result::Result<T, E>;
