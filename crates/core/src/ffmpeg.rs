//! Locating and running the ffmpeg/ffprobe binaries.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use crate::convert::{OutputFormat, Plan};
use crate::media::{self, MediaInfo, MediaKind};
use crate::{Error, Result};

/// Environment variable pointing at an ffmpeg binary, checked before `PATH`.
pub const FFMPEG_ENV: &str = "CATCONVERT_FFMPEG";
/// Environment variable pointing at an ffprobe binary. Defaults to the
/// `ffprobe` next to the chosen ffmpeg.
pub const FFPROBE_ENV: &str = "CATCONVERT_FFPROBE";

/// A usable ffmpeg + ffprobe pair.
///
/// Cheap to clone; the encoder list is shared.
#[derive(Debug, Clone)]
pub struct Ffmpeg {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    version: String,
    encoders: Arc<HashSet<String>>,
}

/// One progress sample while a conversion runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunProgress {
    /// Completion between `0.0` and `1.0`, if the output length is known.
    pub fraction: Option<f32>,
    /// Encoding speed relative to real time, e.g. `2.3` for 2.3×.
    pub speed: Option<f32>,
}

/// How a conversion that did not error ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// The output was written; its size in bytes.
    Done { output_bytes: u64 },
    /// The cancel flag was raised; the partial output was removed.
    Cancelled,
}

impl Ffmpeg {
    /// Find ffmpeg and ffprobe.
    ///
    /// Looks at [`FFMPEG_ENV`]/[`FFPROBE_ENV`] first, then every directory on
    /// `PATH`, then common install locations (desktop apps on macOS do not
    /// inherit the shell's `PATH`, so Homebrew's directories are listed
    /// explicitly).
    ///
    /// # Errors
    ///
    /// [`Error::FfmpegNotFound`] if no directory holds both binaries, or the
    /// error from [`Ffmpeg::at`] if the binaries found do not run.
    pub fn locate() -> Result<Self> {
        if let Some(ffmpeg) = std::env::var_os(FFMPEG_ENV).map(PathBuf::from) {
            let ffprobe = std::env::var_os(FFPROBE_ENV)
                .map(PathBuf::from)
                .or_else(|| sibling(&ffmpeg, "ffprobe"))
                .ok_or(Error::FfmpegNotFound)?;
            return Self::at(ffmpeg, ffprobe);
        }

        let path_dirs = std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
            .unwrap_or_default();
        path_dirs
            .into_iter()
            .chain(extra_search_dirs())
            .find_map(|dir| {
                let ffmpeg = executable(&dir, "ffmpeg")?;
                let ffprobe = executable(&dir, "ffprobe")?;
                Some((ffmpeg, ffprobe))
            })
            .map_or(Err(Error::FfmpegNotFound), |(ffmpeg, ffprobe)| {
                Self::at(ffmpeg, ffprobe)
            })
    }

    /// Use explicit binaries. Runs `ffmpeg -version` and `-encoders` once.
    ///
    /// # Errors
    ///
    /// [`Error::Spawn`] if ffmpeg cannot be started, [`Error::Ffmpeg`] if it
    /// exits with an error.
    pub fn at(ffmpeg: impl Into<PathBuf>, ffprobe: impl Into<PathBuf>) -> Result<Self> {
        let ffmpeg = ffmpeg.into();
        let ffprobe = ffprobe.into();

        let version_out = capture(base_command(&ffmpeg).arg("-version"), &ffmpeg)?;
        let version = parse_version(&String::from_utf8_lossy(&version_out));

        let encoders_out = capture(
            base_command(&ffmpeg).args(["-hide_banner", "-encoders"]),
            &ffmpeg,
        )?;
        let encoders = parse_encoders(&String::from_utf8_lossy(&encoders_out));

        Ok(Self {
            ffmpeg,
            ffprobe,
            version,
            encoders: Arc::new(encoders),
        })
    }

    /// Path of the ffmpeg binary in use.
    #[must_use]
    pub fn ffmpeg_path(&self) -> &Path {
        &self.ffmpeg
    }

