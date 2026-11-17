//! Pictures → animation: one GIF, WebP, JPEG XL or video slideshow made from
//! several still pictures.
//!
//! [`plan_sequence`] is pure like [`crate::plan`]. ffmpeg reads the pictures
//! through its concat demuxer from generated list files (see
//! [`Plan::writes`]), which handles any file name. A few things about that
//! demuxer shape the plan:
//!
//! - It decodes every file of a list with the decoder of the first, and
//!   changing picture sizes mid-stream garble the scaling. So each list holds
//!   a *run* of consecutive pictures sharing codec, size and orientation, and
//!   the runs are joined by the concat filter. When there would be too many
//!   runs, every picture is first scaled into a same-sized PNG instead.
//! - ffmpeg turns a picture upright by its EXIF orientation once per input,
//!   from its first frame; that is why orientation is part of a run.
//! - Tiled HEIC photos are stitched from a stream group, which a list can't
//!   express; each of those is scaled into a PNG first.
//! - Pixel formats may still change within a run (a grey JPEG among colour
//!   ones). `-reinit_filter 0` keeps one filter graph, which `scale` copes
//!   with, so the GIF palette survives.
//! - It rounds timestamps to 1/25 s and ignores the last file's duration.
//!   Timestamps are therefore recomputed from the picture count, exact to the
//!   hundredth, the last picture is listed twice so it ends on time, and
//!   `-frames:v` cuts the copy off.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::convert::{
    ANIMATED_JXL_MAX_SIZE, ANIMATED_WEBP_MAX_SIZE, FRAME_MEMORY_BUDGET, GIF_MAX_SIZE, VideoRate,
    cjxl_step, free_path, lerp, push_animated_webp, push_ffmpeg_jxl, push_video_encoder,
    scratch_path, turn_filters,
};
use crate::ffmpeg::format_secs;
use crate::jxl::is_jxl;
use crate::{
    ConvertOptions, Direction, Error, MediaInfo, MediaKind, Naming, OutputFormat, Plan, Result,
    Step, Support, Tool, VideoCodec,
};

/// How pictures of another shape fit the frame, which takes the size of the
/// first picture.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    /// Shrink to fit and fill the rest with black bars.
    #[default]
    Pad,
    /// Enlarge to cover the frame and crop what sticks out.
    Crop,
}

impl Direction {
    /// Indices into a list of `count` pictures, in the order they show.
    /// A boomerang doesn't repeat the turning points: `0 1 2 3 2 1`, then the
    /// loop starts over at `0`.
    #[must_use]
    pub fn order(self, count: usize) -> Vec<usize> {
        match self {
            Self::Forward => (0..count).collect(),
            Self::Reverse => (0..count).rev().collect(),
            Self::Boomerang => (0..count)
                .chain((1..count.saturating_sub(1)).rev())
                .collect(),
        }
    }
}

/// Settings that only matter when making an animation from pictures. The
/// rest (format, size cap, turn, mirror, direction, quality, JPEG XL
/// settings) comes from [`ConvertOptions`]; its trim, frame rate, speed,
/// crop, sound and metadata settings don't apply. Animations never carry the
/// first picture's tags.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SequenceOptions {
    /// How long each picture shows, in seconds. Rounded to hundredths, the
    /// precision of GIF.
    pub frame_secs: f64,
    pub fit: Fit,
}

impl Default for SequenceOptions {
    fn default() -> Self {
        Self {
            frame_secs: 0.5,
            fit: Fit::Pad,
        }
    }
}

impl OutputFormat {
    /// Whether pictures can become an animation in this format: GIF, WebP,
    /// JPEG XL or a video slideshow.
    #[must_use]
    pub fn animates_pictures(self) -> bool {
        matches!(self, Self::Gif | Self::Webp | Self::Jxl) || self.kind() == MediaKind::Video
    }
}

/// Most pictures one animation takes.
pub const MAX_PICTURES: usize = 1000;
/// Shortest and longest time a picture can show, in hundredths of a second.
const FRAME_CENTIS: std::ops::RangeInclusive<u32> = 2..=6000;
/// Lowest frame rate of video slideshows: each picture is repeated to reach
/// it, since players and sites cope badly with a few frames per second.
const SLIDESHOW_MIN_FPS: u32 = 24;
/// More runs than this (changes of codec, size or orientation along the way)
/// and the pictures are scaled one by one first, rather than opening a
/// concat input per run.
const MAX_RUNS: usize = 8;

