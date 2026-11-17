//! `catconvert`: command-line front end over `catconvert-core`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use catconvert_core::{
    AudioRate, ConvertOptions, Crop, Direction, Ffmpeg, Fit, JxlOptions, Metadata, OutputFormat,
    Plan, QualitySummary, RunOutcome, SequenceOptions, SoundOptions, VIDEO_KBPS, VideoCodec,
};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "catconvert",
    version,
    about = "Offline photo, video and audio converter"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print information about a media file as JSON (stream details need ffprobe).
    Probe {
        /// File to inspect.
        file: PathBuf,
        /// Print compact single-line JSON instead of pretty-printed output.
        #[arg(long)]
        compact: bool,
    },
    /// Convert a file with ffmpeg (and libjxl's tools for JPEG XL). The result
    /// is written next to the input.
    Convert {
        /// File to convert.
        file: PathBuf,
        /// Target format.
        #[arg(long, short)]
        to: Format,
        /// Start of the kept range, in seconds.
        #[arg(long)]
        start: Option<f64>,
        /// End of the kept range, in seconds.
        #[arg(long)]
        end: Option<f64>,
        /// Shrink so the short side is at most this many pixels (e.g. 720).
        #[arg(long)]
        size: Option<u32>,
        /// Output frame rate.
        #[arg(long)]
        fps: Option<f64>,
        /// Clockwise rotation in degrees (0, 90, 180, 270).
        #[arg(long, default_value_t = 0)]
        rotate: u16,
        /// Mirror horizontally.
        #[arg(long)]
        flip: bool,
        /// Crop to this width:height, keeping the centre (e.g. 1:1, 9:16).
        #[arg(long, value_name = "W:H", value_parser = parse_aspect)]
        crop: Option<[u32; 2]>,
        /// 0 = smallest file, 100 = best quality.
        #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u8).range(0..=100))]
        quality: u8,
        /// MP4, MKV and WebM: video codec. Defaults to H.264 (VP9 for WebM);
        /// AV1 makes smaller files and keeps HDR, but is slower. MOV is H.264 only.
        #[arg(long, value_enum)]
        codec: Option<Codec>,
        /// MP4, MOV, MKV and WebM: aim for this average video bitrate instead
        /// of a quality level, in kbit/s or with a unit (e.g. 800k, 4M). H.264
        /// and VP9 are encoded in two passes.
        #[arg(long, value_name = "RATE", value_parser = parse_bitrate)]
        bitrate: Option<u32>,
        /// Drop the audio track.
        #[arg(long)]
        mute: bool,
        /// Even out loudness (to -16 LUFS).
        #[arg(long)]
        normalize: bool,
        /// Fade in over this many seconds (video fades from black too).
        #[arg(long, default_value_t = 0.0)]
        fade_in: f64,
        /// Fade out over this many seconds (video fades to black too).
        #[arg(long, default_value_t = 0.0)]
        fade_out: f64,
        /// JPEG XL: mathematically lossless (an unedited JPEG is recompressed
        /// losslessly and can be restored bit for bit).
        #[arg(long)]
        lossless: bool,
        /// JPEG XL: encoder effort, 1 (fast) to 10 (smallest).
        #[arg(long, default_value_t = 7, value_parser = clap::value_parser!(u8).range(1..=10))]
        effort: u8,
        /// JPEG XL: progressive decoding.
        #[arg(long)]
        progressive: bool,
        /// Which dates, places, titles and other tags to carry over.
        #[arg(long, value_enum, default_value_t = Tags::NoLocation)]
        metadata: Tags,
        /// Same as `--metadata none`.
        #[arg(long, hide = true, conflicts_with = "metadata")]
        strip_metadata: bool,
        /// GIF and animated JPEG XL: playback speed, 0.25 to 4 (e.g. 2 for 2×).
        #[arg(long, default_value_t = 1.0)]
        speed: f64,
        /// GIF and animated JPEG XL: play forward, reversed, or forward then back.
        #[arg(long, value_enum, default_value_t = Playback::Forward)]
        direction: Playback,
        /// Print the command(s) instead of running them.
        #[arg(long)]
        dry_run: bool,
    },
    /// Make one animation (GIF, WebP, JPEG XL or a video slideshow) from
    /// several pictures, in the order given. It takes the size of the first
    /// picture and is written next to it.
    Animate {
        /// Pictures, in order.
        #[arg(required = true, num_args = 2..)]
        pictures: Vec<PathBuf>,
        /// Target format.
        #[arg(long, short)]
        to: Format,
        /// Seconds each picture shows.
        #[arg(long, default_value_t = 0.5)]
        frame: f64,
        /// Crop pictures of another shape to fill the frame instead of adding
        /// black bars.
        #[arg(long)]
        crop: bool,
        /// Order the pictures play in.
        #[arg(long, value_enum, default_value_t = Playback::Forward)]
        direction: Playback,
        /// Shrink so the short side is at most this many pixels (e.g. 480).
        #[arg(long)]
        size: Option<u32>,
        /// Clockwise rotation in degrees (0, 90, 180, 270).
        #[arg(long, default_value_t = 0)]
        rotate: u16,
        /// Mirror horizontally.
        #[arg(long)]
        flip: bool,
        /// 0 = smallest file, 100 = best quality.
        #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u8).range(0..=100))]
        quality: u8,
        /// MP4 slideshow: video codec (H.264 unless set).
        #[arg(long, value_enum)]
        codec: Option<Codec>,
        /// JPEG XL: mathematically lossless.
        #[arg(long)]
        lossless: bool,
        /// JPEG XL: encoder effort, 1 (fast) to 10 (smallest).
        #[arg(long, default_value_t = 7, value_parser = clap::value_parser!(u8).range(1..=10))]
        effort: u8,
        /// Print the command(s) instead of running them.
        #[arg(long)]
        dry_run: bool,
    },
    /// Copy a JPEG or PNG without its location or all of its metadata. The
    /// picture itself is copied untouched, so a JPEG stays bit-identical
    /// apart from the metadata.
    Scrub {
        /// JPEG or PNG to read.
        input: PathBuf,
        /// Where to write the copy; must not exist yet.
        output: PathBuf,
        /// What to take out.
        #[arg(long, value_enum)]
        drop: Strip,
    },
}

