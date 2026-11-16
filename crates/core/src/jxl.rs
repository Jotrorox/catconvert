//! JPEG XL helpers: the libjxl command-line tools (`cjxl`, `djxl`, `jxlinfo`)
//! and a little header sniffing that needs no tools at all.
//!
//! ffmpeg can read and write JPEG XL only when built with libjxl, which many
//! builds (Homebrew's default among them) are not. The libjxl tools are the
//! reference implementation and do more than ffmpeg exposes anyway: lossless
//! JPEG recompression, reconstructing the original JPEG, progressive files and
//! metadata passthrough. So CatConvert uses them when installed and falls back
//! to ffmpeg's libjxl otherwise.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::{MediaInfo, MediaKind};

/// Environment variable naming a directory that holds `cjxl`, `djxl` and
/// `jxlinfo`, searched before `PATH`.
pub const LIBJXL_ENV: &str = "CATCONVERT_LIBJXL";

/// The libjxl tools that were found. Any of them may be missing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JxlTools {
    pub cjxl: Option<PathBuf>,
    pub djxl: Option<PathBuf>,
    pub jxlinfo: Option<PathBuf>,
    /// libjxl version reported by `cjxl --version`, e.g. `"0.13.0"`.
    pub version: Option<String>,
}

impl JxlTools {
    /// Look in [`LIBJXL_ENV`], then `dirs` in order. Each tool is taken from
    /// the first directory that has it.
    pub(crate) fn locate(dirs: impl IntoIterator<Item = PathBuf>) -> Self {
        let dirs: Vec<PathBuf> = std::env::var_os(LIBJXL_ENV)
            .map(PathBuf::from)
            .into_iter()
            .chain(dirs)
            .collect();
        let find = |name: &str| {
            dirs.iter()
                .find_map(|dir| crate::ffmpeg::executable(dir, name))
        };
        let cjxl = find("cjxl");
        let version = cjxl.as_deref().and_then(|cjxl| {
            let out = crate::ffmpeg::base_command(cjxl)
                .arg("--version")
                .output()
                .ok()?;
            parse_version(&String::from_utf8_lossy(&out.stdout))
        });
        Self {
            cjxl,
            djxl: find("djxl"),
            jxlinfo: find("jxlinfo"),
            version,
        }
    }
}

/// `cjxl v0.13.0 0.13.0 [NEON] {...}` → `0.13.0`.
fn parse_version(output: &str) -> Option<String> {
    let first = output.lines().next()?;
    let token = first.split_whitespace().nth(1)?;
    let version = token.strip_prefix('v').unwrap_or(token);
    version
        .starts_with(|c: char| c.is_ascii_digit())
        .then(|| version.to_owned())
}

/// Signature of a bare JPEG XL codestream.
const CODESTREAM_MAGIC: [u8; 2] = [0xFF, 0x0A];
/// Signature box that starts a JPEG XL container file.
const CONTAINER_MAGIC: [u8; 12] = [
    0, 0, 0, 0x0C, b'J', b'X', b'L', b' ', 0x0D, 0x0A, 0x87, 0x0A,
];

/// Whether the file starts with a JPEG XL signature.
#[must_use]
pub fn is_jxl_file(path: &Path) -> bool {
    let mut head = [0u8; 12];
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let n = read_up_to(&mut file, &mut head);
    head[..n.min(2)] == CODESTREAM_MAGIC || (n == 12 && head == CONTAINER_MAGIC)
}

/// Whether a probed file is JPEG XL, judging by its extension or codec.
pub(crate) fn is_jxl(info: &MediaInfo) -> bool {
    info.extension.as_deref() == Some("jxl")
        || info
            .video_codec
            .as_deref()
            .is_some_and(|codec| codec.starts_with("jpegxl"))
}