    /// Path of the ffprobe binary in use.
    #[must_use]
    pub fn ffprobe_path(&self) -> &Path {
        &self.ffprobe
    }

    /// Version string, e.g. `"7.1"`, or `"unknown"`.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Whether this build of ffmpeg has the encoder `name`, e.g. `"libx264"`.
    #[must_use]
    pub fn has_encoder(&self, name: &str) -> bool {
        self.encoders.contains(name)
    }

    /// Whether this build of ffmpeg can write `format`.
    #[must_use]
    pub fn supports(&self, format: OutputFormat) -> bool {
        self.has_encoder(format.encoder())
    }

    /// Inspect a file with ffprobe: duration, display size, frame rate and codecs.
    ///
    /// # Errors
    ///
    /// The errors of [`media::probe`], plus [`Error::Ffmpeg`] if ffprobe cannot
    /// read the file (for example because it is not media at all).
    pub fn probe(&self, path: impl AsRef<Path>) -> Result<MediaInfo> {
        let path = path.as_ref();
        let mut info = media::probe(path)?;
        let out = capture(
            base_command(&self.ffprobe)
                .args(["-v", "error", "-print_format", "json"])
                .args(["-show_format", "-show_streams"])
                .arg(path),
            &self.ffprobe,
        )?;
        let json: Value = serde_json::from_slice(&out).map_err(|err| Error::Ffmpeg {
            message: format!("ffprobe returned unreadable output: {err}"),
        })?;
        apply_probe(&mut info, &json);
        Ok(info)
    }

    /// Grab one frame as a JPEG, scaled down to at most `max_height` pixels.
    ///
    /// Works for videos (any time within the clip) and still images (`at_secs`
    /// is ignored when it is zero).
    ///
    /// # Errors
    ///
    /// [`Error::Ffmpeg`] if decoding fails or there is no frame at `at_secs`.
    pub fn frame(&self, input: &Path, at_secs: f64, max_height: u32) -> Result<Vec<u8>> {
        let mut cmd = base_command(&self.ffmpeg);
        cmd.args(["-hide_banner", "-nostdin", "-v", "error"]);
        if at_secs > 0.0 {
            cmd.arg("-ss").arg(format_secs(at_secs));
        }
        cmd.arg("-i")
            .arg(input)
            .args(["-frames:v", "1", "-an", "-sn"])
            .arg("-vf")
            .arg(format!("scale=-2:'min(ih,{})'", even(max_height)))
            .args(["-c:v", "mjpeg", "-q:v", "4", "-f", "image2pipe", "pipe:1"]);
        let bytes = capture(&mut cmd, &self.ffmpeg)?;
        if bytes.is_empty() {
            return Err(Error::Ffmpeg {
                message: "no frame at that time".into(),
            });
        }
        Ok(bytes)
    }

    /// Render the first audio track as a PNG waveform of `width`×`height`
    /// pixels, drawn in `color` (`#rrggbb`) on a transparent background.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] for a malformed colour, [`Error::Ffmpeg`] if the
    /// file has no audio or cannot be decoded.
    pub fn waveform(&self, input: &Path, width: u32, height: u32, color: &str) -> Result<Vec<u8>> {
        let valid_color = color.len() == 7
            && color.starts_with('#')
            && color[1..].chars().all(|c| c.is_ascii_hexdigit());
        if !valid_color {
            return Err(Error::unsupported(format!("not a #rrggbb colour: {color}")));
        }
        let (width, height) = (width.clamp(16, 4096), height.clamp(16, 1024));
        let filter = format!(
            "[0:a:0]aformat=channel_layouts=mono,showwavespic=s={width}x{height}:colors={color}:scale=sqrt[w]"
        );
        let mut cmd = base_command(&self.ffmpeg);
        cmd.args(["-hide_banner", "-nostdin", "-v", "error", "-i"])
            .arg(input)
            .arg("-filter_complex")
            .arg(filter)
            .args(["-map", "[w]", "-frames:v", "1"])
            .args(["-c:v", "png", "-f", "image2pipe", "pipe:1"]);
        capture(&mut cmd, &self.ffmpeg)
    }

