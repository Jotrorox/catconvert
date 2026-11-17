//! Commands edited by hand.
//!
//! The editor shows the command(s) a conversion runs ([`Plan::command_line`])
//! and lets the user change them. [`custom_plan`] reads the edited text back
//! into a [`Plan`] for the queue, without a shell:
//!
//! - The text is split into words with shell-style quoting, and steps are
//!   joined with `&&`. Anything else a shell would act on (`;`, pipes,
//!   redirects, `$variables`, backticks) is refused rather than passed along.
//! - Only the tools a plan can use run: `ffmpeg`, `cjxl`, `djxl` and the
//!   built-in `catconvert scrub`, by name, so always the copies CatConvert
//!   found or bundles.
//! - Files are only written where the generated plan writes: its output and
//!   its own intermediate files. The last step must write the output, nothing
//!   may write over the original, and ffmpeg's `-y` is refused (the runner
//!   adds `-n`, so ffmpeg never overwrites anything either way).
//! - The runner still adds `-progress` and friends to every ffmpeg call, so
//!   progress keeps working; a command can't set them itself.
//!
//! Commands saved in recipes use [`command_template`], which swaps one file's
//! paths for placeholders, and [`fill_command`], which puts another file's
//! paths back in.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::convert::{Plan, Step, Support, Tool, file_stem, render_command};
use crate::{Error, MediaInfo, Result};

/// Stands for the file being converted in a [`command_template`].
pub const INPUT_PLACEHOLDER: &str = "{input}";
/// Stands for the output in a [`command_template`].
pub const OUTPUT_PLACEHOLDER: &str = "{output}";
/// Starts a placeholder for an intermediate file next to the output, as in
/// `{scratch:frames.png}`.
const SCRATCH_PLACEHOLDER: &str = "{scratch:";

/// Plan running the command `command`, as edited by the user, instead of the
/// generated plan `generated` for `input`.
///
/// `command` is read like [`Plan::command_line`] writes it. The result writes
/// the same output and uses the same intermediate files as `generated`.
/// Placeholders from a [`command_template`] are filled in first. A command
/// written for another free name of the output (`clip.mp4` when the next free
/// one is now `clip 2.mp4`) is pointed at the current one. Unchanged steps
/// keep their progress share and length; edited ffmpeg steps get a length
/// worked out from their `-t` and `-ss`, so the progress bar still moves.
///
/// # Errors
///
/// [`Error::Unsupported`] with a message for the user if the text can't be
/// read without a shell, runs a program other than CatConvert's tools, needs
/// a libjxl tool `support` says is missing, sets an option the runner owns,
/// or writes anywhere but the output and the plan's intermediate files. That
/// includes writing over the original and not writing the output last.
pub fn custom_plan(
    input: &MediaInfo,
    generated: &Plan,
    command: &str,
    support: impl Into<Support>,
) -> Result<Plan> {
    let support = support.into();
    let mut calls = parse(command)?;
    fill(&mut calls, &input.path, &generated.output)?;
    retarget(&mut calls, &input.path, &generated.output);

    let writable: Vec<&Path> = std::iter::once(generated.output.as_path())
        .chain(generated.scratch.iter().map(PathBuf::as_path))
        .collect();
    let last = calls.len() - 1;
    for (index, call) in calls.iter().enumerate() {
        let missing = match call.tool {
            Tool::Cjxl => !support.jxl.cjxl,
            Tool::Djxl => !support.jxl.djxl,
            Tool::Ffmpeg | Tool::Scrub => false,
        };
        if missing {
            return Err(invalid(format!(
                "{} from libjxl isn't installed, so this command can't run",
                call.tool.name()
            )));
        }
        let written_files = outputs(call)?;
        for written in &written_files {
            let written = written.as_str();
            let path = Path::new(written);
            if path == input.path {
                return Err(invalid(format!(
                    "This command would write over the original, {}. CatConvert never does that.",
                    input.file_name
                )));
            }
            if !writable.contains(&path) {
                return Err(invalid(format!(
                    "{written} isn't where this conversion saves. A command can only write {}{}.",
                    generated.output.display(),
                    if generated.scratch.is_empty() {
                        ""
                    } else {
                        " and the plan's own intermediate files"
                    }
                )));
            }
        }
        if index == last && written_files.last().map(Path::new) != Some(&generated.output) {
            return Err(invalid(format!(
                "The last step has to write {}",
                generated.output.display()
            )));
        }
    }

    let steps = calls
        .into_iter()
        .enumerate()
        .map(|(index, call)| {
            let args: Vec<OsString> = call.args.iter().map(OsString::from).collect();
            let matched = generated.steps.get(index).filter(|s| s.tool == call.tool);
            let (duration_secs, weight) = match matched {
                Some(step) if step.args == args => (step.duration_secs, step.weight),
                Some(step) => (edited_duration(input, step, &call.args), step.weight),
                None if call.tool == Tool::Ffmpeg => (span(input, &call.args), 1.0),
                None => (None, default_weight(call.tool)),
            };
            Step {
                tool: call.tool,
                args,
                duration_secs,
                weight,
            }
        })
        .collect();
    Ok(Plan {
        steps,
        output: generated.output.clone(),
        scratch: generated.scratch.clone(),
        writes: generated.writes.clone(),
    })
}