/// Validate the request and plan the steps that make one animation from
/// `inputs`, in the order given, and write it to `output`.
///
/// The pictures play as `options.direction` says. The frame takes the size
/// of the first picture (after its EXIF rotation), shrunk by
/// `options.max_size` or the format's usual cap, as for a video. `support`
/// says which JPEG XL tools are installed.
///
/// # Errors
///
/// [`Error::Unsupported`] for fewer than two or more than [`MAX_PICTURES`]
/// pictures, an input that isn't a still picture or whose name can't go in a
/// list file, a format that doesn't animate, an unknown size of the first
/// picture, a frame time outside 0.02–60 s, or missing JPEG XL tools.
pub fn plan_sequence(
    inputs: &[MediaInfo],
    options: &ConvertOptions,
    sequence: &SequenceOptions,
    output: &Path,
    support: impl Into<Support>,
) -> Result<Plan> {
    let jxl = support.into().jxl;
    let format = options.format;
    if inputs.len() < 2 {
        return Err(Error::unsupported(
            "an animation needs at least two pictures",
        ));
    }
    if inputs.len() > MAX_PICTURES {
        return Err(Error::unsupported(format!(
            "an animation can hold at most {MAX_PICTURES} pictures"
        )));
    }
    if let Some(other) = inputs.iter().find(|info| info.kind != MediaKind::Image) {
        return Err(Error::unsupported(format!(
            "{} is not a still picture",
            other.file_name
        )));
    }
    if !format.animates_pictures() {
        return Err(Error::unsupported(format!(
            "can't make an animation as {}",
            format.extension().to_uppercase()
        )));
    }
    options.check_codec()?;
    if !matches!(options.rotate, 0 | 90 | 180 | 270) {
        return Err(Error::unsupported(
            "rotation must be 0, 90, 180 or 270 degrees",
        ));
    }
    if inputs.iter().any(|info| info.path == output) {
        return Err(Error::unsupported(
            "the output would overwrite one of the pictures",
        ));
    }
    let centis = frame_centis(sequence.frame_secs)?;
    let (width, height) = frame_size(&inputs[0], options)?;

    let mut steps = Vec::new();
    let mut scratch = Vec::new();
    let mut writes = Vec::new();

    let mut sources = inputs.iter().map(Source::of).collect::<Result<Vec<_>>>()?;
    // JPEG XL that ffmpeg can't decode: djxl unpacks it to PNG first.
    for (i, info) in inputs.iter().enumerate() {
        if !is_jxl(info) || jxl.ffmpeg_decoder {
            continue;
        }
        if !jxl.djxl {
            return Err(Error::unsupported(
                "reading JPEG XL needs djxl from libjxl or an ffmpeg built with libjxl",
            ));
        }
        let decoded = scratch_path(output, &format!("decoded{}", i + 1), "png");
        steps.push(Step {
            tool: Tool::Djxl,
            args: vec![
                info.path.clone().into_os_string(),
                decoded.clone().into_os_string(),
                "--quiet".into(),
            ],
            duration_secs: None,
            weight: 0.1,
        });
        sources[i].path = decoded.clone();
        sources[i].codec = Some("png".into());
        sources[i].orientation = None;
        scratch.push(decoded);
    }

    let fit = match sequence.fit {
        Fit::Pad => format!(
            "scale={width}:{height}:force_original_aspect_ratio=decrease:flags=lanczos,\
             pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black"
        ),
        Fit::Crop => format!(
            "scale={width}:{height}:force_original_aspect_ratio=increase:flags=lanczos,\
             crop={width}:{height}"
        ),
    };

    let order = options.direction.order(inputs.len());
    let mut used: Vec<usize> = order.clone();
    used.sort_unstable();
    used.dedup();
    let weight = 1.0 / used.len() as f32;
    let mut prescale = |i: usize, sources: &mut [Source]| {
        let (step, scaled) = prescale_step(&sources[i], &fit, (width, height), output, i, weight);
        steps.push(step);
        scratch.push(scaled.path.clone());
        sources[i] = scaled;
    };
    for &i in &used {
        if sources[i].tile_grid.is_some() {
            prescale(i, &mut sources);
        }
    }
    let mut runs = runs(&order, &sources);
    let normalized = runs.len() > MAX_RUNS;
    if normalized {
        // Too many changes along the way: scale every picture first, so they
        // all read as one run.
        for &i in &used {
            if !sources[i].prescaled {
                prescale(i, &mut sources);
            }
        }
        runs = vec![order.clone()];
    }

    // One concat input per run.
    let secs = format_secs(f64::from(centis) / 100.0);
    let mut args: Vec<OsString> = Vec::new();
    for (n, run) in runs.iter().enumerate() {
        let mut list = String::from("ffconcat version 1.0\n");
        for &i in run {
            let _ = writeln!(list, "file {}\nduration {secs}", sources[i].quoted());
        }
        if n + 1 == runs.len()
            && let Some(&last) = run.last()
        {
            let _ = writeln!(list, "file {}", sources[last].quoted());
        }
        let path = scratch_path(output, &format!("list{}", n + 1), "txt");
        args.extend(
            ["-reinit_filter", "0", "-f", "concat", "-safe", "0", "-i"].map(OsString::from),
        );
        args.push(path.clone().into());
        writes.push((path.clone(), list));
        scratch.push(path);
    }

    // Fit every run to the frame, join them, then turn, time and encode.
    let mut graph = String::new();
    let fit = if normalized {
        "setsar=1".to_owned()
    } else {
        format!("{fit},setsar=1")
    };
    if runs.len() == 1 {
        let _ = write!(graph, "[0:v]{fit}");
    } else {
        for n in 0..runs.len() {
            let _ = write!(graph, "[{n}:v]{fit}[p{n}];");
        }
        for n in 0..runs.len() {
            let _ = write!(graph, "[p{n}]");
        }
        let _ = write!(graph, "concat=n={}:v=1:a=0", runs.len());
    }
    let slideshow = format.kind() == MediaKind::Video && format != OutputFormat::Gif;
    let per_picture = if slideshow {
        (SLIDESHOW_MIN_FPS * centis).div_ceil(100)
    } else {
        1
    };
    let mut tail = turn_filters(options);
    tail.extend([
        "settb=1/100".to_owned(),
        format!("setpts=N*{centis}"),
        format!("fps={}/{centis}", 100 * per_picture),
    ]);
    let _ = write!(graph, ",{}", tail.join(","));

    let q = options.quality.min(100);
    if format == OutputFormat::Gif {
        // A palette per picture: slideshow frames have little in common.
        let colors = lerp(q, 32.0, 256.0);
        let _ = write!(
            graph,
            ",split[a][b];[a]palettegen=max_colors={colors}:stats_mode=single[p];\
             [b][p]paletteuse=new=1:dither=bayer:bayer_scale=4"
        );
    }
    graph.push_str("[out]");

    let frames = order.len();
    let total_frames = frames * usize::try_from(per_picture).unwrap_or(1);
    args.extend(["-filter_complex".into(), graph.into()]);
    args.extend(["-map", "[out]", "-frames:v", &total_frames.to_string()].map(OsString::from));
    let mut push = |items: &[&str]| args.extend(items.iter().map(OsString::from));

    let mut frames_for_cjxl = None;
    match format {
        OutputFormat::Gif => push(&["-loop", "0"]),
        OutputFormat::Webp => push_animated_webp(&mut push, q),
        OutputFormat::Jxl => {
            let raw = f64::from(width) * f64::from(height) * 4.0 * frames as f64;
            let too_big = raw > FRAME_MEMORY_BUDGET;
            if jxl.cjxl && !(too_big && jxl.ffmpeg_anim_encoder) {
                if too_big {
                    return Err(Error::unsupported(
                        "too many pictures for an animated JPEG XL: use fewer, or a smaller size",
                    ));
                }
                // Lossless APNG for cjxl; fast zlib, it only lives until cjxl reads it.
                push(&["-c:v", "apng", "-compression_level", "1", "-plays", "0"]);
                push(&["-f", "apng"]);
                frames_for_cjxl = Some(scratch_path(output, "frames", "apng"));
            } else if jxl.ffmpeg_anim_encoder {
                push(&["-c:v", "libjxl_anim"]);
                push_ffmpeg_jxl(&mut push, options);
                push(&["-f", "data"]);
            } else {
                return Err(Error::unsupported(
                    "animated JPEG XL needs cjxl from libjxl or an ffmpeg with the libjxl_anim encoder",
                ));
            }
        }
        _ => {
            let codec = options.codec().unwrap_or(VideoCodec::H264);
            // A slideshow is mostly still: constant quality suits it, and
            // an exact bitrate would mostly go to the first frame.
            push_video_encoder(&mut push, VideoRate::constant(codec, q), None);
            if codec == VideoCodec::H264 {
                push(&["-tune", "stillimage"]);
            }
            if matches!(format, OutputFormat::Mp4 | OutputFormat::Mov) {
                push(&["-movflags", "+faststart"]);
            }
        }
    }
    // Tags of the first picture (place, camera) don't describe the animation.
    push(&["-an", "-sn", "-dn", "-map_metadata", "-1"]);

    let encode_weight = if normalized { 0.5 } else { 1.0 };
    let duration_secs = Some(frames as f64 * f64::from(centis) / 100.0);
    match frames_for_cjxl {
        Some(apng) => {
            args.push(apng.clone().into());
            steps.push(Step {
                tool: Tool::Ffmpeg,
                args,
                duration_secs,
                weight: 0.4 * encode_weight,
            });
            steps.push(cjxl_step(
                &apng,
                output,
                options,
                false,
                0.6 * encode_weight,
            ));
            scratch.push(apng);
        }
        None => {
            args.push(output.as_os_str().to_owned());
            steps.push(Step {
                tool: Tool::Ffmpeg,
                args,
                duration_secs,
                weight: encode_weight,
            });
        }
    }

    Ok(Plan {
        steps,
        output: output.to_path_buf(),
        scratch,
        writes,
    })
}