    /// Run a planned conversion to completion, reporting progress.
    ///
    /// Raising `cancel` stops ffmpeg at its next progress report (a few times a
    /// second). Partial output is removed on cancellation and on failure.
    ///
    /// # Errors
    ///
    /// [`Error::Spawn`] if ffmpeg cannot be started, [`Error::Ffmpeg`] with the
    /// most relevant line of its error output if the conversion fails.
    pub fn run(
        &self,
        plan: &Plan,
        cancel: &AtomicBool,
        mut on_progress: impl FnMut(RunProgress),
    ) -> Result<RunOutcome> {
        let mut cmd = base_command(&self.ffmpeg);
        cmd.args(["-hide_banner", "-nostdin", "-v", "error", "-nostats"])
            .args(["-progress", "pipe:1", "-stats_period", "0.25"])
            // Never overwrite: the output path was chosen to be free.
            .arg("-n")
            .args(&plan.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|source| Error::Spawn {
            program: self.ffmpeg.clone(),
            source,
        })?;

        // Drain stderr on its own thread so a chatty ffmpeg never blocks on a full pipe.
        let stderr = child.stderr.take().map(|mut pipe| {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = pipe.read_to_end(&mut buf);
                String::from_utf8_lossy(&buf).into_owned()
            })
        });

        let mut cancelled = false;
        if let Some(stdout) = child.stdout.take() {
            let mut sample = ProgressParser::new(plan.duration_secs);
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if cancel.load(Ordering::Relaxed) {
                    cancelled = true;
                    let _ = child.kill();
                    break;
                }
                if let Some(progress) = sample.feed(&line) {
                    on_progress(progress);
                }
            }
        }
        if !cancelled && cancel.load(Ordering::Relaxed) {
            cancelled = true;
            let _ = child.kill();
        }

        let status = child.wait().map_err(|source| Error::Spawn {
            program: self.ffmpeg.clone(),
            source,
        })?;
        let stderr = stderr
            .and_then(|handle| handle.join().ok())
            .unwrap_or_default();

        if cancelled {
            let _ = std::fs::remove_file(&plan.output);
            return Ok(RunOutcome::Cancelled);
        }
        if !status.success() {
            let _ = std::fs::remove_file(&plan.output);
            return Err(Error::Ffmpeg {
                message: friendly_error(&stderr)
                    .unwrap_or_else(|| format!("ffmpeg stopped unexpectedly ({status})")),
            });
        }
        let output_bytes = std::fs::metadata(&plan.output)
            .map(|meta| meta.len())
            .map_err(|source| Error::Io {
                path: plan.output.clone(),
                source,
            })?;
        Ok(RunOutcome::Done { output_bytes })
    }
}

// ---------------------------------------------------------------------------
// Process helpers
// ---------------------------------------------------------------------------

fn base_command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: don't flash a console window from the GUI app.
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

/// Run to completion and return stdout, or an [`Error::Ffmpeg`] built from stderr.
fn capture(cmd: &mut Command, program: &Path) -> Result<Vec<u8>> {
    let output = cmd.output().map_err(|source| Error::Spawn {
        program: program.to_path_buf(),
        source,
    })?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(Error::Ffmpeg {
            message: friendly_error(&stderr).unwrap_or_else(|| {
                format!(
                    "{} failed ({})",
                    program.file_name().map_or_else(
                        || program.display().to_string(),
                        |n| n.to_string_lossy().into_owned()
                    ),
                    output.status
                )
            }),
        })
    }
}

fn executable(dir: &Path, name: &str) -> Option<PathBuf> {
    let path = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    path.is_file().then_some(path)
}

fn sibling(binary: &Path, name: &str) -> Option<PathBuf> {
    executable(binary.parent()?, name)
}

fn extra_search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.extend(
            ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"]
                .iter()
                .map(PathBuf::from),
        );
    } else if cfg!(windows) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(Path::new(&local).join("Microsoft/WinGet/Links"));
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            dirs.push(Path::new(&home).join("scoop/shims"));
        }
        dirs.push(PathBuf::from(r"C:\ProgramData\chocolatey\bin"));
        dirs.push(PathBuf::from(r"C:\ffmpeg\bin"));
    } else {
        dirs.extend(
            ["/usr/local/bin", "/usr/bin", "/snap/bin"]
                .iter()
                .map(PathBuf::from),
        );
    }
    dirs
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