/// `command` with the paths of `input`, its output and the output's
/// intermediate files swapped for placeholders, so it can be saved in a
/// recipe and used for other files with [`fill_command`]. The output is
/// whatever the last step writes.
///
/// # Errors
///
/// [`Error::Unsupported`] if `command` can't be read (see [`custom_plan`]).
pub fn command_template(input: &Path, command: &str) -> Result<String> {
    let mut calls = parse(command)?;
    let written = final_output(&calls);
    for arg in calls.iter_mut().flat_map(|call| call.args.iter_mut()) {
        let path = Path::new(arg.as_str());
        let placeholder = if path == input {
            Some(INPUT_PLACEHOLDER.to_owned())
        } else if written.as_deref() == Some(path) {
            Some(OUTPUT_PLACEHOLDER.to_owned())
        } else {
            written
                .as_deref()
                .and_then(|written| scratch_rest(path, written))
                .map(|rest| format!("{SCRATCH_PLACEHOLDER}{rest}}}"))
        };
        if let Some(placeholder) = placeholder {
            *arg = placeholder;
        }
    }
    Ok(render(&calls))
}

/// A [`command_template`] with the placeholders filled in for `input` and
/// `output`, as text for the editor.
///
/// # Errors
///
/// [`Error::Unsupported`] if `template` can't be read (see [`custom_plan`])
/// or has a malformed placeholder.
pub fn fill_command(template: &str, input: &Path, output: &Path) -> Result<String> {
    let mut calls = parse(template)?;
    fill(&mut calls, input, output)?;
    Ok(render(&calls))
}

// ---------------------------------------------------------------------------
// Reading the text
// ---------------------------------------------------------------------------

/// One program call as written.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Call {
    tool: Tool,
    args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Word(String),
    /// `&&` between two steps.
    And,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::unsupported(message)
}

fn needs_shell(what: &str) -> Error {
    invalid(format!(
        "{what} only works in a shell, and CatConvert runs the tools directly. Quote it if it's \
         part of an argument, and join steps with &&."
    ))
}

fn no_expansion() -> Error {
    invalid(
        "$ and ` only work in a shell, and CatConvert runs the tools directly. Put the argument \
         in single quotes to pass it as it is.",
    )
}

/// Split `text` into words and `&&`s the way a POSIX shell would, refusing
/// whatever would need the shell to do more than that.
fn tokenize(text: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    // A word has started, even if it is empty so far (`''`).
    let mut started = false;
    let mut chars = text.chars().peekable();
    let flush = |tokens: &mut Vec<Token>, word: &mut String, started: &mut bool| {
        if *started {
            tokens.push(Token::Word(std::mem::take(word)));
            *started = false;
        }
    };
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' | '\r' => flush(&mut tokens, &mut word, &mut started),
            '\\' => match chars.next() {
                // Line continuation.
                Some('\n') => {}
                Some('\r') if chars.peek() == Some(&'\n') => {
                    chars.next();
                }
                Some(escaped) => {
                    word.push(escaped);
                    started = true;
                }
                None => return Err(invalid("The command ends with a lone backslash")),
            },
            '\'' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => word.push(ch),
                        None => return Err(invalid("A ' quote is never closed")),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(ch @ ('"' | '\\' | '$' | '`')) => word.push(ch),
                            Some('\n') => {}
                            Some(ch) => {
                                word.push('\\');
                                word.push(ch);
                            }
                            None => return Err(invalid("A \" quote is never closed")),
                        },
                        Some('$' | '`') => return Err(no_expansion()),
                        Some(ch) => word.push(ch),
                        None => return Err(invalid("A \" quote is never closed")),
                    }
                }
            }
            // A comment runs to the end of the line.
            '#' if !started => {
                while chars.peek().is_some_and(|&ch| ch != '\n') {
                    chars.next();
                }
            }
            '&' if chars.peek() == Some(&'&') => {
                chars.next();
                flush(&mut tokens, &mut word, &mut started);
                tokens.push(Token::And);
            }
            '&' => return Err(needs_shell("&")),
            '|' if chars.peek() == Some(&'|') => return Err(needs_shell("||")),
            '|' => return Err(needs_shell("A pipe (|)")),
            ';' => return Err(needs_shell(";")),
            '<' | '>' => return Err(needs_shell(&format!("A redirect ({c})"))),
            '(' | ')' => return Err(needs_shell(&format!("A bracket ({c})"))),
            '$' | '`' => return Err(no_expansion()),
            '\0' => return Err(invalid("The command contains a NUL character")),
            _ => {
                word.push(c);
                started = true;
            }
        }
    }
    flush(&mut tokens, &mut word, &mut started);
    Ok(tokens)
}

/// The steps of `text`, each starting with a tool CatConvert runs.
fn parse(text: &str) -> Result<Vec<Call>> {
    let tokens = tokenize(text)?;
    if tokens.is_empty() {
        return Err(invalid("The command is empty"));
    }
    let mut calls = Vec::new();
    let mut words = Vec::new();
    for token in tokens.into_iter().chain(std::iter::once(Token::And)) {
        match token {
            Token::Word(word) => words.push(word),
            Token::And if words.is_empty() => {
                return Err(invalid("There's no command on one side of &&"));
            }
            Token::And => calls.push(call(std::mem::take(&mut words))?),
        }
    }
    Ok(calls)
}

const TOOLS: &str = "ffmpeg, cjxl, djxl and catconvert scrub";

