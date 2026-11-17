//! Turning edit options into command lines.
//!
//! [`plan`] is a pure function: it validates [`ConvertOptions`] against a
//! probed [`MediaInfo`] and produces the steps to run, so the exact commands
//! can be shown to the user before anything runs. Most conversions are one
//! ffmpeg call; JPEG XL may add libjxl's `cjxl`/`djxl` (see [`crate::jxl`]).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ffmpeg::format_secs;
use crate::jxl::is_jxl;
use crate::{Error, MediaInfo, MediaKind, Result};

/// A file type CatConvert can write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Mp4,
    Webm,
    Mov,
    Mkv,
    Gif,
    Mp3,
    M4a,
    Wav,
    Flac,
    Ogg,
    Png,
    Jpg,
    Webp,
    Avif,
    Jxl,
}

impl OutputFormat {
    /// Every format, in the order the UI lists them.
    pub const ALL: [Self; 15] = [
        Self::Mp4,
        Self::Webm,
        Self::Mov,
        Self::Mkv,
        Self::Gif,
        Self::Mp3,
        Self::M4a,
        Self::Wav,
        Self::Flac,
        Self::Ogg,
        Self::Png,
        Self::Jpg,
        Self::Webp,
        Self::Avif,
        Self::Jxl,
    ];

    /// File extension without the dot.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Webm => "webm",
            Self::Mov => "mov",
            Self::Mkv => "mkv",
            Self::Gif => "gif",
            Self::Mp3 => "mp3",
            Self::M4a => "m4a",
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Ogg => "ogg",
            Self::Png => "png",
            Self::Jpg => "jpg",
            Self::Webp => "webp",
            Self::Avif => "avif",
            Self::Jxl => "jxl",
        }
    }

    /// What kind of file this produces. GIFs count as (silent) video. WebP
    /// and JPEG XL count as images, but become animations when made from a
    /// video (see [`OutputFormat::animates`]).
    #[must_use]
    pub fn kind(self) -> MediaKind {
        match self {
            Self::Mp4 | Self::Webm | Self::Mov | Self::Mkv | Self::Gif => MediaKind::Video,
            Self::Mp3 | Self::M4a | Self::Wav | Self::Flac | Self::Ogg => MediaKind::Audio,
            Self::Png | Self::Jpg | Self::Webp | Self::Avif | Self::Jxl => MediaKind::Image,
        }
    }

    /// The ffmpeg encoder this format needs. JPEG XL can also be written by
    /// libjxl's `cjxl` instead (see [`crate::Ffmpeg::supports`]). For video
    /// files this is the encoder of their usual codec; see
    /// [`OutputFormat::video_codecs`] for the others.
    #[must_use]
    pub fn encoder(self) -> &'static str {
        match self {
            Self::Mp4 | Self::Mov | Self::Mkv => VideoCodec::H264.encoder(),
            Self::Webm => VideoCodec::Vp9.encoder(),
            Self::Gif => "gif",
            Self::Mp3 => "libmp3lame",
            Self::M4a => "aac",
            Self::Wav => "pcm_s16le",
            Self::Flac => "flac",
            Self::Ogg => "libopus",
            Self::Png => "png",
            Self::Jpg => "mjpeg",
            Self::Webp => "libwebp",
            Self::Avif => "libsvtav1",
            Self::Jxl => "libjxl",
        }
    }

    /// Whether an input of `kind` can become this format: video makes video,
    /// sound or a still frame; audio makes audio; images make images.
    #[must_use]
    pub fn accepts(self, kind: MediaKind) -> bool {
        match self.kind() {
            MediaKind::Video => kind == MediaKind::Video,
            MediaKind::Audio => matches!(kind, MediaKind::Video | MediaKind::Audio),
            MediaKind::Image => matches!(kind, MediaKind::Video | MediaKind::Image),
            MediaKind::Unknown => false,
        }
    }

    /// Whether writing this format from an input of `kind` makes an animation
    /// rather than a still: GIFs always, WebP and JPEG XL when the input moves.
    #[must_use]
    pub fn animates(self, kind: MediaKind) -> bool {
        match self {
            Self::Gif => true,
            Self::Webp | Self::Jxl => kind == MediaKind::Video,
            _ => false,
        }
    }

    /// The video codecs this format can carry, its usual one first. Empty for
    /// formats without a choice: GIFs, animated images, stills and sound.
    /// MOV takes H.264 or H.265, which is what the Apple tools it is meant
    /// for play; WebM only the royalty-free VP9 and AV1.
    #[must_use]
    pub fn video_codecs(self) -> &'static [VideoCodec] {
        match self {
            Self::Mp4 | Self::Mkv => &[VideoCodec::H264, VideoCodec::H265, VideoCodec::Av1],
            Self::Mov => &[VideoCodec::H264, VideoCodec::H265],
            Self::Webm => &[VideoCodec::Vp9, VideoCodec::Av1],
            _ => &[],
        }
    }

    /// The ffmpeg encoder that writes this format as an animation, where it
    /// differs from [`OutputFormat::encoder`].
    #[must_use]
    pub fn animation_encoder(self) -> &'static str {
        match self {
            Self::Webp => "libwebp_anim",
            Self::Jxl => "libjxl_anim",
            _ => self.encoder(),
        }
    }
}

/// How the picture of a video file is compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    /// H.264 (AVC) with x264: plays everywhere.
    H264,
    /// H.265 (HEVC) with x265: 30–50% smaller than H.264 at the same look,
    /// with hardware decoding on nearly every recent device. HDR sources are
    /// written in 10 bits (Main 10) and stay HDR. `"hevc"` is accepted too.
    #[serde(alias = "hevc")]
    H265,
    /// VP9 with libvpx: WebM's usual codec.
    Vp9,
    /// AV1 with SVT-AV1, in 10 bits: about a third smaller than H.264 at the
    /// same look, keeps HDR, but is slower to make and needs a newer player.
    Av1,
}

impl VideoCodec {
    /// The ffmpeg encoder that writes this codec.
    #[must_use]
    pub fn encoder(self) -> &'static str {
        match self {
            Self::H264 => "libx264",
            Self::H265 => "libx265",
            Self::Vp9 => "libvpx-vp9",
            Self::Av1 => "libsvtav1",
        }
    }

    /// The codec's usual name, e.g. `"H.264"`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::H265 => "H.265",
            Self::Vp9 => "VP9",
            Self::Av1 => "AV1",
        }
    }
}

/// VideoToolbox's H.265 encoder, which runs on the media engine of Apple
/// silicon Macs. See [`ConvertOptions::hardware`].
pub const HEVC_HARDWARE_ENCODER: &str = "hevc_videotoolbox";

/// Settings that only matter when writing JPEG XL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JxlOptions {
    /// Mathematically lossless; the quality setting is ignored. An unedited
    /// JPEG is recompressed without decoding it, so the original JPEG can be
    /// restored bit for bit later.
    pub lossless: bool,
    /// Encoder effort from 1 (fast) to 10 (slow, smallest).
    pub effort: u8,
    /// Lay the file out so a sharp preview shows early while it loads.
    pub progressive: bool,
}

impl Default for JxlOptions {
    fn default() -> Self {
        Self {
            lossless: false,
            effort: 7,
            progressive: false,
        }
    }
}

/// Which way an animation plays.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    #[default]
    Forward,
    /// Back to front.
    Reverse,
    /// Forward, then back again, so the loop has no seam.
    Boomerang,
}

impl Direction {
    /// How many times the kept range plays per loop.
    #[must_use]
    pub fn passes(self) -> f64 {
        if self == Self::Boomerang { 2.0 } else { 1.0 }
    }
}

/// Clean-up for the sound. Only applies when the output has sound; video
/// fades the picture along with it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SoundOptions {
    /// Even out loudness to -16 LUFS with ffmpeg's single-pass `loudnorm`.
    pub normalize: bool,
    /// Seconds to fade in from silence (and black) at the start; 0 is off.
    pub fade_in: f64,
    /// Seconds to fade out to silence (and black) at the end; 0 is off.
    pub fade_out: f64,
}

/// Target loudness of [`SoundOptions::normalize`]: -16 LUFS integrated,
/// -1.5 dBTP peak ceiling, loudness range 11 LU.
const LOUDNORM: &str = "loudnorm=I=-16:TP=-1.5:LRA=11";
/// Sample rate after `loudnorm` when the source rate is unknown.
const FALLBACK_SAMPLE_RATE: u32 = 48_000;

/// Part of the picture to keep, measured on the picture as it looks after the
/// turn and mirror, so it matches what the user sees in the editor.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Crop {
    /// Width to height to keep, e.g. `[9, 16]`. `None` for a free-form crop.
    #[serde(default)]
    pub aspect: Option<[u32; 2]>,
    /// The kept area as fractions of the picture. `None` keeps the largest
    /// centred area with `aspect`, which is how recipes and batches apply a
    /// crop shape to files of any size.
    #[serde(default)]
    pub area: Option<CropArea>,
}

/// A rectangle in fractions (0 to 1) of the picture's width and height.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropArea {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl CropArea {
    const FULL: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };
}

/// A crop in pixels, as ffmpeg's `crop` filter takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CropBox {
    width: u32,
    height: u32,
    x: u32,
    y: u32,
}

/// The edits and encoding choices for one conversion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConvertOptions {
    pub format: OutputFormat,
    /// Start of the kept range in seconds (video and audio).
    pub trim_start: Option<f64>,
    /// End of the kept range in seconds (video and audio).
    pub trim_end: Option<f64>,
    /// Shrink so the short side is at most this many pixels, e.g. `720` for
    /// 720p. Never upscales.
    pub max_size: Option<u32>,
    /// Output frame rate (video and GIF). Never raises the source rate.
    pub fps: Option<f64>,
    /// Clockwise rotation in degrees: 0, 90, 180 or 270.
    pub rotate: u16,
    /// Mirror horizontally.
    pub flip: bool,
    /// Keep only part of the picture. Applied after the turn and mirror.
    pub crop: Option<Crop>,
    /// 0 = smallest file, 100 = best quality.
    pub quality: u8,
    /// Keep the sound when writing video.
    pub keep_audio: bool,
    /// Codec of the picture in MP4, MOV, MKV and WebM files; `None` is the
    /// format's usual one (see [`OutputFormat::video_codecs`]). Ignored for
    /// other formats.
    pub video_codec: Option<VideoCodec>,
    /// Exact average bitrate of the picture in kbit/s instead of the CRF
    /// that `quality` picks, for MP4, MOV, MKV and WebM files. H.264, H.265
    /// and VP9 get there in two passes, AV1 and hardware H.265 in one. `None` uses `quality`. Ignored
    /// for other formats and for animations made from pictures.
    pub video_bitrate: Option<u32>,
    /// Encode H.265 on the graphics hardware where it can
    /// ([`HEVC_HARDWARE_ENCODER`] on Apple silicon Macs, see
    /// [`Support::hevc_hardware`]): several times faster than x265, but
    /// the files come out bigger at the same look. Falls back to x265 where
    /// there is no hardware encoder. Ignored for other codecs and for
    /// animations made from pictures.
    pub hardware: bool,
    /// JPEG XL settings, ignored for other formats.
    pub jxl: JxlOptions,
    /// Loudness and fades, ignored when the output has no sound.
    pub sound: SoundOptions,
    /// Which dates, places, titles and other tags to carry over.
    pub metadata: Metadata,
    /// Playback speed of animations (GIF, animated JPEG XL), from 0.25 to 4.
    /// Ignored for other outputs.
    pub speed: f64,
    /// Playback direction of animations. Ignored for other outputs.
    pub direction: Direction,
}

/// What to carry over from the original besides the picture and sound.
///
/// Only where the output can hold it: ffmpeg writes no EXIF into JPEG, WebP or
/// AVIF, so those come out without photo metadata whatever this says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Metadata {
    /// Dates, location, camera details, titles and the rest.
    Keep,
    /// Everything except where the file was made: the GPS coordinates in
    /// EXIF and XMP, and location tags in videos.
    #[default]
    NoLocation,
    /// No tags at all. Colour profiles stay, as they are part of the picture.
    None,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            format: OutputFormat::Mp4,
            trim_start: None,
            trim_end: None,
            max_size: None,
            fps: None,
            rotate: 0,
            flip: false,
            crop: None,
            quality: 60,
            keep_audio: true,
            video_codec: None,
            video_bitrate: None,
            hardware: false,
            jxl: JxlOptions::default(),
            sound: SoundOptions::default(),
            metadata: Metadata::default(),
            speed: 1.0,
            direction: Direction::Forward,
        }
    }
}

impl ConvertOptions {
    /// The codec the picture is encoded with: the chosen one, or the
    /// format's usual one. `None` for formats without a codec choice.
    #[must_use]
    pub fn codec(&self) -> Option<VideoCodec> {
        let usual = *self.format.video_codecs().first()?;
        Some(self.video_codec.unwrap_or(usual))
    }

    /// Fails if the chosen codec doesn't fit in the format, e.g. AV1 in MOV.
    pub(crate) fn check_codec(&self) -> Result<()> {
        match self.codec() {
            Some(codec) if !self.format.video_codecs().contains(&codec) => {
                Err(Error::unsupported(format!(
                    "{} files can't hold {} video",
                    self.format.extension().to_uppercase(),
                    codec.label()
                )))
            }
            _ => Ok(()),
        }
    }

    /// Whether speed or direction change how an animation plays.
    fn changes_playback(&self) -> bool {
        (self.speed - 1.0).abs() > 1e-9 || self.direction != Direction::Forward
    }
}

/// Which JPEG XL code paths are installed. [`crate::Ffmpeg::jxl_support`]
/// fills this in; the default (nothing) suits ffmpeg-only planning.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JxlSupport {
    /// libjxl's `cjxl` encoder.
    pub cjxl: bool,
    /// libjxl's `djxl` decoder.
    pub djxl: bool,
    /// ffmpeg's `libjxl` encoder (stills).
    pub ffmpeg_encoder: bool,
    /// ffmpeg's `libjxl_anim` encoder (animations).
    pub ffmpeg_anim_encoder: bool,
    /// ffmpeg's `libjxl` decoder (stills).
    pub ffmpeg_decoder: bool,
    /// ffmpeg's `libjxl_anim` decoder (animations).
    pub ffmpeg_anim_decoder: bool,
}

impl JxlSupport {
    /// Whether some JPEG XL encoder is installed.
    #[must_use]
    pub fn can_write(self) -> bool {
        self.cjxl || self.ffmpeg_encoder || self.ffmpeg_anim_encoder
    }

    fn ffmpeg_reads(self, animated: bool) -> bool {
        if animated {
            self.ffmpeg_anim_decoder
        } else {
            self.ffmpeg_decoder
        }
    }
}

/// What the installed tools can do, as far as [`plan`] cares.
/// [`crate::Ffmpeg::support`] fills this in; the default assumes the least.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Support {
    /// Which JPEG XL code paths are installed.
    pub jxl: JxlSupport,
    /// ffmpeg's `scale` filter converts between transfer functions (ffmpeg
    /// 8 and later), so HDR video can be tone-mapped for pictures and GIFs.
    pub tone_map: bool,
    /// H.265 can be encoded in hardware (see [`ConvertOptions::hardware`]).
    pub hevc_hardware: bool,
}

impl From<JxlSupport> for Support {
    fn from(jxl: JxlSupport) -> Self {
        Self {
            jxl,
            ..Self::default()
        }
    }
}

/// HDR (PQ or HLG) to SDR BT.709 with swscale's colour management. Without
/// it, pictures from iPhone videos come out flat and washed out.
pub(crate) const TONE_MAP_FILTER: &str =
    "scale=out_transfer=bt709:out_primaries=bt709:out_color_matrix=bt709";

/// A program a [`Step`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    Ffmpeg,
    /// libjxl's encoder.
    Cjxl,
    /// libjxl's decoder.
    Djxl,
    /// CatConvert itself, taking metadata out of a JPEG or PNG without
    /// re-encoding it (see [`crate::scrub_file`]). Shown as the CLI command
    /// that does the same.
    Scrub,
}

impl Tool {
    /// The program's usual name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Ffmpeg => "ffmpeg",
            Self::Cjxl => "cjxl",
            Self::Djxl => "djxl",
            Self::Scrub => "catconvert scrub",
        }
    }
}

/// One program run within a [`Plan`].
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub tool: Tool,
    /// Arguments after the program name.
    pub args: Vec<OsString>,
    /// Length of what an ffmpeg step writes, used to turn its timestamps into
    /// a percentage. `None` for stills and other tools.
    pub duration_secs: Option<f64>,
    /// Rough share of the whole job's time, for the progress bar.
    pub weight: f32,
}

/// A validated conversion: one or more programs run in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub steps: Vec<Step>,
    /// Where the result is written.
    pub output: PathBuf,
    /// Intermediate files the steps pass along; deleted when the job ends,
    /// whatever the outcome.
    pub scratch: Vec<PathBuf>,
    /// Text files the steps read, such as ffmpeg concat lists: written just
    /// before the first step. Their paths are in `scratch` too.
    pub writes: Vec<(PathBuf, String)>,
}

impl Plan {
    /// The commands as shell-quoted lines joined with `&&`, for display.
    #[must_use]
    pub fn command_line(&self) -> String {
        render_command(
            self.steps
                .iter()
                .map(|step| (step.tool, step.args.iter().map(|arg| arg.to_string_lossy()))),
        )
    }

    /// Length of the output in seconds, if it is timed.
    #[must_use]
    pub fn duration_secs(&self) -> Option<f64> {
        self.steps.iter().rev().find_map(|step| step.duration_secs)
    }
}

/// Default output height for GIFs, which get huge quickly.
pub(crate) const GIF_MAX_SIZE: u32 = 480;
/// Default frame rate for GIFs.
const GIF_FPS: f64 = 15.0;
/// Default short side of animated JPEG XL.
pub(crate) const ANIMATED_JXL_MAX_SIZE: u32 = 720;
/// Default frame-rate cap of animated JPEG XL.
const ANIMATED_JXL_FPS: f64 = 30.0;
/// Default short side of animated WebP. libwebp has no motion prediction,
/// so big animations get heavy; this matches GIFs.
pub(crate) const ANIMATED_WEBP_MAX_SIZE: u32 = 480;
/// Default frame-rate cap of animated WebP.
const ANIMATED_WEBP_FPS: f64 = 24.0;
/// `cjxl` and ffmpeg's `reverse` filter hold every decoded frame in memory;
/// refuse animations whose raw frames would exceed this many bytes.
pub(crate) const FRAME_MEMORY_BUDGET: f64 = 4.0 * 1024.0 * 1024.0 * 1024.0;
/// Slowest and fastest animation speed.
const SPEEDS: std::ops::RangeInclusive<f64> = 0.25..=4.0;
/// Inputs `cjxl` reads itself, keeping metadata, bit depth and (for JPEG) the
/// original bitstream.
const CJXL_INPUTS: [&str; 6] = ["png", "apng", "gif", "jpg", "jpeg", "jxl"];