fn parse_version(output: &str) -> String {
    output
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("ffmpeg version "))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or("unknown")
        .to_owned()
}

/// Encoder names from `ffmpeg -encoders`, whose rows look like
/// ` V....D libx264              libx264 H.264 / AVC ...`.
fn parse_encoders(output: &str) -> HashSet<String> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let flags = parts.next()?;
            let name = parts.next()?;
            let is_row = flags.len() == 6
                && flags.starts_with(['V', 'A', 'S'])
                && flags
                    .chars()
                    .skip(1)
                    .all(|c| c == '.' || c.is_ascii_uppercase())
                && name != "=";
            is_row.then(|| name.to_owned())
        })
        .collect()
}

/// Pick the line of ffmpeg's error output most likely to make sense to a person.
fn friendly_error(stderr: &str) -> Option<String> {
    let line = stderr.lines().rev().map(str::trim).find(|line| {
        !line.is_empty()
            && !line.starts_with("Conversion failed")
            && !line.starts_with("Error opening output files")
            && !line.starts_with("Error opening input files")
    })?;
    // Drop the `[component @ 0x1234] ` prefix ffmpeg puts on most messages.
    let line = match line
        .strip_prefix('[')
        .and_then(|rest| rest.split_once("] "))
    {
        Some((_, message)) => message,
        None => line,
    };
    let mut message: String = line.chars().take(300).collect();
    if let Some(first) = message.get(0..1) {
        let upper = first.to_uppercase();
        message.replace_range(0..1, &upper);
    }
    Some(message)
}

/// Fill the stream fields of `info` from ffprobe's JSON.
fn apply_probe(info: &mut MediaInfo, json: &Value) {
    let streams = json["streams"].as_array().map_or(&[][..], Vec::as_slice);
    let format = &json["format"];

    let video = streams.iter().find(|s| {
        s["codec_type"] == "video" && s["disposition"]["attached_pic"].as_i64() != Some(1)
    });
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");

    let duration = as_f64(&format["duration"])
        .or_else(|| video.and_then(|v| as_f64(&v["duration"])))
        .or_else(|| audio.and_then(|a| as_f64(&a["duration"])))
        .filter(|d| d.is_finite() && *d > 0.0);
    let format_name = format["format_name"].as_str().unwrap_or_default();

    let ext_kind = info.kind;
    info.kind = match (ext_kind, video, audio) {
        // Animated GIFs behave like (silent) video clips.
        (MediaKind::Image, Some(_), _) if format_name.contains("gif") && duration.is_some() => {
            MediaKind::Video
        }
        (MediaKind::Image, _, _) => MediaKind::Image,
        (_, Some(v), _) if is_still_image(v, format_name) => MediaKind::Image,
        (_, Some(_), _) => MediaKind::Video,
        (_, None, Some(_)) => MediaKind::Audio,
        (kind, None, None) => kind,
    };

    if let Some(v) = video {
        let width = v["width"].as_u64().and_then(|w| u32::try_from(w).ok());
        let height = v["height"].as_u64().and_then(|h| u32::try_from(h).ok());
        let (width, height) = if is_quarter_turn(v) {
            (height, width)
        } else {
            (width, height)
        };
        info.width = width;
        info.height = height;
        info.video_codec = v["codec_name"].as_str().map(str::to_owned);
        if info.kind == MediaKind::Video {
            info.fps = v["avg_frame_rate"]
                .as_str()
                .and_then(parse_ratio)
                .or_else(|| v["r_frame_rate"].as_str().and_then(parse_ratio));
        }
    }
    info.audio_codec = audio.and_then(|a| a["codec_name"].as_str().map(str::to_owned));
    info.duration_secs = if info.kind == MediaKind::Image {
        None
    } else {
        duration
    };
}

fn is_still_image(stream: &Value, format_name: &str) -> bool {
    format_name.ends_with("_pipe") || format_name == "image2" || stream["nb_frames"] == "1"
}