fn call(words: Vec<String>) -> Result<Call> {
    let mut words = words.into_iter();
    let program = words.next().unwrap_or_default();
    let tool = match program.as_str() {
        "ffmpeg" => Tool::Ffmpeg,
        "cjxl" => Tool::Cjxl,
        "djxl" => Tool::Djxl,
        "catconvert" => match words.next().as_deref() {
            Some("scrub") => Tool::Scrub,
            Some(other) => {
                return Err(invalid(format!(
                    "catconvert {other} can't run here. Commands can use {TOOLS}."
                )));
            }
            None => return Err(invalid(format!("Commands can use {TOOLS}."))),
        },
        _ => {
            // A folder in front, in either spelling, wherever the command came from.
            let name = program.rsplit(['/', '\\']).next().unwrap_or_default();
            let known = ["ffmpeg", "cjxl", "djxl"].iter().find(|tool| {
                name.eq_ignore_ascii_case(tool) || name.eq_ignore_ascii_case(&format!("{tool}.exe"))
            });
            return Err(invalid(match known {
                Some(tool) => format!(
                    "Write just {tool}, without a folder: CatConvert always runs its own copy"
                ),
                None => {
                    format!("{program} isn't a tool CatConvert runs. Commands can use {TOOLS}.")
                }
            }));
        }
    };
    Ok(Call {
        tool,
        args: words.collect(),
    })
}

fn render(calls: &[Call]) -> String {
    render_command(calls.iter().map(|call| (call.tool, call.args.iter())))
}

// ---------------------------------------------------------------------------
// What a call reads and writes
// ---------------------------------------------------------------------------

/// ffmpeg options that take no value. Any other option is taken to be
/// followed by one; a mistake there makes a value look like an extra output,
/// which is refused, so it errs on the safe side.
const FFMPEG_FLAGS: &[&str] = &[
    "-n",
    "-an",
    "-vn",
    "-sn",
    "-dn",
    "-shortest",
    "-nostdin",
    "-stdin",
    "-hide_banner",
    "-nostats",
    "-stats",
    "-copyts",
    "-start_at_zero",
    "-accurate_seek",
    "-re",
    "-benchmark",
    "-benchmark_all",
    "-debug_ts",
    "-xerror",
    "-ignore_unknown",
    "-copy_unknown",
    "-dump",
    "-hex",
    "-bitexact",
    "-autorotate",
    "-autoscale",
    "-fix_sub_duration",
    "-find_stream_info",
    "-print_graphs",
    "-psnr",
    "-qphist",
    "-intra",
];

/// ffmpeg options a command may not use, and why.
fn refused_ffmpeg_option(name: &str) -> Option<&'static str> {
    Some(match name {
        "-y" => "-y lets ffmpeg overwrite files, and CatConvert never does that",
        "-progress" => "CatConvert sets -progress itself to show how far along the conversion is",
        "-report" | "-vstats" | "-vstats_file" | "-sdp_file" | "-dump_attachment" => {
            return Some(
                "This option makes ffmpeg write extra files; a command may only write the output",
            );
        }
        _ => return None,
    })
}

fn ffmpeg_takes_value(name: &str) -> bool {
    let negated = name
        .strip_prefix("-no")
        .is_some_and(|rest| FFMPEG_FLAGS.contains(&format!("-{rest}").as_str()));
    !(FFMPEG_FLAGS.contains(&name) || negated)
}

/// The log files ffmpeg writes for a two-pass encode with `-passlogfile
/// prefix`: libvpx's `prefix-0.log`, plus x264's `.mbtree`, each as a
/// `.temp` file until it is done. Two-pass plans list them all as scratch.
pub(crate) const PASSLOG_SUFFIXES: [&str; 4] = [
    "-0.log",
    "-0.log.temp",
    "-0.log.mbtree",
    "-0.log.mbtree.temp",
];

/// What x265 writes for a two-pass encode with `-x265-stats file`: the
/// statistics and its `.cutree` file, each as a `.temp` file until it is
/// done. Two-pass H.265 plans list them all as scratch.
pub(crate) const X265_STATS_SUFFIXES: [&str; 4] = ["", ".temp", ".cutree", ".cutree.temp"];

