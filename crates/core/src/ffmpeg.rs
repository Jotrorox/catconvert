//! Locating and running the ffmpeg/ffprobe binaries (and libjxl's tools,
//! which [`Ffmpeg`] finds and runs alongside them).

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::convert::{JxlSupport, OutputFormat, Plan, Step, Support, TONE_MAP_FILTER, Tool};
use crate::jxl::{self, JxlTools};
use crate::media::{self, MediaInfo, MediaKind};
use crate::scrub;
use crate::{Error, Result};

/// Environment variable pointing at an ffmpeg binary, checked before `PATH`.
pub const FFMPEG_ENV: &str = "CATCONVERT_FFMPEG";
/// Environment variable pointing at an ffprobe binary. Defaults to the
/// `ffprobe` next to the chosen ffmpeg.
pub const FFPROBE_ENV: &str = "CATCONVERT_FFPROBE";
/// File-name prefix of the tools an app ships next to its executable:
/// `catconvert-ffmpeg`, `catconvert-ffprobe`, `catconvert-cjxl`, … The prefix
/// keeps a Linux package that installs them into `/usr/bin` from clashing with
/// the distribution's own ffmpeg.
pub const BUNDLED_PREFIX: &str = "catconvert-";

/// A usable ffmpeg + ffprobe pair, plus whichever libjxl tools are installed.
///
/// Cheap to clone; the codec lists and preview cache are shared.
#[derive(Debug, Clone)]
pub struct Ffmpeg {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    bundled: bool,
    version: String,
    encoders: Arc<HashSet<String>>,
    decoders: Arc<HashSet<String>>,
    /// The `scale` filter can tone-map HDR to SDR.
    tone_map: bool,
    jxl: Arc<JxlTools>,
    /// JPEG XL files djxl decoded for previews, when ffmpeg can't read them.
    decoded: Arc<Mutex<DecodeCache>>,
    /// Sound decoded to WAV for playback, when the webview can't play a file.
    listenable: Arc<Mutex<DecodeCache>>,
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
        Self::locate_with(None)
    }

    /// Like [`Ffmpeg::locate`], but an app that ships its own tools passes the
    /// directory holding them (named with [`BUNDLED_PREFIX`]). They come after
    /// the environment variables and before `PATH`, and the bundled libjxl
    /// tools are preferred over installed ones the same way.
    ///
    /// # Errors
    ///
    /// As for [`Ffmpeg::locate`].
    pub fn locate_with(bundled: Option<&Path>) -> Result<Self> {
        if let Some(ffmpeg) = std::env::var_os(FFMPEG_ENV).map(PathBuf::from) {
            let ffprobe = std::env::var_os(FFPROBE_ENV)
                .map(PathBuf::from)
                .or_else(|| sibling(&ffmpeg, "ffprobe"))
                .ok_or(Error::FfmpegNotFound)?;
            return Self::open(ffmpeg, ffprobe, bundled, false);
        }

        if let Some((ffmpeg, ffprobe)) = bundled.and_then(bundled_pair) {
            return Self::open(ffmpeg, ffprobe, bundled, true);
        }

        search_dirs()
            .into_iter()
            .find_map(|dir| {
                let ffmpeg = executable(&dir, "ffmpeg")?;
                let ffprobe = executable(&dir, "ffprobe")?;
                Some((ffmpeg, ffprobe))
            })
            .map_or(Err(Error::FfmpegNotFound), |(ffmpeg, ffprobe)| {
                Self::open(ffmpeg, ffprobe, bundled, false)
            })
    }

    /// Use explicit binaries. Runs `ffmpeg -version`, `-encoders` and
    /// `-decoders` once, and looks for libjxl's tools next to ffmpeg, on
    /// `PATH` and in the usual install folders.
    ///
    /// # Errors
    ///
    /// [`Error::Spawn`] if ffmpeg cannot be started, [`Error::Ffmpeg`] if it
    /// exits with an error.
    pub fn at(ffmpeg: impl Into<PathBuf>, ffprobe: impl Into<PathBuf>) -> Result<Self> {
        Self::open(ffmpeg.into(), ffprobe.into(), None, false)
    }

    /// [`Ffmpeg::at`], also looking for libjxl's tools in `bundled`.
    fn open(
        ffmpeg: PathBuf,
        ffprobe: PathBuf,
        bundled: Option<&Path>,
        is_bundled: bool,
    ) -> Result<Self> {
        let version_out = capture(base_command(&ffmpeg).arg("-version"), &ffmpeg)?;
        let version = parse_version(&String::from_utf8_lossy(&version_out));

        let encoders_out = capture(
            base_command(&ffmpeg).args(["-hide_banner", "-encoders"]),
            &ffmpeg,
        )?;
        let encoders = parse_codecs(&String::from_utf8_lossy(&encoders_out));
        let decoders_out = capture(
            base_command(&ffmpeg).args(["-hide_banner", "-decoders"]),
            &ffmpeg,
        )?;
        let decoders = parse_codecs(&String::from_utf8_lossy(&decoders_out));
        // Older ffmpegs lack these options and would fail on them; without
        // them HDR video simply isn't tone-mapped.
        let tone_map = capture(
            base_command(&ffmpeg).args(["-hide_banner", "-h", "filter=scale"]),
            &ffmpeg,
        )
        .is_ok_and(|out| scale_can_tone_map(&String::from_utf8_lossy(&out)));

        let jxl = JxlTools::locate(
            bundled,
            ffmpeg
                .parent()
                .map(Path::to_path_buf)
                .into_iter()
                .chain(search_dirs()),
        );

        Ok(Self {
            ffmpeg,
            ffprobe,
            bundled: is_bundled,
            version,
            encoders: Arc::new(encoders),
            decoders: Arc::new(decoders),
            tone_map,
            jxl: Arc::new(jxl),
            decoded: Arc::new(Mutex::new(DecodeCache::new(4))),
            // WAV files are large; keep just the current file and the last one.
            listenable: Arc::new(Mutex::new(DecodeCache::new(2))),
        })
    }

    /// The libjxl tools found.
    #[must_use]
    pub fn jxl_tools(&self) -> &JxlTools {
        &self.jxl
    }

    /// Which JPEG XL paths are available, for [`crate::plan`].
    #[must_use]
    pub fn jxl_support(&self) -> JxlSupport {
        JxlSupport {
            cjxl: self.jxl.cjxl.is_some(),
            djxl: self.jxl.djxl.is_some(),
            ffmpeg_encoder: self.has_encoder("libjxl"),
            ffmpeg_anim_encoder: self.has_encoder("libjxl_anim"),
            ffmpeg_decoder: self.decoders.contains("libjxl"),
            ffmpeg_anim_decoder: self.decoders.contains("libjxl_anim"),
        }
    }

    /// What the installed tools can do, for [`crate::plan`].
    #[must_use]
    pub fn support(&self) -> Support {
        Support {
            jxl: self.jxl_support(),
            tone_map: self.tone_map,
        }
    }

    /// Path of the ffmpeg binary in use.
    #[must_use]
    pub fn ffmpeg_path(&self) -> &Path {
        &self.ffmpeg
    }

    /// Whether this is the ffmpeg the app ships (see [`Ffmpeg::locate_with`]).
    #[must_use]
    pub fn is_bundled(&self) -> bool {
        self.bundled
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

    /// Whether `format` can be written: ffmpeg has its encoder, or for JPEG
    /// XL, libjxl's `cjxl` is installed.
    #[must_use]
    pub fn supports(&self, format: OutputFormat) -> bool {
        self.missing(format).is_none()
    }

    /// Why `format` can't be written, or `None` if it can.
    #[must_use]
    pub fn missing(&self, format: OutputFormat) -> Option<String> {
        match format {
            OutputFormat::Jxl if self.jxl_support().can_write() => None,
            OutputFormat::Jxl => Some(
                "writing JPEG XL needs libjxl's cjxl (e.g. `brew install jpeg-xl`) or an ffmpeg built with libjxl"
                    .into(),
            ),
            _ if self.has_encoder(format.encoder()) => None,
            _ => Some(format!(
                "this ffmpeg can't write {} (missing the {} encoder)",
                format.extension().to_uppercase(),
                format.encoder()
            )),
        }
    }

    /// Why `format` can't be written as an animation, or `None` if it can.
    /// WebP and JPEG XL animations need other encoders than their stills.
    #[must_use]
    pub fn missing_animation(&self, format: OutputFormat) -> Option<String> {
        let jxl = self.jxl_support();
        match format {
            OutputFormat::Jxl if jxl.cjxl || jxl.ffmpeg_anim_encoder => None,
            OutputFormat::Jxl => Some(
                "animated JPEG XL needs libjxl's cjxl (e.g. `brew install jpeg-xl`) or an ffmpeg with the libjxl_anim encoder"
                    .into(),
            ),
            OutputFormat::Webp if self.has_encoder(format.animation_encoder()) => None,
            OutputFormat::Webp => Some(format!(
                "this ffmpeg can't write animated WebP (missing the {} encoder)",
                format.animation_encoder()
            )),
            _ => self.missing(format),
        }
    }

    /// Whether pictures or a video can become an animation in `format`.
    #[must_use]
    pub fn supports_animation(&self, format: OutputFormat) -> bool {
        format.animates_pictures() && self.missing_animation(format).is_none()
    }

    /// Inspect a file with ffprobe: duration, display size, frame rate and codecs.
    ///
    /// JPEG XL files are also asked of `jxlinfo` (ffprobe can't tell how long
    /// an animation is without a libjxl decoder) and checked for JPEG
    /// reconstruction data.
    ///
    /// # Errors
    ///
    /// The errors of [`media::probe`], plus [`Error::Ffmpeg`] if ffprobe cannot
    /// read the file (for example because it is not media at all).
    pub fn probe(&self, path: impl AsRef<Path>) -> Result<MediaInfo> {
        let path = path.as_ref();
        let mut info = media::probe(path)?;
        let is_jxl = info.extension.as_deref() == Some("jxl") || jxl::is_jxl_file(path);
        let ffprobe = |groups: bool| {
            let mut cmd = base_command(&self.ffprobe);
            cmd.args(["-v", "error", "-print_format", "json"])
                .args(["-show_format", "-show_streams"]);
            if groups {
                cmd.arg("-show_stream_groups");
            }
            capture(cmd.arg(path), &self.ffprobe)
        };
        // Stream groups (HEIF tile grids) are new in ffprobe 7; older ones
        // reject the option, so ask again without it.
        let probed = ffprobe(true).or_else(|_| ffprobe(false)).and_then(|out| {
            serde_json::from_slice::<Value>(&out).map_err(|err| Error::Ffmpeg {
                message: format!("ffprobe returned unreadable output: {err}"),
            })
        });
        match probed {
            Ok(json) => apply_probe(&mut info, &json),
            // Older ffprobes don't know JPEG XL at all; jxlinfo may.
            Err(err) if !is_jxl => return Err(err),
            Err(_) => {}
        }
        // ffprobe reports a JPEG's stored size, but ffmpeg turns it upright
        // by its EXIF orientation when decoding.
        if info.kind == MediaKind::Image
            && info.video_codec.as_deref() == Some("mjpeg")
            && let Some(orientation) = media::jpeg_orientation(path)
        {
            info.orientation = Some(orientation);
            if orientation >= 5 {
                (info.width, info.height) = (info.height, info.width);
            }
        }
        if is_jxl {
            if info.kind == MediaKind::Unknown {
                info.kind = MediaKind::Image;
            }
            if info.video_codec.is_none() {
                info.video_codec = Some("jpegxl".into());
            }
            info.reconstructs_jpeg = jxl::has_jpeg_reconstruction(path);
            if let Some(jxlinfo) = &self.jxl.jxlinfo
                && let Ok(out) = capture(base_command(jxlinfo).arg(path), jxlinfo)
                && let Some(parsed) = jxl::parse_jxlinfo(&String::from_utf8_lossy(&out))
            {
                jxl::apply_jxlinfo(&mut info, &parsed);
            }
        }
        // ffprobe doesn't reliably show GPS in EXIF, so photos are read here.
        if info.kind == MediaKind::Image && scrub::has_location(path) {
            info.has_location = true;
        }
        Ok(info)
    }

    /// A path ffmpeg can decode: `input` itself, or for JPEG XL that this
    /// ffmpeg can't read, a (cached) PNG/APNG decoded by djxl.
    fn readable(&self, input: &Path) -> Result<PathBuf> {
        let support = self.jxl_support();
        if (support.ffmpeg_decoder && support.ffmpeg_anim_decoder) || !jxl::is_jxl_file(input) {
            return Ok(input.to_path_buf());
        }
        let Some(djxl) = self.jxl.djxl.as_deref() else {
            return Err(Error::unsupported(
                "previewing JPEG XL needs djxl from libjxl or an ffmpeg built with libjxl",
            ));
        };
        let stamp = FileStamp::of(input)?;
        let mut cache = self.decoded.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(hit) = cache.get(input, &stamp) {
            return Ok(hit);
        }
        let dir = cache.dir()?;
        let decoded = dir.join(format!("{}.png", cache.next_id()));
        capture(
            base_command(djxl).arg(input).arg(&decoded).arg("--quiet"),
            djxl,
        )?;
        // djxl writes animations as APNG; give it the extension ffmpeg expects.
        let decoded = if is_apng(&decoded) {
            let apng = decoded.with_extension("apng");
            std::fs::rename(&decoded, &apng).map_err(|source| Error::Io {
                path: decoded.clone(),
                source,
            })?;
            apng
        } else {
            decoded
        };
        cache.insert(input.to_path_buf(), stamp, decoded.clone());
        Ok(decoded)
    }

    /// The tile grid of a HEIF/AVIF picture (see [`MediaInfo::tile_grid`]).
    /// Only those files are asked of ffprobe, so previews of anything else
    /// cost no extra process.
    fn tile_grid(&self, input: &Path) -> Option<usize> {
        if !media::is_heif_file(input) {
            return None;
        }
        let out = capture(
            base_command(&self.ffprobe)
                .args([
                    "-v",
                    "error",
                    "-print_format",
                    "json",
                    "-show_stream_groups",
                ])
                .arg(input),
            &self.ffprobe,
        )
        .ok()?;
        let json = serde_json::from_slice::<Value>(&out).ok()?;
        tile_grid(&json).map(|(index, _)| index)
    }

    /// Grab one frame as a JPEG, scaled down to at most `max_height` pixels.
    ///
    /// Works for videos (any time within the clip) and still images (`at_secs`
    /// is ignored when it is zero). Like a conversion, it takes the first
    /// frame at or after `at_secs`. Pass [`MediaInfo::hdr`] as `hdr` to
    /// tone-map the way pictures made from the file will be.
    ///
    /// # Errors
    ///
    /// [`Error::Ffmpeg`] if decoding fails or there is no frame at `at_secs`.
    pub fn frame(&self, input: &Path, at_secs: f64, max_height: u32, hdr: bool) -> Result<Vec<u8>> {
        let input = &self.readable(input)?;
        let grid = self.tile_grid(input);
        // ffmpeg can't seek in animated JPEG XL; decode up to the frame instead.
        let seek_input = !jxl::is_jxl_file(input);
        let at = (at_secs > 0.0).then(|| format_secs(at_secs));
        let mut cmd = base_command(&self.ffmpeg);
        cmd.args(["-hide_banner", "-nostdin", "-v", "error"]);
        if let Some(at) = at.as_deref().filter(|_| seek_input) {
            cmd.args(["-ss", at]);
        }
        cmd.arg("-i").arg(input);
        if let Some(at) = at.as_deref().filter(|_| !seek_input) {
            cmd.args(["-ss", at]);
        }
        cmd.args(["-frames:v", "1", "-an", "-sn"]);
        let scale = format!("scale=-2:'min(ih,{})'", even(max_height));
        let filters = if hdr && self.tone_map {
            format!("{TONE_MAP_FILTER},{scale}")
        } else {
            scale
        };
        // `-vf` can't follow the filtergraph that stitches a tile grid.
        match grid {
            Some(group) => cmd
                .arg("-filter_complex")
                .arg(format!("[0:g:{group}]{filters}[v]"))
                .args(["-map", "[v]"]),
            None => cmd.arg("-vf").arg(filters),
        };
        cmd.args(["-c:v", "mjpeg", "-q:v", "4", "-f", "image2pipe", "pipe:1"]);
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

    /// The first audio track of `input` as a 16-bit WAV file that every
    /// webview can play, for previewing sound it can't decode itself (WMA,
    /// AIFF and Opus in some webviews, a video's AC-3 track, …). Surround
    /// sound is mixed down to stereo.
    ///
    /// The file lives in a private temp folder that disappears with the last
    /// clone of [`Ffmpeg`]. The last two are kept, so playing a file again
    /// doesn't decode it again unless it changed.
    ///
    /// # Errors
    ///
    /// [`Error::Ffmpeg`] if the file has no sound or cannot be decoded,
    /// [`Error::Io`] if the temp folder cannot be created.
    pub fn listenable(&self, input: &Path) -> Result<PathBuf> {
        let stamp = FileStamp::of(input)?;
        let mut cache = self
            .listenable
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(hit) = cache.get(input, &stamp).filter(|hit| hit.is_file()) {
            return Ok(hit);
        }
        let wav = cache.dir()?.join(format!("{}.wav", cache.next_id()));
        let mut cmd = base_command(&self.ffmpeg);
        cmd.args(["-hide_banner", "-nostdin", "-v", "error", "-i"])
            .arg(input)
            .args(["-map", "0:a:0", "-vn", "-sn", "-dn", "-map_metadata", "-1"])
            .args(["-af", "aformat=channel_layouts=mono|stereo"])
            // RF64 past 4 GiB, which plain WAV can't address (hours of sound).
            .args(["-c:a", "pcm_s16le", "-rf64", "auto", "-f", "wav", "-y"])
            .arg(&wav);
        if let Err(err) = capture(&mut cmd, &self.ffmpeg) {
            let _ = std::fs::remove_file(&wav);
            return Err(err);
        }
        cache.insert(input.to_path_buf(), stamp, wav.clone());
        Ok(wav)
    }

    /// Integrated loudness of the first audio track in LUFS (EBU R 128), or
    /// `None` when it is silent. The editor uses it to let you hear what
    /// [`crate::SoundOptions::normalize`] will do before converting.
    ///
    /// # Errors
    ///
    /// [`Error::Ffmpeg`] if the file has no sound or cannot be decoded.
    pub fn loudness(&self, input: &Path) -> Result<Option<f64>> {
        let mut cmd = base_command(&self.ffmpeg);
        // The summary is logged at the info level.
        cmd.args(["-hide_banner", "-nostdin", "-nostats", "-v", "info", "-i"])
            .arg(input)
            .args(["-map", "0:a:0", "-vn", "-sn", "-dn"])
            .args(["-af", "ebur128=framelog=quiet", "-f", "null", "-"]);
        let output = cmd.output().map_err(|source| Error::Spawn {
            program: self.ffmpeg.clone(),
            source,
        })?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        check_status(Tool::Ffmpeg, output.status, &stderr)?;
        parse_integrated_loudness(&stderr).ok_or_else(|| Error::Ffmpeg {
            message: "ffmpeg reported no loudness".into(),
        })
    }

    /// Run a planned conversion to completion, reporting progress.
    ///
    /// Raising `cancel` stops the running step within a fraction of a second.
    /// Intermediate files are always removed; partial output is removed on
    /// cancellation and on failure.
    ///
    /// # Errors
    ///
    /// [`Error::Spawn`] if a program cannot be started, [`Error::Ffmpeg`] with
    /// the most relevant line of its error output if a step fails, and
    /// [`Error::Unsupported`] if the plan needs a libjxl tool that is missing.
    pub fn run(
        &self,
        plan: &Plan,
        cancel: &AtomicBool,
        mut on_progress: impl FnMut(RunProgress),
    ) -> Result<RunOutcome> {
        let remove_scratch = || {
            for path in &plan.scratch {
                let _ = std::fs::remove_file(path);
            }
        };
        // Leftovers of a crashed earlier run would make ffmpeg's `-n` refuse.
        remove_scratch();
        for (path, contents) in &plan.writes {
            if let Err(source) = std::fs::write(path, contents) {
                remove_scratch();
                return Err(Error::Io {
                    path: path.clone(),
                    source,
                });
            }
        }

        let total: f32 = plan.steps.iter().map(|s| s.weight.max(0.0)).sum();
        let total = total.max(f32::EPSILON);
        let several = plan.steps.len() > 1;
        let mut done = 0.0_f32;
        let mut outcome = Ok(true);
        for step in &plan.steps {
            let (base, share) = (done / total, step.weight.max(0.0) / total);
            if several {
                on_progress(RunProgress {
                    fraction: Some(base),
                    speed: None,
                });
            }
            outcome = match step.tool {
                Tool::Ffmpeg => self.run_ffmpeg(step, cancel, |sample| {
                    on_progress(RunProgress {
                        fraction: sample.fraction.map(|f| base + f * share),
                        // Speed relative to real time only makes sense for a lone ffmpeg.
                        speed: sample.speed.filter(|_| !several),
                    });
                }),
                Tool::Scrub if cancel.load(Ordering::Relaxed) => Ok(false),
                Tool::Scrub => scrub::run_step(&step.args).map(|()| true),
                tool => self.run_tool(tool, step, cancel),
            };
            if !matches!(outcome, Ok(true)) {
                break;
            }
            done += step.weight.max(0.0);
        }
        remove_scratch();

        match outcome {
            Ok(true) => {
                let output_bytes = std::fs::metadata(&plan.output)
                    .map(|meta| meta.len())
                    .map_err(|source| Error::Io {
                        path: plan.output.clone(),
                        source,
                    })?;
                Ok(RunOutcome::Done { output_bytes })
            }
            Ok(false) => {
                let _ = std::fs::remove_file(&plan.output);
                Ok(RunOutcome::Cancelled)
            }
            Err(err) => {
                let _ = std::fs::remove_file(&plan.output);
                Err(err)
            }
        }
    }

    fn tool_path(&self, tool: Tool) -> Result<&Path> {
        let path = match tool {
            Tool::Ffmpeg => Some(self.ffmpeg.as_path()),
            Tool::Cjxl => self.jxl.cjxl.as_deref(),
            Tool::Djxl => self.jxl.djxl.as_deref(),
            Tool::Scrub => return Err(Error::unsupported("metadata cleaning runs in-process")),
        };
        path.ok_or_else(|| {
            Error::unsupported(format!("{} from libjxl is not installed", tool.name()))
        })
    }

    /// One ffmpeg step. `Ok(false)` means it was cancelled.
    fn run_ffmpeg(
        &self,
        step: &Step,
        cancel: &AtomicBool,
        mut on_progress: impl FnMut(RunProgress),
    ) -> Result<bool> {
        let mut cmd = base_command(&self.ffmpeg);
        cmd.args(["-hide_banner", "-nostdin", "-v", "error", "-nostats"])
            .args(["-progress", "pipe:1", "-stats_period", "0.25"])
            // Never overwrite: the output path was chosen to be free.
            .arg("-n")
            .args(&step.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|source| Error::Spawn {
            program: self.ffmpeg.clone(),
            source,
        })?;
        let stderr = drain(child.stderr.take());

        let mut cancelled = false;
        if let Some(stdout) = child.stdout.take() {
            let mut sample = ProgressParser::new(step.duration_secs);
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
            return Ok(false);
        }
        check_status(Tool::Ffmpeg, status, &stderr)?;
        Ok(true)
    }

    /// One cjxl/djxl step, which report no progress. `Ok(false)` means it was
    /// cancelled.
    fn run_tool(&self, tool: Tool, step: &Step, cancel: &AtomicBool) -> Result<bool> {
        let program = self.tool_path(tool)?;
        let mut child = base_command(program)
            .args(&step.args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| Error::Spawn {
                program: program.to_path_buf(),
                source,
            })?;
        let stderr = drain(child.stderr.take());
        let status = loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(false);
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(source) => {
                    return Err(Error::Spawn {
                        program: program.to_path_buf(),
                        source,
                    });
                }
            }
        };
        let stderr = stderr
            .and_then(|handle| handle.join().ok())
            .unwrap_or_default();
        check_status(tool, status, &stderr)?;
        Ok(true)
    }
}