/// CLI mirror of [`Metadata`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Tags {
    /// Everything, where the format can hold it.
    Keep,
    /// Everything except where the file was made (GPS, location tags).
    NoLocation,
    /// Nothing; colour profiles stay.
    None,
}

impl From<Tags> for Metadata {
    fn from(tags: Tags) -> Self {
        match tags {
            Tags::Keep => Self::Keep,
            Tags::NoLocation => Self::NoLocation,
            Tags::None => Self::None,
        }
    }
}

/// What `scrub` takes out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Strip {
    /// GPS coordinates in EXIF and XMP.
    Location,
    /// EXIF, XMP, IPTC, comments and text; colour profiles stay.
    All,
}

/// CLI mirror of [`Direction`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Playback {
    Forward,
    Reverse,
    Boomerang,
}

impl From<Playback> for Direction {
    fn from(playback: Playback) -> Self {
        match playback {
            Playback::Forward => Self::Forward,
            Playback::Reverse => Self::Reverse,
            Playback::Boomerang => Self::Boomerang,
        }
    }
}

/// CLI mirror of [`VideoCodec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Codec {
    H264,
    Vp9,
    Av1,
}

impl From<Codec> for VideoCodec {
    fn from(codec: Codec) -> Self {
        match codec {
            Codec::H264 => Self::H264,
            Codec::Vp9 => Self::Vp9,
            Codec::Av1 => Self::Av1,
        }
    }
}

/// CLI mirror of [`OutputFormat`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
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