/// Validate `options` for `input` and plan the steps that write `output`.
///
/// `support` says what the installed tools can do: which JPEG XL tools there
/// are (only matters when the input or output is JPEG XL), and whether HDR
/// video can be tone-mapped for pictures and GIFs.
///
/// # Errors
///
/// [`Error::Unsupported`] if the format cannot be made from this input (e.g.
/// MP3 from a silent video), the trim range, rotation or crop is invalid, or the
/// JPEG XL tools this needs are missing.
pub fn plan(
    input: &MediaInfo,
    options: &ConvertOptions,
    output: &Path,
    support: impl Into<Support>,
) -> Result<Plan> {
    let support = support.into();
    let Support { jxl, .. } = support;
    let format = options.format;
    if !format.accepts(input.kind) {
        return Err(Error::unsupported(format!(
            "can't turn {} into {}",
            kind_noun(input.kind),
            format.extension().to_uppercase()
        )));
    }
    if format.kind() == MediaKind::Audio && !input.has_audio() {
        return Err(Error::unsupported("this file has no sound to keep"));
    }
    options.check_codec()?;
    if let Some(kbps) = options.video_bitrate
        && !VIDEO_KBPS.contains(&kbps)
        && options.codec().is_some()
    {
        return Err(Error::unsupported(format!(
            "the video bitrate must be between {} kbps and {} Mbps, not {kbps} kbps",
            VIDEO_KBPS.start(),
            VIDEO_KBPS.end() / 1000
        )));
    }
    if !matches!(options.rotate, 0 | 90 | 180 | 270) {
        return Err(Error::unsupported(
            "rotation must be 0, 90, 180 or 270 degrees",
        ));
    }
    let fades = [options.sound.fade_in, options.sound.fade_out];
    if fades.iter().any(|f| !f.is_finite() || *f < 0.0) {
        return Err(Error::unsupported("fade lengths must be positive numbers"));
    }
    if let Some(crop) = &options.crop {
        validate_crop(crop)?;
    }
    if output == input.path {
        return Err(Error::unsupported(
            "the output would overwrite the original",
        ));
    }
    let trim = trim_range(input, options)?;
    let jxl_in = is_jxl(input);
    let animated = format.animates(input.kind);
    if animated && !(options.speed.is_finite() && SPEEDS.contains(&options.speed)) {
        return Err(Error::unsupported("speed must be between 0.25× and 4×"));
    }
    // ffmpeg's `reverse` buffers the whole (already shrunk) clip.
    let defaults = animation_defaults(format);
    if animated
        && options.direction != Direction::Forward
        && animation_bytes(input, options, defaults, trim).is_some_and(|b| b > FRAME_MEMORY_BUDGET)
    {
        return Err(Error::unsupported(
            "this clip is too long to play backwards: trim it, or lower the size or frame rate",
        ));
    }
    let untouched = is_untouched(input, options);
    // Photo metadata that ffmpeg's -map_metadata can't reach (see
    // crate::scrub): it has to be cleaned out of a JPEG or PNG on the way.
    let scrub = photo_scrub(input, options.metadata);

    let mut steps = Vec::new();
    let mut scratch = Vec::new();

    // Shortcuts for unedited files. libjxl does these best: they keep colour
    // profiles, high bit depth and, for JPEG, the original bitstream. Its
    // tools copy metadata as is, so JPEG and PNG get cleaned first or after.
    if untouched && format == OutputFormat::Jxl && jxl.cjxl {
        let ext = input.extension.as_deref().unwrap_or_default();
        let fits = !animated
            || animation_bytes(input, options, (None, None), trim)
                .is_none_or(|b| b <= FRAME_MEMORY_BUDGET);
        let jpeg = matches!(ext, "jpg" | "jpeg");
        let cleanable = jpeg || matches!(ext, "png" | "apng");
        // GIFs hold no EXIF; JPEG XL input with metadata to drop is decoded.
        if CJXL_INPUTS.contains(&ext) && fits && (scrub.is_none() || cleanable || ext == "gif") {
            let source = match scrub.filter(|_| cleanable) {
                Some(metadata) => {
                    let cleaned = scratch_path(output, "clean", ext);
                    steps.push(scrub_step(&input.path, &cleaned, metadata));
                    scratch.push(cleaned.clone());
                    cleaned
                }
                None => input.path.clone(),
            };
            steps.push(cjxl_step(&source, output, options, jpeg, 1.0));
            return Ok(Plan {
                steps,
                output: output.to_path_buf(),
                scratch,
                writes: Vec::new(),
            });
        }
    }
    if untouched && jxl_in && jxl.djxl && input.kind == MediaKind::Image {
        let restore_jpeg = format == OutputFormat::Jpg && input.reconstructs_jpeg;
        if format == OutputFormat::Png || restore_jpeg {
            let decoded = match scrub {
                Some(_) => {
                    let decoded = scratch_path(output, "decoded", format.extension());
                    scratch.push(decoded.clone());
                    decoded
                }
                None => output.to_path_buf(),
            };
            let mut args = vec![input.path.clone().into_os_string(), decoded.clone().into()];
            if restore_jpeg {
                args.push("--reconstruct_jpeg".into());
            }
            args.push("--quiet".into());
            steps.push(Step {
                tool: Tool::Djxl,
                args,
                duration_secs: None,
                weight: 1.0,
            });
            if let Some(metadata) = scrub {
                steps.push(scrub_step(&decoded, output, metadata));
            }
            return Ok(Plan {
                steps,
                output: output.to_path_buf(),
                scratch,
                writes: Vec::new(),
            });
        }
    }

    // JPEG XL that ffmpeg can't decode: let djxl unpack it to (A)PNG first.
    let mut source = input.clone();
    if jxl_in && !jxl.ffmpeg_reads(input.kind == MediaKind::Video) {
        if !jxl.djxl {
            return Err(Error::unsupported(
                "reading JPEG XL needs djxl from libjxl or an ffmpeg built with libjxl",
            ));
        }
        let ext = if input.kind == MediaKind::Video {
            "apng"
        } else {
            "png"
        };
        let decoded = scratch_path(output, "decoded", ext);
        steps.push(Step {
            tool: Tool::Djxl,
            args: vec![
                input.path.clone().into_os_string(),
                decoded.clone().into_os_string(),
                "--quiet".into(),
            ],
            duration_secs: None,
            weight: 0.2,
        });
        source.path = decoded.clone();
        source.extension = Some(ext.into());
        source.video_codec = Some(ext.into());
        // Whether djxl's PNG keeps HDR tags is up to libjxl; don't guess.
        source.hdr = false;
        source.tile_grid = None;
        scratch.push(decoded);
    }

    if format == OutputFormat::Jxl {
        // cjxl holds the whole animation, both halves of a boomerang.
        let raw_bytes = animated
            .then(|| animation_bytes(input, options, defaults, trim))
            .flatten()
            .map(|b| b * options.direction.passes());
        let too_big_for_cjxl = raw_bytes.is_some_and(|b| b > FRAME_MEMORY_BUDGET);
        let ffmpeg_can = if animated {
            jxl.ffmpeg_anim_encoder
        } else {
            jxl.ffmpeg_encoder
        };
        if jxl.cjxl && !(too_big_for_cjxl && ffmpeg_can) {
            if too_big_for_cjxl {
                return Err(Error::unsupported(
                    "this clip is too long for an animated JPEG XL: trim it, or lower the size or frame rate",
                ));
            }
            // ffmpeg applies the edits into a lossless (A)PNG, cjxl encodes that.
            let ext = if animated { "apng" } else { "png" };
            let frames = scratch_path(output, "frames", ext);
            steps.push(ffmpeg_step(
                &source,
                options,
                trim,
                Sink::Frames { animated },
                &frames,
                support,
            ));
            scratch.push(frames.clone());
            let frames = match scrub.filter(|_| !animated) {
                Some(metadata) => {
                    let cleaned = scratch_path(output, "clean", ext);
                    steps.push(scrub_step(&frames, &cleaned, metadata));
                    scratch.push(cleaned.clone());
                    cleaned
                }
                None => frames,
            };
            steps.push(cjxl_step(&frames, output, options, false, 0.6));
        } else if let (true, Some(metadata)) = (ffmpeg_can && !animated, scrub) {
            // ffmpeg's libjxl writes the EXIF it decoded: edit into a PNG,
            // clean that, then encode it as it is.
            let frames = scratch_path(output, "frames", "png");
            let cleaned = scratch_path(output, "clean", "png");
            steps.push(ffmpeg_step(
                &source,
                options,
                trim,
                Sink::Frames { animated: false },
                &frames,
                support,
            ));
            steps.push(scrub_step(&frames, &cleaned, metadata));
            let (width, height) = output_dims(input, options, None).unzip();
            let edited = MediaInfo {
                path: cleaned.clone(),
                extension: Some("png".into()),
                width,
                height,
                video_codec: Some("png".into()),
                has_location: false,
                location_tags: Vec::new(),
                // Already stitched together and tone-mapped by the first pass.
                tile_grid: None,
                hdr: false,
                ..source.clone()
            };
            let as_is = ConvertOptions {
                trim_start: None,
                trim_end: None,
                max_size: None,
                fps: None,
                rotate: 0,
                flip: false,
                crop: None,
                metadata: Metadata::Keep,
                ..options.clone()
            };
            let no_tone_map = Support {
                tone_map: false,
                ..support
            };
            let mut encode = ffmpeg_step(
                &edited,
                &as_is,
                (0.0, None),
                Sink::Output,
                output,
                no_tone_map,
            );
            encode.weight = 0.6;
            steps.push(encode);
            scratch.extend([frames, cleaned]);
        } else if ffmpeg_can {
            steps.push(ffmpeg_step(
                &source,
                options,
                trim,
                Sink::Output,
                output,
                support,
            ));
        } else if animated {
            return Err(Error::unsupported(
                "animated JPEG XL needs cjxl from libjxl or an ffmpeg with the libjxl_anim encoder",
            ));
        } else {
            return Err(Error::unsupported(
                "writing JPEG XL needs cjxl from libjxl or an ffmpeg built with libjxl",
            ));
        }
    } else if let (OutputFormat::Png, Some(metadata)) = (format, scrub) {
        // ffmpeg's PNG encoder writes the EXIF it decoded.
        let encoded = scratch_path(output, "encoded", "png");
        steps.push(ffmpeg_step(
            &source,
            options,
            trim,
            Sink::Output,
            &encoded,
            support,
        ));
        steps.push(scrub_step(&encoded, output, metadata));
        scratch.push(encoded);
    } else if let Some(rate) = video_rate(options, support).filter(|rate| rate.passes == 2) {
        // An exact bitrate, met evenly across the clip: the first pass
        // measures how hard each part is to encode, the second spends the
        // bits accordingly (see `PASSLOG_SUFFIXES` for its log files).
        let log = scratch_path(output, "passlog", "stats");
        steps.push(ffmpeg_step(
            &source,
            options,
            trim,
            Sink::FirstPass { log: &log },
            Path::new("-"),
            support,
        ));
        steps.push(ffmpeg_step(
            &source,
            options,
            trim,
            Sink::SecondPass { log: &log },
            output,
            support,
        ));
        let stats = log.to_string_lossy();
        scratch.extend(
            crate::custom::PASSLOG_SUFFIXES.map(|suffix| PathBuf::from(format!("{stats}{suffix}"))),
        );
        if rate.codec == VideoCodec::H265 {
            let stats = x265_stats(&log);
            scratch.extend(
                crate::custom::X265_STATS_SUFFIXES
                    .map(|suffix| PathBuf::from(format!("{stats}{suffix}"))),
            );
        }
    } else {
        steps.push(ffmpeg_step(
            &source,
            options,
            trim,
            Sink::Output,
            output,
            support,
        ));
    }

    Ok(Plan {
        steps,
        output: output.to_path_buf(),
        scratch,
        writes: Vec::new(),
    })
}

