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
    /// Which dates, places, titles and other tags to carry over.
    pub metadata: Metadata,
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
            quality: 60,
            keep_audio: true,
            jxl: JxlOptions::default(),
            metadata: Metadata::default(),
        }
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
}

/// Default output height for GIFs, which get huge quickly.
const GIF_MAX_SIZE: u32 = 480;
/// Default frame rate for GIFs.
const GIF_FPS: f64 = 15.0;
/// Default short side of animated JPEG XL.
const ANIMATED_JXL_MAX_SIZE: u32 = 720;
/// Default frame-rate cap of animated JPEG XL.
const ANIMATED_JXL_FPS: f64 = 30.0;
/// `cjxl` holds every decoded frame in memory; refuse animations whose raw
/// frames would exceed this many bytes.
const CJXL_MEMORY_BUDGET: f64 = 4.0 * 1024.0 * 1024.0 * 1024.0;
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
    let untouched = is_untouched(input, options);
    let jxl_in = is_jxl(input);
    let animated = format.animates(input.kind);
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
            || animation_bytes(input, options, None, None, trim)
                .is_none_or(|b| b <= CJXL_MEMORY_BUDGET);
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
        scratch.push(decoded);
    }

    if format == OutputFormat::Jxl {
        let raw_bytes = animated
            .then(|| {
                animation_bytes(
                    input,
                    options,
                    Some(ANIMATED_JXL_MAX_SIZE),
                    Some(ANIMATED_JXL_FPS),
                    trim,
                )
            })
            .flatten();
        let too_big_for_cjxl = raw_bytes.is_some_and(|b| b > CJXL_MEMORY_BUDGET);
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
                ..source.clone()
            };
            let as_is = ConvertOptions {
                trim_start: None,
                trim_end: None,
                max_size: None,
                fps: None,
                rotate: 0,
                flip: false,
                metadata: Metadata::Keep,
                ..options.clone()
            };
            let mut encode = ffmpeg_step(&edited, &as_is, (0.0, None), Sink::Output, output);
            encode.weight = 0.6;
            steps.push(encode);
            scratch.extend([frames, cleaned]);
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
    } else if let (OutputFormat::Png, Some(metadata)) = (format, scrub) {
        // ffmpeg's PNG encoder writes the EXIF it decoded.
        let encoded = scratch_path(output, "encoded", "png");
        steps.push(ffmpeg_step(&source, options, trim, Sink::Output, &encoded));
        steps.push(scrub_step(&encoded, output, metadata));
        scratch.push(encoded);
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
    if timed_input && start > 0.0 && seek_input {
        push(&["-ss", &format_secs(start)]);
    }
    push(&["-i"]);
    args.push(input.path.clone().into_os_string());
    if timed_input && start > 0.0 && !seek_input {
        args.extend(["-ss".into(), format_secs(start).into()]);
    }
    let duration_secs = if timed_input && out_kind != MediaKind::Image {
        let end = end.or(input.duration_secs);
        if let Some(end) = end.filter(|_| options.trim_end.is_some()) {
            args.extend(["-t".into(), format_secs(end - start).into()]);
        }
        end.map(|end| (end - start).max(0.0))
    } else {
        None
    };

    let mut push = |items: &[&str]| args.extend(items.iter().map(OsString::from));
    let q = options.quality.min(100);

    match (format, sink) {
        (OutputFormat::Jxl, Sink::Frames { animated }) => {
            let filters = animation_filters(input, options);
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
            let mut filters = video_filters(input, options, Some(GIF_MAX_SIZE), false);
            if options.fps.is_none() && input.fps.is_none_or(|fps| fps > GIF_FPS) {
                filters.push(format!("fps={GIF_FPS}"));
            }
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
            let filters = animation_filters(input, options);
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

    match options.metadata {
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

/// Filters for animated or still JPEG XL: like [`video_filters`], plus the
/// default size and frame-rate caps of animations.
fn animation_filters(input: &MediaInfo, options: &ConvertOptions) -> Vec<String> {
    if !options.format.animates(input.kind) {
        return video_filters(input, options, None, false);
    }
    let mut filters = video_filters(input, options, Some(ANIMATED_JXL_MAX_SIZE), false);
    if options.fps.is_none() && input.fps.is_none_or(|fps| fps > ANIMATED_JXL_FPS + 0.01) {
        filters.push(format!("fps={}", format_secs(ANIMATED_JXL_FPS)));
    }
    filters
}

/// Whether the options leave the picture exactly as it is: no trim, turn,
/// mirror, downscale or frame-rate change.
fn is_untouched(input: &MediaInfo, options: &ConvertOptions) -> bool {
    let untrimmed = options.trim_start.is_none_or(|s| s <= 0.0) && options.trim_end.is_none();
    (input.kind == MediaKind::Image || untrimmed)
        && video_filters(input, options, None, false).is_empty()
}

/// Rough size of every decoded frame of an animation at 4 bytes per pixel,
/// or `None` if the size, rate or length is unknown.
fn animation_bytes(
    input: &MediaInfo,
    options: &ConvertOptions,
    default_max: Option<u32>,
    default_fps: Option<f64>,
    (start, end): (f64, Option<f64>),
) -> Option<f64> {
    let (w, h) = output_dims(input, options, default_max)?;
    let source_fps = input.fps?;
    let fps = options
        .fps
        .or(default_fps)
        .map_or(source_fps, |fps| fps.min(source_fps));
    let length = end.or(input.duration_secs)? - start;
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
/// `default_max` caps the short side when the user picked no size (GIFs).
/// `even` forces even dimensions, which yuv420p encoders require.
fn video_filters(
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

    let fps = options.fps.filter(|f| f.is_finite() && *f > 0.0);
    if let Some(fps) = fps.filter(|&fps| input.fps.is_none_or(|source| fps < source - 0.01)) {
        filters.push(format!("fps={}", format_secs(fps)));
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
            has_location: false,
            location_tags: Vec::new(),
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
            cmd.contains("scale=-2:480:flags=lanczos,fps=15,split[a][b]"),
            "{cmd}"
        );
        assert!(cmd.contains("palettegen=max_colors=166"), "{cmd}");
        assert!(
            cmd.ends_with("-loop 0 -an -sn -dn /clips/loop.gif"),
            "{cmd}"
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
                "ffmpeg -ss 1 -i /clips/clip.mov -t 3 -map 0:v:0 -vf scale=-2:720:flags=lanczos,fps=30 \
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
