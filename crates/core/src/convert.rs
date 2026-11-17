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

    /// What kind of file this produces. GIFs count as (silent) video. JPEG XL
    /// counts as an image, but becomes an animation when made from a video.
    #[must_use]
    pub fn kind(self) -> MediaKind {
        match self {
            Self::Mp4 | Self::Webm | Self::Mov | Self::Mkv | Self::Gif => MediaKind::Video,
            Self::Mp3 | Self::M4a | Self::Wav | Self::Flac | Self::Ogg => MediaKind::Audio,
            Self::Png | Self::Jpg | Self::Webp | Self::Avif | Self::Jxl => MediaKind::Image,
        }
    }

    /// The ffmpeg encoder this format needs. JPEG XL can also be written by
    /// libjxl's `cjxl` instead (see [`crate::Ffmpeg::supports`]).
    #[must_use]
    pub fn encoder(self) -> &'static str {
        match self {
            Self::Mp4 | Self::Mov | Self::Mkv => "libx264",
            Self::Webm => "libvpx-vp9",
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
    /// rather than a still: GIFs always, JPEG XL when the input moves.
    #[must_use]
    pub fn animates(self, kind: MediaKind) -> bool {
        match self {
            Self::Gif => true,
            Self::Jxl => kind == MediaKind::Video,
            _ => false,
        }
    }
}

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
    /// 0 = smallest file, 100 = best quality.
    pub quality: u8,
    /// Keep the sound when writing video.
    pub keep_audio: bool,
    /// JPEG XL settings, ignored for other formats.
    pub jxl: JxlOptions,
    /// Carry over dates, location, titles and other tags where the output
    /// format can hold them. Off strips them.
    pub keep_metadata: bool,
    /// Playback speed of animations (GIF, animated JPEG XL), from 0.25 to 4.
    /// Ignored for other outputs.
    pub speed: f64,
    /// Playback direction of animations. Ignored for other outputs.
    pub direction: Direction,
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
            quality: 60,
            keep_audio: true,
            jxl: JxlOptions::default(),
            keep_metadata: true,
            speed: 1.0,
            direction: Direction::Forward,
        }
    }
}

impl ConvertOptions {
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

/// A program a [`Step`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    Ffmpeg,
    /// libjxl's encoder.
    Cjxl,
    /// libjxl's decoder.
    Djxl,
}

impl Tool {
    /// The program's usual name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Ffmpeg => "ffmpeg",
            Self::Cjxl => "cjxl",
            Self::Djxl => "djxl",
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
}