/// The files a call writes, as written. Inputs aren't limited: reading a
/// watermark or a subtitle file is fine.
fn outputs(call: &Call) -> Result<Vec<String>> {
    let mut outputs = Vec::new();
    match call.tool {
        Tool::Ffmpeg => {
            let mut args = call.args.iter();
            let mut format = None;
            let mut pass = false;
            let mut passlog = None;
            let mut x265 = false;
            let mut x265_stats = None;
            while let Some(arg) = args.next() {
                if arg == "-i" {
                    args.next()
                        .ok_or_else(|| invalid("ffmpeg's -i needs a file after it"))?;
                    // Options before an input apply to that input.
                    format = None;
                } else if arg.len() > 1 && arg.starts_with('-') {
                    // Stream specifiers as in -c:v or -pass:v.
                    let name = arg.split(':').next().unwrap_or(arg);
                    if let Some(reason) = refused_ffmpeg_option(name) {
                        return Err(invalid(format!("{arg} isn't allowed: {reason}")));
                    }
                    let value = if ffmpeg_takes_value(name) {
                        args.next()
                    } else {
                        None
                    };
                    match name {
                        "-f" => format = value.map(String::as_str),
                        "-pass" => pass = true,
                        "-passlogfile" => passlog = value,
                        "-x265-stats" => x265_stats = value,
                        "-c" | "-codec" | "-vcodec" if value.is_some_and(|v| v == "libx265") => {
                            x265 = true;
                        }
                        _ => {}
                    }
                } else if arg == "-" && format == Some("null") {
                    // The analysis pass of a two-pass encode writes nothing.
                } else {
                    outputs.push(arg.clone());
                }
            }
            match passlog {
                // The pass logs come first: the last file is what a step makes.
                Some(prefix) => {
                    let logs = PASSLOG_SUFFIXES.map(|suffix| format!("{prefix}{suffix}"));
                    outputs.splice(0..0, logs);
                }
                None if pass => {
                    return Err(invalid(
                        "-pass needs a -passlogfile next to the output, or ffmpeg writes its \
                         log where CatConvert can't clean it up",
                    ));
                }
                None => {}
            }
            match x265_stats {
                Some(stats) => {
                    let files = X265_STATS_SUFFIXES.map(|suffix| format!("{stats}{suffix}"));
                    outputs.splice(0..0, files);
                }
                None if pass && x265 => {
                    return Err(invalid(
                        "two-pass libx265 needs -x265-stats next to the output, or x265 writes \
                         its statistics where CatConvert can't clean them up",
                    ));
                }
                None => {}
            }
        }
        Tool::Cjxl | Tool::Djxl => {
            let name = call.tool.name();
            match call.args.as_slice() {
                [input, output, ..] if !input.starts_with('-') && !output.starts_with('-') => {
                    outputs.push(output.clone());
                }
                _ => {
                    return Err(invalid(format!(
                        "{name} takes the input and output file first, then its options"
                    )));
                }
            }
        }
        Tool::Scrub => match call.args.as_slice() {
            [_, output, flag, what]
                if flag == "--drop" && matches!(what.as_str(), "location" | "all") =>
            {
                outputs.push(output.clone());
            }
            _ => {
                return Err(invalid(
                    "catconvert scrub takes an input, an output and --drop location or --drop all",
                ));
            }
        },
    }
    Ok(outputs)
}

/// What the last step writes, if the steps can be read.
fn final_output(calls: &[Call]) -> Option<PathBuf> {
    let last = calls.last()?;
    outputs(last).ok()?.last().map(PathBuf::from)
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// `tag.ext` if `path` is the intermediate file `.{name}.tag.ext` that
/// [`crate::convert::scratch_path`] puts next to `output`.
fn scratch_rest(path: &Path, output: &Path) -> Option<String> {
    if path.parent() != output.parent() {
        return None;
    }
    let prefix = format!(".{}.", output.file_name()?.to_string_lossy());
    let name = path.file_name()?.to_str()?;
    name.strip_prefix(&prefix)
        .filter(|rest| !rest.is_empty())
        .map(str::to_owned)
}

fn scratch_for(output: &Path, rest: &str) -> PathBuf {
    let name = output
        .file_name()
        .map_or_else(|| "output".into(), |n| n.to_string_lossy().into_owned());
    output.with_file_name(format!(".{name}.{rest}"))
}

/// Replace [`command_template`] placeholders with real paths.
fn fill(calls: &mut [Call], input: &Path, output: &Path) -> Result<()> {
    for arg in calls.iter_mut().flat_map(|call| call.args.iter_mut()) {
        if arg == INPUT_PLACEHOLDER {
            *arg = input.to_string_lossy().into_owned();
        } else if arg == OUTPUT_PLACEHOLDER {
            *arg = output.to_string_lossy().into_owned();
        } else if let Some(rest) = arg.strip_prefix(SCRATCH_PLACEHOLDER) {
            let rest = rest
                .strip_suffix('}')
                .filter(|r| !r.is_empty() && !r.contains(['/', '\\']))
                .ok_or_else(|| invalid(format!("{arg} isn't a placeholder CatConvert knows")))?;
            *arg = scratch_for(output, rest).to_string_lossy().into_owned();
        }
    }
    Ok(())
}

/// Whether `path` is one of the names [`crate::output_path`] picks for
/// `input` with the extension of `output`: `clip.mp4`, `clip 2.mp4`, …
fn is_output_name(path: &Path, input: &Path, output: &Path) -> bool {
    if path.parent() != output.parent() || path.extension() != output.extension() {
        return false;
    }
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    let base = file_stem(input);
    stem == base
        || stem
            .strip_prefix(&base)
            .and_then(|rest| rest.strip_prefix(' '))
            .and_then(|n| n.parse::<u32>().ok())
            .is_some_and(|n| n >= 2)
}

/// Point a command written for another free name of the output at `output`,
/// along with its intermediate files. Commands writing anything else are
/// left alone for [`custom_plan`] to refuse.
fn retarget(calls: &mut [Call], input: &Path, output: &Path) {
    let Some(written) = final_output(calls) else {
        return;
    };
    if written == output || written == input || !is_output_name(&written, input, output) {
        return;
    }
    for arg in calls.iter_mut().flat_map(|call| call.args.iter_mut()) {
        let path = Path::new(arg.as_str());
        let moved = if path == written {
            Some(output.to_path_buf())
        } else {
            scratch_rest(path, &written).map(|rest| scratch_for(output, &rest))
        };
        if let Some(moved) = moved {
            *arg = moved.to_string_lossy().into_owned();
        }
    }
}

// ---------------------------------------------------------------------------
// Progress
// ---------------------------------------------------------------------------

fn default_weight(tool: Tool) -> f32 {
    match tool {
        Tool::Ffmpeg => 1.0,
        Tool::Cjxl => 0.6,
        Tool::Djxl => 0.2,
        Tool::Scrub => 0.05,
    }
}

/// The value after the last `option` in `args`.
fn value_of<'a>(args: &'a [String], option: &str) -> Option<&'a str> {
    args.windows(2)
        .rev()
        .find(|pair| pair[0] == option)
        .map(|pair| pair[1].as_str())
}