/// Whether a JPEG XL file carries JPEG reconstruction data (a `jbrd` box), i.e.
/// it is a losslessly recompressed JPEG that `djxl` can turn back into the
/// original file bit for bit.
#[must_use]
pub fn has_jpeg_reconstruction(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 12];
    if read_up_to(&mut file, &mut magic) != 12 || magic != CONTAINER_MAGIC {
        // A bare codestream has no boxes, so no reconstruction data.
        return false;
    }
    // Walk the ISO BMFF-style boxes. Real files have a handful; the cap only
    // guards against garbage.
    for _ in 0..256 {
        let mut header = [0u8; 8];
        if read_up_to(&mut file, &mut header) != 8 {
            return false;
        }
        let size = u64::from(u32::from_be_bytes([
            header[0], header[1], header[2], header[3],
        ]));
        let kind = &header[4..8];
        if kind == b"jbrd" {
            return true;
        }
        let skip = match size {
            // Box runs to the end of the file.
            0 => return false,
            1 => {
                let mut large = [0u8; 8];
                if read_up_to(&mut file, &mut large) != 8 {
                    return false;
                }
                u64::from_be_bytes(large).checked_sub(16)
            }
            n => n.checked_sub(8),
        };
        let Some(skip) = skip.and_then(|s| i64::try_from(s).ok()) else {
            return false;
        };
        if file.seek(SeekFrom::Current(skip)).is_err() {
            return false;
        }
    }
    false
}

fn read_up_to(file: &mut File, buf: &mut [u8]) -> usize {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => filled += n,
        }
    }
    filled
}

/// What `jxlinfo` says about a file.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct JxlInfo {
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Frames that are actually shown (zero-duration frames are layers).
    pub shown_frames: usize,
    pub animation_secs: Option<f64>,
}

impl JxlInfo {
    pub(crate) fn is_animated(&self) -> bool {
        self.shown_frames > 1 && self.animation_secs.is_some_and(|s| s > 0.0)
    }
}

/// Parse `jxlinfo` output:
///
/// ```text
/// JPEG XL animation, 320x240, lossy, 8-bit RGB
/// Frame: Full image size, duration: 100.0 ms
/// Animation length: 2.000 seconds (looping)
/// ```
pub(crate) fn parse_jxlinfo(output: &str) -> Option<JxlInfo> {
    let mut info = JxlInfo {
        width: None,
        height: None,
        shown_frames: 0,
        animation_secs: None,
    };
    let mut recognised = false;
    for line in output.lines().map(str::trim) {
        if let Some(rest) = line
            .strip_prefix("JPEG XL image, ")
            .or_else(|| line.strip_prefix("JPEG XL animation, "))
        {
            recognised = true;
            let size = rest.split(',').next().unwrap_or_default().trim();
            if let Some((w, h)) = size.split_once('x') {
                info.width = w.parse().ok();
                info.height = h.parse().ok();
            }
        } else if line.starts_with("Frame:") {
            let ms = line
                .rsplit_once("duration: ")
                .and_then(|(_, d)| d.trim_end_matches("ms").trim().parse::<f64>().ok());
            if ms.is_some_and(|ms| ms > 0.0) {
                info.shown_frames += 1;
            }
        } else if let Some(rest) = line.strip_prefix("Animation length: ") {
            info.animation_secs = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .filter(|s: &f64| s.is_finite());
        }
    }
    recognised.then_some(info)
}