/// Rotation of ±90° (phone videos) swaps the displayed width and height.
fn is_quarter_turn(stream: &Value) -> bool {
    let from_side_data = stream["side_data_list"]
        .as_array()
        .and_then(|list| list.iter().find_map(|d| d["rotation"].as_f64()));
    let from_tag = stream["tags"]["rotate"]
        .as_str()
        .and_then(|r| r.parse::<f64>().ok());
    from_side_data
        .or(from_tag)
        .is_some_and(|rotation| (rotation.abs().round() as i64) % 180 == 90)
}

fn as_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

fn parse_ratio(ratio: &str) -> Option<f64> {
    let (num, den) = ratio.split_once('/')?;
    let (num, den): (f64, f64) = (num.parse().ok()?, den.parse().ok()?);
    (num > 0.0 && den > 0.0).then(|| num / den)
}

/// Turns the `key=value` lines of `-progress pipe:1` into samples, one per
/// `progress=` line.
struct ProgressParser {
    total_us: Option<f64>,
    out_time_us: Option<f64>,
    speed: Option<f32>,
}

impl ProgressParser {
    fn new(duration_secs: Option<f64>) -> Self {
        Self {
            total_us: duration_secs.filter(|d| *d > 0.0).map(|d| d * 1e6),
            out_time_us: None,
            speed: None,
        }
    }

    fn feed(&mut self, line: &str) -> Option<RunProgress> {
        let (key, value) = line.trim().split_once('=')?;
        match key {
            "out_time_us" | "out_time_ms" => {
                // Despite its name, `out_time_ms` is also in microseconds.
                self.out_time_us = value.parse().ok().filter(|t: &f64| *t >= 0.0);
                None
            }
            "speed" => {
                self.speed = value.trim_end_matches('x').trim().parse().ok();
                None
            }
            "progress" => {
                let fraction = if value == "end" {
                    Some(1.0)
                } else {
                    match (self.out_time_us, self.total_us) {
                        #[allow(clippy::cast_possible_truncation)]
                        (Some(t), Some(total)) => Some((t / total).clamp(0.0, 1.0) as f32),
                        _ => None,
                    }
                };
                Some(RunProgress {
                    fraction,
                    speed: self.speed,
                })
            }
            _ => None,
        }
    }
}

fn even(n: u32) -> u32 {
    (n.max(2)) & !1
}

