//! `catconvert`: command-line front end over `catconvert-core`.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

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
    /// Print basic information about a media file as JSON.
    Probe {
        /// File to inspect.
        file: PathBuf,
        /// Print compact single-line JSON instead of pretty-printed output.
        #[arg(long)]
        compact: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Probe { file, compact } => match catconvert_core::probe(&file) {
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
        let Command::Probe { file, compact } = cli.command;
        assert_eq!(file, PathBuf::from("a.png"));
        assert!(compact);
    }
}
