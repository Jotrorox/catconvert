use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Broad media category: guessed from the extension by [`probe`], refined from
/// the actual streams by [`crate::Ffmpeg::probe`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Unknown,
}

impl MediaKind {
    /// Guess the kind from a file extension (case-insensitive, without the dot).
    #[must_use]
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "heic" | "heif" | "tif" | "tiff"
            | "bmp" | "jxl" | "apng" => Self::Image,
            "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "wmv" | "mpg" | "mpeg" => Self::Video,
            "mp3" | "m4a" | "aac" | "flac" | "wav" | "ogg" | "opus" | "aiff" | "wma" => Self::Audio,
            _ => Self::Unknown,
        }
    }
}

/// Information about a media file.
///
/// [`probe`] fills only the filesystem fields; [`crate::Ffmpeg::probe`] also
/// fills the stream fields (duration, dimensions, codecs) from ffprobe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    /// Path as given by the caller.
    pub path: PathBuf,
    /// Final path component, lossily converted to UTF-8.
    pub file_name: String,
    /// Lower-cased extension without the leading dot, if any.
    pub extension: Option<String>,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Media category.
    pub kind: MediaKind,
    /// Playback length in seconds; `None` for still images or when unknown.
    #[serde(default)]
    pub duration_secs: Option<f64>,
    /// Display width in pixels, with rotation metadata already applied.
    #[serde(default)]
    pub width: Option<u32>,
    /// Display height in pixels, with rotation metadata already applied.
    #[serde(default)]
    pub height: Option<u32>,
    /// Average frame rate of the video stream.
    #[serde(default)]
    pub fps: Option<f64>,
    /// Codec name of the primary video stream, e.g. `"h264"`.
    #[serde(default)]
    pub video_codec: Option<String>,
    /// Codec name of the primary audio stream, e.g. `"aac"`.
    #[serde(default)]
    pub audio_codec: Option<String>,
    /// JPEG XL only: the file is a losslessly recompressed JPEG, so the
    /// original JPEG can be restored bit for bit.
    #[serde(default)]
    pub reconstructs_jpeg: bool,
    /// How the picture is turned or mirrored for display, as an EXIF
    /// orientation from 2 to 8 (`None` when it is shown as stored). ffmpeg
    /// applies it while decoding; `width` and `height` already account for it.
    #[serde(default)]
    pub orientation: Option<u8>,
}

impl MediaInfo {
    /// Whether the file has an audio stream (only known after an ffprobe probe).
    #[must_use]
    pub fn has_audio(&self) -> bool {
        self.audio_codec.is_some()
    }
}

/// Inspect a file and return its [`MediaInfo`].
///
/// Reads only filesystem metadata (size) and the extension; it does not open
/// or decode the file. Use [`crate::Ffmpeg::probe`] for stream details.
///
/// # Errors
///
/// Returns [`Error::Io`] if the metadata cannot be read and
/// [`Error::NotAFile`] if the path is a directory or other non-file.
pub fn probe(path: impl AsRef<Path>) -> Result<MediaInfo> {
    let path = path.as_ref();
    let metadata = std::fs::metadata(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(Error::NotAFile {
            path: path.to_path_buf(),
        });
    }

    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase());
    let kind = extension
        .as_deref()
        .map_or(MediaKind::Unknown, MediaKind::from_extension);
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();

    Ok(MediaInfo {
        path: path.to_path_buf(),
        file_name,
        extension,
        size_bytes: metadata.len(),
        kind,
        duration_secs: None,
        width: None,
        height: None,
        fps: None,
        video_codec: None,
        audio_codec: None,
        reconstructs_jpeg: false,
        orientation: None,
    })
}