/// Fill in what ffprobe cannot know about JPEG XL without a libjxl decoder:
/// whether it is animated, how long, and at what frame rate.
pub(crate) fn apply_jxlinfo(info: &mut MediaInfo, jxl: &JxlInfo) {
    if info.width.is_none() || info.height.is_none() {
        info.width = jxl.width;
        info.height = jxl.height;
    }
    if jxl.is_animated() {
        let secs = jxl.animation_secs.unwrap_or_default();
        info.kind = MediaKind::Video;
        info.duration_secs = Some(secs);
        #[allow(clippy::cast_precision_loss)]
        let fps = jxl.shown_frames as f64 / secs;
        info.fps = Some(fps);
        info.video_codec = Some("jpegxl_anim".into());
    } else {
        info.kind = MediaKind::Image;
        info.duration_secs = None;
        info.fps = None;
        if info.video_codec.is_none() {
            info.video_codec = Some("jpegxl".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        File::create(&path)
            .and_then(|mut f| f.write_all(bytes))
            .expect("write");
        path
    }

    fn jxl_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = u32::try_from(8 + payload.len()).expect("small box");
        let mut out = size.to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn sniffs_signatures_and_reconstruction_boxes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let bare = write(dir.path(), "bare.jxl", &[0xFF, 0x0A, 1, 2, 3]);
        assert!(is_jxl_file(&bare));
        assert!(!has_jpeg_reconstruction(&bare));

        let mut container = CONTAINER_MAGIC.to_vec();
        container.extend(jxl_box(b"ftyp", b"jxl \0\0\0\0jxl "));
        container.extend(jxl_box(b"jbrd", &[0; 20]));
        container.extend(jxl_box(b"jxlc", &[0xFF, 0x0A]));
        let recompressed = write(dir.path(), "photo.jxl", &container);
        assert!(is_jxl_file(&recompressed));
        assert!(has_jpeg_reconstruction(&recompressed));

        let mut plain = CONTAINER_MAGIC.to_vec();
        plain.extend(jxl_box(b"ftyp", b"jxl \0\0\0\0jxl "));
        plain.extend(jxl_box(b"Exif", &[0; 40]));
        plain.extend(jxl_box(b"jxlc", &[0xFF, 0x0A]));
        let not = write(dir.path(), "plain.jxl", &plain);
        assert!(!has_jpeg_reconstruction(&not));

        let png = write(dir.path(), "x.png", b"\x89PNG\r\n\x1a\n");
        assert!(!is_jxl_file(&png));
        assert!(!has_jpeg_reconstruction(&png));
        let truncated = write(dir.path(), "t.jxl", &CONTAINER_MAGIC[..6]);
        assert!(!is_jxl_file(&truncated));
    }

    #[test]
    fn parses_jxlinfo_for_animation_and_still() {
        let anim = parse_jxlinfo(
            "JPEG XL animation, 320x240, (possibly) lossless, 8-bit RGB+Alpha\n\
             Color space: RGB\n\
             Frame: Full image size, duration: 100.0 ms\n\
             Frame: 320x200 at position (0,0), duration: 0.0 ms\n\
             Frame: 320x193 at position (0,6), duration: 100.0 ms\n\
             Frame: 320x193 at position (0,6), duration: 100.0 ms\n\
             Animation length: 0.300 seconds (looping)\n",
        )
        .expect("parse");
        assert_eq!((anim.width, anim.height), (Some(320), Some(240)));
        assert_eq!(anim.shown_frames, 3);
        assert_eq!(anim.animation_secs, Some(0.3));
        assert!(anim.is_animated());

        let still = parse_jxlinfo(
            "JPEG XL file format container (ISO/IEC 18181-2)\n\
             JPEG XL image, 4032x3024, (possibly) lossless, 8-bit RGB\n\
             JPEG bitstream reconstruction data available\n",
        )
        .expect("parse");
        assert_eq!(still.width, Some(4032));
        assert!(!still.is_animated());
        assert_eq!(parse_jxlinfo("Error: not a JPEG XL file"), None);
    }

    #[test]
    fn jxlinfo_turns_animations_into_clips() {
        let mut info = crate::media::tests::sample("loop.jxl", MediaKind::Image);
        apply_jxlinfo(
            &mut info,
            &JxlInfo {
                width: Some(320),
                height: Some(240),
                shown_frames: 20,
                animation_secs: Some(2.0),
            },
        );
        assert_eq!(info.kind, MediaKind::Video);
        assert_eq!(info.duration_secs, Some(2.0));
        assert_eq!(info.fps, Some(10.0));
        assert_eq!(info.width, Some(320));
        assert_eq!(info.video_codec.as_deref(), Some("jpegxl_anim"));
        assert!(is_jxl(&info));
    }

    #[test]
    fn parses_cjxl_version() {
        assert_eq!(
            parse_version("cjxl v0.13.0 0.13.0 [NEON] {AppleClang}\nCopyright").as_deref(),
            Some("0.13.0")
        );
        assert_eq!(parse_version("weird"), None);
    }
}