/// A free path for the animation next to the first picture, named after
/// what the pictures' names share (`cat 1.jpg`, `cat 2.jpg` → `cat.gif`), or
/// else their folder.
///
/// `taken` rules out paths reserved but not yet on disk, as for
/// [`crate::output_path`].
#[must_use]
pub fn sequence_output_path(
    inputs: &[MediaInfo],
    format: OutputFormat,
    taken: impl Fn(&Path) -> bool,
) -> PathBuf {
    let first = inputs
        .first()
        .map_or_else(|| Path::new(""), |info| &info.path);
    free_path(first, &sequence_name(inputs), format.extension(), |p| {
        inputs.iter().any(|info| info.path == p) || taken(p)
    })
}

/// How to name an animation made from `inputs` with `options`: next to
/// the first picture, called as [`sequence_output_path`] would, unless an
/// [`crate::OutputTarget`] says otherwise.
#[must_use]
pub fn sequence_naming(inputs: &[MediaInfo], options: &ConvertOptions) -> Naming {
    Naming::sequence(inputs, options, sequence_name(inputs))
}

/// The shared start of the pictures' names, without trailing numbers and
/// separators, if at least two letters or digits remain; otherwise the
/// folder's name, or `animation`.
fn sequence_name(inputs: &[MediaInfo]) -> String {
    let stems: Vec<String> = inputs
        .iter()
        .map(|info| {
            info.path
                .file_stem()
                .map_or_else(String::new, |s| s.to_string_lossy().into_owned())
        })
        .collect();
    let mut prefix: Vec<char> = stems.first().map_or_else(Vec::new, |s| s.chars().collect());
    for stem in &stems[1.min(stems.len())..] {
        let shared = prefix
            .iter()
            .zip(stem.chars())
            .take_while(|(a, b)| *a == b)
            .count();
        prefix.truncate(shared);
    }
    let prefix: String = prefix.into_iter().collect();
    let trimmed = prefix
        .trim_end_matches(|c: char| c.is_ascii_digit() || c.is_whitespace() || "_-.#(".contains(c));
    if trimmed.chars().filter(|c| c.is_alphanumeric()).count() >= 2 {
        return trimmed.to_owned();
    }
    inputs
        .first()
        .and_then(|info| info.path.parent())
        .and_then(Path::file_name)
        // Never a hidden file, even from a hidden folder.
        .map(|name| name.to_string_lossy().trim_start_matches('.').to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "animation".to_owned())
}

