//! Before/after samples for comparing a conversion with its original.
//!
//! The editor can show the original next to what the current settings make
//! of it. [`sample_plan`] plans either half for one frame, usually at a
//! reduced size so it is quick to make:
//!
//! - The **original** half runs the conversion's own trim, turn, mirror,
//!   crop and frame-rate filters but writes a lossless PNG instead of
//!   encoding, so the two halves line up pixel for pixel. It skips the
//!   chosen downscale: seeing the detail a smaller size loses is part of
//!   the comparison.
//! - The **converted** half runs the real [`plan`](crate::plan) with the
//!   current settings, so it is encoded exactly as converting would.
//!   Pictures in formats every webview shows (PNG, JPEG, WebP) are shown as
//!   written; everything else, and every video, is decoded to a PNG.
//!
//! Videos and animations are sampled as a short clip that ends on the frame
//! the preview shows (the first at or after the time asked for), [`SAMPLE_LEAD_SECS`] long, so that frame is encoded the
//! way frames in the middle of a clip are rather than as the key frame a
//! clip starts with. Both halves keep the clip's last frame.
//!
//! [`Samples`] renders samples into a private temp folder and keeps only the
//! last few.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use crate::convert::{
    self, ConvertOptions, Direction, FALLBACK_FPS, Metadata, OutputFormat, SoundOptions, Support,
    animation_defaults, result_dims,
};
use crate::{Destination, Error, Ffmpeg, MediaInfo, MediaKind, Plan, Result, RunOutcome};

/// How much of a video plays into the frame a sample compares.
pub const SAMPLE_LEAD_SECS: f64 = 0.5;

/// The shortest clip [`crate::plan`] takes. A frame too close to the start
/// of a video for a lead-in of this length is sampled from where it starts.
const MIN_CLIP_SECS: f64 = 0.06;

/// How far below a frame's start to aim, as the editor's frame stepping
/// does (see `frames.ts`).
const FRAME_NUDGE_SECS: f64 = 0.001;

/// How many rendered samples [`Samples`] keeps on disk.
const KEPT_SAMPLES: usize = 12;

/// Which half of a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SampleSide {
    /// The original, with the conversion's turn, mirror, crop and frame
    /// rate but losslessly, at its own size.
    Original,
    /// What the conversion makes of it.
    Converted,
}

/// Which frame a decoded picture shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Pick {
    /// The first frame at or after this many seconds.
    At(f64),
    /// The very last frame.
    Last,
}

/// One half of a comparison, planned by [`sample_plan`].
#[derive(Debug, Clone, PartialEq)]
pub struct SamplePlan {
    /// Writes `plan.output`: the PNG to show for the original, the encoded
    /// sample for the converted half.
    pub plan: Plan,
    /// How to turn the encoded sample into a picture a webview shows, or
    /// `None` to show `plan.output` as it is.
    pub decode: Option<SampleDecode>,
    /// The converted sample is the whole conversion at full size, so its
    /// size is what converting writes. Only ever true for pictures.
    pub exact: bool,
}

/// Decoding an encoded sample into a PNG (see [`SamplePlan::decode`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleDecode {
    pub to: PathBuf,
    /// Keep the last frame (a video or animation sample), not the first.
    pub last: bool,
    /// The sample is HDR video: tone-map it as the original half is.
    pub tone_map: bool,
}

