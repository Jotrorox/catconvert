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
//! 2. [`plan`] turns [`ConvertOptions`] (format, trim, size, rotation, crop, …) into
//!    one or more command lines ([`Step`]s). [`quality_summary`] tells what its
//!    quality setting means in bitrates and CRF.
//! 3. [`Queue`] runs planned conversions one after another and reports
//!    [`JobProgress`] events.
//!
//! The planned commands can also be edited by hand: [`custom_plan`] checks
//! the edited text and turns it into a [`Plan`] ([`Queue::submit_custom`]).
//!
//! Several pictures can also become one animation: [`plan_sequence`] plans
//! it and [`Queue::submit_sequence`] runs it.
//!
//! Results go next to the original, into a folder, or in the original's
//! place ([`Destination`], chosen in [`ConvertOptions::destination`]). An
//! [`OutputTarget`] can pick another folder, name or name pattern for one
//! conversion (see [`Naming`] and [`resolve_output`]).

mod convert;
mod custom;
mod destination;
mod error;
mod ffmpeg;
mod job;
mod jxl;
mod media;
mod output;
mod scrub;
mod sequence;

pub use convert::{
    AudioRate, ConvertOptions, Crop, CropArea, Direction, HARDWARE_QUALITY_RANGE,
    HEVC_HARDWARE_ENCODER, JxlOptions, JxlSupport, MAX_SIDE, MAX_UPSCALE_FACTOR, MAX_VIDEO_SIDE,
    Metadata, OutputFormat, Plan, QualitySummary, Scaler, SoundOptions, Step, Support, Tool,
    Upscale, VIDEO_KBPS, VideoCodec, VideoRate, crf_range, frame_output_path, frame_plan,
    output_path, plan, quality_summary, upscale_warning,
};
pub use custom::{
    INPUT_PLACEHOLDER, OUTPUT_PLACEHOLDER, command_template, custom_plan, fill_command,
};
pub use destination::{
    Destination, Replaced, Target, frame_target, output_target, resolve_output, sequence_target,
};
pub use error::{Error, Result};
pub use ffmpeg::{BUNDLED_PREFIX, FFMPEG_ENV, FFPROBE_ENV, Ffmpeg, RunOutcome, RunProgress};
pub use job::{JobId, JobProgress, JobStage, JobTicket, Queue, SavedFrame};
pub use jxl::{JxlTools, LIBJXL_ENV, has_jpeg_reconstruction, is_jxl_file};
pub use media::{MediaInfo, MediaKind, jpeg_orientation, probe};
pub use output::{Collision, Naming, OutputPreview, OutputTarget, PLACEHOLDERS, check_stem};
pub use scrub::scrub_file;
pub use sequence::{
    Fit, MAX_PICTURES, SequenceOptions, plan_sequence, sequence_naming, sequence_output_path,
};