/// Where an ffmpeg step writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sink<'a> {
    /// The final file, in `options.format`.
    Output,
    /// The analysis pass of a two-pass video encode: it writes only the
    /// encoder's statistics, to log files starting with `log`.
    FirstPass { log: &'a Path },
    /// The final file, encoded with the statistics of a [`Sink::FirstPass`].
    SecondPass { log: &'a Path },
    /// Lossless PNG (or APNG when `animated`) for another tool to pick up.
    Frames { animated: bool },
}

/// The ffmpeg call that applies the edits in `options` to `input` and writes `out`.
///
/// `support` says whether this ffmpeg can turn HDR into SDR, which is used
/// for HDR input written as pictures or GIFs (they can't hold HDR), and
/// whether H.265 can be encoded in hardware.
fn ffmpeg_step(
    input: &MediaInfo,
    options: &ConvertOptions,
    (start, end): (f64, Option<f64>),
    sink: Sink,
    out: &Path,
    support: Support,
) -> Step {
    let format = options.format;
    let passlog = match sink {
        Sink::SecondPass { log } => Some(log),
        _ => None,
    };
    let animated = format.animates(input.kind);
    let out_kind = if animated {
        MediaKind::Video
    } else {
        format.kind()
    };
    let tone_map = support.tone_map
        && input.hdr
        && (format == OutputFormat::Gif || format.kind() == MediaKind::Image);
    let tone_mapped = |mut filters: Vec<String>| {
        if tone_map {
            filters.insert(0, TONE_MAP_FILTER.to_owned());
        }
        filters
    };
    let mut args: Vec<OsString> = Vec::new();
    let mut push = |items: &[&str]| args.extend(items.iter().map(OsString::from));

    // Input and trim. Seeking before `-i` is fast and frame-accurate when
    // re-encoding, except that ffmpeg's animated JPEG XL demuxer can't seek
    // (the decoder then fails); those short clips are decoded from the start.
    let timed_input = input.kind != MediaKind::Image;
    let seek_input = !is_jxl(input);
    // A new speed or direction retimes the output, so output-side -ss/-t
    // would cut the wrong part: the range is limited on the input instead
    // (or by a trim filter where the input can't seek).
    let retimed = animated && options.changes_playback();
    let trim_end = end.filter(|_| options.trim_end.is_some());
    let mut lead_trim = None;
    if timed_input && start > 0.0 && seek_input {
        push(&["-ss", &format_secs(start)]);
    }
    if timed_input && retimed {
        if let Some(end) = trim_end {
            let read = if seek_input { end - start } else { end };
            push(&["-t", &format_secs(read)]);
        }
        if start > 0.0 && !seek_input {
            lead_trim = Some(start);
        }
    }
    push(&["-i"]);
    args.push(input.path.clone().into_os_string());
    if timed_input && start > 0.0 && !seek_input && !retimed {
        args.extend(["-ss".into(), format_secs(start).into()]);
    }
    // Where the kept range starts on the filters' clock: 0 after an input
    // seek or a lead trim, the trim start after an output seek.
    let t0 = if timed_input && !seek_input && !retimed {
        start
    } else {
        0.0
    };
    let duration_secs = if timed_input && out_kind != MediaKind::Image {
        if let Some(end) = trim_end.filter(|_| !retimed) {
            args.extend(["-t".into(), format_secs(end - start).into()]);
        }
        end.or(input.duration_secs).map(|end| {
            let kept = (end - start).max(0.0);
            if animated {
                kept / options.speed * options.direction.passes()
            } else {
                kept
            }
        })
    } else {
        None
    };

    let mut push = |items: &[&str]| args.extend(items.iter().map(OsString::from));
    let q = options.quality.min(100);

    match (format, sink) {
        (OutputFormat::Jxl, Sink::Frames { animated }) => {
            let filters = tone_mapped(animation_filters(input, options, lead_trim));
            if animated {
                push_video(&mut push, input, &[], &filters);
                // Fast zlib: this file only lives until cjxl has read it.
                push(&["-c:v", "apng", "-compression_level", "1", "-plays", "0"]);
                push(&["-an", "-sn", "-dn", "-f", "apng"]);
            } else {
                push_video(&mut push, input, &["-frames:v", "1"], &filters);
                push(&["-c:v", "png", "-compression_level", "1"]);
                push(&["-update", "1", "-an", "-sn", "-dn"]);
            }
        }
        (_, Sink::Frames { .. }) => unreachable!("only JPEG XL goes through intermediate frames"),
        (
            OutputFormat::Mp4 | OutputFormat::Mov | OutputFormat::Mkv | OutputFormat::Webm,
            Sink::FirstPass { log },
        ) => {
            // The analysis pass sees the same frames as the real one: the
            // fades of the sound take the picture along there too.
            let mut filters = video_filters(input, options, None, true);
            if options.keep_audio && input.has_audio() {
                filters.extend(fades("fade", options.sound, t0, duration_secs));
            }
            push_video(&mut push, input, &[], &filters);
            let rate = video_rate(options, support)
                .unwrap_or_else(|| VideoRate::constant(VideoCodec::H264, options.quality));
            push_video_encoder(&mut push, rate, input.hdr, Some((1, log)));
            push(&["-an", "-sn", "-dn", "-f", "null"]);
        }
        (OutputFormat::Mp4 | OutputFormat::Mov | OutputFormat::Mkv | OutputFormat::Webm, _) => {
            let keep_audio = options.keep_audio && input.has_audio();
            let audio: &[&str] = if keep_audio { &["-map", "0:a:0"] } else { &[] };
            let mut filters = video_filters(input, options, None, true);
            if keep_audio {
                // The sound's fades take the picture along, from and to black.
                filters.extend(fades("fade", options.sound, t0, duration_secs));
            }
            push_video(&mut push, input, audio, &filters);
            if keep_audio {
                let audio = audio_filters(input, options.sound, t0, duration_secs);
                if !audio.is_empty() {
                    push(&["-af", &audio.join(",")]);
                }
            }
            let rate = video_rate(options, support)
                .unwrap_or_else(|| VideoRate::constant(VideoCodec::H264, options.quality));
            push_video_encoder(&mut push, rate, input.hdr, passlog.map(|log| (2, log)));
            match audio_rate(input, options).filter(|_| keep_audio) {
                Some(rate) => push_audio_encoder(&mut push, format, rate),
                None => push(&["-an"]),
            }
            push(&["-sn", "-dn"]);
            push_mp4_flags(&mut push, format, rate.codec);
        }
        (_, Sink::FirstPass { .. } | Sink::SecondPass { .. }) => {
            unreachable!("only video files are encoded in two passes")
        }
        (OutputFormat::Gif, _) => {
            let mut filters = tone_mapped(animation_filters(input, options, lead_trim));
            let colors = lerp(q, 32.0, 256.0);
            filters.push(format!(
                "split[a][b];[a]palettegen=max_colors={colors}:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4"
            ));
            push_video(&mut push, input, &[], &filters);
            push(&["-loop", "0", "-an", "-sn", "-dn"]);
        }
        (OutputFormat::Jxl, Sink::Output) if animated => {
            // ffmpeg's libjxl_anim emits the codestream frame by frame; the
            // data muxer writes the packets back to back as one file.
            let filters = tone_mapped(animation_filters(input, options, lead_trim));
            push_video(&mut push, input, &[], &filters);
            push(&["-c:v", "libjxl_anim"]);
            push_ffmpeg_jxl(&mut push, options);
            push(&["-an", "-sn", "-dn", "-f", "data"]);
        }
        (OutputFormat::Webp, Sink::Output) if animated => {
            let filters = tone_mapped(animation_filters(input, options, lead_trim));
            push_video(&mut push, input, &[], &filters);
            push_animated_webp(&mut push, q);
            push(&["-an", "-sn", "-dn"]);
        }
        (
            OutputFormat::Mp3
            | OutputFormat::M4a
            | OutputFormat::Wav
            | OutputFormat::Flac
            | OutputFormat::Ogg,
            _,
        ) => {
            push(&["-map", "0:a:0", "-vn", "-sn", "-dn"]);
            let audio = audio_filters(input, options.sound, t0, duration_secs);
            if !audio.is_empty() {
                push(&["-af", &audio.join(",")]);
            }
            if let Some(rate) = audio_rate(input, options) {
                push_audio_encoder(&mut push, format, rate);
            }
            if format == OutputFormat::M4a {
                push(&["-movflags", "+faststart"]);
            }
        }
        (
            OutputFormat::Png
            | OutputFormat::Jpg
            | OutputFormat::Webp
            | OutputFormat::Avif
            | OutputFormat::Jxl,
            Sink::Output,
        ) => {
            let needs_even = format == OutputFormat::Avif;
            let filters = tone_mapped(video_filters(input, options, None, needs_even));
            push_video(&mut push, input, &["-frames:v", "1"], &filters);
            match format {
                OutputFormat::Png => push(&["-c:v", "png"]),
                OutputFormat::Jpg => {
                    push(&["-c:v", "mjpeg", "-q:v", &lerp(q, 20.0, 2.0).to_string()]);
                }
                OutputFormat::Webp => push(&[
                    "-c:v",
                    "libwebp",
                    "-quality",
                    &lerp(q, 40.0, 95.0).to_string(),
                ]),
                OutputFormat::Jxl => {
                    push(&["-c:v", "libjxl"]);
                    push_ffmpeg_jxl(&mut push, options);
                }
                _ => push(&[
                    "-c:v",
                    "libsvtav1",
                    "-crf",
                    &lerp(q, 45.0, 20.0).to_string(),
                    "-pix_fmt",
                    "yuv420p",
                ]),
            }
            push(&["-update", "1", "-an", "-sn", "-dn"]);
        }
    }

    match options.metadata {
        _ if matches!(sink, Sink::FirstPass { .. }) => {}
        Metadata::Keep => {}
        // Videos keep their place in container tags, e.g. `location` or
        // `com.apple.quicktime.location.ISO6709`; an empty value deletes one.
        Metadata::NoLocation => {
            for tag in &input.location_tags {
                args.extend(["-metadata".into(), format!("{tag}=").into()]);
            }
        }
        Metadata::None => {
            args.extend(["-map_metadata", "-1", "-map_chapters", "-1"].map(OsString::from));
        }
    }
    args.push(out.as_os_str().to_owned());
    let weight = match sink {
        Sink::Frames { .. } => 0.4,
        // x264 and libvpx analyse quickly on the first pass.
        Sink::FirstPass { .. } => 0.5,
        Sink::Output | Sink::SecondPass { .. } => 1.0,
    };
    Step {
        tool: Tool::Ffmpeg,
        args,
        duration_secs,
        weight,
    }
}

/// Whether photo metadata must be cleaned out of JPEG or PNG files on the
/// way, and how. Only stills carry EXIF; with all but the location kept, only
/// files known to have a location need it.
fn photo_scrub(input: &MediaInfo, metadata: Metadata) -> Option<Metadata> {
    let needed = match metadata {
        Metadata::Keep => false,
        Metadata::NoLocation => input.has_location,
        Metadata::None => true,
    };
    (needed && input.kind == MediaKind::Image).then_some(metadata)
}

/// The built-in step that copies `input` to `output` without the metadata
/// `metadata` rules out.
fn scrub_step(input: &Path, output: &Path, metadata: Metadata) -> Step {
    Step {
        tool: Tool::Scrub,
        args: crate::scrub::step_args(input, output, metadata),
        duration_secs: None,
        weight: 0.05,
    }
}

/// Loudness, fade and resampling filters for the sound. `t0` is where the
/// kept range starts on the filters' clock and `length` how long it is.
fn audio_filters(
    input: &MediaInfo,
    sound: SoundOptions,
    t0: f64,
    length: Option<f64>,
) -> Vec<String> {
    let mut filters = Vec::new();
    // Normalize first: fading afterwards keeps loudnorm from lifting the fades.
    if sound.normalize {
        filters.push(LOUDNORM.to_owned());
    }
    filters.extend(fades("afade", sound, t0, length));
    if sound.normalize {
        // loudnorm resamples to 192 kHz and passes that on. Asking for the
        // source rate in the graph rather than with -ar lets ffmpeg settle on
        // the nearest rate the encoder takes (Opus only does 48 kHz).
        let rate = input.sample_rate.unwrap_or(FALLBACK_SAMPLE_RATE);
        filters.push(format!("aresample={rate}"));
    }
    filters
}

/// Fade-in and fade-out with ffmpeg's `fade` (picture) or `afade` (sound)
/// filter. A fade-out needs the output's `length`, and neither fade runs
/// longer than the clip.
fn fades(filter: &str, sound: SoundOptions, t0: f64, length: Option<f64>) -> Vec<String> {
    let mut filters = Vec::new();
    let fade_in = length.map_or(sound.fade_in, |l| sound.fade_in.min(l));
    if fade_in > 0.0 {
        let st = if t0 > 0.0 {
            format!(":st={}", format_secs(t0))
        } else {
            String::new()
        };
        filters.push(format!("{filter}=t=in{st}:d={}", format_secs(fade_in)));
    }
    if let Some(length) = length {
        let fade_out = sound.fade_out.min(length);
        if fade_out > 0.0 {
            filters.push(format!(
                "{filter}=t=out:st={}:d={}",
                format_secs(t0 + length - fade_out),
                format_secs(fade_out)
            ));
        }
    }
    filters
}

/// Pick the picture of `input`, push `extra`, and run `filters` on the picture.
///
/// HEIF tile grids come out of a filtergraph ffmpeg builds itself (stitching
/// the tiles, then applying the rotation), and `-vf` can't be added to that,
/// so their filters go into a complex filtergraph fed by the grid instead.
fn push_video(
    push: &mut impl FnMut(&[&str]),
    input: &MediaInfo,
    extra: &[&str],
    filters: &[String],
) {
    let chain = filters.join(",");
    match input.tile_grid {
        Some(group) if chain.is_empty() => push(&["-map", &format!("[0:g:{group}]")]),
        Some(group) => push(&[
            "-filter_complex",
            &format!("[0:g:{group}]{chain}[v]"),
            "-map",
            "[v]",
        ]),
        None => push(&["-map", "0:v:0"]),
    }
    push(extra);
    if input.tile_grid.is_none() && !chain.is_empty() {
        push(&["-vf", &chain]);
    }
}

/// The video encoder of `rate.codec` at `rate`, in a pixel format every
/// player of that codec reads. `hdr` says the picture is HDR. `pass` is the
/// pass number and log file prefix of a two-pass encode.
///
/// AV1 is written in 10 bits: every AV1 decoder handles them (they are part
/// of the Main profile), they compress better than 8 bits even for SDR
/// sources, and HDR video keeps its HDR (ffmpeg passes the source's colour
/// tags on to SVT-AV1). SVT-AV1's preset 8 is about as quick as x264's
/// `fast` and still well ahead of it in size.
///
/// H.265 is written in 10 bits (Main 10) only for HDR sources, which keeps
/// them HDR the same way; SDR stays 8-bit Main, which older HEVC decoders
/// need. x265's `fast` preset is several times slower than x264's, so
/// [`ConvertOptions::hardware`] can hand it to VideoToolbox instead.
pub(crate) fn push_video_encoder(
    push: &mut impl FnMut(&[&str]),
    rate: VideoRate,
    hdr: bool,
    pass: Option<(u8, &Path)>,
) {
    let target = rate.kbps.map(|kbps| format!("{kbps}k"));
    let crf = rate.crf.map(|crf| crf.to_string());
    let ten_bit = rate.codec == VideoCodec::Av1 || (rate.codec == VideoCodec::H265 && hdr);
    match rate.codec {
        VideoCodec::H264 => {
            push(&["-c:v", "libx264", "-preset", "fast"]);
            match (&target, &crf) {
                (Some(target), _) => push(&["-b:v", target]),
                (None, Some(crf)) => push(&["-crf", crf]),
                (None, None) => {}
            }
        }
        VideoCodec::H265 if rate.hardware => {
            push(&["-c:v", HEVC_HARDWARE_ENCODER]);
            match (&target, rate.hardware_quality) {
                (Some(target), _) => push(&["-b:v", target]),
                (None, Some(quality)) => push(&["-q:v", &quality.to_string()]),
                (None, None) => {}
            }
        }
        VideoCodec::H265 => {
            push(&["-c:v", "libx265", "-preset", "fast"]);
            match (&target, &crf) {
                (Some(target), _) => push(&["-b:v", target]),
                (None, Some(crf)) => push(&["-crf", crf]),
                (None, None) => {}
            }
        }
        VideoCodec::Vp9 => {
            push(&["-c:v", "libvpx-vp9"]);
            match (&target, &crf) {
                (Some(target), _) => push(&["-b:v", target]),
                // Constant quality needs the bitrate cap off.
                (None, Some(crf)) => push(&["-crf", crf, "-b:v", "0"]),
                (None, None) => {}
            }
            push(&["-row-mt", "1", "-deadline", "good", "-cpu-used", "4"]);
        }
        VideoCodec::Av1 => {
            push(&["-c:v", "libsvtav1", "-preset", "8"]);
            match (&target, &crf) {
                // SVT-AV1 aims for an average (VBR) when given a bitrate.
                (Some(target), _) => push(&["-b:v", target]),
                (None, Some(crf)) => push(&["-crf", crf]),
                (None, None) => {}
            }
        }
    }
    if let Some((pass, log)) = pass {
        push(&["-pass", &pass.to_string()]);
        push(&["-passlogfile", &log.to_string_lossy()]);
        if rate.codec == VideoCodec::H265 {
            // x265 keeps its own statistics and ignores -passlogfile.
            push(&["-x265-stats", &x265_stats(log)]);
        }
    }
    let pix_fmt = match (ten_bit, rate.hardware) {
        (true, true) => "p010le",
        (true, false) => "yuv420p10le",
        (false, _) => "yuv420p",
    };
    push(&["-pix_fmt", pix_fmt]);
    if ten_bit && rate.codec == VideoCodec::H265 {
        push(&["-profile:v", "main10"]);
    }
}

/// Where x265 keeps the statistics of a two-pass encode whose ffmpeg log
/// prefix is `log`. It writes the files of
/// [`crate::custom::X265_STATS_SUFFIXES`] next to it.
fn x265_stats(log: &Path) -> String {
    format!("{}{X265_STATS_SUFFIX}", log.to_string_lossy())
}

/// What x265's statistics file adds to the ffmpeg log prefix of a two-pass
/// encode.
const X265_STATS_SUFFIX: &str = "-0.x265.log";

/// MP4 and MOV flags: the index up front so playback starts before the file
/// has loaded, and H.265 tagged `hvc1`, without which QuickTime, Safari,
/// iOS and the Finder's preview won't play it.
pub(crate) fn push_mp4_flags(
    push: &mut impl FnMut(&[&str]),
    format: OutputFormat,
    codec: VideoCodec,
) {
    if !matches!(format, OutputFormat::Mp4 | OutputFormat::Mov) {
        return;
    }
    if codec == VideoCodec::H265 {
        push(&["-tag:v", "hvc1"]);
    }
    push(&["-movflags", "+faststart"]);
}

/// The CRF of each video encoder at quality 0 and 100. x264's default
/// quality of 60 lands at 24; x265's at 28 and SVT-AV1's at 32, which look
/// about the same.
#[must_use]
pub fn crf_range(codec: VideoCodec) -> (u8, u8) {
    match codec {
        VideoCodec::H264 => (34, 18),
        VideoCodec::H265 => (38, 22),
        VideoCodec::Vp9 => (48, 28),
        VideoCodec::Av1 => (48, 22),
    }
}

/// VideoToolbox's constant quality (`-q:v`, higher is better) at quality 0
/// and 100. The default of 60 lands at 64.
pub const HARDWARE_QUALITY_RANGE: (u8, u8) = (40, 80);

/// The CRF `codec` is encoded with at quality `q`.
fn crf(codec: VideoCodec, q: u8) -> u8 {
    let (worst, best) = crf_range(codec);
    let crf = lerp(q.min(100), f64::from(worst), f64::from(best));
    u8::try_from(crf).unwrap_or(worst)
}

/// VideoToolbox's quality at quality `q`.
fn hardware_quality(q: u8) -> u8 {
    let (worst, best) = HARDWARE_QUALITY_RANGE;
    let quality = lerp(q.min(100), f64::from(worst), f64::from(best));
    u8::try_from(quality).unwrap_or(worst)
}

/// Slowest and fastest exact video bitrate in kbit/s.
pub const VIDEO_KBPS: std::ops::RangeInclusive<u32> = 100..=200_000;

/// Opus bitrates in kbit/s for low, middle and high quality.
const OPUS_KBPS: [u32; 3] = [64, 96, 160];
/// AAC bitrates in kbit/s alongside a picture (MP4, MOV, MKV).
const AAC_VIDEO_KBPS: [u32; 3] = [96, 128, 192];
/// AAC bitrates in kbit/s for music (M4A), a step above video's.
const AAC_MUSIC_KBPS: [u32; 3] = [128, 192, 256];
/// LAME's typical average bitrate in kbit/s for VBR levels V0 to V7.
const MP3_VBR_KBPS: [u32; 8] = [245, 225, 190, 175, 165, 130, 115, 100];

/// How the sound of an output is encoded: what the quality setting
/// becomes for it. See [`quality_summary`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum AudioRate {
    /// AAC or Opus aiming for this many kbit/s.
    Bitrate { kbps: u32 },
    /// MP3 at LAME's VBR level (`-q:a`, 0 best to 7 smallest), which
    /// averages about `kbps` kbit/s depending on the sound.
    Vbr { level: u8, kbps: u32 },
    /// FLAC: every sample kept, at whatever size that takes.
    Lossless,
    /// WAV: uncompressed 16-bit PCM at `kbps` kbit/s, `None` when the
    /// source's sample rate or channel count is unknown.
    Pcm { kbps: Option<u32> },
}

/// How the picture of a video file is encoded: what the quality setting (or
/// an exact bitrate) becomes for it. See [`quality_summary`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoRate {
    pub codec: VideoCodec,
    /// Constant rate factor; `None` with an exact bitrate or in hardware.
    pub crf: Option<u8>,
    /// Exact average bitrate in kbit/s; `None` with a CRF.
    pub kbps: Option<u32>,
    /// Encoding passes: 2 for an exact H.264, H.265 or VP9 bitrate in
    /// software, otherwise 1.
    pub passes: u8,
    /// Encoded by the graphics hardware ([`HEVC_HARDWARE_ENCODER`]) rather
    /// than in software.
    pub hardware: bool,
    /// VideoToolbox's constant quality (see [`HARDWARE_QUALITY_RANGE`]) in
    /// hardware without an exact bitrate, otherwise `None`.
    pub hardware_quality: Option<u8>,
}

impl VideoRate {
    /// The CRF `codec` gets at quality `q`.
    pub(crate) fn constant(codec: VideoCodec, q: u8) -> Self {
        Self {
            codec,
            crf: Some(crf(codec, q)),
            kbps: None,
            passes: 1,
            hardware: false,
            hardware_quality: None,
        }
    }
}

/// What the quality setting means in numbers for one conversion. The planned
/// command uses exactly these values; `None` where the output has no sound,
/// or no picture with a bitrate (stills, GIFs and other animations).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualitySummary {
    pub audio: Option<AudioRate>,
    pub video: Option<VideoRate>,
}

/// The audio bitrate and video CRF (or exact bitrate) that converting
/// `input` with `options` uses, so the quality setting can be shown in
/// numbers. `support` is what [`plan`] gets. Doesn't validate the options;
/// [`plan`] does.
#[must_use]
pub fn quality_summary(
    input: &MediaInfo,
    options: &ConvertOptions,
    support: impl Into<Support>,
) -> QualitySummary {
    let format = options.format;
    let has_picture = format.kind() == MediaKind::Video && !format.animates(input.kind);
    let support = support.into();
    let video = has_picture.then(|| video_rate(options, support)).flatten();
    let has_sound = input.has_audio()
        && match format.kind() {
            MediaKind::Audio => true,
            MediaKind::Video => options.keep_audio && video.is_some(),
            _ => false,
        };
    QualitySummary {
        audio: has_sound.then(|| audio_rate(input, options)).flatten(),
        video,
    }
}

/// How the picture of a video file is encoded, or `None` for formats
/// without a video codec choice (GIFs, animations, stills, sound).
fn video_rate(options: &ConvertOptions, support: Support) -> Option<VideoRate> {
    let codec = options.codec()?;
    let hardware = codec == VideoCodec::H265 && options.hardware && support.hevc_hardware;
    Some(match (options.video_bitrate, hardware) {
        (Some(kbps), _) => VideoRate {
            codec,
            crf: None,
            kbps: Some(kbps),
            // ffmpeg's SVT-AV1 wrapper has no two-pass mode, and
            // VideoToolbox aims for the average by itself.
            passes: if codec == VideoCodec::Av1 || hardware {
                1
            } else {
                2
            },
            hardware,
            hardware_quality: None,
        },
        (None, true) => VideoRate {
            crf: None,
            hardware: true,
            hardware_quality: Some(hardware_quality(options.quality)),
            ..VideoRate::constant(codec, options.quality)
        },
        (None, false) => VideoRate::constant(codec, options.quality),
    })
}

/// How the sound is encoded in `options.format`, or `None` if that format
/// has no sound.
fn audio_rate(input: &MediaInfo, options: &ConvertOptions) -> Option<AudioRate> {
    let q = options.quality.min(100);
    Some(match options.format {
        OutputFormat::Mp4 | OutputFormat::Mov | OutputFormat::Mkv => AudioRate::Bitrate {
            kbps: tiered(q, AAC_VIDEO_KBPS),
        },
        OutputFormat::Webm | OutputFormat::Ogg => AudioRate::Bitrate {
            kbps: tiered(q, OPUS_KBPS),
        },
        OutputFormat::M4a => AudioRate::Bitrate {
            kbps: tiered(q, AAC_MUSIC_KBPS),
        },
        OutputFormat::Mp3 => {
            let level = u8::try_from(lerp(q, 7.0, 0.0)).unwrap_or(4);
            AudioRate::Vbr {
                level,
                kbps: MP3_VBR_KBPS[usize::from(level.min(7))],
            }
        }
        OutputFormat::Flac => AudioRate::Lossless,
        OutputFormat::Wav => {
            // loudnorm resamples, and the rate it settles on is the source's.
            let rate = input
                .sample_rate
                .or(options.sound.normalize.then_some(FALLBACK_SAMPLE_RATE));
            let kbps = rate
                .zip(input.audio_channels)
                .map(|(rate, channels)| rate.saturating_mul(channels).saturating_mul(16) / 1000);
            AudioRate::Pcm { kbps }
        }
        _ => return None,
    })
}

/// The ffmpeg encoder arguments for the sound in `format` at `rate`.
fn push_audio_encoder(push: &mut impl FnMut(&[&str]), format: OutputFormat, rate: AudioRate) {
    match rate {
        AudioRate::Bitrate { kbps } => {
            let codec = if matches!(format, OutputFormat::Webm | OutputFormat::Ogg) {
                "libopus"
            } else {
                "aac"
            };
            push(&["-c:a", codec, "-b:a", &format!("{kbps}k")]);
        }
        AudioRate::Vbr { level, .. } => {
            push(&["-c:a", "libmp3lame", "-q:a", &level.to_string()]);
        }
        AudioRate::Lossless => push(&["-c:a", "flac"]),
        AudioRate::Pcm { .. } => push(&["-c:a", "pcm_s16le"]),
    }
}

/// libwebp's animation encoder at quality `q`, looping forever.
pub(crate) fn push_animated_webp(push: &mut impl FnMut(&[&str]), q: u8) {
    let quality = lerp(q, 40.0, 95.0).to_string();
    push(&["-c:v", "libwebp_anim", "-quality", &quality, "-loop", "0"]);
}

/// Distance and effort for ffmpeg's libjxl encoders.
pub(crate) fn push_ffmpeg_jxl(push: &mut impl FnMut(&[&str]), options: &ConvertOptions) {
    let distance = jxl_distance(options);
    // ffmpeg's libjxl wrapper caps effort at 9.
    let effort = options.jxl.effort.clamp(1, 9).to_string();
    push(&["-distance", &distance, "-effort", &effort]);
}