/// Read a child's stderr on its own thread so a chatty program never blocks
/// on a full pipe.
fn drain(pipe: Option<std::process::ChildStderr>) -> Option<std::thread::JoinHandle<String>> {
    pipe.map(|mut pipe| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        })
    })
}

fn check_status(tool: Tool, status: ExitStatus, stderr: &str) -> Result<()> {
    if status.success() {
        return Ok(());
    }
    Err(Error::Ffmpeg {
        message: friendly_error(stderr)
            .unwrap_or_else(|| format!("{} stopped unexpectedly ({status})", tool.name())),
    })
}

/// Size and modification time, to notice a file changed since it was decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl FileStamp {
    fn of(path: &Path) -> Result<Self> {
        let meta = std::fs::metadata(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self {
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }
}

/// The last few JPEG XL files djxl decoded for previews, in a private temp
/// folder that disappears with the last clone of [`Ffmpeg`].
#[derive(Debug)]
struct DecodeCache {
    dir: Option<tempfile::TempDir>,
    entries: Vec<(PathBuf, FileStamp, PathBuf)>,
    counter: u64,
    /// How many decoded files to keep.
    capacity: usize,
}

impl DecodeCache {
    fn new(capacity: usize) -> Self {
        Self {
            dir: None,
            entries: Vec::new(),
            counter: 0,
            capacity: capacity.max(1),
        }
    }

    fn get(&self, source: &Path, stamp: &FileStamp) -> Option<PathBuf> {
        self.entries
            .iter()
            .find(|(path, s, _)| path == source && s == stamp)
            .map(|(_, _, decoded)| decoded.clone())
    }

    fn dir(&mut self) -> Result<PathBuf> {
        if self.dir.is_none() {
            let dir = tempfile::Builder::new()
                .prefix("catconvert-preview-")
                .tempdir()
                .map_err(|source| Error::Io {
                    path: std::env::temp_dir(),
                    source,
                })?;
            self.dir = Some(dir);
        }
        Ok(self
            .dir
            .as_ref()
            .map(|d| d.path().to_path_buf())
            .unwrap_or_default())
    }

    fn next_id(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }

    fn insert(&mut self, source: PathBuf, stamp: FileStamp, decoded: PathBuf) {
        self.entries.retain(|(path, _, old)| {
            let keep = *path != source;
            if !keep {
                let _ = std::fs::remove_file(old);
            }
            keep
        });
        if self.entries.len() >= self.capacity {
            let (_, _, old) = self.entries.remove(0);
            let _ = std::fs::remove_file(old);
        }
        self.entries.push((source, stamp, decoded));
    }
}

/// Whether a PNG file is animated: an `acTL` chunk before the first `IDAT`.
fn is_apng(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = Vec::with_capacity(64 * 1024);
    if file.take(256 * 1024).read_to_end(&mut head).is_err() {
        return false;
    }
    let find = |needle: &[u8]| head.windows(4).position(|w| w == needle);
    match (find(b"acTL"), find(b"IDAT")) {
        (Some(actl), Some(idat)) => actl < idat,
        (Some(_), None) => true,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Process helpers
// ---------------------------------------------------------------------------

pub(crate) fn base_command(program: &Path) -> Command {
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

pub(crate) fn executable(dir: &Path, name: &str) -> Option<PathBuf> {
    let path = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    path.is_file().then_some(path)
}

fn sibling(binary: &Path, name: &str) -> Option<PathBuf> {
    executable(binary.parent()?, name)
}

/// The bundled ffmpeg and ffprobe in `dir`, if both are there.
fn bundled_pair(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let ffmpeg = executable(dir, &format!("{BUNDLED_PREFIX}ffmpeg"))?;
    let ffprobe = executable(dir, &format!("{BUNDLED_PREFIX}ffprobe"))?;
    Some((ffmpeg, ffprobe))
}

/// Every directory on `PATH`, then common install locations.
fn search_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(extra_search_dirs())
        .collect()
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

/// Codec names from `ffmpeg -encoders` or `-decoders`, whose rows look like
/// ` V....D libx264              libx264 H.264 / AVC ...`.
fn parse_codecs(output: &str) -> HashSet<String> {
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
        // Animated JPEG XL; jxlinfo fills in the length when installed.
        (MediaKind::Image, Some(_), _) if format_name == "jpegxl_anim" => MediaKind::Video,
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
        info.orientation = rotation_orientation(v);
        info.video_codec = v["codec_name"].as_str().map(str::to_owned);
        info.hdr = matches!(
            v["color_transfer"].as_str(),
            Some("smpte2084" | "arib-std-b67")
        );
        if info.kind == MediaKind::Video {
            info.fps = v["avg_frame_rate"]
                .as_str()
                .and_then(parse_ratio)
                .or_else(|| v["r_frame_rate"].as_str().and_then(parse_ratio));
        }
    }
    // A tiled HEIF picture: each tile is a stream, the grid has the real size.
    if info.kind == MediaKind::Image
        && let Some((index, grid)) = tile_grid(json)
    {
        info.tile_grid = Some(index);
        let width = grid["width"].as_u64().and_then(|w| u32::try_from(w).ok());
        let height = grid["height"].as_u64().and_then(|h| u32::try_from(h).ok());
        (info.width, info.height) = if is_quarter_turn(grid) {
            (height, width)
        } else {
            (width, height)
        };
        info.orientation = rotation_orientation(grid);
    }
    info.audio_codec = audio.and_then(|a| a["codec_name"].as_str().map(str::to_owned));
    info.location_tags = format["tags"]
        .as_object()
        .map(|tags| {
            tags.keys()
                .filter(|key| is_location_tag(key))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    info.has_location = !info.location_tags.is_empty();
    info.sample_rate = audio
        .and_then(|a| as_f64(&a["sample_rate"]))
        .filter(|rate| rate.is_finite() && *rate >= 1.0 && *rate <= f64::from(u32::MAX))
        .map(|rate| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let rate = rate.round() as u32;
            rate
        });
    info.duration_secs = if info.kind == MediaKind::Image {
        None
    } else {
        duration
    };
}

/// Whether `ffmpeg -h filter=scale` lists the colour-management options of
/// ffmpeg 8's swscale, which tone-map HDR when the output transfer is SDR.
fn scale_can_tone_map(help: &str) -> bool {
    let has = |option: &str| {
        help.lines()
            .filter_map(|line| line.split_whitespace().next())
            .any(|name| name.trim_start_matches('-') == option)
    };
    has("out_transfer") && has("out_primaries") && has("intent")
}

/// Container tags that say where a recording was made: QuickTime's `©xyz`
/// (read as `location`, plus `location-eng` and the like), Apple's
/// `com.apple.quicktime.location.*` keys and Matroska's `LOCATION`.
fn is_location_tag(key: &str) -> bool {
    key.to_ascii_lowercase().contains("location")
}

/// The tile grid ffmpeg would show by default, as its stream group index and
/// its component (size, rotation), from ffprobe's `-show_stream_groups`.
fn tile_grid(json: &Value) -> Option<(usize, &Value)> {
    let grids = json["stream_groups"]
        .as_array()?
        .iter()
        .filter(|g| g["type"] == "Tile Grid");
    let group = grids
        .clone()
        .find(|g| g["disposition"]["default"] == 1)
        .or_else(|| grids.clone().next())?;
    let index = usize::try_from(group["index"].as_u64()?).ok()?;
    Some((index, &group["components"][0]))
}

fn is_still_image(stream: &Value, format_name: &str) -> bool {
    format_name.ends_with("_pipe") || format_name == "image2" || stream["nb_frames"] == "1"
}

/// The display rotation of a stream or tile grid component, in degrees
/// counter-clockwise as ffprobe reports it.
fn rotation(stream: &Value) -> Option<f64> {
    let from_side_data = stream["side_data_list"]
        .as_array()
        .and_then(|list| list.iter().find_map(|d| d["rotation"].as_f64()));
    let from_tag = stream["tags"]["rotate"]
        .as_str()
        .and_then(|r| r.parse::<f64>().ok());
    from_side_data.or(from_tag)
}

/// Rotation of ±90° (phone videos and photos) swaps the displayed width and
/// height. Takes a stream or a tile grid component.
fn is_quarter_turn(stream: &Value) -> bool {
    rotation(stream).is_some_and(|rotation| (rotation.abs().round() as i64) % 180 == 90)
}

/// A stream's display rotation as the EXIF orientation it amounts to:
/// counter-clockwise 90° (ffprobe's `-90`, a clockwise turn) is 6, and so on.
fn rotation_orientation(stream: &Value) -> Option<u8> {
    #[allow(clippy::cast_possible_truncation)]
    let degrees = (rotation(stream)?.round() as i64).rem_euclid(360);
    match degrees {
        90 => Some(8),
        180 => Some(3),
        270 => Some(6),
        _ => None,
    }
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

/// The integrated loudness from the summary `ebur128` logs at the end:
///
/// ```text
///   Integrated loudness:
///     I:         -31.8 LUFS
/// ```
///
/// `Some(None)` for silence, which ebur128 reports as -70 LUFS (its floor).
fn parse_integrated_loudness(log: &str) -> Option<Option<f64>> {
    let summary = &log[log.rfind("Integrated loudness:")?..];
    let value: f64 = summary
        .lines()
        .find_map(|line| line.trim().strip_prefix("I:"))?
        .trim()
        .strip_suffix("LUFS")?
        .trim()
        .parse()
        .ok()?;
    Some((value.is_finite() && value > -69.5).then_some(value))
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
            sample_rate: None,
            reconstructs_jpeg: false,
            hdr: false,
            has_location: false,
            location_tags: Vec::new(),
            tile_grid: None,
            orientation: None,
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
        let encoders = parse_codecs(out);
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
                    {"codec_type": "audio", "codec_name": "aac", "sample_rate": "48000"}
                ],
                "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "42.5"}
            }),
        );
        assert_eq!(info.kind, MediaKind::Video);
        assert_eq!((info.width, info.height), (Some(1080), Some(1920)));
        // ffprobe's -90 is a clockwise quarter turn: EXIF orientation 6.
        assert_eq!(info.orientation, Some(6));
        assert_eq!(info.duration_secs, Some(42.5));
        assert!((info.fps.unwrap() - 29.97).abs() < 0.01);
        assert_eq!(info.video_codec.as_deref(), Some("hevc"));
        assert!(info.has_audio());
        assert!(!info.has_location);
        assert_eq!(info.sample_rate, Some(48_000));
        assert!(!info.hdr);
    }

    #[test]
    fn probe_json_spots_hdr_transfers() {
        for (transfer, hdr) in [
            ("arib-std-b67", true),
            ("smpte2084", true),
            ("bt709", false),
        ] {
            let mut info = base("clip.mov");
            apply_probe(
                &mut info,
                &json!({
                    "streams": [{"codec_type": "video", "codec_name": "hevc", "width": 3840,
                                 "height": 2160, "color_transfer": transfer}],
                    "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "5"}
                }),
            );
            assert_eq!(info.hdr, hdr, "{transfer}");
        }
    }

    #[test]
    fn tone_mapping_needs_swscale_colour_options() {
        let modern = "scale AVOptions:\n   out_primaries     <int>   ..FV....... set output primaries\n   out_transfer      <int>   ..FV....... set output color transfer\nSWScaler AVOptions:\n  -intent            <int>   E..V....... color mapping intent\n";
        assert!(scale_can_tone_map(modern));
        let old = "scale AVOptions:\n  w                 <string>     ..FV.......\n  out_color_matrix  <string>     ..FV.......\n";
        assert!(!scale_can_tone_map(old));
    }

    #[test]
    fn probe_json_for_tiled_iphone_photo() {
        // What ffprobe -show_stream_groups says about a portrait iPhone HEIC:
        // 48 tiles of 512×512, a thumbnail, and the grid that joins the tiles.
        let tile = json!({"codec_type": "video", "codec_name": "hevc", "width": 512,
                          "height": 512, "nb_frames": "1", "disposition": {"default": 0}});
        let mut streams = vec![tile; 48];
        streams.push(
            json!({"codec_type": "video", "codec_name": "hevc", "width": 320,
                            "height": 240, "nb_frames": "1", "disposition": {"default": 0}}),
        );
        let mut info = base("IMG_0001.HEIC");
        apply_probe(
            &mut info,
            &json!({
                "streams": streams,
                "stream_groups": [
                    {"index": 0, "id": "0x31", "type": "Tile Grid",
                     "disposition": {"default": 1},
                     "components": [{"nb_tiles": 48, "width": 4032, "height": 3024,
                                     "side_data_list": [{"side_data_type": "Display Matrix",
                                                         "rotation": -90}]}]}
                ],
                "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2"}
            }),
        );
        assert_eq!(info.kind, MediaKind::Image);
        assert_eq!(info.tile_grid, Some(0));
        assert_eq!((info.width, info.height), (Some(3024), Some(4032)));
        assert_eq!(info.video_codec.as_deref(), Some("hevc"));
        assert_eq!(info.duration_secs, None);
    }

    #[test]
    fn probe_json_for_small_untiled_heic() {
        let mut info = base("small.heic");
        apply_probe(
            &mut info,
            &json!({
                "streams": [{"codec_type": "video", "codec_name": "hevc", "width": 400,
                             "height": 300, "nb_frames": "1"}],
                "stream_groups": [],
                "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2"}
            }),
        );
        assert_eq!(info.kind, MediaKind::Image);
        assert_eq!(info.tile_grid, None);
        assert_eq!((info.width, info.height), (Some(400), Some(300)));
    }

    #[test]
    fn probe_json_finds_location_tags() {
        let mut info = base("clip.mov");
        apply_probe(
            &mut info,
            &json!({
                "streams": [{"codec_type": "video", "codec_name": "hevc"}],
                "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "tags": {
                    "title": "Cat",
                    "location": "+52.5200+013.4050/",
                    "location-eng": "+52.5200+013.4050/",
                    "com.apple.quicktime.location.ISO6709": "+52.5200+013.4050+034.000/",
                    "com.apple.quicktime.make": "Apple"
                }}
            }),
        );
        assert!(info.has_location);
        let mut tags = info.location_tags.clone();
        tags.sort();
        assert_eq!(
            tags,
            [
                "com.apple.quicktime.location.ISO6709",
                "location",
                "location-eng"
            ]
        );
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

    #[test]
    fn reads_integrated_loudness_from_the_ebur128_summary() {
        let log = "[Parsed_ebur128_0 @ 0x1] t: 2.9   M: -31.8 S: -31.8     I: -31.8 LUFS\n\
                   [Parsed_ebur128_0 @ 0x1] Summary:\n\n  Integrated loudness:\n    \
                   I:         -18.4 LUFS\n    Threshold: -28.4 LUFS\n";
        assert_eq!(parse_integrated_loudness(log), Some(Some(-18.4)));
        let silent = "  Integrated loudness:\n    I:         -70.0 LUFS\n";
        assert_eq!(parse_integrated_loudness(silent), Some(None));
        assert_eq!(parse_integrated_loudness("no summary"), None);
    }

    /// Real ffmpeg: decoding for playback and measuring loudness. Skipped
    /// (passes) when ffmpeg is not installed.
    #[test]
    fn decodes_sound_for_playback_and_measures_it() {
        let Ok(ffmpeg) = Ffmpeg::locate() else {
            eprintln!("ffmpeg not found; skipping playback test");
            return;
        };
        let dir = tempfile::tempdir().expect("temp dir");
        let input = dir.path().join("tone.aiff");
        let status = base_command(ffmpeg.ffmpeg_path())
            .args(["-v", "error", "-f", "lavfi", "-i", "sine=f=440:d=2"])
            .args(["-af", "volume=-10dB,pan=5.1|FL=c0|FR=c0"])
            .arg(&input)
            .status()
            .expect("run ffmpeg");
        assert!(status.success());

        let wav = ffmpeg.listenable(&input).expect("decode");
        let bytes = std::fs::read(&wav).expect("read wav");
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        let info = ffmpeg.probe(&wav).expect("probe wav");
        assert_eq!(info.audio_codec.as_deref(), Some("pcm_s16le"));
        assert!((info.duration_secs.unwrap_or(0.0) - 2.0).abs() < 0.1);
        assert_eq!(
            ffmpeg.listenable(&input).expect("cached"),
            wav,
            "decoded once"
        );

        let lufs = ffmpeg
            .loudness(&input)
            .expect("measure")
            .expect("not silent");
        assert!((-40.0..-10.0).contains(&lufs), "{lufs} LUFS");

        let silence = dir.path().join("silence.wav");
        let status = base_command(ffmpeg.ffmpeg_path())
            .args(["-v", "error", "-f", "lavfi", "-i", "anullsrc=d=1"])
            .arg(&silence)
            .status()
            .expect("run ffmpeg");
        assert!(status.success());
        assert_eq!(ffmpeg.loudness(&silence).expect("measure"), None);

        let picture = dir.path().join("still.png");
        let status = base_command(ffmpeg.ffmpeg_path())
            .args(["-v", "error", "-f", "lavfi", "-i", "color=s=16x16:d=1"])
            .args(["-frames:v", "1"])
            .arg(&picture)
            .status()
            .expect("run ffmpeg");
        assert!(status.success());
        assert!(ffmpeg.listenable(&picture).is_err(), "no sound to play");
    }

    #[test]
    fn bundled_tools_need_the_prefix_and_both_binaries() {
        let dir = tempfile::tempdir().expect("temp dir");
        let touch = |name: &str| {
            let path = dir
                .path()
                .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
            std::fs::write(&path, b"").expect("write");
            path
        };
        // Unprefixed names are a system install, not the app's copy.
        touch("ffmpeg");
        touch("ffprobe");
        assert_eq!(bundled_pair(dir.path()), None);

        let ffmpeg = touch("catconvert-ffmpeg");
        assert_eq!(bundled_pair(dir.path()), None, "ffprobe is missing");

        let ffprobe = touch("catconvert-ffprobe");
        assert_eq!(bundled_pair(dir.path()), Some((ffmpeg, ffprobe)));
    }
}
