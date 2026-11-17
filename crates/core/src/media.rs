use std::io::Read;
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
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "heic" | "heif" | "hif" | "tif"
            | "tiff" | "bmp" | "jxl" | "apng" => Self::Image,
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
    /// Sample rate of the primary audio stream in Hz, e.g. `48000`.
    #[serde(default)]
    pub sample_rate: Option<u32>,
    /// JPEG XL only: the file is a losslessly recompressed JPEG, so the
    /// original JPEG can be restored bit for bit.
    #[serde(default)]
    pub reconstructs_jpeg: bool,
    /// The file records where it was made: GPS coordinates in a photo's EXIF
    /// or XMP, or a location tag in a video or song.
    #[serde(default)]
    pub has_location: bool,
    /// Container tags that hold a location, e.g. `location`, to clear when
    /// keeping everything else.
    #[serde(default)]
    pub location_tags: Vec<String>,
    /// HEIF/AVIF only: index of the stream group (a tile grid) holding the
    /// picture. Large HEIC photos are stored as tiles that ffprobe lists as
    /// separate streams; ffmpeg stitches them back together from this group.
    #[serde(default)]
    pub tile_grid: Option<usize>,
}

impl MediaInfo {
    /// Whether the file has an audio stream (only known after an ffprobe probe).
    #[must_use]
    pub fn has_audio(&self) -> bool {
        self.audio_codec.is_some()
    }
}

/// Whether a file is HEIF (HEIC, AVIF, …), judging by the brands in its
/// `ftyp` box. MP4 and QuickTime files share the box but not these brands.
#[must_use]
pub fn is_heif_file(path: &Path) -> bool {
    const BRANDS: [&[u8; 4]; 10] = [
        b"mif1", b"msf1", b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx", b"avif", b"avis",
    ];
    let mut head = Vec::with_capacity(64);
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    if file.take(64).read_to_end(&mut head).is_err() || head.len() < 12 || &head[4..8] != b"ftyp" {
        return false;
    }
    // Box size, type, major brand, minor version, then compatible brands.
    let size = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    let (compatible, _) = head
        .get(16..size.min(head.len()))
        .unwrap_or_default()
        .as_chunks::<4>();
    let major = [head[8], head[9], head[10], head[11]];
    std::iter::once(&major)
        .chain(compatible)
        .any(|brand| BRANDS.contains(&brand))
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
        sample_rate: None,
        reconstructs_jpeg: false,
        has_location: false,
        location_tags: Vec::new(),
        tile_grid: None,
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
            sample_rate: None,
            reconstructs_jpeg: false,
            has_location: false,
            location_tags: Vec::new(),
            tile_grid: None,
        }
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
    fn recognises_heif_by_its_brands() {
        // iPhone: major brand heic, compatible mif1, heic.
        let heic = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic";
        let (_dir, path) = temp_file("photo.jpg", heic);
        assert!(is_heif_file(&path), "brand, not extension, decides");
        // AVIF lists its brand among the compatible ones.
        let avif = b"\0\0\0\x1cftypmif1\0\0\0\0mif1avifmiaf";
        let (_dir, path) = temp_file("a.avif", avif);
        assert!(is_heif_file(&path));
        // MP4 and QuickTime use the same box with other brands.
        let mp4 = b"\0\0\0\x18ftypisom\0\0\x02\0isomiso2";
        let (_dir, path) = temp_file("clip.mp4", mp4);
        assert!(!is_heif_file(&path));
        let (_dir, path) = temp_file("tiny.heic", b"ftyp");
        assert!(!is_heif_file(&path));
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