impl From<Format> for OutputFormat {
    fn from(format: Format) -> Self {
        match format {
            Format::Mp4 => Self::Mp4,
            Format::Webm => Self::Webm,
            Format::Mov => Self::Mov,
            Format::Mkv => Self::Mkv,
            Format::Gif => Self::Gif,
            Format::Mp3 => Self::Mp3,
            Format::M4a => Self::M4a,
            Format::Wav => Self::Wav,
            Format::Flac => Self::Flac,
            Format::Ogg => Self::Ogg,
            Format::Png => Self::Png,
            Format::Jpg => Self::Jpg,
            Format::Webp => Self::Webp,
            Format::Avif => Self::Avif,
            Format::Jxl => Self::Jxl,
        }
    }
}

/// `9:16` → `[9, 16]`.
fn parse_aspect(text: &str) -> Result<[u32; 2], String> {
    let parse = |part: &str| part.trim().parse::<u32>().ok().filter(|&n| n > 0);
    text.split_once(':')
        .and_then(|(w, h)| Some([parse(w)?, parse(h)?]))
        .ok_or_else(|| format!("expected a shape like 1:1 or 9:16, got `{text}`"))
}

fn convert(file: &PathBuf, options: &ConvertOptions, dry_run: bool) -> Result<(), String> {
    let ffmpeg = Ffmpeg::locate().map_err(|e| e.to_string())?;
    if let Some(reason) = ffmpeg.missing_output(options) {
        return Err(reason);
    }
    let info = ffmpeg.probe(file).map_err(|e| e.to_string())?;
    if let Some(reason) = ffmpeg.unreadable(&info) {
        return Err(reason);
    }
    let output = catconvert_core::output_path(&info.path, options.format, |_| false);
    let plan = catconvert_core::plan(&info, options, &output, ffmpeg.support())
        .map_err(|e| e.to_string())?;
    if let Some(line) = describe_quality(catconvert_core::quality_summary(&info, options)) {
        eprintln!("{line}");
    }
    run(&ffmpeg, &plan, dry_run)
}