/// The EXIF orientation of a JPEG (2 to 8), or `None` when it has none, it
/// is the default (1), or the file is not a JPEG.
///
/// ffprobe doesn't report it, but ffmpeg turns the picture accordingly when
/// decoding, so the displayed size swaps for orientations 5 to 8.
#[must_use]
pub fn jpeg_orientation(path: &Path) -> Option<u8> {
    use std::io::Read;
    // EXIF lives in an APP1 segment near the start; 256 KiB covers files with
    // big embedded thumbnails or colour profiles before it.
    let mut head = Vec::with_capacity(64 * 1024);
    std::fs::File::open(path)
        .ok()?
        .take(256 * 1024)
        .read_to_end(&mut head)
        .ok()?;
    parse_jpeg_orientation(&head)
}

fn parse_jpeg_orientation(data: &[u8]) -> Option<u8> {
    if !data.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut at = 2;
    while at + 4 <= data.len() {
        if data[at] != 0xFF {
            return None;
        }
        let marker = data[at + 1];
        // Start of scan or end of image: no EXIF before the picture data.
        if matches!(marker, 0xDA | 0xD9) {
            return None;
        }
        let len = usize::from(u16::from_be_bytes([data[at + 2], data[at + 3]]));
        let body = data.get(at + 4..at + 2 + len)?;
        if marker == 0xE1
            && let Some(tiff) = body.strip_prefix(b"Exif\0\0")
        {
            return tiff_orientation(tiff);
        }
        at += 2 + len;
    }
    None
}