/// Seconds formatted for ffmpeg's `-ss`/`-t`, trimmed to millisecond precision.
pub(crate) fn format_secs(secs: f64) -> String {
    let s = format!("{:.3}", secs.max(0.0));
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() { "0".into() } else { s.into() }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn base(name: &str) -> MediaInfo {
        let ext = Path::new(name)
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase());
        MediaInfo {
            path: name.into(),
            file_name: name.into(),
            kind: ext
                .as_deref()
                .map_or(MediaKind::Unknown, MediaKind::from_extension),
            extension: ext,
            size_bytes: 1,
            duration_secs: None,
            width: None,
            height: None,
            fps: None,
            video_codec: None,
            audio_codec: None,
        }
    }

    #[test]
    fn parses_version_line() {
        assert_eq!(
            parse_version("ffmpeg version 7.1.1 Copyright (c) 2000-2025\nbuilt with"),
            "7.1.1"
        );
        assert_eq!(parse_version("garbage"), "unknown");
    }

    #[test]
    fn parses_encoder_rows_only() {
        let out = "Encoders:\n V..... = Video\n A..... = Audio\n ------\n V....D libx264              libx264 H.264\n A....D aac                  AAC\n VF...D png                  PNG\n";
        let encoders = parse_encoders(out);
        assert!(encoders.contains("libx264"));
        assert!(encoders.contains("aac"));
        assert!(encoders.contains("png"));
        assert!(!encoders.contains("="));
        assert!(!encoders.contains("Video"));
    }

    #[test]
    fn friendly_error_strips_noise() {
        let stderr = "[mov,mp4 @ 0x7f8] moov atom not found\nin.mov: invalid data found when processing input\nConversion failed!\n";
        assert_eq!(
            friendly_error(stderr).as_deref(),
            Some("In.mov: invalid data found when processing input")
        );
        assert_eq!(
            friendly_error("[libx264 @ 0xabc] width not divisible by 2\n").as_deref(),
            Some("Width not divisible by 2")
        );
        assert_eq!(friendly_error("\n\n"), None);
    }

    #[test]
    fn probe_json_for_rotated_phone_video() {
        let mut info = base("clip.mov");
        apply_probe(
            &mut info,
            &json!({
                "streams": [
                    {"codec_type": "video", "codec_name": "hevc", "width": 1920, "height": 1080,
                     "avg_frame_rate": "30000/1001", "side_data_list": [{"rotation": -90}],
                     "disposition": {"attached_pic": 0}},
                    {"codec_type": "audio", "codec_name": "aac"}
                ],
                "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "42.5"}
            }),
        );
        assert_eq!(info.kind, MediaKind::Video);
        assert_eq!((info.width, info.height), (Some(1080), Some(1920)));
        assert_eq!(info.duration_secs, Some(42.5));
        assert!((info.fps.unwrap() - 29.97).abs() < 0.01);
        assert_eq!(info.video_codec.as_deref(), Some("hevc"));
        assert!(info.has_audio());
    }

    #[test]
    fn probe_json_for_song_with_cover_art() {
        let mut info = base("song.mp3");
        apply_probe(
            &mut info,
            &json!({
                "streams": [
                    {"codec_type": "audio", "codec_name": "mp3", "duration": "180.2"},
                    {"codec_type": "video", "codec_name": "mjpeg", "width": 500, "height": 500,
                     "disposition": {"attached_pic": 1}}
                ],
                "format": {"format_name": "mp3", "duration": "180.2"}
            }),
        );
        assert_eq!(info.kind, MediaKind::Audio);
        assert_eq!(info.width, None);
        assert_eq!(info.duration_secs, Some(180.2));
    }

    #[test]
    fn probe_json_for_still_and_animated_images() {
        let mut png = base("cat.png");
        apply_probe(
            &mut png,
            &json!({
                "streams": [{"codec_type": "video", "codec_name": "png", "width": 801, "height": 600}],
                "format": {"format_name": "png_pipe"}
            }),
        );
        assert_eq!(png.kind, MediaKind::Image);
        assert_eq!(png.duration_secs, None);
        assert_eq!(png.width, Some(801));

        let mut gif = base("loop.gif");
        apply_probe(
            &mut gif,
            &json!({
                "streams": [{"codec_type": "video", "codec_name": "gif", "width": 320, "height": 240,
                             "avg_frame_rate": "10/1"}],
                "format": {"format_name": "gif", "duration": "2.0"}
            }),
        );
        assert_eq!(gif.kind, MediaKind::Video);
        assert_eq!(gif.duration_secs, Some(2.0));
    }

    #[test]
    fn progress_parser_reports_fraction_and_speed() {
        let mut parser = ProgressParser::new(Some(10.0));
        assert_eq!(parser.feed("frame=12"), None);
        assert_eq!(parser.feed("out_time_us=2500000"), None);
        assert_eq!(parser.feed("speed=2.31x"), None);
        let sample = parser.feed("progress=continue").expect("sample");
        assert_eq!(sample.fraction, Some(0.25));
        assert_eq!(sample.speed, Some(2.31));
        assert_eq!(parser.feed("speed=N/A"), None);
        let end = parser.feed("progress=end").expect("sample");
        assert_eq!(end.fraction, Some(1.0));
        assert_eq!(end.speed, None);

        let mut unknown = ProgressParser::new(None);
        unknown.feed("out_time_us=100");
        assert_eq!(unknown.feed("progress=continue").unwrap().fraction, None);
    }

    #[test]
    fn formats_seconds_compactly() {
        assert_eq!(format_secs(3.2), "3.2");
        assert_eq!(format_secs(31.0), "31");
        assert_eq!(format_secs(0.0), "0");
        assert_eq!(format_secs(1.23456), "1.235");
        assert_eq!(format_secs(-4.0), "0");
    }
}