/// `cjxl` reading `input` and writing `output`. `jpeg` marks a JPEG input,
/// which is recompressed losslessly when `lossless` is on.
pub(crate) fn cjxl_step(
    input: &Path,
    output: &Path,
    options: &ConvertOptions,
    jpeg: bool,
    weight: f32,
) -> Step {
    let mut args: Vec<OsString> = vec![input.into(), output.into()];
    let mut push = |items: &[&str]| args.extend(items.iter().map(OsString::from));
    if jpeg {
        if options.jxl.lossless {
            push(&["--lossless_jpeg=1"]);
        } else {
            push(&["--lossless_jpeg=0", "-d", &jxl_distance(options)]);
        }
    } else {
        push(&["-d", &jxl_distance(options)]);
    }
    push(&["-e", &options.jxl.effort.clamp(1, 10).to_string()]);
    if options.jxl.progressive {
        push(&["-p"]);
    }
    push(&["--quiet"]);
    Step {
        tool: Tool::Cjxl,
        args,
        duration_secs: None,
        weight,
    }
}

/// Butteraugli distance for the quality setting: 0 when lossless, otherwise
/// 3.0 (smallest) down to 0.5 (best), libjxl's recommended range. The
/// default quality of 60 lands at 1.5, just above "visually lossless".
fn jxl_distance(options: &ConvertOptions) -> String {
    if options.jxl.lossless {
        return "0".into();
    }
    let q = f64::from(options.quality.min(100));
    let distance = 3.0 - 2.5 * q / 100.0;
    format_secs((distance * 100.0).round() / 100.0)
}

/// The size and frame-rate caps an animation gets when the user picks none.
fn animation_defaults(format: OutputFormat) -> (Option<u32>, Option<f64>) {
    match format {
        OutputFormat::Gif => (Some(GIF_MAX_SIZE), Some(GIF_FPS)),
        OutputFormat::Jxl => (Some(ANIMATED_JXL_MAX_SIZE), Some(ANIMATED_JXL_FPS)),
        OutputFormat::Webp => (Some(ANIMATED_WEBP_MAX_SIZE), Some(ANIMATED_WEBP_FPS)),
        _ => (None, None),
    }
}

/// Filters for GIFs and JPEG XL (animated or still), minus the GIF palette.
///
/// Animations run: trim (`lead_trim`, where the input couldn't seek) →
/// speed → frame-rate cap → turn, mirror and shrink → reverse or boomerang.
/// Speeding up before the cap means 2× also makes the file smaller, and
/// reversing last means `reverse` buffers the smallest frames.
fn animation_filters(
    input: &MediaInfo,
    options: &ConvertOptions,
    lead_trim: Option<f64>,
) -> Vec<String> {
    if !options.format.animates(input.kind) {
        return video_filters(input, options, None, false);
    }
    let (default_max, default_fps) = animation_defaults(options.format);
    let mut filters = Vec::new();
    if let Some(start) = lead_trim {
        filters.push(format!(
            "trim=start={},setpts=PTS-STARTPTS",
            format_secs(start)
        ));
    }
    let speed = options.speed;
    if (speed - 1.0).abs() > 1e-9 {
        filters.push(format!("setpts=(PTS-STARTPTS)/{}", format_secs(speed)));
    }
    let source_fps = input.fps.map(|fps| fps * speed);
    if let Some(fps) = fps_filter(options.fps.or(default_fps), source_fps) {
        filters.push(fps);
    }
    filters.extend(picture_filters(input, options, default_max, false));
    match options.direction {
        Direction::Forward => {}
        Direction::Reverse => filters.push("reverse".to_owned()),
        // The way back skips both turnaround frames (the last one, and the
        // first, which the loop shows next), or the loop visibly stutters.
        Direction::Boomerang => filters.push(
            "split[fwd][back];[back]trim=start_frame=1,reverse,trim=start_frame=1,setpts=PTS-STARTPTS[rev];[fwd][rev]concat=n=2:v=1:a=0".to_owned(),
        ),
    }
    filters
}

/// Whether the options leave the picture exactly as it is: no trim, turn,
/// mirror, crop, downscale, frame-rate change or (for animations) new speed
/// or direction.
fn is_untouched(input: &MediaInfo, options: &ConvertOptions) -> bool {
    let untrimmed = options.trim_start.is_none_or(|s| s <= 0.0) && options.trim_end.is_none();
    let replayed = options.format.animates(input.kind) && options.changes_playback();
    (input.kind == MediaKind::Image || untrimmed)
        && !replayed
        && video_filters(input, options, None, false).is_empty()
}

/// Rough size of every decoded frame of one pass through an animation at 4
/// bytes per pixel, or `None` if the size, rate or length is unknown.
fn animation_bytes(
    input: &MediaInfo,
    options: &ConvertOptions,
    (default_max, default_fps): (Option<u32>, Option<f64>),
    (start, end): (f64, Option<f64>),
) -> Option<f64> {
    let (w, h) = output_dims(input, options, default_max)?;
    let speed = options.speed;
    let source_fps = input.fps? * speed;
    let fps = options
        .fps
        .or(default_fps)
        .map_or(source_fps, |fps| fps.min(source_fps));
    let length = (end.or(input.duration_secs)? - start) / speed;
    Some(f64::from(w) * f64::from(h) * 4.0 * fps * length.max(0.0))
}

/// Picture size after rotation, cropping and downscaling, if the source size
/// is known.
fn output_dims(
    input: &MediaInfo,
    options: &ConvertOptions,
    default_max: Option<u32>,
) -> Option<(u32, u32)> {
    let (w, h) = turned_dims(input, options)?;
    let (w, h) = options
        .crop
        .and_then(|crop| crop_box(w, h, &crop, false))
        .map_or((w, h), |b| (b.width, b.height));
    let target = options.max_size.or(default_max).map(|t| t.max(2) & !1);
    Some(match target {
        Some(t) if t < w.min(h) => {
            let scale = f64::from(t) / f64::from(w.min(h));
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let long = (f64::from(w.max(h)) * scale).round() as u32;
            if w >= h { (long, t) } else { (t, long) }
        }
        _ => (w, h),
    })
}

/// A hidden intermediate next to `output`, e.g. `.cat.jxl.frames.png`.
/// Unique because the output path is.
pub(crate) fn scratch_path(output: &Path, tag: &str, ext: &str) -> PathBuf {
    let name = output
        .file_name()
        .map_or_else(|| "output".into(), |n| n.to_string_lossy().into_owned());
    output.with_file_name(format!(".{name}.{tag}.{ext}"))
}

/// Pick a free output path next to `input`: `clip.mp4`, then `clip 2.mp4`, `clip 3.mp4`, …
///
/// `taken` lets the caller rule out paths that are reserved but not yet on disk.
#[must_use]
pub fn output_path(input: &Path, format: OutputFormat, taken: impl Fn(&Path) -> bool) -> PathBuf {
    free_path(input, &file_stem(input), format.extension(), taken)
}

/// Frame rate assumed for a video that doesn't say.
const FALLBACK_FPS: f64 = 30.0;

/// Plan saving the frame of `input` at `at_secs` as a PNG, with the crop,
/// turn, mirror, size and metadata choices of `options`. Their trim, frame
/// rate, speed, direction, sound and format don't apply.
///
/// ffmpeg takes the first frame at or after `at_secs`, as
/// [`crate::Ffmpeg::frame`] does for the preview, so the saved frame is the
/// one on screen. A time past the last frame saves the last frame.
///
/// # Errors
///
/// [`Error::Unsupported`] if `input` is not a video, `at_secs` is not a
/// positive number, or the options are invalid (see [`plan`]).
pub fn frame_plan(
    input: &MediaInfo,
    options: &ConvertOptions,
    at_secs: f64,
    output: &Path,
    support: impl Into<Support>,
) -> Result<Plan> {
    if input.kind != MediaKind::Video {
        return Err(Error::unsupported("only videos have frames to save"));
    }
    if !at_secs.is_finite() || at_secs < 0.0 {
        return Err(Error::unsupported(
            "the frame time must be a positive number",
        ));
    }
    // Seeking to the very end finds no frame to grab.
    let last = input
        .duration_secs
        .map(|d| (d - 1.0 / input.fps.unwrap_or(FALLBACK_FPS)).max(0.0));
    let at = last.map_or(at_secs, |last| at_secs.min(last));
    let still = ConvertOptions {
        format: OutputFormat::Png,
        trim_start: (at > 0.0).then_some(at),
        trim_end: None,
        fps: None,
        keep_audio: false,
        sound: SoundOptions::default(),
        speed: 1.0,
        direction: Direction::Forward,
        ..options.clone()
    };
    plan(input, &still, output, support)
}

/// Pick a free name for the frame at `at_secs` next to `input`:
/// `clip-frame-0m12s.png`, then `clip-frame-0m12s 2.png`, …
#[must_use]
pub fn frame_output_path(input: &Path, at_secs: f64, taken: impl Fn(&Path) -> bool) -> PathBuf {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let whole = at_secs.max(0.0).floor() as u64;
    let (h, m, s) = (whole / 3600, whole / 60 % 60, whole % 60);
    let stamp = if h > 0 {
        format!("{h}h{m:02}m{s:02}s")
    } else {
        format!("{m}m{s:02}s")
    };
    let stem = format!("{}-frame-{stamp}", file_stem(input));
    free_path(input, &stem, OutputFormat::Png.extension(), taken)
}

pub(crate) fn file_stem(input: &Path) -> String {
    input
        .file_stem()
        .map_or_else(|| "converted".into(), |s| s.to_string_lossy().into_owned())
}

/// `{stem}.{ext}` next to `input`, or `{stem} 2.{ext}`, `{stem} 3.{ext}`, …,
/// whichever is first neither `input`, on disk, nor `taken`.
pub(crate) fn free_path(
    input: &Path,
    stem: &str,
    ext: &str,
    taken: impl Fn(&Path) -> bool,
) -> PathBuf {
    let dir = input.parent().unwrap_or_else(|| Path::new(""));
    (1..)
        .map(|n| {
            let name = if n == 1 {
                format!("{stem}.{ext}")
            } else {
                format!("{stem} {n}.{ext}")
            };
            dir.join(name)
        })
        .find(|candidate| candidate != input && !candidate.exists() && !taken(candidate))
        .expect("an unbounded range always yields a free name")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn trim_range(input: &MediaInfo, options: &ConvertOptions) -> Result<(f64, Option<f64>)> {
    let start = options.trim_start.unwrap_or(0.0);
    let end = options.trim_end;
    if !start.is_finite() || start < 0.0 || end.is_some_and(|e| !e.is_finite()) {
        return Err(Error::unsupported("trim times must be positive numbers"));
    }
    if input
        .duration_secs
        .is_some_and(|duration| start >= duration)
    {
        return Err(Error::unsupported(
            "the trim starts after the end of the clip",
        ));
    }
    let end = end.map(|e| input.duration_secs.map_or(e, |d| e.min(d)));
    if end.is_some_and(|e| e - start < 0.05) {
        return Err(Error::unsupported("the trimmed clip is empty"));
    }
    Ok((start, end))
}

fn validate_crop(crop: &Crop) -> Result<()> {
    if crop
        .aspect
        .is_some_and(|[w, h]| w == 0 || h == 0 || w > 1000 || h > 1000)
    {
        return Err(Error::unsupported(
            "the crop shape must be two whole numbers like 9:16",
        ));
    }
    if let Some(a) = crop.area {
        let parts = [a.x, a.y, a.width, a.height];
        // A little slack for rounding in the front end.
        let inside = parts
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0 + 1e-6).contains(v))
            && a.x + a.width <= 1.0 + 1e-6
            && a.y + a.height <= 1.0 + 1e-6;
        if !inside || a.width <= 0.0 || a.height <= 0.0 {
            return Err(Error::unsupported("the crop must lie inside the picture"));
        }
    }
    Ok(())
}

/// Display size after our rotation, if known.
fn turned_dims(input: &MediaInfo, options: &ConvertOptions) -> Option<(u32, u32)> {
    let (w, h) = input.width.zip(input.height)?;
    Some(if options.rotate % 180 == 90 {
        (h, w)
    } else {
        (w, h)
    })
}