/// What ffmpeg reads for one picture, and what decides whether neighbours
/// can share a concat list.
#[derive(Debug, Clone)]
struct Source {
    /// Absolute: the concat demuxer resolves relative paths against the list.
    path: PathBuf,
    codec: Option<String>,
    dims: Option<(u32, u32)>,
    orientation: Option<u8>,
    /// A tiled HEIF picture's stream group; see [`MediaInfo::tile_grid`].
    tile_grid: Option<usize>,
    /// Already scaled to the frame by [`prescale_step`].
    prescaled: bool,
}

impl Source {
    fn of(info: &MediaInfo) -> Result<Self> {
        let unlistable = info
            .path
            .to_str()
            .is_none_or(|path| path.contains(['\n', '\r']));
        if unlistable {
            return Err(Error::unsupported(format!(
                "{}: names with line breaks or invalid characters can't go in an animation",
                info.file_name
            )));
        }
        Ok(Self {
            path: std::path::absolute(&info.path).unwrap_or_else(|_| info.path.clone()),
            codec: info.video_codec.clone(),
            dims: info.width.zip(info.height),
            orientation: info.orientation,
            tile_grid: info.tile_grid,
            prescaled: false,
        })
    }

    /// Whether `other` decodes like this picture and comes out the same size.
    fn joins(&self, other: &Self) -> bool {
        self.codec.is_some()
            && self.dims.is_some()
            && self.codec == other.codec
            && self.dims == other.dims
            && self.orientation == other.orientation
    }

    /// The path as a quoted concat-list token: inside single quotes nothing
    /// is special except the quote itself, written `'\''`.
    fn quoted(&self) -> String {
        format!("'{}'", self.path.to_string_lossy().replace('\'', r"'\''"))
    }
}

/// The ffmpeg call that fits picture `index` (read from `source`) to the
/// frame and writes it as a PNG, and what to read in its place.
fn prescale_step(
    source: &Source,
    fit: &str,
    (width, height): (u32, u32),
    output: &Path,
    index: usize,
    weight: f32,
) -> (Step, Source) {
    let picture = scratch_path(output, &format!("picture{}", index + 1), "png");
    let chain = format!("{fit},setsar=1");
    let mut args: Vec<OsString> = vec!["-i".into(), source.path.clone().into()];
    match source.tile_grid {
        Some(group) => args.extend(
            [
                "-filter_complex",
                &format!("[0:g:{group}]{chain}[v]"),
                "-map",
                "[v]",
            ]
            .map(OsString::from),
        ),
        None => args.extend(["-map", "0:v:0", "-vf", &chain].map(OsString::from)),
    }
    args.extend(
        [
            "-frames:v",
            "1",
            "-c:v",
            "png",
            "-compression_level",
            "1",
            "-pix_fmt",
            "rgb24",
            "-update",
            "1",
        ]
        .map(OsString::from),
    );
    args.push(picture.clone().into());
    let step = Step {
        tool: Tool::Ffmpeg,
        args,
        duration_secs: None,
        weight,
    };
    let scaled = Source {
        path: picture,
        codec: Some("png".into()),
        dims: Some((width, height)),
        orientation: None,
        tile_grid: None,
        prescaled: true,
    };
    (step, scaled)
}

