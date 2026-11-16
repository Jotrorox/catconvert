//! # catconvert-core
//!
//! Platform-independent core of CatConvert. Every front end (Tauri desktop app,
//! CLI, future Swift/Kotlin shells through `catconvert-ffi`) is a thin layer over
//! this crate, so all logic lives here.
//!
//! Conversion is done by an external ffmpeg/ffprobe found at runtime
//! ([`Ffmpeg::locate`]):
//!
//! 1. [`Ffmpeg::probe`] reads duration, size and codecs of a file.
//! 2. [`plan`] turns [`ConvertOptions`] (format, trim, size, rotation, …) into
//!    an ffmpeg command line.
//! 3. [`Queue`] runs planned conversions one after another and reports
//!    [`JobProgress`] events.

mod convert;
mod error;
mod ffmpeg;
mod job;
mod media;

pub use convert::{ConvertOptions, OutputFormat, Plan, output_path, plan};
pub use error::{Error, Result};
pub use ffmpeg::{FFMPEG_ENV, FFPROBE_ENV, Ffmpeg, RunOutcome, RunProgress};
pub use job::{JobId, JobProgress, JobStage, JobTicket, Queue};
pub use media::{MediaInfo, MediaKind, probe};
