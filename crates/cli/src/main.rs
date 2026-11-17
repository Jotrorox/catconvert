//! `catconvert`: command-line front end over `catconvert-core`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use catconvert_core::{
    ConvertOptions, Crop, Direction, Ffmpeg, JxlOptions, OutputFormat, RunOutcome, SoundOptions,
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
        /// Drop dates, location, titles and other tags.
        #[arg(long)]
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
    if let Some(reason) = ffmpeg.missing(options.format) {
        return Err(reason);
    }
    let info = ffmpeg.probe(file).map_err(|e| e.to_string())?;
    let output = catconvert_core::output_path(&info.path, options.format, |_| false);
    let plan = catconvert_core::plan(&info, options, &output, ffmpeg.jxl_support())
        .map_err(|e| e.to_string())?;
    if dry_run {
        println!("{}", plan.command_line());
        return Ok(());
    }
    let cancel = AtomicBool::new(false);
    let mut shown = 0.0_f32;
    let outcome = ffmpeg
        .run(&plan, &cancel, |p| {
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
            mute,
            normalize,
            fade_in,
            fade_out,
            lossless,
            effort,
            progressive,
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
                keep_metadata: !strip_metadata,
                speed,
                direction: direction.into(),
            };
            match convert(&file, &options, dry_run) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("error: {err}");
                    ExitCode::FAILURE
                }
            }
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
        let Command::Convert { crop, .. } = cli.command else {
            panic!("expected convert");
        };
        assert_eq!(crop, Some([9, 16]));
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
}