/// Orientation tag (0x0112) of the first IFD of a TIFF/EXIF block.
fn tiff_orientation(tiff: &[u8]) -> Option<u8> {
    let big = match tiff.get(..4)? {
        b"MM\0\x2a" => true,
        b"II\x2a\0" => false,
        _ => return None,
    };
    let u16_at = |at: usize| {
        let b = tiff.get(at..at + 2)?;
        Some(if big {
            u16::from_be_bytes([b[0], b[1]])
        } else {
            u16::from_le_bytes([b[0], b[1]])
        })
    };
    let u32_at = |at: usize| {
        let b = tiff.get(at..at + 4)?;
        let b = [b[0], b[1], b[2], b[3]];
        Some(if big {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        })
    };
    let ifd = usize::try_from(u32_at(4)?).ok()?;
    let count = usize::from(u16_at(ifd)?);
    (0..count).find_map(|i| {
        let entry = ifd + 2 + i * 12;
        // Tag 0x0112, type SHORT (3): the value sits in the entry itself.
        (u16_at(entry)? == 0x0112 && u16_at(entry + 2)? == 3)
            .then(|| u16_at(entry + 8))
            .flatten()
            .and_then(|value| u8::try_from(value).ok())
            .filter(|value| (2..=8).contains(value))
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;

    use super::*;

    /// A `MediaInfo` as the extension-only probe would make it, for unit tests.
    pub(crate) fn sample(name: &str, kind: MediaKind) -> MediaInfo {
        let path = PathBuf::from(name);
        MediaInfo {
            extension: path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase()),
            file_name: name.into(),
            path,
            size_bytes: 1,
            kind,
            duration_secs: None,
            width: None,
            height: None,
            fps: None,
            video_codec: None,
            audio_codec: None,
            reconstructs_jpeg: false,
            orientation: None,
        }
    }

    /// A minimal JPEG start with an EXIF orientation tag, in either byte order.
    pub(crate) fn jpeg_with_orientation(orientation: u16, big_endian: bool) -> Vec<u8> {
        let u16b = |v: u16| {
            if big_endian {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let u32b = |v: u32| {
            if big_endian {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let mut tiff = if big_endian {
            b"MM\0\x2a".to_vec()
        } else {
            b"II\x2a\0".to_vec()
        };
        tiff.extend(u32b(8));
        tiff.extend(u16b(2));
        // An unrelated tag first (ImageDescription), then Orientation.
        for (tag, kind, value) in [(0x010E_u16, 2_u16, 0_u16), (0x0112, 3, orientation)] {
            tiff.extend(u16b(tag));
            tiff.extend(u16b(kind));
            tiff.extend(u32b(1));
            tiff.extend(u16b(value));
            tiff.extend([0, 0]);
        }
        tiff.extend(u32b(0));
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(tiff);
        let mut jpeg = vec![0xFF, 0xD8];
        // An APP0 (JFIF) segment before the EXIF one, as cameras write it.
        jpeg.extend([0xFF, 0xE0, 0x00, 0x06, b'J', b'F', b'I', b'F']);
        jpeg.extend([0xFF, 0xE1]);
        jpeg.extend(u16::try_from(app1.len() + 2).expect("small").to_be_bytes());
        jpeg.extend(app1);
        jpeg.extend([0xFF, 0xDA, 0x00, 0x02]);
        jpeg
    }

    #[test]
    fn reads_jpeg_orientation() {
        assert_eq!(
            parse_jpeg_orientation(&jpeg_with_orientation(6, true)),
            Some(6)
        );
        assert_eq!(
            parse_jpeg_orientation(&jpeg_with_orientation(8, false)),
            Some(8)
        );
        // The default orientation and nonsense values count as none.
        assert_eq!(
            parse_jpeg_orientation(&jpeg_with_orientation(1, true)),
            None
        );
        assert_eq!(
            parse_jpeg_orientation(&jpeg_with_orientation(9, true)),
            None
        );
        // Not a JPEG, truncated, or no EXIF before the picture data.
        assert_eq!(parse_jpeg_orientation(b"\x89PNG\r\n"), None);
        let full = jpeg_with_orientation(6, true);
        assert_eq!(parse_jpeg_orientation(&full[..20]), None);
        assert_eq!(
            parse_jpeg_orientation(&[0xFF, 0xD8, 0xFF, 0xDA, 0, 2]),
            None
        );

        let (_dir, path) = temp_file("turned.jpg", &full);
        assert_eq!(jpeg_orientation(&path), Some(6));
    }

    fn temp_file(name: &str, contents: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join(name);
        let mut file = std::fs::File::create(&path).expect("create temp file");
        file.write_all(contents).expect("write temp file");
        (dir, path)
    }

    #[test]
    fn probe_reads_size_and_extension() {
        let (_dir, path) = temp_file("Holiday.JPG", &[0u8; 1234]);
        let info = probe(&path).expect("probe succeeds");

        assert_eq!(info.path, path);
        assert_eq!(info.file_name, "Holiday.JPG");
        assert_eq!(info.extension.as_deref(), Some("jpg"));
        assert_eq!(info.size_bytes, 1234);
        assert_eq!(info.kind, MediaKind::Image);
    }

    #[test]
    fn probe_handles_missing_extension() {
        let (_dir, path) = temp_file("README", b"hello");
        let info = probe(&path).expect("probe succeeds");

        assert_eq!(info.extension, None);
        assert_eq!(info.kind, MediaKind::Unknown);
        assert_eq!(info.size_bytes, 5);
    }

    #[test]
    fn probe_missing_file_is_io_error() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let err = probe(dir.path().join("nope.mp4")).expect_err("must fail");
        assert!(matches!(err, Error::Io { .. }), "got {err:?}");
    }

    #[test]
    fn probe_directory_is_rejected() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let err = probe(dir.path()).expect_err("must fail");
        assert!(matches!(err, Error::NotAFile { .. }), "got {err:?}");
    }

    #[test]
    fn media_kind_from_extension() {
        assert_eq!(MediaKind::from_extension("MOV"), MediaKind::Video);
        assert_eq!(MediaKind::from_extension("flac"), MediaKind::Audio);
        assert_eq!(MediaKind::from_extension("webp"), MediaKind::Image);
        assert_eq!(MediaKind::from_extension("JXL"), MediaKind::Image);
        assert_eq!(MediaKind::from_extension("apng"), MediaKind::Image);
        assert_eq!(MediaKind::from_extension("txt"), MediaKind::Unknown);
    }

    #[test]
    fn media_info_serializes_camel_case() {
        let (_dir, path) = temp_file("song.mp3", b"abc");
        let json = serde_json::to_value(probe(&path).expect("probe succeeds")).expect("serialize");

        assert_eq!(json["fileName"], "song.mp3");
        assert_eq!(json["sizeBytes"], 3);
        assert_eq!(json["kind"], "audio");
    }
}