/// An ffmpeg duration: `[-][HH:]MM:SS[.m…]` or `S[.m…][s|ms|us]`.
fn parse_time(text: &str) -> Option<f64> {
    let secs = if text.contains(':') {
        let parts: Vec<&str> = text.split(':').collect();
        if parts.len() > 3 {
            return None;
        }
        parts.iter().try_fold(0.0, |acc, part| {
            Some(acc * 60.0 + part.parse::<f64>().ok()?)
        })?
    } else if let Some(ms) = text.strip_suffix("ms") {
        ms.parse::<f64>().ok()? / 1e3
    } else if let Some(us) = text.strip_suffix("us") {
        us.parse::<f64>().ok()? / 1e6
    } else {
        text.strip_suffix('s').unwrap_or(text).parse().ok()?
    };
    (secs.is_finite() && secs >= 0.0).then_some(secs)
}

/// How much of the input an ffmpeg call reads: its `-t`, or what's left
/// after its `-ss`.
fn span(input: &MediaInfo, args: &[String]) -> Option<f64> {
    if let Some(t) = value_of(args, "-t").and_then(parse_time) {
        return Some(t);
    }
    let start = value_of(args, "-ss").and_then(parse_time).unwrap_or(0.0);
    input.duration_secs.map(|d| (d - start).max(0.0))
}

