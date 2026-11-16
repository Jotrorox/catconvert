//! `catconvert`: command-line front end over `catconvert-core`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use catconvert_core::{ConvertOptions, Ffmpeg, JxlOptions, OutputFormat, RunOutcome};
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
        /// 0 = smallest file, 100 = best quality.
        #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u8).range(0..=100))]
        quality: u8,
        /// Drop the audio track.
        #[arg(long)]
        mute: bool,
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
        /// Print the command(s) instead of running them.
        #[arg(long)]
        dry_run: bool,
    },
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
            quality,
            mute,
            lossless,
            effort,
            progressive,
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
                quality,
                keep_audio: !mute,
                jxl: JxlOptions {
                    lossless,
                    effort,
                    progressive,
                },
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
        ])
        .expect("parse");
        let Command::Convert {
            to,
            start,
            size,
            quality,
            ..
        } = cli.command
        else {
            panic!("expected convert");
        };
        assert_eq!(OutputFormat::from(to), OutputFormat::Gif);
        assert_eq!(start, Some(1.5));
        assert_eq!(size, Some(480));
        assert_eq!(quality, 60);
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