/// Plan one half of a comparison of `input` converted with `options`, at
/// the frame `at_secs` seconds in (ignored for pictures), writing into
/// `dir`.
///
/// `limit` caps the short side of both halves in pixels, which keeps
/// samples quick; `None` samples at full size. The trim, sound, playback
/// speed and direction of `options` don't change how a frame looks and are
/// left out.
///
/// # Errors
///
/// [`Error::Unsupported`] for sound (input or output), a time that is not
/// a positive number, and anything [`crate::plan`] refuses.
pub fn sample_plan(
    input: &MediaInfo,
    options: &ConvertOptions,
    side: SampleSide,
    at_secs: f64,
    limit: Option<u32>,
    dir: &Path,
    support: impl Into<Support>,
) -> Result<SamplePlan> {
    let support = support.into();
    let format = options.format;
    if input.kind == MediaKind::Audio || format.kind() == MediaKind::Audio {
        return Err(Error::unsupported("sound has no picture to compare"));
    }
    if !at_secs.is_finite() || at_secs < 0.0 {
        return Err(Error::unsupported(
            "the frame time must be a positive number",
        ));
    }
    let video_in = input.kind == MediaKind::Video;
    let animated = format.animates(input.kind);
    // A video or animation comes from a clip; a picture from one frame.
    let moving = video_in && (format.kind() == MediaKind::Video || animated);
    let fps = input.fps.filter(|f| *f > 0.0).unwrap_or(FALLBACK_FPS);
    // Seeking to the very end finds no frame to grab.
    let at = match (video_in, input.duration_secs) {
        (false, _) => 0.0,
        (true, Some(d)) => at_secs.min((d - 1.0 / fps).max(0.0)),
        (true, None) => at_secs,
    };

    // The part of a video sampled, and whether its last frame is the one shown.
    let (trim_start, trim_end, last) = if !video_in {
        (None, None, false)
    } else if !moving {
        // A picture made from a video: the frame at `at`, as the preview shows it.
        ((at > 0.0).then_some(at), None, false)
    } else {
        // The frame the preview shows: the first at or after `at`, which
        // ffmpeg gets in whole milliseconds.
        let ms = (at * 1000.0).round() / 1000.0;
        let mut shown = (ms * fps - 1e-6).ceil().max(0.0);
        if let Some(d) = input.duration_secs {
            shown = shown.min(((d * fps - 1e-6).ceil() - 1.0).max(0.0));
        }
        // Aim a hair below a frame's start, as the editor does: both ends of
        // the clip then sit well clear of frame boundaries, which ffmpeg
        // rounds times to.
        let frame_start = |frame: f64| (frame > 0.0).then(|| frame / fps - FRAME_NUDGE_SECS);
        let end = (shown + 0.5) / fps;
        if end < MIN_CLIP_SECS {
            // Too close to the start to lead in: start at the frame, as the clip does.
            let start = frame_start(shown);
            (start, Some(start.unwrap_or(0.0) + SAMPLE_LEAD_SECS), false)
        } else {
            // Lead in, and stop half a frame after the frame shown, so it comes last.
            let first = (shown - (SAMPLE_LEAD_SECS * fps).round()).max(0.0);
            (frame_start(first), Some(end), true)
        }
    };

    // Sound, playback and destination don't change how a frame looks.
    let base = ConvertOptions {
        trim_start,
        trim_end,
        keep_audio: false,
        sound: SoundOptions::default(),
        speed: 1.0,
        direction: Direction::Forward,
        destination: Destination::NextToInput,
        ..options.clone()
    };

    match side {
        SampleSide::Original => {
            let output = dir.join("original.png");
            let plan = if moving {
                // The animation defaults (e.g. 480p GIFs) are lifted like the
                // chosen size; only `limit` shrinks the original.
                let max_size = limit.or_else(|| animated.then_some(u32::MAX));
                let options = ConvertOptions { max_size, ..base };
                convert::frames_plan(input, &options, &output, last, support)?
            } else {
                let options = ConvertOptions {
                    format: OutputFormat::Png,
                    max_size: limit,
                    metadata: Metadata::Keep,
                    video_codec: None,
                    video_bitrate: None,
                    hardware: false,
                    ..base
                };
                convert::plan(input, &options, &output, support)?
            };
            Ok(SamplePlan {
                plan,
                decode: None,
                exact: false,
            })
        }
        SampleSide::Converted => {
            let shown = options
                .max_size
                .or_else(|| animated.then(|| animation_defaults(format).0).flatten());
            let max_size = match (shown, limit) {
                (Some(size), Some(limit)) => Some(size.min(limit)),
                (size, limit) => size.or(limit),
            };
            let options_now = ConvertOptions { max_size, ..base };
            let output = dir.join(format!("sample.{}", format.extension()));
            let plan = convert::plan(input, &options_now, &output, support)?;
            let shows_itself = !moving
                && matches!(
                    format,
                    OutputFormat::Png | OutputFormat::Jpg | OutputFormat::Webp
                );
            let decode = (!shows_itself).then(|| SampleDecode {
                to: dir.join("shown.png"),
                last,
                // Video files keep HDR; pictures and animations were tone-mapped.
                tone_map: input.hdr
                    && format.kind() == MediaKind::Video
                    && format != OutputFormat::Gif,
            });
            let exact = !video_in
                && limit.is_none_or(|limit| {
                    result_dims(input, options).is_some_and(|(w, h)| w.min(h) <= limit)
                });
            Ok(SamplePlan {
                plan,
                decode,
                exact,
            })
        }
    }
}