impl Plan {
    /// The commands as shell-quoted lines joined with `&&`, for display.
    #[must_use]
    pub fn command_line(&self) -> String {
        self.steps
            .iter()
            .map(|step| {
                std::iter::once(step.tool.name().to_owned())
                    .chain(
                        step.args
                            .iter()
                            .map(|arg| shell_quote(&arg.to_string_lossy())),
                    )
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join(" && \\\n  ")
    }

    /// Length of the output in seconds, if it is timed.
    #[must_use]
    pub fn duration_secs(&self) -> Option<f64> {
        self.steps.iter().rev().find_map(|step| step.duration_secs)
    }

    fn single(step: Step, output: &Path) -> Self {
        Self {
            steps: vec![step],
            output: output.to_path_buf(),
            scratch: Vec::new(),
        }
    }
}

/// Default output height for GIFs, which get huge quickly.
const GIF_MAX_SIZE: u32 = 480;
/// Default frame rate for GIFs.
const GIF_FPS: f64 = 15.0;
/// Default short side of animated JPEG XL.
const ANIMATED_JXL_MAX_SIZE: u32 = 720;
/// Default frame-rate cap of animated JPEG XL.
const ANIMATED_JXL_FPS: f64 = 30.0;
/// `cjxl` and ffmpeg's `reverse` filter hold every decoded frame in memory;
/// refuse animations whose raw frames would exceed this many bytes.
const FRAME_MEMORY_BUDGET: f64 = 4.0 * 1024.0 * 1024.0 * 1024.0;
/// Slowest and fastest animation speed.
const SPEEDS: std::ops::RangeInclusive<f64> = 0.25..=4.0;
/// Inputs `cjxl` reads itself, keeping metadata, bit depth and (for JPEG) the
/// original bitstream.
const CJXL_INPUTS: [&str; 6] = ["png", "apng", "gif", "jpg", "jpeg", "jxl"];

/// Validate `options` for `input` and plan the steps that write `output`.
///
/// `jxl` says which JPEG XL tools are installed; it only matters when the
/// input or output is JPEG XL.
///
/// # Errors
///
/// [`Error::Unsupported`] if the format cannot be made from this input (e.g.
/// MP3 from a silent video), the trim range or rotation is invalid, or the
/// JPEG XL tools this needs are missing.
pub fn plan(
    input: &MediaInfo,
    options: &ConvertOptions,
    output: &Path,
    jxl: JxlSupport,
) -> Result<Plan> {
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
    if !matches!(options.rotate, 0 | 90 | 180 | 270) {
        return Err(Error::unsupported(
            "rotation must be 0, 90, 180 or 270 degrees",
        ));
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
    // cjxl and djxl carry metadata over and can't drop it, so stripping
    // means going through ffmpeg.
    let untouched = is_untouched(input, options) && options.keep_metadata;

    // Single-tool shortcuts for unedited files. libjxl does these best: they
    // keep metadata, colour profiles, high bit depth and, for JPEG, the
    // original bitstream.
    if untouched && format == OutputFormat::Jxl && jxl.cjxl {
        let ext = input.extension.as_deref().unwrap_or_default();
        let fits = !animated
            || animation_bytes(input, options, (None, None), trim)
                .is_none_or(|b| b <= FRAME_MEMORY_BUDGET);
        if CJXL_INPUTS.contains(&ext) && fits {
            let jpeg = matches!(ext, "jpg" | "jpeg");
            return Ok(Plan::single(
                cjxl_step(&input.path, output, options, jpeg, 1.0),
                output,
            ));
        }
    }
    if untouched && jxl_in && jxl.djxl && input.kind == MediaKind::Image {
        let restore_jpeg = format == OutputFormat::Jpg && input.reconstructs_jpeg;
        if format == OutputFormat::Png || restore_jpeg {
            let mut args = vec![input.path.clone().into_os_string(), output.into()];
            if restore_jpeg {
                args.push("--reconstruct_jpeg".into());
            }
            args.push("--quiet".into());
            return Ok(Plan::single(
                Step {
                    tool: Tool::Djxl,
                    args,
                    duration_secs: None,
                    weight: 1.0,
                },
                output,
            ));
        }
    }

    let mut steps = Vec::new();
    let mut scratch = Vec::new();

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
            ));
            steps.push(cjxl_step(&frames, output, options, false, 0.6));
            scratch.push(frames);
        } else if ffmpeg_can {
            steps.push(ffmpeg_step(&source, options, trim, Sink::Output, output));
        } else if animated {
            return Err(Error::unsupported(
                "animated JPEG XL needs cjxl from libjxl or an ffmpeg with the libjxl_anim encoder",
            ));
        } else {
            return Err(Error::unsupported(
                "writing JPEG XL needs cjxl from libjxl or an ffmpeg built with libjxl",
            ));
        }
    } else {
        steps.push(ffmpeg_step(&source, options, trim, Sink::Output, output));
    }

    Ok(Plan {
        steps,
        output: output.to_path_buf(),
        scratch,
    })
}

/// Where an ffmpeg step writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sink {
    /// The final file, in `options.format`.
    Output,
    /// Lossless PNG (or APNG when `animated`) for another tool to pick up.
    Frames { animated: bool },
}