/// The quality setting in numbers, e.g. `video H.264 CRF 24, audio 128 kbps`.
fn describe_quality(summary: QualitySummary) -> Option<String> {
    let video = summary.video.map(|rate| match (rate.kbps, rate.crf) {
        (Some(kbps), _) if rate.passes > 1 => {
            format!(
                "video {} {kbps} kbps, {} passes",
                rate.codec.label(),
                rate.passes
            )
        }
        (Some(kbps), _) => format!("video {} {kbps} kbps average", rate.codec.label()),
        (None, crf) => format!(
            "video {} CRF {}",
            rate.codec.label(),
            crf.unwrap_or_default()
        ),
    });
    let audio = summary.audio.map(|rate| match rate {
        AudioRate::Bitrate { kbps } => format!("audio {kbps} kbps"),
        AudioRate::Vbr { level, kbps } => format!("audio VBR V{level} (about {kbps} kbps)"),
        AudioRate::Lossless => "audio lossless".to_owned(),
        AudioRate::Pcm { kbps: Some(kbps) } => format!("audio PCM {kbps} kbps"),
        AudioRate::Pcm { kbps: None } => "audio PCM".to_owned(),
    });
    let parts: Vec<_> = video.into_iter().chain(audio).collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// A bitrate in kbit/s from `800`, `800k`, `4M` or `4.5M`.
fn parse_bitrate(text: &str) -> Result<u32, String> {
    let lower = text.trim().to_ascii_lowercase();
    let lower = lower.trim_end_matches("bps").trim_end_matches("bit/s");
    let (number, scale) = match lower.strip_suffix('m') {
        Some(number) => (number, 1000.0),
        None => (lower.strip_suffix('k').unwrap_or(lower), 1.0),
    };
    let kbps = number
        .trim()
        .parse::<f64>()
        .ok()
        .map(|n| (n * scale).round())
        .filter(|kbps| kbps.is_finite() && *kbps >= 0.0 && *kbps <= f64::from(u32::MAX))
        .ok_or_else(|| format!("expected a bitrate like 800k or 4M, got `{text}`"))?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let kbps = kbps as u32;
    if VIDEO_KBPS.contains(&kbps) {
        Ok(kbps)
    } else {
        Err(format!(
            "the bitrate must be between {}k and {}M",
            VIDEO_KBPS.start(),
            VIDEO_KBPS.end() / 1000
        ))
    }
}

fn animate(
    pictures: &[PathBuf],
    options: &ConvertOptions,
    sequence: &SequenceOptions,
    dry_run: bool,
) -> Result<(), String> {
    let ffmpeg = Ffmpeg::locate().map_err(|e| e.to_string())?;
    if let Some(reason) = ffmpeg
        .missing_animation(options.format)
        .or_else(|| ffmpeg.missing_output(options))
    {
        return Err(reason);
    }
    let infos = pictures
        .iter()
        .map(|path| ffmpeg.probe(path))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if let Some(reason) = infos.iter().find_map(|info| ffmpeg.unreadable(info)) {
        return Err(reason);
    }
    let output = catconvert_core::sequence_output_path(&infos, options.format, |_| false);
    let plan = catconvert_core::plan_sequence(&infos, options, sequence, &output, ffmpeg.support())
        .map_err(|e| e.to_string())?;
    run(&ffmpeg, &plan, dry_run)
}

fn run(ffmpeg: &Ffmpeg, plan: &Plan, dry_run: bool) -> Result<(), String> {
    let output = &plan.output;
    if dry_run {
        println!("{}", plan.command_line());
        return Ok(());
    }
    let cancel = AtomicBool::new(false);
    let mut shown = 0.0_f32;
    let outcome = ffmpeg
        .run(plan, &cancel, |p| {
            if let Some(fraction) = p.fraction.filter(|f| *f > shown) {
                shown = fraction;
                eprint!("\r{:>5.1}%", fraction * 100.0);
            }
        })
        .map_err(|e| e.to_string())?;
    eprintln!();
    if let RunOutcome::Done { output_bytes } = outcome {
        println!("{} ({output_bytes} bytes)", output.display());
    }
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Probe { file, compact } => match Ffmpeg::locate().map_or_else(
            |_| catconvert_core::probe(&file),
            |ffmpeg| ffmpeg.probe(&file),
        ) {
            Ok(info) => {
                let json = if compact {
                    serde_json::to_string(&info)
                } else {
                    serde_json::to_string_pretty(&info)
                };
                match json {
                    Ok(json) => {
                        println!("{json}");
                        ExitCode::SUCCESS
                    }
                    Err(err) => {
                        eprintln!("error: failed to serialize result: {err}");
                        ExitCode::FAILURE
                    }
                }
            }
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::FAILURE
            }
        },
        Command::Convert {
            file,
            to,
            start,
            end,
            size,
            fps,
            rotate,
            flip,
            crop,
            quality,
            codec,
            bitrate,
            mute,
            normalize,
            fade_in,
            fade_out,
            lossless,
            effort,
            progressive,
            metadata,
            strip_metadata,
            speed,
            direction,
            dry_run,
        } => {
            let options = ConvertOptions {
                format: to.into(),
                trim_start: start,
                trim_end: end,
                max_size: size,
                fps,
                rotate,
                flip,
                crop: crop.map(|aspect| Crop {
                    aspect: Some(aspect),
                    area: None,
                }),
                quality,
                keep_audio: !mute,
                video_codec: codec.map(Into::into),
                video_bitrate: bitrate,
                jxl: JxlOptions {
                    lossless,
                    effort,
                    progressive,
                },
                sound: SoundOptions {
                    normalize,
                    fade_in,
                    fade_out,
                },
                metadata: if strip_metadata {
                    Metadata::None
                } else {
                    metadata.into()
                },
                speed,
                direction: direction.into(),
                // The command line always writes next to the original.
                destination: catconvert_core::Destination::NextToInput,
            };
            report(convert(&file, &options, dry_run))
        }
        Command::Animate {
            pictures,
            to,
            frame,
            crop,
            direction,
            size,
            rotate,
            flip,
            quality,
            codec,
            lossless,
            effort,
            dry_run,
        } => {
            let options = ConvertOptions {
                format: to.into(),
                max_size: size,
                rotate,
                flip,
                quality,
                video_codec: codec.map(Into::into),
                jxl: JxlOptions {
                    lossless,
                    effort,
                    progressive: false,
                },
                direction: direction.into(),
                ..ConvertOptions::default()
            };
            let sequence = SequenceOptions {
                frame_secs: frame,
                fit: if crop { Fit::Crop } else { Fit::Pad },
            };
            report(animate(&pictures, &options, &sequence, dry_run))
        }
        Command::Scrub {
            input,
            output,
            drop,
        } => {
            let metadata = match drop {
                Strip::Location => Metadata::NoLocation,
                Strip::All => Metadata::None,
            };
            report(
                catconvert_core::scrub_file(&input, &output, metadata).map_err(|e| e.to_string()),
            )
        }
    }
}

