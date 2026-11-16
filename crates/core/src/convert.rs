//! Turning edit options into an ffmpeg command line.
//!
//! [`plan`] is a pure function: it validates [`ConvertOptions`] against a
//! probed [`MediaInfo`] and produces the arguments, so the exact command can be
//! shown to the user before anything runs.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ffmpeg::format_secs;
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
}

impl OutputFormat {
    /// Every format, in the order the UI lists them.
    pub const ALL: [Self; 14] = [
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
        }
    }

    /// What kind of file this produces. GIFs count as (silent) video.
    #[must_use]
    pub fn kind(self) -> MediaKind {
        match self {
            Self::Mp4 | Self::Webm | Self::Mov | Self::Mkv | Self::Gif => MediaKind::Video,
            Self::Mp3 | Self::M4a | Self::Wav | Self::Flac | Self::Ogg => MediaKind::Audio,
            Self::Png | Self::Jpg | Self::Webp | Self::Avif => MediaKind::Image,
        }
    }

    /// The ffmpeg encoder this format needs.
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
        }
    }
}

/// A validated ffmpeg invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Arguments after `ffmpeg` (input, filters, codecs, output path).
    pub args: Vec<OsString>,
    /// Where the result is written.
    pub output: PathBuf,
    /// Length of the output in seconds, used to turn ffmpeg's timestamps into
    /// a percentage. `None` for still images.
    pub duration_secs: Option<f64>,
}

impl Plan {
    /// The command as a shell-quoted string, for display.
    #[must_use]
    pub fn command_line(&self) -> String {
        std::iter::once("ffmpeg".to_owned())
            .chain(
                self.args
                    .iter()
                    .map(|arg| shell_quote(&arg.to_string_lossy())),
            )
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Default output height for GIFs, which get huge quickly.
const GIF_MAX_SIZE: u32 = 480;
/// Default frame rate for GIFs.
const GIF_FPS: f64 = 15.0;

/// Validate `options` for `input` and build the ffmpeg arguments that write `output`.
///
/// # Errors
///
/// [`Error::Unsupported`] if the format cannot be made from this input (e.g.
/// MP3 from a silent video) or the trim range or rotation is invalid.
pub fn plan(input: &MediaInfo, options: &ConvertOptions, output: &Path) -> Result<Plan> {
    let format = options.format;
    let out_kind = format.kind();
    if !format.accepts(input.kind) {
        return Err(Error::unsupported(format!(
            "can't turn {} into {}",
            kind_noun(input.kind),
            format.extension().to_uppercase()
        )));
    }
    if out_kind == MediaKind::Audio && !input.has_audio() {
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

    let (start, end) = trim_range(input, options)?;
    let mut args: Vec<OsString> = Vec::new();
    let mut push = |items: &[&str]| args.extend(items.iter().map(OsString::from));

    // Input and trim. Seeking before `-i` is fast and frame-accurate when re-encoding.
    let timed_input = input.kind != MediaKind::Image;
    if timed_input && start > 0.0 {
        push(&["-ss", &format_secs(start)]);
    }
    push(&["-i"]);
    args.push(input.path.clone().into_os_string());
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

    match format {
        OutputFormat::Mp4 | OutputFormat::Mov | OutputFormat::Mkv | OutputFormat::Webm => {
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
        OutputFormat::Gif => {
            let mut filters = video_filters(input, options, Some(GIF_MAX_SIZE), false);
            if options.fps.is_none() && input.fps.is_none_or(|fps| fps > GIF_FPS) {
                filters.push(format!("fps={}", GIF_FPS));
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
        OutputFormat::Mp3
        | OutputFormat::M4a
        | OutputFormat::Wav
        | OutputFormat::Flac
        | OutputFormat::Ogg => {
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
        OutputFormat::Png | OutputFormat::Jpg | OutputFormat::Webp | OutputFormat::Avif => {
            let needs_even = format == OutputFormat::Avif;
            let filters = video_filters(input, options, None, needs_even);
            push(&["-map", "0:v:0", "-frames:v", "1"]);
            if !filters.is_empty() {
                push(&["-vf", &filters.join(",")]);
            }
            match format {
                OutputFormat::Png => push(&["-c:v", "png"]),
                OutputFormat::Jpg => {
                    push(&["-c:v", "mjpeg", "-q:v", &lerp(q, 20.0, 2.0).to_string()])
                }
                OutputFormat::Webp => push(&[
                    "-c:v",
                    "libwebp",
                    "-quality",
                    &lerp(q, 40.0, 95.0).to_string(),
                ]),
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

    args.push(output.as_os_str().to_owned());
    Ok(Plan {
        args,
        output: output.to_path_buf(),
        duration_secs,
    })
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
        }
    }

    fn command(input: &MediaInfo, options: &ConvertOptions, out: &str) -> String {
        plan(input, options, Path::new(out))
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
        )
        .expect("plan");
        assert_eq!(
            plan.command_line(),
            "ffmpeg -ss 3.2 -i /clips/garden.mov -t 27.8 -map 0:v:0 -map 0:a:0 -vf scale=-2:720 \
             -c:v libx264 -preset fast -crf 24 -pix_fmt yuv420p -c:a aac -b:a 128k -sn -dn \
             -movflags +faststart /clips/garden.mp4"
        );
        assert!((plan.duration_secs.unwrap() - 27.8).abs() < 1e-9);
    }

    #[test]
    fn untrimmed_plan_uses_full_duration_without_t() {
        let plan = plan(
            &video("a.mov"),
            &ConvertOptions::default(),
            Path::new("/clips/a.mp4"),
        )
        .expect("plan");
        assert!(!plan.args.iter().any(|a| a == "-t" || a == "-ss"));
        assert_eq!(plan.duration_secs, Some(42.0));
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
        let plan = plan(&video("talk.mov"), &options, Path::new("/clips/talk.mp3")).expect("plan");
        assert_eq!(
            plan.command_line(),
            "ffmpeg -i /clips/talk.mov -t 10 -map 0:a:0 -vn -sn -dn -c:a libmp3lame -q:a 0 /clips/talk.mp3"
        );
        assert_eq!(plan.duration_secs, Some(10.0));
    }

    #[test]
    fn audio_from_silent_video_is_rejected() {
        let mut input = video("silent.mov");
        input.audio_codec = None;
        let options = ConvertOptions {
            format: OutputFormat::Mp3,
            ..ConvertOptions::default()
        };
        let err = plan(&input, &options, Path::new("/clips/silent.mp3")).expect_err("no audio");
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
        let plan = plan(&input, &options, Path::new("/clips/photo.jpg")).expect("plan");
        assert_eq!(
            plan.command_line(),
            "ffmpeg -i /clips/photo.png -map 0:v:0 -frames:v 1 -vf hflip,vflip,scale=-2:1080:flags=lanczos \
             -c:v mjpeg -q:v 2 -update 1 -an -sn -dn /clips/photo.jpg"
        );
        assert_eq!(plan.duration_secs, None);
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
                plan(&input, &options, Path::new("/clips/a.mp4")).is_err(),
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
                Path::new("/clips/p.mp4")
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
                &input.path
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

    #[test]
    fn quotes_awkward_paths() {
        assert_eq!(shell_quote("/a/b.mp4"), "/a/b.mp4");
        assert_eq!(shell_quote("my clip.mov"), "'my clip.mov'");
        assert_eq!(shell_quote("it's.mov"), r"'it'\''s.mov'");
    }
}