/// Length of what an edited ffmpeg step writes: the generated step's,
/// scaled by how much more or less of the input it now reads. Stills stay
/// without one.
fn edited_duration(input: &MediaInfo, generated: &Step, args: &[String]) -> Option<f64> {
    let before = generated.duration_secs?;
    let generated_args: Vec<String> = generated
        .args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    match (span(input, &generated_args), span(input, args)) {
        (Some(old), Some(new)) if old > 0.0 => Some(before * new / old),
        _ => Some(before),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConvertOptions, JxlSupport, MediaKind, Metadata, OutputFormat, plan};

    fn video(path: &str) -> MediaInfo {
        let path = PathBuf::from(path);
        MediaInfo {
            file_name: path.file_name().unwrap().to_string_lossy().into_owned(),
            extension: path.extension().map(|e| e.to_string_lossy().to_lowercase()),
            path,
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

    fn photo(path: &str) -> MediaInfo {
        MediaInfo {
            kind: MediaKind::Image,
            duration_secs: None,
            fps: None,
            video_codec: Some("png".into()),
            audio_codec: None,
            sample_rate: None,
            has_location: true,
            location_tags: vec!["GPSLatitude".into()],
            ..video(path)
        }
    }

    fn mp4(trim: bool) -> ConvertOptions {
        ConvertOptions {
            format: OutputFormat::Mp4,
            trim_start: trim.then_some(2.0),
            trim_end: trim.then_some(12.0),
            ..ConvertOptions::default()
        }
    }

    fn all_tools() -> JxlSupport {
        JxlSupport {
            cjxl: true,
            djxl: true,
            ..JxlSupport::default()
        }
    }

    fn generated(input: &MediaInfo, options: &ConvertOptions, out: &str) -> Plan {
        plan(input, options, Path::new(out), all_tools()).expect("plan")
    }

    fn words(text: &str) -> Vec<String> {
        tokenize(text)
            .expect("tokenize")
            .into_iter()
            .map(|t| match t {
                Token::Word(w) => w,
                Token::And => "&&".into(),
            })
            .collect()
    }

    fn error(result: Result<impl std::fmt::Debug>) -> String {
        result.expect_err("should be refused").to_string()
    }

    #[test]
    fn splits_words_like_a_shell() {
        assert_eq!(
            words(r#"ffmpeg -i 'my clip.mov' -vf "scale=-2:720,fps=\"30\"" a\ b.mp4"#),
            [
                "ffmpeg",
                "-i",
                "my clip.mov",
                "-vf",
                r#"scale=-2:720,fps="30""#,
                "a b.mp4"
            ]
        );
        // Single quotes keep everything, the way shell_quote escapes them.
        assert_eq!(words(r"'it'\''s' ''"), ["it's", ""]);
        // Line continuations, comments, && with and without spaces.
        assert_eq!(
            words("ffmpeg -i a \\\n  b # the output\n&&cjxl a&&djxl"),
            ["ffmpeg", "-i", "a", "b", "&&", "cjxl", "a", "&&", "djxl"]
        );
        // A backslash in double quotes only escapes what a shell would.
        assert_eq!(words(r#""C:\clips\a.mov""#), [r"C:\clips\a.mov"]);
        assert_eq!(words("scale=iw*2:-2 ~/a.mp4"), ["scale=iw*2:-2", "~/a.mp4"]);
    }

    #[test]
    fn refuses_what_needs_a_shell() {
        for text in [
            "ffmpeg -i a.mov b.mp4; rm -rf ~",
            "ffmpeg -i a.mov b.mp4 | tee x",
            "ffmpeg -i a.mov b.mp4 || true",
            "ffmpeg -i a.mov b.mp4 & sleep 1",
            "ffmpeg -i a.mov > b.mp4",
            "ffmpeg -i $(whoami).mov b.mp4",
            "ffmpeg -i `whoami`.mov b.mp4",
            "ffmpeg -i \"$HOME/a.mov\" b.mp4",
            "ffmpeg -vf select=gt(scene,0.4) b.mp4",
        ] {
            let message = error(parse(text));
            assert!(message.contains("shell"), "{text}: {message}");
        }
        assert!(error(parse("ffmpeg -i 'a.mov")).contains("never closed"));
        assert!(error(parse("ffmpeg -i \"a.mov")).contains("never closed"));
        assert!(error(parse("ffmpeg \\")).contains("backslash"));
        assert!(error(parse("  # nothing\n")).contains("empty"));
        assert!(error(parse("&& ffmpeg -i a b")).contains("&&"));
        assert!(error(parse("ffmpeg -i a b &&")).contains("&&"));
        // Quoted, the same characters are plain arguments.
        let calls = parse("ffmpeg -vf 'select=gt(scene,0.4);x|y>z' -metadata 'title=$5 & up' o")
            .expect("parse");
        assert_eq!(calls[0].args[1], "select=gt(scene,0.4);x|y>z");
        assert_eq!(calls[0].args[3], "title=$5 & up");
    }

    #[test]
    fn only_runs_catconvert_tools() {
        for (text, expected) in [
            ("rm -rf /", "rm isn't a tool CatConvert runs"),
            ("bash -c 'ffmpeg -i a b'", "bash isn't a tool"),
            ("ffprobe a.mov", "ffprobe isn't a tool"),
            ("ffmpeg -i a.mov b.mp4 && open b.mp4", "open isn't a tool"),
            (
                "/usr/bin/ffmpeg -i a b",
                "Write just ffmpeg, without a folder",
            ),
            (r"'C:\tools\FFMPEG.EXE' -i a b", "Write just ffmpeg"),
            (
                "catconvert convert a.mov",
                "catconvert convert can't run here",
            ),
        ] {
            let message = error(parse(text));
            assert!(message.contains(expected), "{text}: {message}");
        }
        let calls =
            parse("ffmpeg -i a b && cjxl b c && djxl c d && catconvert scrub d e --drop all")
                .expect("parse");
        let tools: Vec<Tool> = calls.iter().map(|c| c.tool).collect();
        assert_eq!(tools, [Tool::Ffmpeg, Tool::Cjxl, Tool::Djxl, Tool::Scrub]);
        assert_eq!(calls[3].args, ["d", "e", "--drop", "all"]);
    }

    #[test]
    fn unedited_commands_give_back_the_generated_plan() {
        let clip = video("/clips/garden.mov");
        let jpeg = MediaInfo {
            extension: Some("jpg".into()),
            video_codec: Some("mjpeg".into()),
            ..photo("/photos/it's me.jpg")
        };
        let jxl_in = MediaInfo {
            extension: Some("jxl".into()),
            video_codec: Some("jpegxl".into()),
            has_location: false,
            ..photo("/photos/a.jxl")
        };
        let cases = [
            (clip.clone(), mp4(true), "/clips/garden.mp4"),
            (clip.clone(), mp4(false), "/clips/garden 2.mp4"),
            (
                clip,
                ConvertOptions {
                    format: OutputFormat::Jxl,
                    trim_end: Some(2.0),
                    ..ConvertOptions::default()
                },
                "/clips/garden.jxl",
            ),
            (
                jpeg.clone(),
                ConvertOptions {
                    format: OutputFormat::Jxl,
                    metadata: Metadata::None,
                    ..ConvertOptions::default()
                },
                "/photos/it's me.jxl",
            ),
            (
                jpeg,
                ConvertOptions {
                    format: OutputFormat::Png,
                    metadata: Metadata::NoLocation,
                    ..ConvertOptions::default()
                },
                "/photos/it's me.png",
            ),
            (
                jxl_in,
                ConvertOptions {
                    format: OutputFormat::Png,
                    ..ConvertOptions::default()
                },
                "/photos/a.png",
            ),
        ];
        for (input, options, out) in cases {
            let planned = generated(&input, &options, out);
            let custom = custom_plan(&input, &planned, &planned.command_line(), all_tools())
                .unwrap_or_else(|e| panic!("{out}: {e}\n{}", planned.command_line()));
            assert_eq!(custom, planned, "{out}");
        }
    }

    #[test]
    fn an_edited_crf_runs_with_the_same_output_and_progress() {
        let clip = video("/clips/garden.mov");
        let planned = generated(&clip, &mp4(true), "/clips/garden.mp4");
        let edited = planned.command_line().replace("-crf 24", "-crf 18");
        let custom = custom_plan(&clip, &planned, &edited, all_tools()).expect("custom");
        assert_eq!(custom.output, planned.output);
        let args = &custom.steps[0].args;
        let crf = args.iter().position(|a| a == "-crf").expect("-crf");
        assert_eq!(args[crf + 1], "18");
        assert_eq!(custom.duration_secs(), Some(10.0));
        assert_eq!(custom.command_line(), edited);
    }

    #[test]
    fn edited_trims_keep_the_progress_bar_honest() {
        let clip = video("/clips/garden.mov");
        let planned = generated(&clip, &mp4(true), "/clips/garden.mp4");
        let text = planned.command_line();
        let shorter = custom_plan(
            &clip,
            &planned,
            &text.replace("-t 10", "-t 00:00:05"),
            all_tools(),
        )
        .expect("shorter");
        assert_eq!(shorter.duration_secs(), Some(5.0));
        // Without -t it reads from -ss 2 to the end of the 42 s clip.
        let untrimmed =
            custom_plan(&clip, &planned, &text.replace(" -t 10", ""), all_tools()).expect("rest");
        assert_eq!(untrimmed.duration_secs(), Some(40.0));
        // A brand-new ffmpeg step gets a length too.
        let fresh = Plan {
            steps: Vec::new(),
            ..planned.clone()
        };
        let custom = custom_plan(&clip, &fresh, &text, all_tools()).expect("fresh");
        assert_eq!(custom.duration_secs(), Some(10.0));
        assert!((custom.steps[0].weight - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parses_ffmpeg_times() {
        assert_eq!(parse_time("27.8"), Some(27.8));
        assert_eq!(parse_time("1:02.5"), Some(62.5));
        assert_eq!(parse_time("01:00:00"), Some(3600.0));
        assert_eq!(parse_time("1500ms"), Some(1.5));
        assert_eq!(parse_time("250000us"), Some(0.25));
        assert_eq!(parse_time("3s"), Some(3.0));
        assert_eq!(parse_time("-3"), None);
        assert_eq!(parse_time("inf"), None);
        assert_eq!(parse_time("1:2:3:4"), None);
    }

    #[test]
    fn never_writes_over_the_original_or_elsewhere() {
        let clip = video("/clips/garden.mov");
        let planned = generated(&clip, &mp4(false), "/clips/garden.mp4");
        let text = planned.command_line();
        let over = error(custom_plan(
            &clip,
            &planned,
            &text.replace("/clips/garden.mp4", "/clips/garden.mov"),
            all_tools(),
        ));
        assert!(
            over.contains("write over the original, garden.mov"),
            "{over}"
        );
        let elsewhere = error(custom_plan(
            &clip,
            &planned,
            &text.replace("/clips/garden.mp4", "/tmp/x.mp4"),
            all_tools(),
        ));
        assert!(
            elsewhere.contains("/tmp/x.mp4 isn't where this conversion saves"),
            "{elsewhere}"
        );
        let extra = error(custom_plan(
            &clip,
            &planned,
            &text.replace("-sn", "/clips/copy.mp4 -sn"),
            all_tools(),
        ));
        assert!(extra.contains("/clips/copy.mp4 isn't where"), "{extra}");
        let none = error(custom_plan(
            &clip,
            &planned,
            "ffmpeg -i /clips/garden.mov -f null -",
            all_tools(),
        ));
        // Writing nothing is fine for a first pass, but not for the last step.
        assert!(none.contains("The last step has to write"), "{none}");
        let last = error(custom_plan(&clip, &planned, "ffmpeg -version", all_tools()));
        assert!(
            last.contains("The last step has to write /clips/garden.mp4"),
            "{last}"
        );
    }

    #[test]
    fn refuses_options_the_runner_owns() {
        let clip = video("/clips/garden.mov");
        let planned = generated(&clip, &mp4(false), "/clips/garden.mp4");
        let text = planned.command_line();
        for (option, expected) in [
            ("-y", "overwrite"),
            ("-progress pipe:2", "-progress"),
            ("-pass 1", "-passlogfile"),
            ("-pass:v 1", "-passlogfile"),
            ("-report", "extra files"),
            ("-vstats_file /tmp/v", "extra files"),
        ] {
            let message = error(custom_plan(
                &clip,
                &planned,
                &text.replacen("ffmpeg ", &format!("ffmpeg {option} "), 1),
                all_tools(),
            ));
            assert!(message.contains(expected), "{option}: {message}");
        }
        // Flags without a value don't hide the output that follows them.
        let flags = text.replace(
            " /clips/garden.mp4",
            " -shortest -noautorotate /clips/garden.mp4",
        );
        custom_plan(&clip, &planned, &flags, all_tools()).expect("flags");
        assert!(error(custom_plan(&clip, &planned, "ffmpeg -i", all_tools())).contains("-i needs"));
    }

    #[test]
    fn libjxl_steps_only_write_the_plans_files() {
        let jpeg = MediaInfo {
            extension: Some("jpg".into()),
            video_codec: Some("mjpeg".into()),
            ..photo("/photos/a.jpg")
        };
        let options = ConvertOptions {
            format: OutputFormat::Jxl,
            metadata: Metadata::None,
            ..ConvertOptions::default()
        };
        let planned = generated(&jpeg, &options, "/photos/a.jxl");
        let text = planned.command_line();
        assert!(
            text.starts_with("catconvert scrub /photos/a.jpg /photos/.a.jxl.clean.jpg"),
            "{text}"
        );
        // Every step may be edited: here cjxl's effort.
        let edited = custom_plan(&jpeg, &planned, &text.replace("-e 7", "-e 9"), all_tools())
            .expect("edited");
        assert!(edited.steps[1].args.iter().any(|a| a == "9"));
        let over = error(custom_plan(
            &jpeg,
            &planned,
            &text.replace("/photos/a.jxl --", "/photos/a.jpg --"),
            all_tools(),
        ));
        assert!(over.contains("write over the original"), "{over}");
        let order = error(custom_plan(
            &jpeg,
            &planned,
            "cjxl -d 1 /photos/a.jpg /photos/a.jxl",
            all_tools(),
        ));
        assert!(order.contains("input and output file first"), "{order}");
        let missing = error(custom_plan(&jpeg, &planned, &text, JxlSupport::default()));
        assert!(
            missing.contains("cjxl from libjxl isn't installed"),
            "{missing}"
        );
        let scrub = error(custom_plan(
            &jpeg,
            &planned,
            &text.replace("--drop all", "--drop everything"),
            all_tools(),
        ));
        assert!(scrub.contains("--drop location or --drop all"), "{scrub}");
    }

    #[test]
    fn follows_the_output_to_its_current_free_name() {
        let input = video("/clips/cat.mov");
        let options = ConvertOptions {
            format: OutputFormat::Jxl,
            trim_end: Some(2.0),
            ..ConvertOptions::default()
        };
        // Written while cat.jxl was free; another job has taken it since.
        let shown = generated(&input, &options, "/clips/cat.jxl");
        assert!(shown.command_line().contains("/clips/.cat.jxl.frames.apng"));
        let now = generated(&input, &options, "/clips/cat 2.jxl");
        let custom = custom_plan(&input, &now, &shown.command_line(), all_tools()).expect("custom");
        assert_eq!(custom, now);
        // Not for names CatConvert wouldn't pick.
        let renamed = shown
            .command_line()
            .replace("/clips/cat.jxl", "/clips/dog.jxl");
        let message = error(custom_plan(&input, &now, &renamed, all_tools()));
        assert!(
            message.contains("isn't where this conversion saves"),
            "{message}"
        );
    }

    #[test]
    fn templates_swap_paths_for_placeholders_and_back() {
        let first = photo("/photos/cat.png");
        let options = ConvertOptions {
            format: OutputFormat::Jxl,
            ..ConvertOptions::default()
        };
        let planned = generated(&first, &options, "/photos/cat.jxl");
        let edited = planned.command_line().replace("-e 7", "-e 9");
        let template = command_template(&first.path, &edited).expect("template");
        assert!(!template.contains("/photos"), "{template}");
        assert!(
            template.contains("'{input}'") && template.contains("'{output}'"),
            "{template}"
        );

        let second = photo("/other/dog.png");
        let theirs = generated(&second, &options, "/other/dog.jxl");
        let filled = fill_command(&template, &second.path, &theirs.output).expect("fill");
        assert_eq!(filled, theirs.command_line().replace("-e 7", "-e 9"));
        // custom_plan fills placeholders too, so a template can be queued as is.
        let custom = custom_plan(&second, &theirs, &template, all_tools()).expect("custom");
        assert_eq!(custom.command_line(), filled);
        assert!(
            error(fill_command(
                "ffmpeg -i '{scratch:../x}' o",
                &second.path,
                &theirs.output
            ))
            .contains("placeholder")
        );
    }

    #[test]
    fn two_pass_h265_writes_only_its_logs_and_x265_stats() {
        let clip = video("/clips/garden.mov");
        let exact = ConvertOptions {
            video_bitrate: Some(4000),
            video_codec: Some(crate::VideoCodec::H265),
            ..mp4(false)
        };
        let planned = generated(&clip, &exact, "/clips/garden.mp4");
        let text = planned.command_line();
        let stats = "-x265-stats /clips/.garden.mp4.passlog.stats-0.x265.log";
        assert!(text.contains(stats), "{text}");
        let edited = text.replace("-b:v 4000k", "-b:v 2500k");
        let custom = custom_plan(&clip, &planned, &edited, all_tools()).expect("edited");
        assert_eq!(custom.scratch, planned.scratch);

        // x265's statistics have to stay with the plan's other files.
        let elsewhere = text.replace(stats, "-x265-stats /tmp/x265.log");
        assert!(
            error(custom_plan(&clip, &planned, &elsewhere, all_tools())).contains("/tmp/x265.log")
        );
        let missing = text.replace(&format!(" {stats}"), "");
        assert!(error(custom_plan(&clip, &planned, &missing, all_tools())).contains("-x265-stats"));
    }

    #[test]
    fn two_pass_commands_write_only_their_logs() {
        let clip = video("/clips/garden.mov");
        let exact = ConvertOptions {
            video_bitrate: Some(4000),
            ..mp4(false)
        };
        let planned = generated(&clip, &exact, "/clips/garden.mp4");
        let text = planned.command_line();
        assert!(
            text.contains("-pass 1") && text.contains("-f null -"),
            "{text}"
        );

        // Unedited, or with another bitrate in both passes.
        let same = custom_plan(&clip, &planned, &text, all_tools()).expect("unedited");
        assert_eq!(same, planned);
        let edited = text.replace("-b:v 4000k", "-b:v 2500k");
        let custom = custom_plan(&clip, &planned, &edited, all_tools()).expect("edited");
        assert_eq!(custom.steps.len(), 2);
        assert_eq!(custom.scratch, planned.scratch);

        // The logs must be the plan's own, and "-" only means nothing with -f null.
        let elsewhere = text.replace(
            "-passlogfile /clips/.garden.mp4.passlog.stats",
            "-passlogfile /tmp/stats",
        );
        assert!(
            error(custom_plan(&clip, &planned, &elsewhere, all_tools()))
                .contains("/tmp/stats-0.log")
        );
        let no_log = text.replacen(" -passlogfile /clips/.garden.mp4.passlog.stats", "", 1);
        assert!(error(custom_plan(&clip, &planned, &no_log, all_tools())).contains("-passlogfile"));
        let stdout = text.replacen("-f null -", "-f mp4 -", 1);
        assert!(error(custom_plan(&clip, &planned, &stdout, all_tools())).contains("- isn't"));

        // Recipes keep the logs next to whichever file they convert.
        let template = command_template(&clip.path, &edited).expect("template");
        assert!(template.contains("'{scratch:passlog.stats}'"), "{template}");
        let other = video("/other/dog.mov");
        let theirs = generated(&other, &exact, "/other/dog.mp4");
        let filled = fill_command(&template, &other.path, &theirs.output).expect("fill");
        assert_eq!(
            filled,
            theirs.command_line().replace("-b:v 4000k", "-b:v 2500k")
        );
    }
}