fn report(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_probe() {
        let cli =
            Cli::try_parse_from(["catconvert", "probe", "a.png", "--compact"]).expect("parse");
        let Command::Probe { file, compact } = cli.command else {
            panic!("expected probe");
        };
        assert_eq!(file, PathBuf::from("a.png"));
        assert!(compact);
    }

    #[test]
    fn parses_animate() {
        let cli = Cli::try_parse_from([
            "catconvert",
            "animate",
            "a.jpg",
            "b.png",
            "--to",
            "webp",
            "--frame",
            "0.25",
            "--crop",
            "--direction",
            "boomerang",
        ])
        .expect("parse");
        let Command::Animate {
            pictures,
            to,
            frame,
            crop,
            direction,
            ..
        } = cli.command
        else {
            panic!("expected animate");
        };
        assert_eq!(pictures, [PathBuf::from("a.jpg"), PathBuf::from("b.png")]);
        assert_eq!(OutputFormat::from(to), OutputFormat::Webp);
        assert_eq!(frame, 0.25);
        assert!(crop);
        assert_eq!(Direction::from(direction), Direction::Boomerang);
        // One picture is not an animation.
        assert!(Cli::try_parse_from(["catconvert", "animate", "a.jpg", "--to", "gif"]).is_err());
    }

    #[test]
    fn parses_convert() {
        let cli = Cli::try_parse_from([
            "catconvert",
            "convert",
            "in.mov",
            "--to",
            "gif",
            "--start",
            "1.5",
            "--size",
            "480",
            "--speed",
            "2",
            "--direction",
            "boomerang",
        ])
        .expect("parse");
        let Command::Convert {
            to,
            start,
            size,
            quality,
            metadata,
            speed,
            direction,
            ..
        } = cli.command
        else {
            panic!("expected convert");
        };
        assert_eq!(OutputFormat::from(to), OutputFormat::Gif);
        assert_eq!(start, Some(1.5));
        assert_eq!(size, Some(480));
        assert_eq!(quality, 60);
        assert_eq!(Metadata::from(metadata), Metadata::NoLocation);
        assert!((speed - 2.0).abs() < f64::EPSILON);
        assert_eq!(Direction::from(direction), Direction::Boomerang);
        let cli = Cli::try_parse_from([
            "catconvert",
            "convert",
            "a",
            "--to",
            "mp4",
            "--crop",
            "9:16",
        ])
        .expect("parse");
        let Command::Convert { crop, codec, .. } = cli.command else {
            panic!("expected convert");
        };
        assert_eq!(crop, Some([9, 16]));
        assert_eq!(codec, None);
        let cli = Cli::try_parse_from([
            "catconvert",
            "convert",
            "a",
            "--to",
            "mkv",
            "--codec",
            "av1",
        ])
        .expect("parse");
        let Command::Convert { codec, .. } = cli.command else {
            panic!("expected convert");
        };
        assert_eq!(codec.map(VideoCodec::from), Some(VideoCodec::Av1));
        for bad in ["9x16", "0:1", "1:"] {
            assert!(
                Cli::try_parse_from(["catconvert", "convert", "a", "--to", "mp4", "--crop", bad])
                    .is_err(),
                "{bad}"
            );
        }
        assert!(
            Cli::try_parse_from([
                "catconvert",
                "convert",
                "a",
                "--to",
                "gif",
                "--quality",
                "101"
            ])
            .is_err()
        );
    }

    #[test]
    fn parses_bitrates() {
        assert_eq!(parse_bitrate("800"), Ok(800));
        assert_eq!(parse_bitrate("800k"), Ok(800));
        assert_eq!(parse_bitrate("4M"), Ok(4000));
        assert_eq!(parse_bitrate("4.5m"), Ok(4500));
        assert_eq!(parse_bitrate("2500kbps"), Ok(2500));
        for bad in ["", "fast", "50k", "-4M", "300M"] {
            assert!(parse_bitrate(bad).is_err(), "{bad}");
        }
        let cli = Cli::try_parse_from([
            "catconvert",
            "convert",
            "a",
            "--to",
            "mp4",
            "--bitrate",
            "4M",
        ])
        .expect("parse");
        let Command::Convert { bitrate, .. } = cli.command else {
            panic!("expected convert");
        };
        assert_eq!(bitrate, Some(4000));
    }

    #[test]
    fn describes_quality_in_numbers() {
        use catconvert_core::VideoRate;

        let crf = VideoRate {
            codec: VideoCodec::H264,
            crf: Some(24),
            kbps: None,
            passes: 1,
        };
        let summary = QualitySummary {
            audio: Some(AudioRate::Bitrate { kbps: 128 }),
            video: Some(crf),
        };
        assert_eq!(
            describe_quality(summary).as_deref(),
            Some("video H.264 CRF 24, audio 128 kbps")
        );
        let exact = QualitySummary {
            audio: None,
            video: Some(VideoRate {
                crf: None,
                kbps: Some(4000),
                passes: 2,
                ..crf
            }),
        };
        assert_eq!(
            describe_quality(exact).as_deref(),
            Some("video H.264 4000 kbps, 2 passes")
        );
        let mp3 = QualitySummary {
            audio: Some(AudioRate::Vbr {
                level: 2,
                kbps: 190,
            }),
            video: None,
        };
        assert_eq!(
            describe_quality(mp3).as_deref(),
            Some("audio VBR V2 (about 190 kbps)")
        );
        assert_eq!(describe_quality(QualitySummary::default()), None);
    }

    #[test]
    fn parses_metadata_choices_and_scrub() {
        let metadata = |args: &[&str]| {
            let cli = Cli::try_parse_from(
                ["catconvert", "convert", "a.jpg", "--to", "png"]
                    .iter()
                    .chain(args),
            )
            .expect("parse");
            let Command::Convert {
                metadata,
                strip_metadata,
                ..
            } = cli.command
            else {
                panic!("expected convert");
            };
            (Metadata::from(metadata), strip_metadata)
        };
        assert_eq!(metadata(&["--metadata", "keep"]).0, Metadata::Keep);
        assert_eq!(metadata(&["--metadata", "none"]).0, Metadata::None);
        assert!(metadata(&["--strip-metadata"]).1);

        // The command line plans show for in-process cleaning steps.
        let cli = Cli::try_parse_from([
            "catconvert",
            "scrub",
            "a.jpg",
            "b.jpg",
            "--drop",
            "location",
        ])
        .expect("parse");
        let Command::Scrub { drop, .. } = cli.command else {
            panic!("expected scrub");
        };
        assert_eq!(drop, Strip::Location);
    }
}