/// A rendered half of a comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    /// The picture to show: a PNG, or the encoded sample itself where every
    /// webview shows its format.
    pub path: PathBuf,
    /// Size in bytes of the encoded sample, or of the finished file shown.
    /// `None` for the original half of a comparison.
    pub bytes: Option<u64>,
    /// `bytes` is the size of the whole result, not just of a sample.
    pub exact: bool,
}

/// Renders comparison samples, each into its own folder inside a private
/// temp folder that is deleted along with this. Only the last few samples
/// are kept on disk.
#[derive(Debug, Default)]
pub struct Samples {
    root: Mutex<Option<tempfile::TempDir>>,
    recent: Mutex<VecDeque<PathBuf>>,
    next: AtomicU64,
}

impl Samples {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The folder every sample is written under, created on first use, so a
    /// webview can be allowed to read from it.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the temp folder can't be created.
    pub fn root(&self) -> Result<PathBuf> {
        let mut root = self.root.lock().unwrap_or_else(PoisonError::into_inner);
        if root.is_none() {
            let dir = tempfile::Builder::new()
                .prefix("catconvert-compare-")
                .tempdir()
                .map_err(|source| Error::Io {
                    path: std::env::temp_dir(),
                    source,
                })?;
            *root = Some(dir);
        }
        Ok(root
            .as_ref()
            .map(|dir| dir.path().to_path_buf())
            .unwrap_or_default())
    }

