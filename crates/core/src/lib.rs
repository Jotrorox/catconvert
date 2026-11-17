//! # catconvert-core
//!
//! Platform-independent core of CatConvert. Every front end (Tauri desktop app,
//! CLI, future Swift/Kotlin shells through `catconvert-ffi`) is a thin layer over
//! this crate, so all logic lives here.
//!
//! Conversion is done by an external ffmpeg/ffprobe found at runtime
//! ([`Ffmpeg::locate`], or [`Ffmpeg::locate_with`] for the copies an app
//! bundles), helped by libjxl's `cjxl`/`djxl`/`jxlinfo` for JPEG XL when they
//! are available:
//!
//! 1. [`Ffmpeg::probe`] reads duration, size and codecs of a file.
//! 2. [`plan`] turns [`ConvertOptions`] (format, trim, size, rotation, …) into
//!    one or more command lines ([`Step`]s).
//! 3. [`Queue`] runs planned conversions one after another and reports
//!    [`JobProgress`] events.

mod convert;
mod error;
mod ffmpeg;
mod job;
mod jxl;
mod media;
mod scrub;

pub use convert::{
    ConvertOptions, JxlOptions, JxlSupport, Metadata, OutputFormat, Plan, Step, Tool, output_path,
    plan,
};
pub use error::{Error, Result};
pub use ffmpeg::{BUNDLED_PREFIX, FFMPEG_ENV, FFPROBE_ENV, Ffmpeg, RunOutcome, RunProgress};
pub use job::{JobId, JobProgress, JobStage, JobTicket, Queue};
pub use jxl::{JxlTools, LIBJXL_ENV, has_jpeg_reconstruction, is_jxl_file};
pub use media::{MediaInfo, MediaKind, probe};
pub use scrub::scrub_file;