/// `order` cut into runs of neighbours that can share a concat list.
fn runs(order: &[usize], sources: &[Source]) -> Vec<Vec<usize>> {
    let mut runs: Vec<Vec<usize>> = Vec::new();
    for &i in order {
        match runs.last_mut() {
            Some(run) if sources[run[0]].joins(&sources[i]) => run.push(i),
            _ => runs.push(vec![i]),
        }
    }
    runs
}

/// The frame time in hundredths of a second.
fn frame_centis(secs: f64) -> Result<u32> {
    let centis = (secs * 100.0).round();
    if !centis.is_finite()
        || centis < f64::from(*FRAME_CENTIS.start())
        || centis > f64::from(*FRAME_CENTIS.end())
    {
        return Err(Error::unsupported(
            "each picture has to show between 0.02 and 60 seconds",
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(centis as u32)
}

/// The frame: the first picture's displayed size, shrunk so its short side
/// is at most the chosen or default cap, with even sides for video encoders.
fn frame_size(first: &MediaInfo, options: &ConvertOptions) -> Result<(u32, u32)> {
    let (w, h) = first
        .width
        .zip(first.height)
        .filter(|&(w, h)| w > 0 && h > 0)
        .ok_or_else(|| {
            Error::unsupported(format!("couldn't read the size of {}", first.file_name))
        })?;
    let default_max = match options.format {
        OutputFormat::Gif => Some(GIF_MAX_SIZE),
        OutputFormat::Webp => Some(ANIMATED_WEBP_MAX_SIZE),
        OutputFormat::Jxl => Some(ANIMATED_JXL_MAX_SIZE),
        _ => None,
    };
    let target = options.max_size.or(default_max).map(|t| t.max(2) & !1);
    let (w, h) = match target {
        Some(t) if t < w.min(h) => {
            let scale = f64::from(t) / f64::from(w.min(h));
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let long = (f64::from(w.max(h)) * scale).round() as u32;
            if w >= h { (long, t) } else { (t, long) }
        }
        _ => (w, h),
    };
    let even = |n: u32| (n & !1).max(2);
    Ok((even(w), even(h)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JxlSupport;
    use crate::media::tests::sample;

    fn picture(name: &str, codec: &str, w: u32, h: u32) -> MediaInfo {
        MediaInfo {
            width: Some(w),
            height: Some(h),
            video_codec: Some(codec.into()),
            ..sample(&format!("/pics/{name}"), MediaKind::Image)
        }
    }

    fn cats(n: usize) -> Vec<MediaInfo> {
        (1..=n)
            .map(|i| picture(&format!("cat {i}.jpg"), "mjpeg", 4000, 3000))
            .collect()
    }

    fn gif() -> ConvertOptions {
        ConvertOptions {
            format: OutputFormat::Gif,
            ..ConvertOptions::default()
        }
    }

    fn plan_of(inputs: &[MediaInfo], options: &ConvertOptions, sequence: &SequenceOptions) -> Plan {
        plan_sequence(
            inputs,
            options,
            sequence,
            Path::new("/pics/cat.gif"),
            JxlSupport::default(),
        )
        .expect("plan")
    }

    fn error_of(
        inputs: &[MediaInfo],
        options: &ConvertOptions,
        sequence: &SequenceOptions,
    ) -> String {
        plan_sequence(
            inputs,
            options,
            sequence,
            Path::new("/pics/out.gif"),
            JxlSupport::default(),
        )
        .expect_err("must fail")
        .to_string()
    }

    #[test]
    fn direction_orders() {
        assert_eq!(Direction::Forward.order(4), [0, 1, 2, 3]);
        assert_eq!(Direction::Reverse.order(4), [3, 2, 1, 0]);
        assert_eq!(Direction::Boomerang.order(4), [0, 1, 2, 3, 2, 1]);
        assert_eq!(Direction::Boomerang.order(2), [0, 1]);
        assert_eq!(Direction::Boomerang.order(0), Vec::<usize>::new());
    }

    #[test]
    fn photos_from_one_camera_become_a_gif_in_one_pass() {
        let plan = plan_of(&cats(3), &gif(), &SequenceOptions::default());
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(
            plan.command_line(),
            "ffmpeg -reinit_filter 0 -f concat -safe 0 -i /pics/.cat.gif.list1.txt \
             -filter_complex '[0:v]scale=640:480:force_original_aspect_ratio=decrease:flags=lanczos,\
             pad=640:480:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,settb=1/100,setpts=N*50,fps=100/50,\
             split[a][b];[a]palettegen=max_colors=166:stats_mode=single[p];\
             [b][p]paletteuse=new=1:dither=bayer:bayer_scale=4[out]' \
             -map '[out]' -frames:v 3 -loop 0 -an -sn -dn -map_metadata -1 /pics/cat.gif"
        );
        assert_eq!(plan.duration_secs(), Some(1.5));
        assert_eq!(
            plan.writes,
            [(
                PathBuf::from("/pics/.cat.gif.list1.txt"),
                "ffconcat version 1.0\n\
                 file '/pics/cat 1.jpg'\nduration 0.5\n\
                 file '/pics/cat 2.jpg'\nduration 0.5\n\
                 file '/pics/cat 3.jpg'\nduration 0.5\n\
                 file '/pics/cat 3.jpg'\n"
                    .to_owned()
            )]
        );
        assert_eq!(plan.scratch, [PathBuf::from("/pics/.cat.gif.list1.txt")]);
    }

    #[test]
    fn quotes_in_names_are_escaped() {
        let mut inputs = cats(2);
        inputs[1] = picture("it's me.jpg", "mjpeg", 4000, 3000);
        let plan = plan_of(&inputs, &gif(), &SequenceOptions::default());
        assert!(
            plan.writes[0].1.contains(r"file '/pics/it'\''s me.jpg'"),
            "{}",
            plan.writes[0].1
        );
        let mut broken = cats(2);
        broken[0].path = PathBuf::from("/pics/two\nlines.jpg");
        assert!(error_of(&broken, &gif(), &SequenceOptions::default()).contains("line breaks"));
    }

    #[test]
    fn changes_of_codec_size_or_orientation_start_a_new_run() {
        let turned = MediaInfo {
            orientation: Some(6),
            ..picture("turned.jpg", "mjpeg", 3000, 4000)
        };
        let inputs = [
            picture("a.jpg", "mjpeg", 4000, 3000),
            picture("b.jpg", "mjpeg", 4000, 3000),
            picture("c.png", "png", 4000, 3000),
            turned.clone(),
            MediaInfo {
                orientation: None,
                ..turned
            },
        ];
        let sequence = SequenceOptions {
            frame_secs: 0.25,
            fit: Fit::Crop,
        };
        let boomerang = ConvertOptions {
            direction: Direction::Boomerang,
            ..gif()
        };
        let plan = plan_of(&inputs, &boomerang, &sequence);
        let lists: Vec<&str> = plan.writes.iter().map(|(_, l)| l.as_str()).collect();
        // Boomerang: a b c turned plain turned c b; only a and b are
        // neighbours that match.
        assert_eq!(lists.len(), 7, "{lists:#?}");
        assert!(lists[0].contains("a.jpg") && lists[0].contains("b.jpg"));
        assert!(lists[6].ends_with("file '/pics/b.jpg'\nduration 0.25\nfile '/pics/b.jpg'\n"));
        let cmd = plan.command_line();
        assert!(cmd.contains("-i /pics/.cat.gif.list7.txt"), "{cmd}");
        assert!(
            cmd.contains(
                "[6:v]scale=640:480:force_original_aspect_ratio=increase:flags=lanczos,crop=640:480,setsar=1[p6];\
                 [p0][p1][p2][p3][p4][p5][p6]concat=n=7:v=1:a=0,settb=1/100,setpts=N*25,fps=100/25,"
            ),
            "{cmd}"
        );
        assert!(cmd.contains("-frames:v 8 "), "{cmd}");
    }

    #[test]
    fn many_different_pictures_are_scaled_one_by_one_first() {
        let inputs: Vec<MediaInfo> = (0..10)
            .map(|i| picture(&format!("p{i}.png"), "png", 400 + i, 300))
            .collect();
        let options = ConvertOptions {
            format: OutputFormat::Mp4,
            ..ConvertOptions::default()
        };
        let plan = plan_sequence(
            &inputs,
            &options,
            &SequenceOptions::default(),
            Path::new("/pics/p.mp4"),
            JxlSupport::default(),
        )
        .expect("plan");
        assert_eq!(plan.steps.len(), 11);
        let first = plan.steps[0]
            .args
            .iter()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>();
        assert_eq!(
            first.join(" "),
            "-i /pics/p0.png -map 0:v:0 -vf scale=400:300:force_original_aspect_ratio=decrease:flags=lanczos,\
             pad=400:300:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1 -frames:v 1 -c:v png -compression_level 1 -pix_fmt rgb24 \
             -update 1 /pics/.p.mp4.picture1.png"
        );
        assert_eq!(plan.writes.len(), 1);
        assert!(
            plan.writes[0]
                .1
                .contains("file '/pics/.p.mp4.picture10.png'")
        );
        let last = plan.steps.last().expect("encode step");
        let cmd = last
            .args
            .iter()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        // A video slideshow repeats each picture to reach 24 fps.
        assert!(
            cmd.contains("-filter_complex [0:v]setsar=1,settb=1/100,setpts=N*50,fps=1200/50[out]"),
            "{cmd}"
        );
        assert!(cmd.contains("-frames:v 120 "), "{cmd}");
        assert!(
            cmd.contains("-c:v libx264 -preset fast -crf 24 -pix_fmt yuv420p -tune stillimage -movflags +faststart"),
            "{cmd}"
        );
        assert_eq!(plan.duration_secs(), Some(5.0));
        assert_eq!(plan.scratch.len(), 11);
    }

    #[test]
    fn av1_slideshow_skips_the_x264_tuning() {
        let inputs = [
            picture("a.jpg", "mjpeg", 1920, 1080),
            picture("b.jpg", "mjpeg", 1920, 1080),
        ];
        let options = ConvertOptions {
            format: OutputFormat::Mp4,
            video_codec: Some(VideoCodec::Av1),
            ..ConvertOptions::default()
        };
        let cmd = plan_sequence(
            &inputs,
            &options,
            &SequenceOptions::default(),
            Path::new("/pics/a.mp4"),
            JxlSupport::default(),
        )
        .expect("plan")
        .command_line();
        assert!(
            cmd.contains(
                "-c:v libsvtav1 -preset 8 -crf 32 -pix_fmt yuv420p10le -movflags +faststart"
            ),
            "{cmd}"
        );
        assert!(!cmd.contains("stillimage"), "{cmd}");
    }

    #[test]
    fn size_comes_from_the_first_picture_and_turns_apply_after() {
        let inputs = [
            picture("tall.jpg", "mjpeg", 1080, 1920),
            picture("wide.jpg", "mjpeg", 1920, 1080),
        ];
        let options = ConvertOptions {
            format: OutputFormat::Webp,
            rotate: 90,
            flip: true,
            quality: 100,
            ..ConvertOptions::default()
        };
        let plan = plan_sequence(
            &inputs,
            &options,
            &SequenceOptions::default(),
            Path::new("/pics/tall.webp"),
            JxlSupport::default(),
        )
        .expect("plan");
        let cmd = plan.command_line();
        // 1080×1920 capped to WebP's 480 short side: 480×853, evened to 480×852.
        assert!(cmd.contains("[0:v]scale=480:852:"), "{cmd}");
        assert!(
            cmd.contains("concat=n=2:v=1:a=0,transpose=clock,hflip,settb="),
            "{cmd}"
        );
        assert!(
            cmd.contains("-c:v libwebp_anim -quality 95 -loop 0 "),
            "{cmd}"
        );

        // An explicit size and no cap for video.
        let mp4 = ConvertOptions {
            format: OutputFormat::Mp4,
            max_size: Some(720),
            ..ConvertOptions::default()
        };
        let cmd = plan_sequence(
            &inputs,
            &mp4,
            &SequenceOptions::default(),
            Path::new("/pics/tall.mp4"),
            JxlSupport::default(),
        )
        .expect("plan")
        .command_line();
        assert!(cmd.contains("[0:v]scale=720:1280:"), "{cmd}");
    }

    #[test]
    fn animated_jxl_goes_through_apng_for_cjxl_or_straight_through_ffmpeg() {
        let options = ConvertOptions {
            format: OutputFormat::Jxl,
            ..ConvertOptions::default()
        };
        let cjxl = JxlSupport {
            cjxl: true,
            djxl: true,
            ..JxlSupport::default()
        };
        let mut inputs = cats(2);
        inputs.push(picture("drawn.jxl", "jpegxl", 4000, 3000));
        let plan = plan_sequence(
            &inputs,
            &options,
            &SequenceOptions::default(),
            Path::new("/pics/cat.jxl"),
            cjxl,
        )
        .expect("plan");
        let tools: Vec<Tool> = plan.steps.iter().map(|s| s.tool).collect();
        assert_eq!(tools, [Tool::Djxl, Tool::Ffmpeg, Tool::Cjxl]);
        let cmd = plan.command_line();
        assert!(
            cmd.starts_with("djxl /pics/drawn.jxl /pics/.cat.jxl.decoded3.png --quiet && "),
            "{cmd}"
        );
        assert!(cmd.contains("scale=960:720:"), "{cmd}");
        assert!(
            cmd.contains("-c:v apng -compression_level 1 -plays 0 -f apng -an -sn -dn -map_metadata -1 /pics/.cat.jxl.frames.apng"),
            "{cmd}"
        );
        assert!(
            cmd.ends_with("cjxl /pics/.cat.jxl.frames.apng /pics/cat.jxl -d 1.5 -e 7 --quiet"),
            "{cmd}"
        );
        // The decoded PNG is another codec, so it gets its own run.
        assert_eq!(plan.writes.len(), 2);
        assert!(plan.writes[1].1.contains(".cat.jxl.decoded3.png"));

        let ffmpeg = JxlSupport {
            ffmpeg_decoder: true,
            ffmpeg_anim_encoder: true,
            ..JxlSupport::default()
        };
        let cmd = plan_sequence(
            &inputs,
            &options,
            &SequenceOptions::default(),
            Path::new("/pics/cat.jxl"),
            ffmpeg,
        )
        .expect("plan")
        .command_line();
        assert!(cmd.starts_with("ffmpeg "), "{cmd}");
        assert!(
            cmd.contains("-c:v libjxl_anim -distance 1.5 -effort 7 -f data"),
            "{cmd}"
        );

        let none = plan_sequence(
            &inputs,
            &options,
            &SequenceOptions::default(),
            Path::new("/pics/cat.jxl"),
            JxlSupport::default(),
        )
        .expect_err("needs a JPEG XL tool");
        assert!(none.to_string().contains("djxl"), "{none}");
    }

    #[test]
    fn rejects_what_cannot_be_an_animation() {
        let ok = SequenceOptions::default();
        assert!(error_of(&cats(1), &gif(), &ok).contains("at least two"));
        let mut with_clip = cats(2);
        with_clip[1].kind = MediaKind::Video;
        assert!(error_of(&with_clip, &gif(), &ok).contains("not a still picture"));
        let png = ConvertOptions {
            format: OutputFormat::Png,
            ..ConvertOptions::default()
        };
        assert!(error_of(&cats(2), &png, &ok).contains("as PNG"));
        let fast = SequenceOptions {
            frame_secs: 0.001,
            ..ok
        };
        assert!(error_of(&cats(2), &gif(), &fast).contains("between 0.02 and 60"));
        let nan = SequenceOptions {
            frame_secs: f64::NAN,
            ..ok
        };
        assert!(error_of(&cats(2), &gif(), &nan).contains("between"));
        let mut unknown = cats(2);
        unknown[0].width = None;
        assert!(error_of(&unknown, &gif(), &ok).contains("size of"));
        let turned = ConvertOptions {
            rotate: 45,
            ..gif()
        };
        assert!(error_of(&cats(2), &turned, &ok).contains("rotation"));
        let overwrite = plan_sequence(
            &cats(2),
            &gif(),
            &ok,
            Path::new("/pics/cat 1.jpg"),
            JxlSupport::default(),
        );
        assert!(overwrite.is_err());
    }

    #[test]
    fn output_is_named_after_the_shared_name_or_folder() {
        let name = |names: &[&str]| {
            let inputs: Vec<MediaInfo> =
                names.iter().map(|n| picture(n, "mjpeg", 10, 10)).collect();
            sequence_output_path(&inputs, OutputFormat::Gif, |_| false)
        };
        assert_eq!(
            name(&["cat 1.jpg", "cat 2.jpg"]),
            Path::new("/pics/cat.gif")
        );
        assert_eq!(
            name(&["IMG_0012.jpg", "IMG_0013.jpg"]),
            Path::new("/pics/IMG.gif")
        );
        assert_eq!(name(&["dog.jpg", "owl.png"]), Path::new("/pics/pics.gif"));
        let mut hidden = cats(2);
        hidden[0].path = PathBuf::from("/.stash/dog.jpg");
        assert_eq!(
            sequence_output_path(&hidden, OutputFormat::Gif, |_| false),
            Path::new("/.stash/stash.gif")
        );
        let taken = sequence_output_path(&cats(2), OutputFormat::Gif, |p| {
            p == Path::new("/pics/cat.gif")
        });
        assert_eq!(taken, Path::new("/pics/cat 2.gif"));
    }

    #[test]
    fn tiled_heic_photos_are_stitched_into_a_png_first() {
        let tiled = MediaInfo {
            tile_grid: Some(0),
            ..picture("IMG_1.heic", "hevc", 4032, 3024)
        };
        let inputs = [
            tiled.clone(),
            MediaInfo {
                path: PathBuf::from("/pics/IMG_2.heic"),
                ..tiled
            },
        ];
        let plan = plan_of(&inputs, &gif(), &SequenceOptions::default());
        assert_eq!(plan.steps.len(), 3);
        let first = plan.steps[0]
            .args
            .iter()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            first.starts_with(
                "-i /pics/IMG_1.heic -filter_complex [0:g:0]scale=640:480:force_original_aspect_ratio=decrease"
            ),
            "{first}"
        );
        assert!(first.ends_with("setsar=1[v] -map [v] -frames:v 1 -c:v png -compression_level 1 -pix_fmt rgb24 -update 1 /pics/.cat.gif.picture1.png"), "{first}");
        // The stitched PNGs read as one run, already fitted.
        assert_eq!(plan.writes.len(), 1);
        assert!(
            plan.writes[0]
                .1
                .contains("file '/pics/.cat.gif.picture2.png'")
        );
        let cmd = plan.command_line();
        assert!(
            cmd.contains("-filter_complex '[0:v]scale=640:480:"),
            "{cmd}"
        );
    }

    #[test]
    fn options_deserialize_with_defaults() {
        let parsed: SequenceOptions =
            serde_json::from_str(r#"{"frameSecs": 0.1, "fit": "crop"}"#).expect("parse");
        assert_eq!(
            parsed,
            SequenceOptions {
                frame_secs: 0.1,
                fit: Fit::Crop,
            }
        );
        assert!(OutputFormat::Mp4.animates_pictures());
        assert!(OutputFormat::Webp.animates_pictures());
        assert!(!OutputFormat::Png.animates_pictures());
        assert!(!OutputFormat::Mp3.animates_pictures());
    }
}