/// The ffmpeg call that applies the edits in `options` to `input` and writes `out`.
fn ffmpeg_step(
    input: &MediaInfo,
    options: &ConvertOptions,
    (start, end): (f64, Option<f64>),
    sink: Sink,
    out: &Path,
) -> Step {
    let format = options.format;
    let animated = format.animates(input.kind);
    let out_kind = if animated {
        MediaKind::Video
    } else {
        format.kind()
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
            let filters = animation_filters(input, options, lead_trim);
            if animated {
                push(&["-map", "0:v:0"]);
                if !filters.is_empty() {
                    push(&["-vf", &filters.join(",")]);
                }
                // Fast zlib: this file only lives until cjxl has read it.
                push(&["-c:v", "apng", "-compression_level", "1", "-plays", "0"]);
                push(&["-an", "-sn", "-dn", "-f", "apng"]);
            } else {
                push(&["-map", "0:v:0", "-frames:v", "1"]);
                if !filters.is_empty() {
                    push(&["-vf", &filters.join(",")]);
                }
                push(&["-c:v", "png", "-compression_level", "1"]);
                push(&["-update", "1", "-an", "-sn", "-dn"]);
            }
        }
        (_, Sink::Frames { .. }) => unreachable!("only JPEG XL goes through intermediate frames"),
        (OutputFormat::Mp4 | OutputFormat::Mov | OutputFormat::Mkv | OutputFormat::Webm, _) => {
            push(&["-map", "0:v:0"]);
            let keep_audio = options.keep_audio && input.has_audio();
            if keep_audio {
                push(&["-map", "0:a:0"]);
            }
            let filters = video_filters(input, options, None, true);
            if !filters.is_empty() {
                push(&["-vf", &filters.join(",")]);
            }
            if format == OutputFormat::Webm {
                let crf = lerp(q, 48.0, 28.0).to_string();
                push(&["-c:v", "libvpx-vp9", "-crf", &crf, "-b:v", "0"]);
                push(&["-row-mt", "1", "-deadline", "good", "-cpu-used", "4"]);
            } else {
                let crf = lerp(q, 34.0, 18.0).to_string();
                push(&["-c:v", "libx264", "-preset", "fast", "-crf", &crf]);
            }
            push(&["-pix_fmt", "yuv420p"]);
            if keep_audio {
                if format == OutputFormat::Webm {
                    push(&["-c:a", "libopus", "-b:a", tiered(q, ["64k", "96k", "160k"])]);
                } else {
                    push(&["-c:a", "aac", "-b:a", tiered(q, ["96k", "128k", "192k"])]);
                }
            } else {
                push(&["-an"]);
            }
            push(&["-sn", "-dn"]);
            if matches!(format, OutputFormat::Mp4 | OutputFormat::Mov) {
                push(&["-movflags", "+faststart"]);
            }
        }
        (OutputFormat::Gif, _) => {
            let mut filters = animation_filters(input, options, lead_trim);
            let colors = lerp(q, 32.0, 256.0);
            filters.push(format!(
                "split[a][b];[a]palettegen=max_colors={colors}:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4"
            ));
            push(&[
                "-map",
                "0:v:0",
                "-vf",
                &filters.join(","),
                "-loop",
                "0",
                "-an",
                "-sn",
                "-dn",
            ]);
        }
        (OutputFormat::Jxl, Sink::Output) if animated => {
            // ffmpeg's libjxl_anim emits the codestream frame by frame; the
            // data muxer writes the packets back to back as one file.
            let filters = animation_filters(input, options, lead_trim);
            push(&["-map", "0:v:0"]);
            if !filters.is_empty() {
                push(&["-vf", &filters.join(",")]);
            }
            push(&["-c:v", "libjxl_anim"]);
            push_ffmpeg_jxl(&mut push, options);
            push(&["-an", "-sn", "-dn", "-f", "data"]);
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
            match format {
                OutputFormat::Mp3 => {
                    let vbr = lerp(q, 7.0, 0.0).to_string();
                    push(&["-c:a", "libmp3lame", "-q:a", &vbr]);
                }
                OutputFormat::M4a => {
                    push(&["-c:a", "aac", "-b:a", tiered(q, ["128k", "192k", "256k"])]);
                    push(&["-movflags", "+faststart"]);
                }
                OutputFormat::Wav => push(&["-c:a", "pcm_s16le"]),
                OutputFormat::Flac => push(&["-c:a", "flac"]),
                _ => push(&["-c:a", "libopus", "-b:a", tiered(q, ["64k", "96k", "160k"])]),
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
            let filters = video_filters(input, options, None, needs_even);
            push(&["-map", "0:v:0", "-frames:v", "1"]);
            if !filters.is_empty() {
                push(&["-vf", &filters.join(",")]);
            }
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

    if !options.keep_metadata {
        args.extend(["-map_metadata", "-1", "-map_chapters", "-1"].map(OsString::from));
    }
    args.push(out.as_os_str().to_owned());
    let weight = if matches!(sink, Sink::Frames { .. }) {
        0.4
    } else {
        1.0
    };
    Step {
        tool: Tool::Ffmpeg,
        args,
        duration_secs,
        weight,
    }
}

/// Distance and effort for ffmpeg's libjxl encoders.
fn push_ffmpeg_jxl(push: &mut impl FnMut(&[&str]), options: &ConvertOptions) {
    let distance = jxl_distance(options);
    // ffmpeg's libjxl wrapper caps effort at 9.
    let effort = options.jxl.effort.clamp(1, 9).to_string();
    push(&["-distance", &distance, "-effort", &effort]);
}

/// `cjxl` reading `input` and writing `output`. `jpeg` marks a JPEG input,
/// which is recompressed losslessly when `lossless` is on.
fn cjxl_step(
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
/// mirror, downscale, frame-rate change or (for animations) new speed or
/// direction.
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

/// Picture size after rotation and downscaling, if the source size is known.
fn output_dims(
    input: &MediaInfo,
    options: &ConvertOptions,
    default_max: Option<u32>,
) -> Option<(u32, u32)> {
    let (w, h) = input.width.zip(input.height)?;
    let (w, h) = if options.rotate % 180 == 90 {
        (h, w)
    } else {
        (w, h)
    };
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
fn scratch_path(output: &Path, tag: &str, ext: &str) -> PathBuf {
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
    let dir = input.parent().unwrap_or_else(|| Path::new(""));
    let stem = input
        .file_stem()
        .map_or_else(|| "converted".into(), |s| s.to_string_lossy().into_owned());
    let ext = format.extension();
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

/// Rotate, flip, downscale and frame-rate filters, in that order.
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

/// Rotate, flip and downscale filters, in that order.
fn picture_filters(
    input: &MediaInfo,
    options: &ConvertOptions,
    default_max: Option<u32>,
    even: bool,
) -> Vec<String> {
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

    // Display size after our rotation.
    let dims = input.width.zip(input.height).map(|(w, h)| {
        if options.rotate % 180 == 90 {
            (h, w)
        } else {
            (w, h)
        }
    });
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

/// Map quality 0–100 linearly onto `worst..=best` and round.
fn lerp(quality: u8, worst: f64, best: f64) -> i64 {
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
            reconstructs_jpeg: false,
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
    fn stripping_metadata_goes_through_ffmpeg() {
        let strip = ConvertOptions {
            format: OutputFormat::Mp4,
            keep_metadata: false,
            ..ConvertOptions::default()
        };
        let cmd = command(&video("garden.mov"), &strip, "/clips/garden.mp4");
        assert!(
            cmd.ends_with("-map_metadata -1 -map_chapters -1 /clips/garden.mp4"),
            "{cmd}"
        );

        // cjxl would carry the EXIF over, so an unedited JPEG is decoded instead.
        let options = jxl(ConvertOptions {
            jxl: JxlOptions {
                lossless: true,
                ..JxlOptions::default()
            },
            keep_metadata: false,
            ..ConvertOptions::default()
        });
        let plan = plan(
            &image("photo.jpg", 4000, 3000),
            &options,
            Path::new("/clips/photo.jxl"),
            LIBJXL,
        )
        .expect("plan");
        assert_eq!(tools(&plan), [Tool::Ffmpeg, Tool::Cjxl]);
        assert!(
            plan.command_line().contains("-map_metadata -1"),
            "{}",
            plan.command_line()
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

    #[test]
    fn quotes_awkward_paths() {
        assert_eq!(shell_quote("/a/b.mp4"), "/a/b.mp4");
        assert_eq!(shell_quote("my clip.mov"), "'my clip.mov'");
        assert_eq!(shell_quote("it's.mov"), r"'it'\''s.mov'");
    }
}