    /// A fresh folder for one sample. The oldest beyond the last few go.
    fn slot(&self) -> Result<PathBuf> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let dir = self.root()?.join(id.to_string());
        std::fs::create_dir(&dir).map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })?;
        let mut recent = self.recent.lock().unwrap_or_else(PoisonError::into_inner);
        recent.push_back(dir.clone());
        while recent.len() > KEPT_SAMPLES {
            if let Some(old) = recent.pop_front() {
                let _ = std::fs::remove_dir_all(old);
            }
        }
        Ok(dir)
    }

    /// Render one half of a comparison (see [`sample_plan`]). Blocks until
    /// it is written.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] if this ffmpeg can't read the input or write
    /// the format, or [`sample_plan`] refuses; [`Error::Ffmpeg`] if a step
    /// fails.
    pub fn render(
        &self,
        ffmpeg: &Ffmpeg,
        input: &MediaInfo,
        options: &ConvertOptions,
        side: SampleSide,
        at_secs: f64,
        limit: Option<u32>,
    ) -> Result<Sample> {
        if let Some(message) = ffmpeg.unreadable(input) {
            return Err(Error::Unsupported { message });
        }
        if side == SampleSide::Converted {
            let missing = if options.format.animates(input.kind) {
                ffmpeg.missing_animation(options.format)
            } else {
                ffmpeg.missing_output(options)
            };
            if let Some(message) = missing {
                return Err(Error::Unsupported { message });
            }
        }
        let dir = self.slot()?;
        let planned = sample_plan(input, options, side, at_secs, limit, &dir, ffmpeg.support())?;
        let RunOutcome::Done { output_bytes } =
            ffmpeg.run(&planned.plan, &AtomicBool::new(false), |_| {})?
        else {
            unreachable!("nothing raises the cancel flag");
        };
        let path = match &planned.decode {
            Some(decode) => {
                let pick = if decode.last {
                    Pick::Last
                } else {
                    Pick::At(0.0)
                };
                ffmpeg.decode_png(
                    &planned.plan.output,
                    pick,
                    None,
                    decode.tone_map,
                    &decode.to,
                )?;
                decode.to.clone()
            }
            None => planned.plan.output.clone(),
        };
        Ok(Sample {
            path,
            bytes: (side == SampleSide::Converted).then_some(output_bytes),
            exact: planned.exact,
        })
    }

    /// The frame of a finished file at `at_secs` (pictures ignore it, and
    /// a time past the end shows the last frame) as a PNG whose short side
    /// is at most `limit`, to compare a result with its original.
    /// `tone_map` turns an HDR video into SDR as the original half is.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the file is gone, [`Error::Ffmpeg`] if it can't be
    /// decoded.
    pub fn render_file(
        &self,
        ffmpeg: &Ffmpeg,
        path: &Path,
        at_secs: f64,
        limit: Option<u32>,
        tone_map: bool,
    ) -> Result<Sample> {
        let bytes = std::fs::metadata(path)
            .map_err(|source| Error::Io {
                path: path.to_path_buf(),
                source,
            })?
            .len();
        let dir = self.slot()?;
        let out = dir.join("file.png");
        let at = if at_secs.is_finite() {
            at_secs.max(0.0)
        } else {
            0.0
        };
        ffmpeg
            .decode_png(path, Pick::At(at), limit, tone_map, &out)
            .or_else(|err| {
                if at > 0.0 {
                    ffmpeg.decode_png(path, Pick::Last, limit, tone_map, &out)
                } else {
                    Err(err)
                }
            })?;
        Ok(Sample {
            path: out,
            bytes: Some(bytes),
            exact: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{Crop, JxlSupport, Tool, VideoCodec};

    const TONE_MAP: Support = Support {
        jxl: JxlSupport {
            cjxl: false,
            djxl: false,
            ffmpeg_encoder: false,
            ffmpeg_anim_encoder: false,
            ffmpeg_decoder: false,
            ffmpeg_anim_decoder: false,
        },
        tone_map: true,
        hevc_hardware: false,
    };

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

    fn photo(name: &str, w: u32, h: u32) -> MediaInfo {
        MediaInfo {
            kind: MediaKind::Image,
            extension: Path::new(name)
                .extension()
                .map(|e| e.to_string_lossy().into_owned()),
            duration_secs: None,
            fps: None,
            audio_codec: None,
            sample_rate: None,
            audio_channels: None,
            width: Some(w),
            height: Some(h),
            video_codec: Some("mjpeg".into()),
            ..video(name)
        }
    }

    fn dir() -> &'static Path {
        Path::new("/tmp/sample")
    }

    fn sample(
        input: &MediaInfo,
        options: &ConvertOptions,
        side: SampleSide,
        at: f64,
        limit: Option<u32>,
    ) -> SamplePlan {
        sample_plan(input, options, side, at, limit, dir(), TONE_MAP).expect("sample plan")
    }

    fn webp(quality: u8) -> ConvertOptions {
        ConvertOptions {
            format: OutputFormat::Webp,
            quality,
            ..ConvertOptions::default()
        }
    }

    #[test]
    fn a_photo_is_encoded_with_the_settings_at_preview_size() {
        let input = photo("cat.jpg", 4000, 3000);
        let options = ConvertOptions {
            rotate: 90,
            ..webp(73)
        };
        let converted = sample(&input, &options, SampleSide::Converted, 9.0, Some(1080));
        assert_eq!(
            converted.plan.command_line(),
            "ffmpeg -i /clips/cat.jpg -map 0:v:0 -frames:v 1 \
             -vf transpose=clock,scale=1080:-2:flags=lanczos -c:v libwebp -quality 80 \
             -update 1 -an -sn -dn /tmp/sample/sample.webp"
        );
        // Webviews show WebP as it is, but the size is only a sample's.
        assert_eq!(converted.decode, None);
        assert!(!converted.exact);

        let original = sample(&input, &options, SampleSide::Original, 9.0, Some(1080));
        assert_eq!(
            original.plan.command_line(),
            "ffmpeg -i /clips/cat.jpg -map 0:v:0 -frames:v 1 \
             -vf transpose=clock,scale=1080:-2:flags=lanczos -c:v png \
             -update 1 -an -sn -dn /tmp/sample/original.png"
        );
    }

    #[test]
    fn a_full_size_photo_sample_is_the_real_conversion() {
        let input = photo("cat.jpg", 1600, 1200);
        let options = webp(73);
        let real =
            convert::plan(&input, &options, &dir().join("sample.webp"), TONE_MAP).expect("plan");
        for limit in [None, Some(1200), Some(1440)] {
            let converted = sample(&input, &options, SampleSide::Converted, 0.0, limit);
            assert_eq!(converted.plan, real, "limit {limit:?}");
            assert!(converted.exact, "limit {limit:?}");
        }
        assert!(!sample(&input, &options, SampleSide::Converted, 0.0, Some(1080)).exact);
    }

    #[test]
    fn formats_webviews_may_not_show_are_decoded_to_png() {
        let input = photo("cat.jpg", 4000, 3000);
        for format in [OutputFormat::Avif, OutputFormat::Jxl] {
            let options = ConvertOptions {
                format,
                ..ConvertOptions::default()
            };
            let support = Support {
                jxl: JxlSupport {
                    cjxl: true,
                    djxl: true,
                    ..JxlSupport::default()
                },
                ..TONE_MAP
            };
            let converted = sample_plan(
                &input,
                &options,
                SampleSide::Converted,
                0.0,
                Some(720),
                dir(),
                support,
            )
            .expect("plan");
            assert_eq!(
                converted.plan.output,
                dir().join(format!("sample.{}", format.extension()))
            );
            assert_eq!(
                converted.decode,
                Some(SampleDecode {
                    to: dir().join("shown.png"),
                    last: false,
                    tone_map: false,
                })
            );
        }
    }

    #[test]
    fn the_original_half_keeps_the_crop_but_not_the_chosen_size_or_metadata() {
        let input = photo("cat.jpg", 4000, 3000);
        let options = ConvertOptions {
            format: OutputFormat::Jpg,
            max_size: Some(720),
            metadata: Metadata::None,
            crop: Some(Crop {
                aspect: Some([1, 1]),
                area: None,
            }),
            ..ConvertOptions::default()
        };
        let original = sample(&input, &options, SampleSide::Original, 0.0, None)
            .plan
            .command_line();
        assert!(
            original.contains("-vf crop=3000:3000:500:0 -c:v png"),
            "{original}"
        );
        assert!(!original.contains("scale"), "{original}");
        assert!(!original.contains("map_metadata"), "{original}");
        let converted = sample(&input, &options, SampleSide::Converted, 0.0, None)
            .plan
            .command_line();
        assert!(
            converted.contains("crop=3000:3000:500:0,scale=-2:720:flags=lanczos"),
            "{converted}"
        );
        assert!(converted.contains("-map_metadata -1"), "{converted}");
    }

    #[test]
    fn a_video_is_sampled_as_a_short_clip_ending_on_the_frame() {
        let input = video("garden.mov");
        let options = ConvertOptions {
            format: OutputFormat::Mp4,
            video_codec: Some(VideoCodec::H265),
            quality: 40,
            trim_start: Some(30.0),
            trim_end: Some(40.0),
            sound: SoundOptions {
                fade_in: 1.0,
                ..SoundOptions::default()
            },
            ..ConvertOptions::default()
        };
        let converted = sample(&input, &options, SampleSide::Converted, 12.0, Some(720));
        let cmd = converted.plan.command_line();
        // Half a second of lead-in, ending half a frame after the one at 12 s;
        // no sound, no fades.
        assert!(
            cmd.starts_with("ffmpeg -ss 11.499 -i /clips/garden.mov -t 0.518 -map 0:v:0 -vf scale=-2:720 -c:v libx265"),
            "{cmd}"
        );
        assert!(cmd.contains(" -an "), "{cmd}");
        assert!(!cmd.contains("fade"), "{cmd}");
        assert!(cmd.ends_with("/tmp/sample/sample.mp4"), "{cmd}");
        assert_eq!(
            converted.decode,
            Some(SampleDecode {
                to: dir().join("shown.png"),
                last: true,
                tone_map: false,
            })
        );
        assert!(!converted.exact);

        let original = sample(&input, &options, SampleSide::Original, 12.0, Some(720));
        assert_eq!(
            original.plan.command_line(),
            "ffmpeg -ss 11.499 -i /clips/garden.mov -t 0.518 -map 0:v:0 -vf scale=-2:720 \
             -c:v png -compression_level 1 -update 1 -an -sn -dn /tmp/sample/original.png"
        );
    }

    #[test]
    fn a_video_sample_ends_on_the_frame_the_preview_shows() {
        let input = video("garden.mov");
        let options = ConvertOptions::default();
        // At 30 fps frame 60 starts at 2 s; ffmpeg shows it for any time after
        // frame 59 starts (1.9667 s), in milliseconds from 1.967 s. Its clip
        // starts 15 frames earlier, at frame 45 (1.5 s).
        for at in [1.967, 1.98, 1.999, 2.0] {
            let cmd = sample(&input, &options, SampleSide::Original, at, None)
                .plan
                .command_line();
            assert!(cmd.starts_with("ffmpeg -ss 1.499 -i"), "{at}: {cmd}");
            assert!(cmd.contains(" -t 0.518 "), "{at}: {cmd}");
        }
        let cmd = sample(&input, &options, SampleSide::Original, 1.966, None)
            .plan
            .command_line();
        assert!(cmd.starts_with("ffmpeg -ss 1.466 -i"), "{cmd}");
    }

    #[test]
    fn the_start_of_a_video_is_sampled_from_its_first_frame() {
        let input = video("garden.mov");
        let options = ConvertOptions::default();
        let converted = sample(&input, &options, SampleSide::Converted, 0.0, None);
        let cmd = converted.plan.command_line();
        assert!(
            cmd.starts_with("ffmpeg -i /clips/garden.mov -t 0.5 "),
            "{cmd}"
        );
        assert!(!converted.decode.expect("decoded").last);
        let original = sample(&input, &options, SampleSide::Original, 0.0, None);
        let cmd = original.plan.command_line();
        assert!(cmd.contains("-map 0:v:0 -frames:v 1 -c:v png"), "{cmd}");
    }

    #[test]
    fn the_end_of_a_video_is_its_last_frame() {
        let input = video("garden.mov");
        let cmd = sample(
            &input,
            &ConvertOptions::default(),
            SampleSide::Converted,
            99.0,
            None,
        )
        .plan
        .command_line();
        // 42 s at 30 fps: the last frame, 1259, starts at 41.967 s.
        assert!(cmd.starts_with("ffmpeg -ss 41.466 -i"), "{cmd}");
        assert!(cmd.contains(" -t 0.518 "), "{cmd}");
    }

    #[test]
    fn a_picture_from_a_video_is_the_frame_itself() {
        let input = video("garden.mov");
        let options = ConvertOptions {
            format: OutputFormat::Jpg,
            ..ConvertOptions::default()
        };
        for side in [SampleSide::Original, SampleSide::Converted] {
            let planned = sample(&input, &options, side, 7.25, Some(540));
            let cmd = planned.plan.command_line();
            assert!(
                cmd.starts_with("ffmpeg -ss 7.25 -i /clips/garden.mov -map 0:v:0 -frames:v 1"),
                "{cmd}"
            );
            assert!(!cmd.contains(" -t "), "{cmd}");
            assert_eq!(planned.decode, None);
            assert!(!planned.exact);
        }
    }

    #[test]
    fn gifs_keep_their_frame_rate_but_the_original_keeps_its_size() {
        let input = video("garden.mov");
        let options = ConvertOptions {
            format: OutputFormat::Gif,
            speed: 2.0,
            direction: Direction::Boomerang,
            ..ConvertOptions::default()
        };
        let converted = sample(&input, &options, SampleSide::Converted, 5.0, None)
            .plan
            .command_line();
        assert!(
            converted.contains("fps=15,scale=-2:480:flags=lanczos,split"),
            "{converted}"
        );
        // Playback changes don't alter how a frame looks.
        assert!(
            !converted.contains("setpts") && !converted.contains("reverse"),
            "{converted}"
        );
        let original = sample(&input, &options, SampleSide::Original, 5.0, None)
            .plan
            .command_line();
        assert!(original.contains("-vf fps=15 -c:v png"), "{original}");
        let small = sample(&input, &options, SampleSide::Original, 5.0, Some(360))
            .plan
            .command_line();
        assert!(small.contains("fps=15,scale=-2:360"), "{small}");
        let converted = sample(&input, &options, SampleSide::Converted, 5.0, Some(360))
            .plan
            .command_line();
        assert!(converted.contains("scale=-2:360"), "{converted}");
    }

    #[test]
    fn hdr_video_is_tone_mapped_on_both_sides() {
        let input = MediaInfo {
            hdr: true,
            ..video("iphone.mov")
        };
        let options = ConvertOptions {
            format: OutputFormat::Mov,
            video_codec: Some(VideoCodec::H265),
            ..ConvertOptions::default()
        };
        let original = sample(&input, &options, SampleSide::Original, 3.0, None)
            .plan
            .command_line();
        assert!(
            original.contains("-vf scale=out_transfer=bt709"),
            "{original}"
        );
        let converted = sample(&input, &options, SampleSide::Converted, 3.0, None);
        assert!(converted.decode.expect("decoded").tone_map);
        // A GIF is tone-mapped when it is made.
        let gif = ConvertOptions {
            format: OutputFormat::Gif,
            ..ConvertOptions::default()
        };
        let converted = sample(&input, &gif, SampleSide::Converted, 3.0, None);
        assert!(!converted.decode.expect("decoded").tone_map);
    }

    #[test]
    fn jxl_input_is_unpacked_for_the_original_too() {
        let input = MediaInfo {
            extension: Some("jxl".into()),
            video_codec: Some("jpegxl".into()),
            ..video("loop.jxl")
        };
        let support = Support {
            jxl: JxlSupport {
                djxl: true,
                ..JxlSupport::default()
            },
            ..TONE_MAP
        };
        let options = ConvertOptions {
            format: OutputFormat::Webm,
            ..ConvertOptions::default()
        };
        let original = sample_plan(
            &input,
            &options,
            SampleSide::Original,
            1.0,
            None,
            dir(),
            support,
        )
        .expect("plan");
        let tools: Vec<Tool> = original.plan.steps.iter().map(|s| s.tool).collect();
        assert_eq!(tools, [Tool::Djxl, Tool::Ffmpeg]);
        assert_eq!(original.plan.scratch.len(), 1);
    }

    #[test]
    fn sound_and_bad_times_are_refused() {
        let song = MediaInfo {
            kind: MediaKind::Audio,
            width: None,
            height: None,
            video_codec: None,
            ..video("song.mp3")
        };
        let err = sample_plan(
            &song,
            &ConvertOptions::default(),
            SampleSide::Converted,
            0.0,
            None,
            dir(),
            TONE_MAP,
        )
        .expect_err("sound");
        assert!(err.to_string().contains("no picture"), "{err}");
        let mp3 = ConvertOptions {
            format: OutputFormat::Mp3,
            ..ConvertOptions::default()
        };
        assert!(
            sample_plan(
                &video("a.mov"),
                &mp3,
                SampleSide::Original,
                0.0,
                None,
                dir(),
                TONE_MAP
            )
            .is_err()
        );
        let err = sample_plan(
            &video("a.mov"),
            &ConvertOptions::default(),
            SampleSide::Original,
            f64::NAN,
            None,
            dir(),
            TONE_MAP,
        )
        .expect_err("bad time");
        assert!(err.to_string().contains("positive"), "{err}");
    }
}