/// The crop in pixels of a `w`×`h` picture, or `None` when it keeps the
/// whole picture.
///
/// With an aspect, the largest box of that shape inside the area is kept,
/// centred on the area, so the result has the exact shape even when the
/// area's fractions round unevenly. `even` rounds the size down to even
/// numbers, which yuv420p encoders need.
fn crop_box(w: u32, h: u32, crop: &Crop, even: bool) -> Option<CropBox> {
    let area = crop.area.unwrap_or(CropArea::FULL);
    let (fw, fh) = (f64::from(w), f64::from(h));
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let px = |v: f64, max: u32| (v.round().max(1.0) as u32).min(max);
    let mut width = px(area.width * fw, w);
    let mut height = px(area.height * fh, h);
    if let Some([aw, ah]) = crop.aspect {
        if u64::from(width) * u64::from(ah) <= u64::from(height) * u64::from(aw) {
            height = px(f64::from(width) * f64::from(ah) / f64::from(aw), height);
        } else {
            width = px(f64::from(height) * f64::from(aw) / f64::from(ah), width);
        }
    }
    if (width, height) == (w, h) {
        return None;
    }
    if even {
        width = (width & !1).max(2).min(w);
        height = (height & !1).max(2).min(h);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let place = |start: f64, size: f64, kept: u32, total: u32| {
        let centre = (start + size / 2.0) * f64::from(total);
        ((centre - f64::from(kept) / 2.0).round().max(0.0) as u32).min(total - kept)
    };
    Some(CropBox {
        width,
        height,
        x: place(area.x, area.width, width, w),
        y: place(area.y, area.height, height, h),
    })
}

/// The `crop` filter for a picture of unknown size: the aspect's largest
/// centred box, or the area as fractions of the input.
fn crop_expression(crop: &Crop) -> Option<String> {
    let frac = |v: f64| {
        let s = format!("{:.6}", v.clamp(0.0, 1.0));
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    match (crop.aspect, crop.area) {
        (Some([w, h]), _) => Some(format!("crop='min(iw,ih*{w}/{h})':'min(ih,iw*{h}/{w})'")),
        (None, Some(a)) if a != CropArea::FULL => Some(format!(
            "crop='iw*{}':'ih*{}':'iw*{}':'ih*{}'",
            frac(a.width),
            frac(a.height),
            frac(a.x),
            frac(a.y)
        )),
        (None, _) => None,
    }
}

/// Rotate, flip, crop, downscale and frame-rate filters, in that order.
///
/// `default_max` caps the short side when the user picked no size.
/// `even` forces even dimensions, which yuv420p encoders require.
fn video_filters(
    input: &MediaInfo,
    options: &ConvertOptions,
    default_max: Option<u32>,
    even: bool,
) -> Vec<String> {
    let mut filters = picture_filters(input, options, default_max, even);
    filters.extend(fps_filter(options.fps, input.fps));
    filters
}

/// An `fps` filter for `target`, unless that wouldn't lower `source`.
fn fps_filter(target: Option<f64>, source: Option<f64>) -> Option<String> {
    target
        .filter(|f| f.is_finite() && *f > 0.0)
        .filter(|&fps| source.is_none_or(|source| fps < source - 0.01))
        .map(|fps| format!("fps={}", format_secs(fps)))
}

/// Rotate, flip, crop and downscale filters, in that order.
fn picture_filters(
    input: &MediaInfo,
    options: &ConvertOptions,
    default_max: Option<u32>,
    even: bool,
) -> Vec<String> {
    let mut filters = turn_filters(options);

    // Display size after our rotation, then after the crop.
    let mut dims = turned_dims(input, options);
    if let Some(crop) = &options.crop {
        match dims {
            Some((w, h)) => {
                if let Some(b) = crop_box(w, h, crop, even) {
                    filters.push(format!("crop={}:{}:{}:{}", b.width, b.height, b.x, b.y));
                    dims = Some((b.width, b.height));
                }
            }
            None => filters.extend(crop_expression(crop)),
        }
    }
    let target = options.max_size.or(default_max).map(|t| t.max(2) & !1);
    let scale = match (target, dims) {
        (Some(t), Some((w, h))) if t < w.min(h) => Some(if w >= h {
            format!("scale=-2:{t}")
        } else {
            format!("scale={t}:-2")
        }),
        (Some(_), Some(_)) => None,
        (Some(t), None) => Some(format!(
            "scale='if(gte(iw,ih),-2,min(iw,{t}))':'if(gte(iw,ih),min(ih,{t}),-2)'"
        )),
        (None, _) => None,
    };
    let lanczos = options.format.kind() == MediaKind::Image || options.format == OutputFormat::Gif;
    match scale {
        Some(scale) if lanczos => filters.push(format!("{scale}:flags=lanczos")),
        Some(scale) => filters.push(scale),
        None if even && dims.is_none_or(|(w, h)| w % 2 == 1 || h % 2 == 1) => {
            filters.push("scale=trunc(iw/2)*2:trunc(ih/2)*2".to_owned());
        }
        None => {}
    }
    filters
}

/// The user's quarter turns and mirroring, in that order.
pub(crate) fn turn_filters(options: &ConvertOptions) -> Vec<String> {
    let mut filters = Vec::new();
    match options.rotate {
        90 => filters.push("transpose=clock".to_owned()),
        180 => filters.push("hflip,vflip".to_owned()),
        270 => filters.push("transpose=cclock".to_owned()),
        _ => {}
    }
    if options.flip {
        filters.push("hflip".to_owned());
    }
    filters
}

/// Map quality 0–100 linearly onto `worst..=best` and round.
pub(crate) fn lerp(quality: u8, worst: f64, best: f64) -> i64 {
    #[allow(clippy::cast_possible_truncation)]
    let value = (worst + (best - worst) * f64::from(quality) / 100.0).round() as i64;
    value
}

fn tiered<T>(quality: u8, tiers: [T; 3]) -> T {
    let [low, mid, high] = tiers;
    match quality {
        0..34 => low,
        34..67 => mid,
        _ => high,
    }
}

fn kind_noun(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "an image",
        MediaKind::Video => "a video",
        MediaKind::Audio => "audio",
        MediaKind::Unknown => "this file",
    }
}

/// Program calls as shell-quoted lines joined with `&&`, the way
/// [`Plan::command_line`] shows them and [`crate::custom_plan`] reads them back.
pub(crate) fn render_command<A: AsRef<str>>(
    steps: impl IntoIterator<Item = (Tool, impl IntoIterator<Item = A>)>,
) -> String {
    steps
        .into_iter()
        .map(|(tool, args)| {
            std::iter::once(tool.name().to_owned())
                .chain(args.into_iter().map(|arg| shell_quote(arg.as_ref())))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(" && \\\n  ")
}

fn shell_quote(arg: &str) -> String {
    let safe = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=+,@%".contains(c));
    if safe {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(name: &str) -> MediaInfo {
        MediaInfo {
            path: PathBuf::from("/clips").join(name),
            file_name: name.into(),
            extension: Some("mov".into()),
            size_bytes: 48_000_000,
            kind: MediaKind::Video,
            duration_secs: Some(42.0),
            width: Some(1920),
            height: Some(1080),
            fps: Some(30.0),
            video_codec: Some("h264".into()),
            audio_codec: Some("aac".into()),
            sample_rate: Some(44_100),
            audio_channels: Some(2),
            bitrate: None,
            video_bitrate: None,
            audio_bitrate: None,
            reconstructs_jpeg: false,
            hdr: false,
            has_location: false,
            location_tags: Vec::new(),
            tile_grid: None,
            orientation: None,
        }
    }

    fn command(input: &MediaInfo, options: &ConvertOptions, out: &str) -> String {
        plan(input, options, Path::new(out), JxlSupport::default())
            .expect("plan")
            .command_line()
    }

    #[test]
    fn trimmed_720p_mp4() {
        let options = ConvertOptions {
            format: OutputFormat::Mp4,
            trim_start: Some(3.2),
            trim_end: Some(31.0),
            max_size: Some(720),
            ..ConvertOptions::default()
        };
        let plan = plan(
            &video("garden.mov"),
            &options,
            Path::new("/clips/garden.mp4"),
            JxlSupport::default(),
        )
        .expect("plan");
        assert_eq!(
            plan.command_line(),
            "ffmpeg -ss 3.2 -i /clips/garden.mov -t 27.8 -map 0:v:0 -map 0:a:0 -vf scale=-2:720 \
             -c:v libx264 -preset fast -crf 24 -pix_fmt yuv420p -c:a aac -b:a 128k -sn -dn \
             -movflags +faststart /clips/garden.mp4"
        );
        assert!((plan.duration_secs().unwrap() - 27.8).abs() < 1e-9);
    }

    #[test]
    fn untrimmed_plan_uses_full_duration_without_t() {
        let plan = plan(
            &video("a.mov"),
            &ConvertOptions::default(),
            Path::new("/clips/a.mp4"),
            JxlSupport::default(),
        )
        .expect("plan");
        assert!(!plan.steps[0].args.iter().any(|a| a == "-t" || a == "-ss"));
        assert_eq!(plan.duration_secs(), Some(42.0));
    }

    #[test]
    fn av1_mp4_is_ten_bit_svt_av1_with_aac() {
        let options = ConvertOptions {
            format: OutputFormat::Mp4,
            video_codec: Some(VideoCodec::Av1),
            ..ConvertOptions::default()
        };
        assert_eq!(
            command(&video("garden.mov"), &options, "/clips/garden.mp4"),
            "ffmpeg -i /clips/garden.mov -map 0:v:0 -map 0:a:0 \
             -c:v libsvtav1 -preset 8 -crf 32 -pix_fmt yuv420p10le -c:a aac -b:a 128k -sn -dn \
             -movflags +faststart /clips/garden.mp4"
        );
    }

    #[test]
    fn av1_webm_and_mkv_keep_their_sound_codecs() {
        let input = video("garden.mov");
        let webm = ConvertOptions {
            format: OutputFormat::Webm,
            video_codec: Some(VideoCodec::Av1),
            quality: 100,
            ..ConvertOptions::default()
        };
        let cmd = command(&input, &webm, "/clips/garden.webm");
        assert!(
            cmd.contains("-c:v libsvtav1 -preset 8 -crf 22 -pix_fmt yuv420p10le"),
            "{cmd}"
        );
        assert!(cmd.contains("-c:a libopus"), "{cmd}");
        let mkv = ConvertOptions {
            format: OutputFormat::Mkv,
            quality: 0,
            ..webm
        };
        let cmd = command(&input, &mkv, "/clips/garden.mkv");
        assert!(cmd.contains("-c:v libsvtav1 -preset 8 -crf 48"), "{cmd}");
        assert!(cmd.contains("-c:a aac"), "{cmd}");
        assert!(!cmd.contains("faststart"), "{cmd}");
    }

    #[test]
    fn video_codec_defaults_to_the_formats_usual_one() {
        let options = |format, video_codec| ConvertOptions {
            format,
            video_codec,
            ..ConvertOptions::default()
        };
        assert_eq!(
            options(OutputFormat::Mp4, None).codec(),
            Some(VideoCodec::H264)
        );
        assert_eq!(
            options(OutputFormat::Webm, None).codec(),
            Some(VideoCodec::Vp9)
        );
        assert_eq!(
            options(OutputFormat::Mkv, Some(VideoCodec::Av1)).codec(),
            Some(VideoCodec::Av1)
        );
        // Formats without a codec choice ignore it.
        assert_eq!(
            options(OutputFormat::Gif, Some(VideoCodec::Av1)).codec(),
            None
        );
        let cmd = command(
            &video("garden.mov"),
            &options(OutputFormat::Gif, Some(VideoCodec::Av1)),
            "/clips/garden.gif",
        );
        assert!(!cmd.contains("libsvtav1"), "{cmd}");
    }

    #[test]
    fn codecs_a_format_cant_hold_are_refused() {
        for (format, codec) in [
            (OutputFormat::Mov, VideoCodec::Av1),
            (OutputFormat::Mp4, VideoCodec::Vp9),
            (OutputFormat::Webm, VideoCodec::H264),
            (OutputFormat::Webm, VideoCodec::H265),
        ] {
            let options = ConvertOptions {
                format,
                video_codec: Some(codec),
                ..ConvertOptions::default()
            };
            let out = format!("/clips/garden.{}", format.extension());
            let err = plan(
                &video("garden.mov"),
                &options,
                Path::new(&out),
                JxlSupport::default(),
            )
            .expect_err("refused");
            assert!(
                err.to_string()
                    .contains(&format!("can't hold {} video", codec.label())),
                "{err}"
            );
        }
    }

    #[test]
    fn video_codec_parses_from_json() {
        let options: ConvertOptions =
            serde_json::from_value(serde_json::json!({"format": "webm", "videoCodec": "av1"}))
                .expect("parse");
        assert_eq!(options.video_codec, Some(VideoCodec::Av1));
        let options: ConvertOptions =
            serde_json::from_value(serde_json::json!({"format": "mp4", "videoCodec": null}))
                .expect("parse");
        assert_eq!(options.codec(), Some(VideoCodec::H264));
        for name in ["h265", "hevc"] {
            let options: ConvertOptions = serde_json::from_value(
                serde_json::json!({"format": "mov", "videoCodec": name, "hardware": true}),
            )
            .expect("parse");
            assert_eq!(options.codec(), Some(VideoCodec::H265), "{name}");
            assert!(options.hardware);
        }
        assert_eq!(
            serde_json::to_value(VideoCodec::H265).expect("json"),
            serde_json::json!("h265")
        );
    }

    fn h265(format: OutputFormat) -> ConvertOptions {
        ConvertOptions {
            format,
            video_codec: Some(VideoCodec::H265),
            ..ConvertOptions::default()
        }
    }

    /// Software and hardware H.265.
    const HEVC_HARDWARE: Support = Support {
        jxl: JxlSupport {
            cjxl: false,
            djxl: false,
            ffmpeg_encoder: false,
            ffmpeg_anim_encoder: false,
            ffmpeg_decoder: false,
            ffmpeg_anim_decoder: false,
        },
        tone_map: false,
        hevc_hardware: true,
    };

    #[test]
    fn h265_mp4_is_eight_bit_x265_tagged_hvc1() {
        assert_eq!(
            command(
                &video("garden.mov"),
                &h265(OutputFormat::Mp4),
                "/clips/garden.mp4"
            ),
            "ffmpeg -i /clips/garden.mov -map 0:v:0 -map 0:a:0 \
             -c:v libx265 -preset fast -crf 28 -pix_fmt yuv420p -c:a aac -b:a 128k -sn -dn \
             -tag:v hvc1 -movflags +faststart /clips/garden.mp4"
        );
        // MOV is tagged the same way; MKV has no such tags.
        let mov = command(
            &video("garden.mp4"),
            &h265(OutputFormat::Mov),
            "/clips/garden.mov",
        );
        assert!(mov.contains("-crf 28 -pix_fmt yuv420p "), "{mov}");
        assert!(
            mov.ends_with("-tag:v hvc1 -movflags +faststart /clips/garden.mov"),
            "{mov}"
        );
        let mkv = ConvertOptions {
            quality: 100,
            ..h265(OutputFormat::Mkv)
        };
        let mkv = command(&video("garden.mov"), &mkv, "/clips/garden.mkv");
        assert!(mkv.contains("-c:v libx265 -preset fast -crf 22 "), "{mkv}");
        assert!(!mkv.contains("hvc1") && !mkv.contains("faststart"), "{mkv}");
    }

    #[test]
    fn h265_keeps_hdr_in_main10() {
        let hdr = MediaInfo {
            hdr: true,
            ..video("iphone.mov")
        };
        let cmd = command(&hdr, &h265(OutputFormat::Mov), "/clips/iphone 2.mov");
        assert!(
            cmd.contains(
                "-c:v libx265 -preset fast -crf 28 -pix_fmt yuv420p10le -profile:v main10 "
            ),
            "{cmd}"
        );
        // HDR is never tone-mapped for video, and H.264 stays 8-bit as before.
        assert!(!cmd.contains("out_transfer"), "{cmd}");
        let h264 = ConvertOptions {
            format: OutputFormat::Mov,
            ..ConvertOptions::default()
        };
        let cmd = command(&hdr, &h264, "/clips/iphone 2.mov");
        assert!(cmd.contains("-pix_fmt yuv420p "), "{cmd}");
        assert!(!cmd.contains("main10") && !cmd.contains("hvc1"), "{cmd}");
    }

    #[test]
    fn exact_bitrate_for_h265_takes_two_passes_with_x265_stats() {
        let input = video("garden.mov");
        let options = ConvertOptions {
            video_bitrate: Some(3000),
            ..h265(OutputFormat::Mp4)
        };
        let plan = plan(
            &input,
            &options,
            Path::new("/clips/garden.mp4"),
            JxlSupport::default(),
        )
        .expect("plan");
        let [first, second] = [0, 1].map(|i| {
            plan.steps[i]
                .args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" ")
        });
        let log = "/clips/.garden.mp4.passlog.stats";
        assert!(
            first.contains(&format!(
                "-c:v libx265 -preset fast -b:v 3000k -pass 1 -passlogfile {log} \
                 -x265-stats {log}-0.x265.log -pix_fmt yuv420p -an -sn -dn -f null -"
            )),
            "{first}"
        );
        assert!(second.contains("-pass 2"), "{second}");
        assert!(
            second.contains(&format!("-x265-stats {log}-0.x265.log")),
            "{second}"
        );
        assert!(second.contains("-tag:v hvc1"), "{second}");
        assert_eq!(
            plan.scratch,
            [
                "-0.log",
                "-0.log.temp",
                "-0.log.mbtree",
                "-0.log.mbtree.temp",
                "-0.x265.log",
                "-0.x265.log.temp",
                "-0.x265.log.cutree",
                "-0.x265.log.cutree.temp",
            ]
            .map(|suffix| PathBuf::from(format!("{log}{suffix}")))
        );
        assert_eq!(
            quality_summary(&input, &options, JxlSupport::default()).video,
            Some(VideoRate {
                codec: VideoCodec::H265,
                crf: None,
                kbps: Some(3000),
                passes: 2,
                hardware: false,
                hardware_quality: None,
            })
        );
    }

    #[test]
    fn hardware_h265_uses_videotoolbox_where_there_is_one() {
        let input = video("garden.mov");
        let fast = ConvertOptions {
            hardware: true,
            ..h265(OutputFormat::Mp4)
        };
        let run = |input: &MediaInfo, options: &ConvertOptions, support: Support| {
            plan(input, options, Path::new("/clips/garden.mp4"), support).expect("plan")
        };
        let cmd = run(&input, &fast, HEVC_HARDWARE).command_line();
        assert!(
            cmd.contains("-c:v hevc_videotoolbox -q:v 64 -pix_fmt yuv420p -c:a aac"),
            "{cmd}"
        );
        assert!(cmd.contains("-tag:v hvc1 -movflags +faststart"), "{cmd}");
        assert_eq!(
            quality_summary(&input, &fast, HEVC_HARDWARE).video,
            Some(VideoRate {
                codec: VideoCodec::H265,
                crf: None,
                kbps: None,
                passes: 1,
                hardware: true,
                hardware_quality: Some(64),
            })
        );

        // HDR stays HDR in 10 bits.
        let hdr = MediaInfo {
            hdr: true,
            ..video("iphone.mov")
        };
        let cmd = run(&hdr, &fast, HEVC_HARDWARE).command_line();
        assert!(
            cmd.contains("-c:v hevc_videotoolbox -q:v 64 -pix_fmt p010le -profile:v main10"),
            "{cmd}"
        );

        // An exact bitrate is one pass: VideoToolbox aims for it by itself.
        let exact = ConvertOptions {
            video_bitrate: Some(5000),
            quality: 100,
            ..fast.clone()
        };
        let plan = run(&input, &exact, HEVC_HARDWARE);
        assert_eq!(plan.steps.len(), 1);
        assert!(plan.scratch.is_empty());
        assert!(
            plan.command_line()
                .contains("-c:v hevc_videotoolbox -b:v 5000k -pix_fmt yuv420p"),
            "{}",
            plan.command_line()
        );

        // Without a hardware encoder it falls back to x265, and other codecs
        // ignore the setting.
        let cmd = run(&input, &fast, Support::default()).command_line();
        assert!(cmd.contains("-c:v libx265 -preset fast -crf 28"), "{cmd}");
        let h264 = ConvertOptions {
            video_codec: None,
            ..fast.clone()
        };
        let cmd = run(&input, &h264, HEVC_HARDWARE).command_line();
        assert!(cmd.contains("-c:v libx264"), "{cmd}");
        assert!(
            !quality_summary(&input, &fast, Support::default())
                .video
                .expect("video")
                .hardware
        );
    }

    #[test]
    fn muted_rotated_portrait_webm() {
        let mut input = video("phone.mp4");
        (input.width, input.height) = (Some(1080), Some(1920));
        let options = ConvertOptions {
            format: OutputFormat::Webm,
            rotate: 90,
            flip: true,
            max_size: Some(480),
            fps: Some(24.0),
            keep_audio: false,
            quality: 100,
            ..ConvertOptions::default()
        };
        let cmd = command(&input, &options, "/clips/phone.webm");
        // Rotated 90° the portrait clip is landscape, so height is the short side.
        assert!(
            cmd.contains("-vf transpose=clock,hflip,scale=-2:480,fps=24 "),
            "{cmd}"
        );
        assert!(cmd.contains("-c:v libvpx-vp9 -crf 28 -b:v 0"), "{cmd}");
        assert!(cmd.contains(" -an "), "{cmd}");
        assert!(!cmd.contains("0:a:0"), "{cmd}");
    }

    fn cropped(aspect: Option<[u32; 2]>, area: Option<[f64; 4]>) -> Option<Crop> {
        Some(Crop {
            aspect,
            area: area.map(|[x, y, width, height]| CropArea {
                x,
                y,
                width,
                height,
            }),
        })
    }

    #[test]
    fn vertical_crop_keeps_the_centre_and_skips_needless_scaling() {
        let options = ConvertOptions {
            crop: cropped(Some([9, 16]), None),
            max_size: Some(720),
            ..ConvertOptions::default()
        };
        let cmd = command(&video("wide.mov"), &options, "/clips/wide.mp4");
        // 1080 × 9/16 = 607.5, rounded to 608 and centred; 608 is already under 720.
        assert!(cmd.contains(" -vf crop=608:1080:656:0 "), "{cmd}");
    }

    #[test]
    fn crop_follows_turn_and_mirror_and_stays_exact() {
        let mut input = video("phone.mp4");
        (input.width, input.height) = (Some(1080), Some(1920));
        let options = ConvertOptions {
            format: OutputFormat::Webm,
            rotate: 90,
            flip: true,
            // A hair short of square at the right edge: 1080×1079 px.
            crop: cropped(Some([1, 1]), Some([0.4374, 0.0, 0.5626, 0.999])),
            max_size: Some(480),
            ..ConvertOptions::default()
        };
        let cmd = command(&input, &options, "/clips/phone.webm");
        assert!(
            // Square, rounded to even sizes, and kept inside the picture.
            cmd.contains(" -vf transpose=clock,hflip,crop=1078:1078:841:0,scale=-2:480 "),
            "{cmd}"
        );
    }

    #[test]
    fn free_crop_rounds_to_even_sizes_for_video_but_not_png() {
        let crop = cropped(None, Some([0.1, 0.2, 0.3001, 0.4]));
        let mp4 = ConvertOptions {
            crop,
            ..ConvertOptions::default()
        };
        let cmd = command(&video("a.mov"), &mp4, "/clips/a.mp4");
        // 0.3001 × 1920 = 576.2 → 576; 0.4 × 1080 = 432.
        assert!(cmd.contains(" -vf crop=576:432:192:216 "), "{cmd}");

        let mut photo = video("odd.png");
        photo.kind = MediaKind::Image;
        (photo.width, photo.height) = (Some(1001), Some(1001));
        let png = ConvertOptions {
            format: OutputFormat::Png,
            crop: cropped(None, Some([0.0, 0.0, 0.5, 1.0])),
            ..ConvertOptions::default()
        };
        let cmd = command(&photo, &png, "/clips/odd 2.png");
        assert!(cmd.contains(" -vf crop=501:1001:0:0 "), "{cmd}");
    }

    #[test]
    fn crop_of_the_whole_picture_changes_nothing() {
        let options = ConvertOptions {
            crop: cropped(Some([16, 9]), None),
            ..ConvertOptions::default()
        };
        let cmd = command(&video("a.mov"), &options, "/clips/a.mp4");
        assert!(!cmd.contains("-vf"), "{cmd}");
        let free = ConvertOptions {
            crop: cropped(None, None),
            ..ConvertOptions::default()
        };
        assert!(!command(&video("a.mov"), &free, "/clips/a.mp4").contains("-vf"));
    }

    #[test]
    fn crop_without_a_known_size_uses_expressions() {
        let mut input = video("mystery.mov");
        (input.width, input.height) = (None, None);
        let options = ConvertOptions {
            crop: cropped(Some([1, 1]), Some([0.2, 0.0, 0.5, 1.0])),
            ..ConvertOptions::default()
        };
        let cmd = command(&input, &options, "/clips/mystery.mp4");
        assert!(
            cmd.contains(
                "-vf 'crop='\\''min(iw,ih*1/1)'\\'':'\\''min(ih,iw*1/1)'\\'',scale=trunc(iw/2)*2:trunc(ih/2)*2'"
            ),
            "{cmd}"
        );
        let free = ConvertOptions {
            crop: cropped(None, Some([0.25, 0.0, 0.5, 1.0])),
            ..ConvertOptions::default()
        };
        let plan = plan(
            &input,
            &free,
            Path::new("/clips/m.mp4"),
            JxlSupport::default(),
        )
        .expect("plan");
        let vf = plan.steps[0]
            .args
            .iter()
            .skip_while(|a| *a != "-vf")
            .nth(1)
            .expect("filters");
        assert_eq!(
            vf.to_string_lossy(),
            "crop='iw*0.5':'ih*1':'iw*0.25':'ih*0',scale=trunc(iw/2)*2:trunc(ih/2)*2"
        );
    }

    #[test]
    fn gif_caps_the_size_of_the_cropped_picture() {
        let options = ConvertOptions {
            format: OutputFormat::Gif,
            crop: cropped(Some([9, 16]), None),
            ..ConvertOptions::default()
        };
        let cmd = command(&video("a.mov"), &options, "/clips/a.gif");
        assert!(
            cmd.contains("-vf 'fps=15,crop=608:1080:656:0,scale=480:-2:flags=lanczos,split"),
            "{cmd}"
        );
    }

    #[test]
    fn rejects_crops_outside_the_picture_or_without_a_shape() {
        for crop in [
            cropped(None, Some([0.6, 0.0, 0.5, 1.0])),
            cropped(None, Some([0.0, 0.0, 0.0, 1.0])),
            cropped(None, Some([f64::NAN, 0.0, 0.5, 0.5])),
            cropped(Some([0, 1]), None),
        ] {
            let options = ConvertOptions {
                crop,
                ..ConvertOptions::default()
            };
            assert!(
                plan(
                    &video("a.mov"),
                    &options,
                    Path::new("/clips/a.mp4"),
                    JxlSupport::default()
                )
                .is_err(),
                "{crop:?}"
            );
        }
    }

    #[test]
    fn reads_crops_from_the_front_end() {
        let options: ConvertOptions = serde_json::from_value(serde_json::json!({
            "format": "mp4",
            "crop": {"aspect": [9, 16], "area": null}
        }))
        .expect("parse");
        assert_eq!(options.crop, cropped(Some([9, 16]), None));
        let options: ConvertOptions =
            serde_json::from_value(serde_json::json!({"format": "mp4"})).expect("parse");
        assert_eq!(options.crop, None);
    }

    #[test]
    fn never_upscales_or_raises_fps() {
        let mut input = video("small.mov");
        (input.width, input.height) = (Some(640), Some(360));
        let options = ConvertOptions {
            max_size: Some(720),
            fps: Some(60.0),
            ..ConvertOptions::default()
        };
        let cmd = command(&input, &options, "/clips/small.mp4");
        assert!(!cmd.contains("-vf"), "{cmd}");
    }

    #[test]
    fn odd_dimensions_get_evened_for_h264() {
        let mut input = video("odd.mov");
        (input.width, input.height) = (Some(641), Some(361));
        let cmd = command(&input, &ConvertOptions::default(), "/clips/odd.mp4");
        assert!(
            cmd.contains("-vf 'scale=trunc(iw/2)*2:trunc(ih/2)*2'"),
            "{cmd}"
        );
    }

    #[test]
    fn gif_defaults_to_480_and_15fps_with_palette() {
        let options = ConvertOptions {
            format: OutputFormat::Gif,
            ..ConvertOptions::default()
        };
        let cmd = command(&video("loop.mov"), &options, "/clips/loop.gif");
        assert!(
            cmd.contains("-vf 'fps=15,scale=-2:480:flags=lanczos,split[a][b]"),
            "{cmd}"
        );
        assert!(cmd.contains("palettegen=max_colors=166"), "{cmd}");
        assert!(
            cmd.ends_with("-loop 0 -an -sn -dn /clips/loop.gif"),
            "{cmd}"
        );
    }

    #[test]
    fn video_becomes_an_animated_webp_with_caps() {
        let options = ConvertOptions {
            format: OutputFormat::Webp,
            trim_end: Some(4.0),
            ..ConvertOptions::default()
        };
        assert!(OutputFormat::Webp.animates(MediaKind::Video));
        assert!(!OutputFormat::Webp.animates(MediaKind::Image));
        assert_eq!(OutputFormat::Webp.animation_encoder(), "libwebp_anim");
        let plan = plan(
            &video("loop.mov"),
            &options,
            Path::new("/clips/loop.webp"),
            JxlSupport::default(),
        )
        .expect("plan");
        let cmd = plan.command_line();
        assert!(
            cmd.contains("-vf fps=24,scale=-2:480:flags=lanczos -c:v libwebp_anim -quality 73 -loop 0 -an -sn -dn"),
            "{cmd}"
        );
        assert_eq!(plan.duration_secs(), Some(4.0));

        // Played as a boomerang like GIFs and animated JPEG XL.
        let boomerang = ConvertOptions {
            direction: Direction::Boomerang,
            ..options.clone()
        };
        let cmd = command(&video("loop.mov"), &boomerang, "/clips/loop.webp");
        assert!(cmd.contains("reverse"), "{cmd}");

        // A picture still makes a still.
        let cmd = command(&image("cat.png", 800, 600), &options, "/clips/cat.webp");
        assert!(
            cmd.contains("-frames:v 1 -c:v libwebp -quality 73"),
            "{cmd}"
        );
    }

    fn gif(patch: ConvertOptions) -> ConvertOptions {
        ConvertOptions {
            format: OutputFormat::Gif,
            ..patch
        }
    }

    #[test]
    fn fast_boomerang_gif() {
        let options = gif(ConvertOptions {
            trim_start: Some(2.0),
            trim_end: Some(8.0),
            speed: 2.0,
            direction: Direction::Boomerang,
            ..ConvertOptions::default()
        });
        let plan = plan(
            &video("cat.mov"),
            &options,
            Path::new("/clips/cat.gif"),
            JxlSupport::default(),
        )
        .expect("plan");
        let cmd = plan.command_line();
        // The range is cut on the input, since the output is retimed.
        assert!(
            cmd.starts_with("ffmpeg -ss 2 -t 6 -i /clips/cat.mov -map 0:v:0 -vf "),
            "{cmd}"
        );
        // Speed → fps cap → shrink → boomerang → palette.
        assert!(
            cmd.contains(
                "'setpts=(PTS-STARTPTS)/2,fps=15,scale=-2:480:flags=lanczos,\
                 split[fwd][back];[back]trim=start_frame=1,reverse,trim=start_frame=1,setpts=PTS-STARTPTS[rev];\
                 [fwd][rev]concat=n=2:v=1:a=0,split[a][b];"
            ),
            "{cmd}"
        );
        // 6 s at 2× is 3 s, played there and back.
        assert_eq!(plan.duration_secs(), Some(6.0));
    }

    #[test]
    fn slow_reversed_gif_keeps_the_source_rate() {
        // At 0.5× a 24 fps clip plays at 12 fps, under the 15 fps cap.
        let input = MediaInfo {
            fps: Some(24.0),
            ..video("cat.mov")
        };
        let options = gif(ConvertOptions {
            speed: 0.5,
            direction: Direction::Reverse,
            ..ConvertOptions::default()
        });
        let plan = plan(
            &input,
            &options,
            Path::new("/clips/cat.gif"),
            JxlSupport::default(),
        )
        .expect("plan");
        let cmd = plan.command_line();
        assert!(
            cmd.contains("-vf 'setpts=(PTS-STARTPTS)/0.5,scale=-2:480:flags=lanczos,reverse,split"),
            "{cmd}"
        );
        assert_eq!(plan.duration_secs(), Some(84.0));
    }

    #[test]
    fn playback_is_ignored_for_videos_and_checked_for_animations() {
        let options = ConvertOptions {
            speed: 2.0,
            direction: Direction::Reverse,
            ..ConvertOptions::default()
        };
        let cmd = command(&video("a.mov"), &options, "/clips/a.mp4");
        assert!(!cmd.contains("setpts") && !cmd.contains("reverse"), "{cmd}");

        for speed in [0.1, 8.0, f64::NAN] {
            let err = plan_err(
                &video("a.mov"),
                &gif(ConvertOptions {
                    speed,
                    ..ConvertOptions::default()
                }),
                JxlSupport::default(),
            );
            assert!(err.contains("speed"), "{err}");
        }
    }

    #[test]
    fn long_clips_are_refused_backwards() {
        let input = MediaInfo {
            duration_secs: Some(3600.0),
            ..video("long.mov")
        };
        let backwards = gif(ConvertOptions {
            direction: Direction::Reverse,
            ..ConvertOptions::default()
        });
        let err = plan_err(&input, &backwards, JxlSupport::default());
        assert!(err.contains("backwards"), "{err}");

        // Trimmed, or sped up enough, it fits.
        let trimmed = ConvertOptions {
            trim_end: Some(60.0),
            ..backwards.clone()
        };
        assert!(
            plan(
                &input,
                &trimmed,
                Path::new("/clips/a.gif"),
                JxlSupport::default()
            )
            .is_ok()
        );
        let fast = ConvertOptions {
            trim_end: Some(600.0),
            speed: 4.0,
            ..backwards
        };
        assert!(
            plan(
                &input,
                &fast,
                Path::new("/clips/a.gif"),
                JxlSupport::default()
            )
            .is_ok()
        );
        // Forward needs no buffer.
        assert!(
            plan(
                &input,
                &gif(ConvertOptions::default()),
                Path::new("/clips/a.gif"),
                JxlSupport::default()
            )
            .is_ok()
        );
    }

    #[test]
    fn audio_extraction() {
        let options = ConvertOptions {
            format: OutputFormat::Mp3,
            trim_end: Some(10.0),
            quality: 100,
            ..ConvertOptions::default()
        };
        let plan = plan(
            &video("talk.mov"),
            &options,
            Path::new("/clips/talk.mp3"),
            JxlSupport::default(),
        )
        .expect("plan");
        assert_eq!(
            plan.command_line(),
            "ffmpeg -i /clips/talk.mov -t 10 -map 0:a:0 -vn -sn -dn -c:a libmp3lame -q:a 0 /clips/talk.mp3"
        );
        assert_eq!(plan.duration_secs(), Some(10.0));
    }

    #[test]
    fn audio_from_silent_video_is_rejected() {
        let mut input = video("silent.mov");
        input.audio_codec = None;
        let options = ConvertOptions {
            format: OutputFormat::Mp3,
            ..ConvertOptions::default()
        };
        let err = plan(
            &input,
            &options,
            Path::new("/clips/silent.mp3"),
            JxlSupport::default(),
        )
        .expect_err("no audio");
        assert!(matches!(err, Error::Unsupported { .. }), "{err:?}");
    }

    #[test]
    fn image_to_jpg_ignores_trim() {
        let input = MediaInfo {
            kind: MediaKind::Image,
            duration_secs: None,
            fps: None,
            audio_codec: None,
            width: Some(4000),
            height: Some(3000),
            ..video("photo.png")
        };
        let options = ConvertOptions {
            format: OutputFormat::Jpg,
            max_size: Some(1080),
            rotate: 180,
            trim_start: Some(2.0),
            quality: 100,
            ..ConvertOptions::default()
        };
        let plan = plan(
            &input,
            &options,
            Path::new("/clips/photo.jpg"),
            JxlSupport::default(),
        )
        .expect("plan");
        assert_eq!(
            plan.command_line(),
            "ffmpeg -i /clips/photo.png -map 0:v:0 -frames:v 1 -vf hflip,vflip,scale=-2:1080:flags=lanczos \
             -c:v mjpeg -q:v 2 -update 1 -an -sn -dn /clips/photo.jpg"
        );
        assert_eq!(plan.duration_secs(), None);
    }

    #[test]
    fn rejects_bad_requests() {
        let input = video("a.mov");
        let bad = [
            ConvertOptions {
                trim_start: Some(50.0),
                ..ConvertOptions::default()
            },
            ConvertOptions {
                trim_start: Some(5.0),
                trim_end: Some(5.0),
                ..ConvertOptions::default()
            },
            ConvertOptions {
                trim_start: Some(-1.0),
                ..ConvertOptions::default()
            },
            ConvertOptions {
                rotate: 45,
                ..ConvertOptions::default()
            },
        ];
        for options in bad {
            assert!(
                plan(
                    &input,
                    &options,
                    Path::new("/clips/a.mp4"),
                    JxlSupport::default()
                )
                .is_err(),
                "{options:?}"
            );
        }
        let image = MediaInfo {
            kind: MediaKind::Image,
            ..video("p.png")
        };
        assert!(
            plan(
                &image,
                &ConvertOptions::default(),
                Path::new("/clips/p.mp4"),
                JxlSupport::default(),
            )
            .is_err()
        );
        assert!(
            plan(
                &input,
                &ConvertOptions {
                    format: OutputFormat::Mov,
                    ..ConvertOptions::default()
                },
                &input.path,
                JxlSupport::default(),
            )
            .is_err()
        );
    }

    #[test]
    fn output_paths_avoid_existing_and_reserved_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let input = dir.path().join("clip.mp4");
        std::fs::write(&input, b"x").expect("write");

        // Same extension as the input: skip the input itself.
        assert_eq!(
            output_path(&input, OutputFormat::Mp4, |_| false),
            dir.path().join("clip 2.mp4")
        );
        assert_eq!(
            output_path(&input, OutputFormat::Gif, |_| false),
            dir.path().join("clip.gif")
        );

        std::fs::write(dir.path().join("clip.gif"), b"x").expect("write");
        let reserved = dir.path().join("clip 2.gif");
        assert_eq!(
            output_path(&input, OutputFormat::Gif, |p| p == reserved),
            dir.path().join("clip 3.gif")
        );
    }

    #[test]
    fn options_deserialize_with_defaults() {
        let options: ConvertOptions =
            serde_json::from_value(serde_json::json!({"format": "gif", "trimStart": 1.5}))
                .expect("parse");
        assert_eq!(options.format, OutputFormat::Gif);
        assert_eq!(options.trim_start, Some(1.5));
        assert_eq!(options.quality, 60);
        assert!(options.keep_audio);
        assert!((options.speed - 1.0).abs() < f64::EPSILON);
        assert_eq!(options.direction, Direction::Forward);

        let options: ConvertOptions =
            serde_json::from_value(serde_json::json!({"speed": 0.5, "direction": "boomerang"}))
                .expect("parse");
        assert_eq!(options.direction, Direction::Boomerang);
    }

    // -----------------------------------------------------------------------
    // Sound clean-up
    // -----------------------------------------------------------------------

    fn sound(normalize: bool, fade_in: f64, fade_out: f64) -> SoundOptions {
        SoundOptions {
            normalize,
            fade_in,
            fade_out,
        }
    }

    #[test]
    fn trimmed_song_is_normalized_and_faded() {
        let options = ConvertOptions {
            format: OutputFormat::Mp3,
            trim_start: Some(2.0),
            trim_end: Some(12.0),
            quality: 100,
            sound: sound(true, 1.0, 3.0),
            ..ConvertOptions::default()
        };
        assert_eq!(
            command(&video("talk.mov"), &options, "/clips/talk.mp3"),
            "ffmpeg -ss 2 -i /clips/talk.mov -t 10 -map 0:a:0 -vn -sn -dn \
             -af loudnorm=I=-16:TP=-1.5:LRA=11,afade=t=in:d=1,afade=t=out:st=7:d=3,aresample=44100 \
             -c:a libmp3lame -q:a 0 /clips/talk.mp3"
        );
    }

    #[test]
    fn video_fades_picture_and_sound_together() {
        let options = ConvertOptions {
            sound: sound(false, 1.0, 1.0),
            ..ConvertOptions::default()
        };
        let cmd = command(&video("garden.mov"), &options, "/clips/garden.mp4");
        assert!(
            cmd.contains(
                "-vf fade=t=in:d=1,fade=t=out:st=41:d=1 -af afade=t=in:d=1,afade=t=out:st=41:d=1 "
            ),
            "{cmd}"
        );

        // Without sound the section is hidden, so nothing fades.
        let muted = ConvertOptions {
            keep_audio: false,
            ..options
        };
        let cmd = command(&video("garden.mov"), &muted, "/clips/garden.mp4");
        assert!(!cmd.contains("fade"), "{cmd}");
        let gif = ConvertOptions {
            format: OutputFormat::Gif,
            ..options
        };
        let cmd = command(&video("garden.mov"), &gif, "/clips/garden.gif");
        assert!(!cmd.contains("fade="), "{cmd}");
    }

    #[test]
    fn fades_fit_the_clip_and_need_a_known_length() {
        let options = ConvertOptions {
            format: OutputFormat::Ogg,
            trim_end: Some(2.0),
            sound: sound(false, 3.0, 3.0),
            ..ConvertOptions::default()
        };
        let cmd = command(&video("short.mov"), &options, "/clips/short.ogg");
        assert!(
            cmd.contains("-af afade=t=in:d=2,afade=t=out:st=0:d=2 "),
            "{cmd}"
        );

        // Unknown length: fade in only, and the default 48 kHz after loudnorm.
        let input = MediaInfo {
            duration_secs: None,
            sample_rate: None,
            ..video("stream.mkv")
        };
        let options = ConvertOptions {
            format: OutputFormat::Flac,
            sound: sound(true, 1.0, 3.0),
            ..ConvertOptions::default()
        };
        let cmd = command(&input, &options, "/clips/stream.flac");
        assert!(
            cmd.contains("-af loudnorm=I=-16:TP=-1.5:LRA=11,afade=t=in:d=1,aresample=48000 "),
            "{cmd}"
        );
    }

    #[test]
    fn rejects_bad_fades_and_reads_old_recipes() {
        let options = ConvertOptions {
            format: OutputFormat::Mp3,
            sound: sound(false, -1.0, 0.0),
            ..ConvertOptions::default()
        };
        let err = plan_err(&video("a.mov"), &options, JxlSupport::default());
        assert!(err.contains("fade"), "{err}");

        let old: ConvertOptions =
            serde_json::from_value(serde_json::json!({"format": "mp3", "keepAudio": true}))
                .expect("parse");
        assert_eq!(old.sound, SoundOptions::default());
        let partial: ConvertOptions =
            serde_json::from_value(serde_json::json!({"format": "mp3", "sound": {"fadeIn": 1}}))
                .expect("parse");
        assert_eq!(partial.sound, sound(false, 1.0, 0.0));
    }

    // -----------------------------------------------------------------------
    // JPEG XL
    // -----------------------------------------------------------------------

    const LIBJXL: JxlSupport = JxlSupport {
        cjxl: true,
        djxl: true,
        ffmpeg_encoder: false,
        ffmpeg_anim_encoder: false,
        ffmpeg_decoder: false,
        ffmpeg_anim_decoder: false,
    };

    const FFMPEG_JXL: JxlSupport = JxlSupport {
        cjxl: false,
        djxl: false,
        ffmpeg_encoder: true,
        ffmpeg_anim_encoder: true,
        ffmpeg_decoder: true,
        ffmpeg_anim_decoder: true,
    };

    fn image(name: &str, w: u32, h: u32) -> MediaInfo {
        MediaInfo {
            kind: MediaKind::Image,
            extension: Path::new(name)
                .extension()
                .map(|e| e.to_string_lossy().into_owned()),
            duration_secs: None,
            fps: None,
            audio_codec: None,
            width: Some(w),
            height: Some(h),
            video_codec: None,
            ..video(name)
        }
    }

    fn jxl(options: ConvertOptions) -> ConvertOptions {
        ConvertOptions {
            format: OutputFormat::Jxl,
            ..options
        }
    }

    fn tools(plan: &Plan) -> Vec<Tool> {
        plan.steps.iter().map(|s| s.tool).collect()
    }

    #[test]
    fn unedited_jpeg_is_recompressed_losslessly_by_cjxl() {
        let options = jxl(ConvertOptions {
            jxl: JxlOptions {
                lossless: true,
                effort: 9,
                progressive: true,
            },
            ..ConvertOptions::default()
        });
        let plan = plan(
            &image("photo.jpg", 4000, 3000),
            &options,
            Path::new("/clips/photo.jxl"),
            LIBJXL,
        )
        .expect("plan");
        assert_eq!(
            plan.command_line(),
            "cjxl /clips/photo.jpg /clips/photo.jxl --lossless_jpeg=1 -e 9 -p --quiet"
        );
        assert!(plan.scratch.is_empty());

        // Lossy JPEG input is decoded and re-encoded at the chosen distance.
        let lossy = jxl(ConvertOptions {
            quality: 60,
            ..ConvertOptions::default()
        });
        let cmd = command_with(
            &image("photo.jpg", 400, 300),
            &lossy,
            "/clips/photo.jxl",
            LIBJXL,
        );
        assert_eq!(
            cmd,
            "cjxl /clips/photo.jpg /clips/photo.jxl --lossless_jpeg=0 -d 1.5 -e 7 --quiet"
        );
    }

    #[test]
    fn video_metadata_is_dropped_by_ffmpeg() {
        let mut input = video("garden.mov");
        input.location_tags = vec!["location".into(), "location-eng".into()];
        input.has_location = true;
        let with = |metadata| ConvertOptions {
            metadata,
            ..ConvertOptions::default()
        };

        let cmd = command(&input, &with(Metadata::None), "/clips/garden.mp4");
        assert!(
            cmd.ends_with("-map_metadata -1 -map_chapters -1 /clips/garden.mp4"),
            "{cmd}"
        );
        let cmd = command(&input, &with(Metadata::NoLocation), "/clips/garden.mp4");
        assert!(
            cmd.ends_with(
                "-movflags +faststart -metadata location= -metadata location-eng= /clips/garden.mp4"
            ),
            "{cmd}"
        );
        let cmd = command(&input, &with(Metadata::Keep), "/clips/garden.mp4");
        assert!(cmd.ends_with("+faststart /clips/garden.mp4"), "{cmd}");
        // Location is the default thing to drop.
        assert_eq!(ConvertOptions::default().metadata, Metadata::NoLocation);
    }

    fn scrub_args(plan: &Plan, index: usize) -> Vec<String> {
        assert_eq!(plan.steps[index].tool, Tool::Scrub);
        plan.steps[index]
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn jpeg_repack_survives_dropping_metadata() {
        let lossless = |metadata| {
            jxl(ConvertOptions {
                jxl: JxlOptions {
                    lossless: true,
                    ..JxlOptions::default()
                },
                metadata,
                ..ConvertOptions::default()
            })
        };
        let mut photo = image("photo.jpg", 4000, 3000);
        let out = Path::new("/clips/photo.jxl");

        // Nothing to hide: cjxl reads the JPEG itself.
        let planned = plan(&photo, &lossless(Metadata::NoLocation), out, LIBJXL).expect("plan");
        assert_eq!(tools(&planned), [Tool::Cjxl]);

        // A located photo is copied without its GPS, then repacked losslessly.
        photo.has_location = true;
        for (metadata, what) in [(Metadata::NoLocation, "location"), (Metadata::None, "all")] {
            let planned = plan(&photo, &lossless(metadata), out, LIBJXL).expect("plan");
            assert_eq!(tools(&planned), [Tool::Scrub, Tool::Cjxl]);
            assert_eq!(
                scrub_args(&planned, 0),
                [
                    "/clips/photo.jpg",
                    "/clips/.photo.jxl.clean.jpg",
                    "--drop",
                    what
                ]
            );
            assert_eq!(
                planned.steps[1].args[..3],
                [
                    "/clips/.photo.jxl.clean.jpg",
                    "/clips/photo.jxl",
                    "--lossless_jpeg=1"
                ]
                .map(OsString::from)
            );
            assert_eq!(
                planned.scratch,
                [PathBuf::from("/clips/.photo.jxl.clean.jpg")]
            );
        }
        assert!(
            plan(&photo, &lossless(Metadata::None), out, LIBJXL)
                .unwrap()
                .command_line()
                .starts_with(
                    "catconvert scrub /clips/photo.jpg /clips/.photo.jxl.clean.jpg --drop all && "
                ),
        );
    }

    #[test]
    fn png_output_is_cleaned_after_ffmpeg() {
        let mut photo = image("photo.jpg", 400, 300);
        let png = ConvertOptions {
            format: OutputFormat::Png,
            ..ConvertOptions::default()
        };
        let out = Path::new("/clips/photo.png");
        let plan_for = |photo: &MediaInfo, options: &ConvertOptions| {
            plan(photo, options, out, JxlSupport::default()).expect("plan")
        };
        assert_eq!(tools(&plan_for(&photo, &png)), [Tool::Ffmpeg]);

        photo.has_location = true;
        let planned = plan_for(&photo, &png);
        assert_eq!(tools(&planned), [Tool::Ffmpeg, Tool::Scrub]);
        assert_eq!(
            planned.steps[0].args.last(),
            Some(&OsString::from("/clips/.photo.png.encoded.png"))
        );
        assert_eq!(
            scrub_args(&planned, 1),
            [
                "/clips/.photo.png.encoded.png",
                "/clips/photo.png",
                "--drop",
                "location"
            ]
        );

        // JPG, WebP and AVIF come out of ffmpeg without EXIF anyway.
        for format in [OutputFormat::Jpg, OutputFormat::Webp, OutputFormat::Avif] {
            let options = ConvertOptions {
                format,
                metadata: Metadata::None,
                ..png.clone()
            };
            let out = format!("/clips/small.{}", format.extension());
            let planned = plan(&photo, &options, Path::new(&out), JxlSupport::default()).unwrap();
            assert_eq!(tools(&planned), [Tool::Ffmpeg], "{format:?}");
        }
    }

    #[test]
    fn edited_jxl_is_cleaned_between_ffmpeg_and_the_encoder() {
        let options = jxl(ConvertOptions {
            rotate: 90,
            metadata: Metadata::None,
            ..ConvertOptions::default()
        });
        let photo = image("photo.png", 400, 300);
        let out = Path::new("/clips/photo.jxl");

        let plan_cjxl = plan(&photo, &options, out, LIBJXL).expect("plan");
        assert_eq!(tools(&plan_cjxl), [Tool::Ffmpeg, Tool::Scrub, Tool::Cjxl]);
        assert_eq!(plan_cjxl.steps[2].args[0], "/clips/.photo.jxl.clean.png");

        // Without cjxl, ffmpeg encodes the cleaned picture as it is.
        let plan_ffmpeg = plan(&photo, &options, out, FFMPEG_JXL).expect("plan");
        assert_eq!(
            tools(&plan_ffmpeg),
            [Tool::Ffmpeg, Tool::Scrub, Tool::Ffmpeg]
        );
        let encode = plan_ffmpeg.steps[2]
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            encode,
            "-i /clips/.photo.jxl.clean.png -map 0:v:0 -frames:v 1 -c:v libjxl -distance 1.5 \
             -effort 7 -update 1 -an -sn -dn /clips/photo.jxl"
        );
        assert_eq!(plan_ffmpeg.scratch.len(), 2);

        // Animations carry no EXIF.
        let short = ConvertOptions {
            trim_end: Some(2.0),
            ..options
        };
        let clip = plan(&video("clip.mov"), &short, out, LIBJXL).expect("plan");
        assert_eq!(tools(&clip), [Tool::Ffmpeg, Tool::Cjxl]);
    }

    #[test]
    fn restored_jpeg_is_cleaned_after_djxl() {
        let mut packed = image("photo.jxl", 4000, 3000);
        packed.reconstructs_jpeg = true;
        packed.has_location = true;
        let options = ConvertOptions {
            format: OutputFormat::Jpg,
            ..ConvertOptions::default()
        };
        let planned = plan(&packed, &options, Path::new("/clips/photo.jpg"), LIBJXL).expect("plan");
        assert_eq!(
            planned.command_line(),
            "djxl /clips/photo.jxl /clips/.photo.jpg.decoded.jpg --reconstruct_jpeg --quiet && \\\n  \
             catconvert scrub /clips/.photo.jpg.decoded.jpg /clips/photo.jpg --drop location"
        );

        // JPEG XL with metadata to drop isn't re-encoded by cjxl directly.
        let options = jxl(ConvertOptions {
            metadata: Metadata::None,
            ..ConvertOptions::default()
        });
        let planned =
            plan(&packed, &options, Path::new("/clips/photo 2.jxl"), LIBJXL).expect("plan");
        assert_eq!(
            tools(&planned),
            [Tool::Djxl, Tool::Ffmpeg, Tool::Scrub, Tool::Cjxl]
        );
    }

    fn command_with(
        input: &MediaInfo,
        options: &ConvertOptions,
        out: &str,
        jxl: JxlSupport,
    ) -> String {
        plan(input, options, Path::new(out), jxl)
            .expect("plan")
            .command_line()
    }

    #[test]
    fn edited_image_goes_through_a_png_for_cjxl() {
        let options = jxl(ConvertOptions {
            rotate: 90,
            max_size: Some(1080),
            quality: 100,
            ..ConvertOptions::default()
        });
        let plan = plan(
            &image("cat.heic", 4000, 3000),
            &options,
            Path::new("/clips/cat.jxl"),
            LIBJXL,
        )
        .expect("plan");
        assert_eq!(tools(&plan), [Tool::Ffmpeg, Tool::Cjxl]);
        assert_eq!(plan.scratch, [PathBuf::from("/clips/.cat.jxl.frames.png")]);
        assert_eq!(
            plan.command_line(),
            "ffmpeg -i /clips/cat.heic -map 0:v:0 -frames:v 1 -vf transpose=clock,scale=1080:-2:flags=lanczos \
             -c:v png -compression_level 1 -update 1 -an -sn -dn /clips/.cat.jxl.frames.png && \\\n  \
             cjxl /clips/.cat.jxl.frames.png /clips/cat.jxl -d 0.5 -e 7 --quiet"
        );
    }

    #[test]
    fn tiled_heic_is_read_through_its_grid() {
        let photo = MediaInfo {
            tile_grid: Some(0),
            ..image("IMG_0001.heic", 3024, 4032)
        };
        // Edits go into a complex filtergraph fed by the stitched grid.
        let options = ConvertOptions {
            format: OutputFormat::Jpg,
            max_size: Some(1080),
            quality: 100,
            ..ConvertOptions::default()
        };
        let jpg = plan(&photo, &options, Path::new("/clips/IMG_0001.jpg"), LIBJXL).expect("plan");
        assert_eq!(
            jpg.command_line(),
            "ffmpeg -i /clips/IMG_0001.heic -filter_complex '[0:g:0]scale=1080:-2:flags=lanczos[v]' \
             -map '[v]' -frames:v 1 -c:v mjpeg -q:v 2 -update 1 -an -sn -dn /clips/IMG_0001.jpg"
        );
        // Without edits the grid is mapped directly.
        let options = ConvertOptions {
            format: OutputFormat::Png,
            ..ConvertOptions::default()
        };
        let png = plan(&photo, &options, Path::new("/clips/IMG_0001.png"), LIBJXL).expect("plan");
        let cmd = png.command_line();
        assert!(
            cmd.starts_with("ffmpeg -i /clips/IMG_0001.heic -map '[0:g:0]' -frames:v 1 -c:v png "),
            "{cmd}"
        );
    }

    #[test]
    fn video_becomes_an_animated_jxl_with_sensible_caps() {
        let options = jxl(ConvertOptions {
            trim_start: Some(1.0),
            trim_end: Some(4.0),
            ..ConvertOptions::default()
        });
        let input = MediaInfo {
            fps: Some(60.0),
            ..video("clip.mov")
        };
        let plan = plan(&input, &options, Path::new("/clips/clip.jxl"), LIBJXL).expect("plan");
        assert_eq!(tools(&plan), [Tool::Ffmpeg, Tool::Cjxl]);
        let cmd = plan.command_line();
        assert!(
            cmd.starts_with(
                "ffmpeg -ss 1 -i /clips/clip.mov -t 3 -map 0:v:0 -vf fps=30,scale=-2:720:flags=lanczos \
                 -c:v apng -compression_level 1 -plays 0 -an -sn -dn -f apng /clips/.clip.jxl.frames.apng"
            ),
            "{cmd}"
        );
        assert!(
            cmd.ends_with("cjxl /clips/.clip.jxl.frames.apng /clips/clip.jxl -d 1.5 -e 7 --quiet")
        );
        assert_eq!(plan.duration_secs(), Some(3.0));

        // ffmpeg's own encoder when libjxl tools are missing.
        let cmd = command_with(&input, &options, "/clips/clip.jxl", FFMPEG_JXL);
        assert!(
            cmd.ends_with(
                "-c:v libjxl_anim -distance 1.5 -effort 7 -an -sn -dn -f data /clips/clip.jxl"
            ),
            "{cmd}"
        );
    }

    #[test]
    fn long_animations_are_refused_for_cjxl_but_not_ffmpeg() {
        let input = MediaInfo {
            duration_secs: Some(600.0),
            ..video("long.mov")
        };
        let options = jxl(ConvertOptions::default());
        let err =
            plan(&input, &options, Path::new("/clips/long.jxl"), LIBJXL).expect_err("too big");
        assert!(err.to_string().contains("too long"), "{err}");
        let both = JxlSupport {
            cjxl: true,
            ..FFMPEG_JXL
        };
        let plan = plan(&input, &options, Path::new("/clips/long.jxl"), both).expect("plan");
        assert_eq!(tools(&plan), [Tool::Ffmpeg]);
    }

    #[test]
    fn unedited_gif_goes_straight_to_cjxl() {
        let gif = MediaInfo {
            extension: Some("gif".into()),
            width: Some(320),
            height: Some(240),
            fps: Some(10.0),
            duration_secs: Some(2.0),
            audio_codec: None,
            ..video("loop.gif")
        };
        let options = jxl(ConvertOptions {
            jxl: JxlOptions {
                lossless: true,
                ..JxlOptions::default()
            },
            ..ConvertOptions::default()
        });
        assert_eq!(
            command_with(&gif, &options, "/clips/loop.jxl", LIBJXL),
            "cjxl /clips/loop.gif /clips/loop.jxl -d 0 -e 7 --quiet"
        );
    }

    #[test]
    fn retimed_gifs_skip_the_cjxl_shortcut_and_trim_unseekable_jxl_in_the_graph() {
        let loop_gif = MediaInfo {
            extension: Some("gif".into()),
            width: Some(320),
            height: Some(240),
            fps: Some(10.0),
            duration_secs: Some(2.0),
            audio_codec: None,
            ..video("loop.gif")
        };
        let reversed = jxl(ConvertOptions {
            direction: Direction::Reverse,
            ..ConvertOptions::default()
        });
        let plan = plan(&loop_gif, &reversed, Path::new("/clips/loop.jxl"), LIBJXL).expect("plan");
        assert_eq!(tools(&plan), [Tool::Ffmpeg, Tool::Cjxl]);
        assert!(
            plan.command_line().contains("-vf reverse "),
            "{}",
            plan.command_line()
        );

        // ffmpeg's animated JPEG XL decoder can't seek: read up to the end,
        // cut the start in the graph.
        let anim = MediaInfo {
            extension: Some("jxl".into()),
            video_codec: Some("jpegxl_anim".into()),
            audio_codec: None,
            ..video("loop.jxl")
        };
        let fast = gif(ConvertOptions {
            trim_start: Some(1.0),
            trim_end: Some(3.0),
            speed: 2.0,
            ..ConvertOptions::default()
        });
        let cmd = command_with(&anim, &fast, "/clips/loop.gif", FFMPEG_JXL);
        assert!(
            cmd.starts_with(
                "ffmpeg -t 3 -i /clips/loop.jxl -map 0:v:0 \
                 -vf 'trim=start=1,setpts=PTS-STARTPTS,setpts=(PTS-STARTPTS)/2,fps=15,"
            ),
            "{cmd}"
        );
    }

    #[test]
    fn boomerangs_count_twice_against_the_cjxl_budget() {
        let input = MediaInfo {
            duration_secs: Some(30.0),
            ..video("clip.mov")
        };
        let forward = jxl(ConvertOptions::default());
        assert!(plan(&input, &forward, Path::new("/clips/c.jxl"), LIBJXL).is_ok());
        let boomerang = jxl(ConvertOptions {
            direction: Direction::Boomerang,
            ..ConvertOptions::default()
        });
        let err = plan_err(&input, &boomerang, LIBJXL);
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn jxl_input_is_decoded_by_djxl_when_ffmpeg_cannot() {
        let photo = MediaInfo {
            reconstructs_jpeg: true,
            ..image("photo.jxl", 4000, 3000)
        };
        // The original JPEG comes back untouched.
        let restore = ConvertOptions {
            format: OutputFormat::Jpg,
            ..ConvertOptions::default()
        };
        assert_eq!(
            command_with(&photo, &restore, "/clips/photo.jpg", LIBJXL),
            "djxl /clips/photo.jxl /clips/photo.jpg --reconstruct_jpeg --quiet"
        );

        // Edits go through a decoded PNG.
        let edited = ConvertOptions {
            format: OutputFormat::Webp,
            flip: true,
            ..ConvertOptions::default()
        };
        let plan = plan(&photo, &edited, Path::new("/clips/photo.webp"), LIBJXL).expect("plan");
        assert_eq!(tools(&plan), [Tool::Djxl, Tool::Ffmpeg]);
        assert_eq!(
            plan.command_line(),
            "djxl /clips/photo.jxl /clips/.photo.webp.decoded.png --quiet && \\\n  \
             ffmpeg -i /clips/.photo.webp.decoded.png -map 0:v:0 -frames:v 1 -vf hflip \
             -c:v libwebp -quality 73 -update 1 -an -sn -dn /clips/photo.webp"
        );

        // An ffmpeg with libjxl reads it directly.
        let cmd = command_with(&photo, &edited, "/clips/photo.webp", FFMPEG_JXL);
        assert!(cmd.starts_with("ffmpeg -i /clips/photo.jxl "), "{cmd}");

        // Animated JPEG XL becomes an APNG for ffmpeg to trim.
        let anim = MediaInfo {
            extension: Some("jxl".into()),
            video_codec: Some("jpegxl_anim".into()),
            audio_codec: None,
            ..video("loop.jxl")
        };
        let gif = ConvertOptions {
            format: OutputFormat::Gif,
            trim_start: Some(1.0),
            ..ConvertOptions::default()
        };
        let decoded = super::plan(&anim, &gif, Path::new("/clips/loop.gif"), LIBJXL).expect("plan");
        assert_eq!(
            decoded.scratch,
            [PathBuf::from("/clips/.loop.gif.decoded.apng")]
        );
        let cmd = decoded.command_line();
        assert!(
            cmd.contains("ffmpeg -ss 1 -i /clips/.loop.gif.decoded.apng"),
            "{cmd}"
        );

        // ffmpeg's own decoder can't seek in it, so the trim comes after -i.
        let cmd = command_with(&anim, &gif, "/clips/loop.gif", FFMPEG_JXL);
        assert!(cmd.starts_with("ffmpeg -i /clips/loop.jxl -ss 1 "), "{cmd}");

        let none = plan_err(&photo, &edited, JxlSupport::default());
        assert!(none.contains("djxl"), "{none}");
    }

    fn plan_err(input: &MediaInfo, options: &ConvertOptions, jxl: JxlSupport) -> String {
        plan(input, options, Path::new("/clips/out.x"), jxl)
            .expect_err("should fail")
            .to_string()
    }

    #[test]
    fn ffmpeg_libjxl_writes_stills() {
        let options = jxl(ConvertOptions {
            jxl: JxlOptions {
                lossless: true,
                effort: 10,
                progressive: false,
            },
            ..ConvertOptions::default()
        });
        assert_eq!(
            command_with(
                &image("a.png", 64, 64),
                &options,
                "/clips/a.jxl",
                FFMPEG_JXL
            ),
            "ffmpeg -i /clips/a.png -map 0:v:0 -frames:v 1 -c:v libjxl -distance 0 -effort 9 \
             -update 1 -an -sn -dn /clips/a.jxl"
        );
        let err = plan_err(&image("a.png", 64, 64), &options, JxlSupport::default());
        assert!(err.contains("cjxl"), "{err}");
        let err = plan_err(
            &video("a.mov"),
            &options,
            JxlSupport {
                ffmpeg_encoder: true,
                ..JxlSupport::default()
            },
        );
        assert!(err.contains("animated"), "{err}");
    }

    #[test]
    fn jxl_distance_follows_quality() {
        let at = |quality| {
            jxl_distance(&ConvertOptions {
                quality,
                ..ConvertOptions::default()
            })
        };
        assert_eq!(at(0), "3");
        assert_eq!(at(60), "1.5");
        assert_eq!(at(100), "0.5");
    }

    #[test]
    fn jxl_options_deserialize_with_defaults() {
        let options: ConvertOptions =
            serde_json::from_value(serde_json::json!({"format": "jxl", "jxl": {"lossless": true}}))
                .expect("parse");
        assert_eq!(options.format, OutputFormat::Jxl);
        assert!(options.jxl.lossless);
        assert_eq!(options.jxl.effort, 7);
        assert!(!options.jxl.progressive);
    }

    const TONE_MAP: Support = Support {
        hevc_hardware: false,
        jxl: JxlSupport {
            cjxl: false,
            djxl: false,
            ffmpeg_encoder: false,
            ffmpeg_anim_encoder: false,
            ffmpeg_decoder: false,
            ffmpeg_anim_decoder: false,
        },
        tone_map: true,
    };

    #[test]
    fn frame_plan_grabs_one_edited_frame_as_png() {
        let options = ConvertOptions {
            format: OutputFormat::Gif,
            trim_start: Some(2.0),
            trim_end: Some(8.0),
            fps: Some(10.0),
            max_size: Some(720),
            rotate: 90,
            flip: true,
            ..ConvertOptions::default()
        };
        let plan = frame_plan(
            &video("garden.mov"),
            &options,
            12.5,
            Path::new("/clips/garden-frame-0m12s.png"),
            JxlSupport::default(),
        )
        .expect("plan");
        // The trim, frame rate and GIF format belong to the conversion, not the frame.
        assert_eq!(
            plan.command_line(),
            "ffmpeg -ss 12.5 -i /clips/garden.mov -map 0:v:0 -frames:v 1 \
             -vf transpose=clock,hflip,scale=720:-2:flags=lanczos -c:v png -update 1 -an -sn -dn \
             /clips/garden-frame-0m12s.png"
        );
        assert_eq!(plan.duration_secs(), None);
    }

    #[test]
    fn frame_plan_at_start_or_past_the_end() {
        let out = Path::new("/clips/a.png");
        let first = frame_plan(
            &video("a.mov"),
            &ConvertOptions::default(),
            0.0,
            out,
            TONE_MAP,
        )
        .expect("plan");
        assert!(!first.steps[0].args.iter().any(|a| a == "-ss"));
        // 42 s at 30 fps: the last frame starts a frame's length before the end.
        let last = frame_plan(
            &video("a.mov"),
            &ConvertOptions::default(),
            99.0,
            out,
            TONE_MAP,
        )
        .expect("plan")
        .command_line();
        assert!(last.starts_with("ffmpeg -ss 41.967 -i"), "{last}");
    }

    #[test]
    fn frame_plan_needs_a_video() {
        let out = Path::new("/clips/a.png");
        let photo = image("photo.jpg", 4000, 3000);
        let err = frame_plan(&photo, &ConvertOptions::default(), 1.0, out, TONE_MAP)
            .expect_err("still image");
        assert!(err.to_string().contains("only videos"), "{err}");
        let err = frame_plan(
            &video("a.mov"),
            &ConvertOptions::default(),
            f64::NAN,
            out,
            TONE_MAP,
        )
        .expect_err("bad time");
        assert!(err.to_string().contains("positive"), "{err}");
    }

    #[test]
    fn hdr_is_tone_mapped_for_pictures_and_gifs_only() {
        let hdr = MediaInfo {
            hdr: true,
            ..video("iphone.mov")
        };
        let run = |format, support: Support| {
            let options = ConvertOptions {
                format,
                ..ConvertOptions::default()
            };
            plan(&hdr, &options, Path::new("/clips/out"), support)
                .expect("plan")
                .command_line()
        };
        let png = run(OutputFormat::Png, TONE_MAP);
        assert!(png.contains(&format!("-vf {TONE_MAP_FILTER} ")), "{png}");
        let gif = run(OutputFormat::Gif, TONE_MAP);
        assert!(
            gif.contains(&format!(
                "-vf '{TONE_MAP_FILTER},fps=15,scale=-2:480:flags=lanczos,"
            )),
            "{gif}"
        );
        // Videos keep their HDR tags; older ffmpegs can't tone-map at all.
        assert!(!run(OutputFormat::Mp4, TONE_MAP).contains("out_transfer"));
        assert!(!run(OutputFormat::Png, Support::default()).contains("out_transfer"));
        // SDR sources are left alone.
        let sdr = plan(
            &video("garden.mov"),
            &ConvertOptions {
                format: OutputFormat::Png,
                ..ConvertOptions::default()
            },
            Path::new("/clips/out.png"),
            TONE_MAP,
        )
        .expect("plan");
        assert!(!sdr.command_line().contains("out_transfer"));
    }

    #[test]
    fn hdr_photo_is_tone_mapped_once_when_cleaned() {
        let photo = MediaInfo {
            hdr: true,
            has_location: true,
            ..image("sunset.avif", 4000, 3000)
        };
        let ffmpeg_jxl = Support {
            hevc_hardware: false,
            jxl: JxlSupport {
                ffmpeg_encoder: true,
                ..JxlSupport::default()
            },
            tone_map: true,
        };
        for (format, support) in [
            (OutputFormat::Png, TONE_MAP),
            (OutputFormat::Jxl, ffmpeg_jxl),
        ] {
            let options = ConvertOptions {
                format,
                metadata: Metadata::NoLocation,
                ..ConvertOptions::default()
            };
            let plan = plan(&photo, &options, Path::new("/photos/out"), support).expect("plan");
            let mapped: Vec<bool> = plan
                .steps
                .iter()
                .filter(|step| step.tool == Tool::Ffmpeg)
                .map(|step| {
                    step.args
                        .iter()
                        .any(|a| a.to_string_lossy().contains("out_transfer"))
                })
                .collect();
            // The edit pass reads the HDR source; a re-encode of its PNG must not map again.
            assert_eq!(mapped.first(), Some(&true), "{format:?}");
            assert_eq!(mapped.iter().filter(|m| **m).count(), 1, "{format:?}");
        }
    }

    #[test]
    fn frame_names_carry_the_time() {
        let dir = tempfile::tempdir().expect("temp dir");
        let input = dir.path().join("clip.mov");
        let at = |secs| {
            frame_output_path(&input, secs, |_| false)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        assert_eq!(at(12.4), "clip-frame-0m12s.png");
        assert_eq!(at(0.0), "clip-frame-0m00s.png");
        assert_eq!(at(605.9), "clip-frame-10m05s.png");
        assert_eq!(at(3723.0), "clip-frame-1h02m03s.png");
        // A second frame in the same second gets a number.
        std::fs::write(dir.path().join("clip-frame-0m12s.png"), b"x").expect("write");
        assert_eq!(at(12.6), "clip-frame-0m12s 2.png");
    }

    #[test]
    fn quotes_awkward_paths() {
        assert_eq!(shell_quote("/a/b.mp4"), "/a/b.mp4");
        assert_eq!(shell_quote("my clip.mov"), "'my clip.mov'");
        assert_eq!(shell_quote("it's.mov"), r"'it'\''s.mov'");
    }

    #[test]
    fn crf_follows_quality_within_each_codecs_range() {
        for codec in [
            VideoCodec::H264,
            VideoCodec::H265,
            VideoCodec::Vp9,
            VideoCodec::Av1,
        ] {
            let (worst, best) = crf_range(codec);
            assert_eq!(crf(codec, 0), worst, "{codec:?}");
            assert_eq!(crf(codec, 100), best, "{codec:?}");
            assert_eq!(crf(codec, 255), best, "{codec:?} clamps");
            let mut last = worst;
            for q in 0..=100 {
                let value = crf(codec, q);
                assert!(value <= last && value >= best, "{codec:?} at {q}");
                last = value;
            }
        }
        // The default quality of 60.
        assert_eq!(crf(VideoCodec::H264, 60), 24);
        assert_eq!(crf(VideoCodec::H265, 60), 28);
        assert_eq!(crf(VideoCodec::Vp9, 60), 36);
        assert_eq!(crf(VideoCodec::Av1, 60), 32);
    }

    #[test]
    fn audio_bitrate_tiers() {
        let input = video("song.mov");
        let audio = |format, quality| {
            let options = ConvertOptions {
                format,
                quality,
                ..ConvertOptions::default()
            };
            quality_summary(&input, &options, JxlSupport::default()).audio
        };
        let kbps = |kbps| Some(AudioRate::Bitrate { kbps });
        for (quality, aac, opus, music) in [
            (0, 96, 64, 128),
            (33, 96, 64, 128),
            (34, 128, 96, 192),
            (60, 128, 96, 192),
            (66, 128, 96, 192),
            (67, 192, 160, 256),
            (100, 192, 160, 256),
        ] {
            for format in [OutputFormat::Mp4, OutputFormat::Mov, OutputFormat::Mkv] {
                assert_eq!(audio(format, quality), kbps(aac), "{format:?} at {quality}");
            }
            assert_eq!(audio(OutputFormat::Webm, quality), kbps(opus));
            assert_eq!(audio(OutputFormat::Ogg, quality), kbps(opus));
            assert_eq!(audio(OutputFormat::M4a, quality), kbps(music));
        }
        assert_eq!(
            audio(OutputFormat::Mp3, 0),
            Some(AudioRate::Vbr {
                level: 7,
                kbps: 100
            })
        );
        assert_eq!(
            audio(OutputFormat::Mp3, 60),
            Some(AudioRate::Vbr {
                level: 3,
                kbps: 175
            })
        );
        assert_eq!(
            audio(OutputFormat::Mp3, 100),
            Some(AudioRate::Vbr {
                level: 0,
                kbps: 245
            })
        );
        assert_eq!(audio(OutputFormat::Flac, 0), Some(AudioRate::Lossless));
        // 44.1 kHz stereo at 16 bits.
        assert_eq!(
            audio(OutputFormat::Wav, 0),
            Some(AudioRate::Pcm { kbps: Some(1411) })
        );
        let unknown = MediaInfo {
            audio_channels: None,
            ..video("stream.mkv")
        };
        let wav = ConvertOptions {
            format: OutputFormat::Wav,
            ..ConvertOptions::default()
        };
        assert_eq!(
            quality_summary(&unknown, &wav, JxlSupport::default()).audio,
            Some(AudioRate::Pcm { kbps: None })
        );
    }

    #[test]
    fn quality_summary_only_covers_what_the_output_has() {
        let input = video("garden.mov");
        let summary = |input: &MediaInfo, options: ConvertOptions| {
            quality_summary(input, &options, JxlSupport::default())
        };
        let mp4 = ConvertOptions::default();
        assert_eq!(
            summary(&input, mp4.clone()),
            QualitySummary {
                audio: Some(AudioRate::Bitrate { kbps: 128 }),
                video: Some(VideoRate {
                    codec: VideoCodec::H264,
                    crf: Some(24),
                    kbps: None,
                    passes: 1,
                    hardware: false,
                    hardware_quality: None,
                }),
            }
        );
        let muted = ConvertOptions {
            keep_audio: false,
            ..mp4.clone()
        };
        assert_eq!(summary(&input, muted).audio, None);
        let silent = MediaInfo {
            audio_codec: None,
            ..video("silent.mov")
        };
        assert_eq!(summary(&silent, mp4).audio, None);
        for format in [OutputFormat::Gif, OutputFormat::Webp, OutputFormat::Png] {
            let options = ConvertOptions {
                format,
                ..ConvertOptions::default()
            };
            assert_eq!(
                summary(&input, options),
                QualitySummary::default(),
                "{format:?}"
            );
        }
        let mp3 = ConvertOptions {
            format: OutputFormat::Mp3,
            video_bitrate: Some(4000),
            ..ConvertOptions::default()
        };
        assert_eq!(summary(&input, mp3).video, None);
    }

    /// The summary is what the command does: every quality, every format.
    #[test]
    fn quality_summary_matches_the_command() {
        let input = video("garden.mov");
        let formats = [
            OutputFormat::Mp4,
            OutputFormat::Webm,
            OutputFormat::Mov,
            OutputFormat::Mkv,
            OutputFormat::Mp3,
            OutputFormat::M4a,
            OutputFormat::Wav,
            OutputFormat::Flac,
            OutputFormat::Ogg,
        ];
        for format in formats {
            for video_codec in [None, Some(VideoCodec::H265), Some(VideoCodec::Av1)] {
                if video_codec.is_some_and(|codec| !format.video_codecs().contains(&codec)) {
                    continue;
                }
                for quality in (0..=100).step_by(5) {
                    let options = ConvertOptions {
                        format,
                        quality,
                        video_codec,
                        ..ConvertOptions::default()
                    };
                    let out = format!("/clips/out.{}", format.extension());
                    let cmd = command(&input, &options, &out);
                    let summary = quality_summary(&input, &options, JxlSupport::default());
                    let audio = match summary.audio.expect("sound") {
                        AudioRate::Bitrate { kbps } => format!("-b:a {kbps}k "),
                        AudioRate::Vbr { level, .. } => format!("-q:a {level} "),
                        AudioRate::Lossless => "-c:a flac ".into(),
                        AudioRate::Pcm { .. } => "-c:a pcm_s16le ".into(),
                    };
                    assert!(cmd.contains(&audio), "{audio} in {cmd}");
                    if let Some(rate) = summary.video {
                        let preset = match rate.codec {
                            VideoCodec::H264 | VideoCodec::H265 => "-preset fast ",
                            VideoCodec::Vp9 => "",
                            VideoCodec::Av1 => "-preset 8 ",
                        };
                        let crf = rate.crf.expect("crf");
                        let crf = format!("-c:v {} {preset}-crf {crf} ", rate.codec.encoder());
                        assert!(cmd.contains(&crf), "{crf} in {cmd}");
                    } else {
                        assert_eq!(format.kind(), MediaKind::Audio);
                    }
                }
            }
        }
    }

    #[test]
    fn exact_bitrate_encodes_h264_in_two_passes() {
        let options = ConvertOptions {
            video_bitrate: Some(4000),
            sound: sound(false, 1.0, 0.0),
            ..ConvertOptions::default()
        };
        let input = video("garden.mov");
        let plan = plan(
            &input,
            &options,
            Path::new("/clips/garden.mp4"),
            JxlSupport::default(),
        )
        .expect("plan");
        assert_eq!(tools(&plan), [Tool::Ffmpeg, Tool::Ffmpeg]);
        let cmd = plan.command_line();
        assert_eq!(
            cmd,
            "ffmpeg -i /clips/garden.mov -map 0:v:0 -vf fade=t=in:d=1 \
             -c:v libx264 -preset fast -b:v 4000k -pass 1 \
             -passlogfile /clips/.garden.mp4.passlog.stats -pix_fmt yuv420p \
             -an -sn -dn -f null - && \\\n  \
             ffmpeg -i /clips/garden.mov -map 0:v:0 -map 0:a:0 -vf fade=t=in:d=1 \
             -af afade=t=in:d=1 -c:v libx264 -preset fast -b:v 4000k -pass 2 \
             -passlogfile /clips/.garden.mp4.passlog.stats -pix_fmt yuv420p \
             -c:a aac -b:a 128k -sn -dn -movflags +faststart /clips/garden.mp4"
        );
        // Both passes cover the whole clip, for the progress bar.
        assert!(plan.steps.iter().all(|s| s.duration_secs == Some(42.0)));
        assert!(plan.steps[0].weight < plan.steps[1].weight);
        assert_eq!(
            plan.scratch,
            [
                "/clips/.garden.mp4.passlog.stats-0.log",
                "/clips/.garden.mp4.passlog.stats-0.log.temp",
                "/clips/.garden.mp4.passlog.stats-0.log.mbtree",
                "/clips/.garden.mp4.passlog.stats-0.log.mbtree.temp",
            ]
            .map(PathBuf::from)
        );
        assert_eq!(
            quality_summary(&input, &options, JxlSupport::default()).video,
            Some(VideoRate {
                codec: VideoCodec::H264,
                crf: None,
                kbps: Some(4000),
                passes: 2,
                hardware: false,
                hardware_quality: None,
            })
        );
    }

    #[test]
    fn exact_bitrate_for_vp9_and_av1() {
        let input = video("garden.mov");
        let webm = ConvertOptions {
            format: OutputFormat::Webm,
            video_bitrate: Some(2500),
            metadata: Metadata::None,
            ..ConvertOptions::default()
        };
        let plan = plan(
            &input,
            &webm,
            Path::new("/clips/garden.webm"),
            JxlSupport::default(),
        )
        .expect("plan");
        let [first, second] = [0, 1].map(|i| {
            plan.steps[i]
                .args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" ")
        });
        assert!(
            first.contains(
                "-c:v libvpx-vp9 -b:v 2500k -row-mt 1 -deadline good -cpu-used 4 -pass 1"
            ),
            "{first}"
        );
        // The first pass writes nothing, so it has no tags to drop.
        assert!(first.ends_with("-an -sn -dn -f null -"), "{first}");
        assert!(second.contains("-b:v 2500k -row-mt 1"), "{second}");
        assert!(second.contains("-pass 2"), "{second}");
        assert!(second.contains("-map_metadata -1"), "{second}");
        assert!(!second.contains("-crf"), "{second}");

        // SVT-AV1 has no two-pass mode in ffmpeg: one pass aiming for the average.
        let av1 = ConvertOptions {
            video_codec: Some(VideoCodec::Av1),
            video_bitrate: Some(3000),
            ..ConvertOptions::default()
        };
        let one = super::plan(
            &input,
            &av1,
            Path::new("/clips/garden.mp4"),
            JxlSupport::default(),
        )
        .expect("plan");
        assert_eq!(one.steps.len(), 1);
        assert!(one.scratch.is_empty());
        let cmd = one.command_line();
        assert!(
            cmd.contains("-c:v libsvtav1 -preset 8 -b:v 3000k -pix_fmt yuv420p10le"),
            "{cmd}"
        );
        assert_eq!(
            quality_summary(&input, &av1, JxlSupport::default())
                .video
                .map(|v| v.passes),
            Some(1)
        );
    }

    #[test]
    fn exact_bitrate_must_be_sensible() {
        let input = video("garden.mov");
        for kbps in [0, 99, 200_001] {
            let options = ConvertOptions {
                video_bitrate: Some(kbps),
                ..ConvertOptions::default()
            };
            let err = plan(
                &input,
                &options,
                Path::new("/clips/a.mp4"),
                JxlSupport::default(),
            )
            .expect_err("out of range");
            assert!(err.to_string().contains("video bitrate"), "{err}");
        }
        // Formats without a video codec ignore it.
        let gif = ConvertOptions {
            format: OutputFormat::Gif,
            video_bitrate: Some(0),
            ..ConvertOptions::default()
        };
        assert!(
            plan(
                &input,
                &gif,
                Path::new("/clips/a.gif"),
                JxlSupport::default()
            )
            .is_ok()
        );
    }

    #[test]
    fn summaries_serialize_for_the_front_end() {
        let summary = QualitySummary {
            audio: Some(AudioRate::Vbr {
                level: 2,
                kbps: 190,
            }),
            video: Some(VideoRate::constant(VideoCodec::Av1, 60)),
        };
        assert_eq!(
            serde_json::to_value(summary).expect("json"),
            serde_json::json!({
                "audio": {"mode": "vbr", "level": 2, "kbps": 190},
                "video": {
                    "codec": "av1",
                    "crf": 32,
                    "kbps": null,
                    "passes": 1,
                    "hardware": false,
                    "hardwareQuality": null
                }
            })
        );
        assert_eq!(
            serde_json::to_value(AudioRate::Pcm { kbps: None }).expect("json"),
            serde_json::json!({"mode": "pcm", "kbps": null})
        );
        let options: ConvertOptions =
            serde_json::from_value(serde_json::json!({"videoBitrate": 4000})).expect("options");
        assert_eq!(options.video_bitrate, Some(4000));
    }
}
