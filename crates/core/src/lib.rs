//! # catconvert-core
//!
//! Platform-independent core of CatConvert. Every front end (Tauri desktop app,
//! CLI, future Swift/Kotlin shells through `catconvert-ffi`) is a thin layer over
//! this crate, so all logic lives here.
//!
//! This is scaffolding: [`probe`] only looks at file size and extension, and
//! [`submit_job`] emits fake progress. Real decoding/encoding comes later.

mod error;
mod job;
mod media;

pub use error::{Error, Result};
pub use job::{
    Job, JobHandle, JobId, JobProgress, JobStage, StubTiming, submit_job, submit_job_with,
};
pub use media::{MediaInfo, MediaKind, probe};
