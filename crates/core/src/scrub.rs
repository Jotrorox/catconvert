//! Removing where a photo was taken, or all of its tags, without touching the
//! picture.
//!
//! Photos keep GPS coordinates in EXIF, and sometimes in XMP. ffmpeg's
//! `-map_metadata -1` doesn't reach them: ffmpeg carries EXIF along as frame
//! side data and its PNG and libjxl encoders write it back out, and libjxl's
//! `cjxl`/`djxl` copy metadata as is. So CatConvert cleans the JPEG or PNG
//! files going into or coming out of those tools itself:
//!
//! - [`Metadata::NoLocation`]: the GPS block inside EXIF is zeroed and unlinked
//!   in place, so nothing else in the file moves. Maker-note offsets stay
//!   valid, and a JPEG that cjxl repacks still restores bit for bit, just
//!   without the location. XMP packets that mention GPS are dropped whole.
//! - [`Metadata::None`]: EXIF, XMP, IPTC, comments and text chunks go.
//!   Colour profiles stay, since they are part of how the picture looks.
//!
//! [`has_location`] looks in the same places to tell the user that a file
//! knows where it was taken, with `kamadak-exif` reading HEIF, WebP and TIFF.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use crate::{Error, Metadata, Result};

/// Larger files are not read whole just to look for a location.
const SNIFF_LIMIT: u64 = 256 * 1024 * 1024;
/// Cap on decompressed metadata (PNG text chunks, JPEG XL `brob` boxes).
const INFLATE_LIMIT: usize = 16 * 1024 * 1024;

const JPEG_MAGIC: [u8; 2] = [0xFF, 0xD8];
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// The flag value [`crate::Tool::Scrub`] steps carry for `metadata`, or `None`
/// for [`Metadata::Keep`], which needs no step.
pub(crate) fn drop_flag(metadata: Metadata) -> Option<&'static str> {
    match metadata {
        Metadata::Keep => None,
        Metadata::NoLocation => Some("location"),
        Metadata::None => Some("all"),
    }
}

/// Arguments of a [`crate::Tool::Scrub`] step: `<input> <output> --drop <what>`.
pub(crate) fn step_args(input: &Path, output: &Path, metadata: Metadata) -> Vec<OsString> {
    let flag = drop_flag(metadata).unwrap_or("nothing");
    vec![input.into(), output.into(), "--drop".into(), flag.into()]
}

/// Run a step made by [`step_args`].
pub(crate) fn run_step(args: &[OsString]) -> Result<()> {
    let [input, output, flag, what] = args else {
        return Err(Error::unsupported("malformed metadata cleaning step"));
    };
    let metadata = match what.to_str() {
        Some("location") if flag == "--drop" => Metadata::NoLocation,
        Some("all") if flag == "--drop" => Metadata::None,
        _ => return Err(Error::unsupported("malformed metadata cleaning step")),
    };
    scrub_file(Path::new(input), Path::new(output), metadata)
}

/// Copy the JPEG or PNG `input` to `output` without the metadata `metadata`
/// rules out. The picture data is copied untouched.
///
/// `output` must not exist yet.
///
/// # Errors
///
/// [`Error::Io`] if a file can't be read or written, [`Error::Unsupported`]
/// if `input` is neither JPEG nor PNG.
pub fn scrub_file(input: &Path, output: &Path, metadata: Metadata) -> Result<()> {
    let data = std::fs::read(input).map_err(io_error(input))?;
    let cleaned = scrub(&data, metadata).ok_or_else(|| {
        Error::unsupported(format!(
            "{} is not a JPEG or PNG, so its metadata can't be cleaned",
            input.display()
        ))
    })?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(io_error(output))?;
    file.write_all(&cleaned).map_err(io_error(output))
}

/// `data` without the metadata `metadata` rules out, or `None` if it is
/// neither JPEG nor PNG.
fn scrub(data: &[u8], metadata: Metadata) -> Option<Vec<u8>> {
    if metadata == Metadata::Keep && (data.starts_with(&JPEG_MAGIC) || data.starts_with(&PNG_MAGIC))
    {
        return Some(data.to_vec());
    }
    if data.starts_with(&JPEG_MAGIC) {
        Some(scrub_jpeg(data, metadata))
    } else if data.starts_with(&PNG_MAGIC) {
        Some(scrub_png(data, metadata))
    } else {
        None
    }
}

/// Whether the picture at `path` records where it was taken: GPS coordinates
/// in its EXIF or XMP. Unreadable files and other formats count as no.
pub(crate) fn has_location(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut head = [0u8; 12];
    let n = read_up_to(&mut file, &mut head);
    let head = &head[..n];
    let walked = head.starts_with(&JPEG_MAGIC)
        || head.starts_with(&PNG_MAGIC)
        || head == crate::jxl::CONTAINER_MAGIC;
    if !walked {
        // HEIF, WebP, TIFF and whatever else kamadak-exif can open.
        return exif_crate_finds_gps(path);
    }
    if !file.metadata().is_ok_and(|m| m.len() <= SNIFF_LIMIT) {
        return false;
    }
    let Ok(data) = std::fs::read(path) else {
        return false;
    };
    if data.starts_with(&JPEG_MAGIC) {
        jpeg_has_location(&data)
    } else if data.starts_with(&PNG_MAGIC) {
        png_has_location(&data)
    } else {
        jxl_has_location(&data)
    }
}

fn exif_crate_finds_gps(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    exif::Reader::new()
        .read_from_container(&mut BufReader::new(file))
        .is_ok_and(|exif| {
            exif.fields()
                .any(|f| matches!(f.tag, exif::Tag::GPSLatitude | exif::Tag::GPSLongitude))
        })
}

fn io_error(path: &Path) -> impl Fn(std::io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: PathBuf::from(path),
        source,
    }
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

/// Whether XMP (or any text) names GPS coordinates.
fn mentions_gps(text: &[u8]) -> bool {
    [&b"GPSLatitude"[..], b"GPSLongitude"]
        .iter()
        .any(|needle| text.windows(needle.len()).any(|w| w == *needle))
}

// ---------------------------------------------------------------------------
// EXIF (TIFF structure)
// ---------------------------------------------------------------------------

/// The EXIF is not laid out as TIFF should be.
#[derive(Debug)]
struct Malformed;

/// Tag in IFD0 that points at the GPS IFD.
const GPS_POINTER: u16 = 0x8825;
/// GPS IFD tags holding the coordinates themselves.
const GPS_LATITUDE: u16 = 2;
const GPS_LONGITUDE: u16 = 4;
/// Real files chain IFD0 and IFD1; more only happens in garbage.
const MAX_IFDS: usize = 8;

#[derive(Clone, Copy)]
struct Tiff {
    big_endian: bool,
}

impl Tiff {
    fn of(data: &[u8]) -> Result<Self, Malformed> {
        match data.get(..4) {
            Some(b"MM\0*") => Ok(Self { big_endian: true }),
            Some(b"II*\0") => Ok(Self { big_endian: false }),
            _ => Err(Malformed),
        }
    }

    fn u16(self, data: &[u8], at: usize) -> Result<u16, Malformed> {
        let bytes: [u8; 2] = data
            .get(at..at + 2)
            .ok_or(Malformed)?
            .try_into()
            .map_err(|_| Malformed)?;
        Ok(if self.big_endian {
            u16::from_be_bytes(bytes)
        } else {
            u16::from_le_bytes(bytes)
        })
    }

    fn u32(self, data: &[u8], at: usize) -> Result<usize, Malformed> {
        let bytes: [u8; 4] = data
            .get(at..at + 4)
            .ok_or(Malformed)?
            .try_into()
            .map_err(|_| Malformed)?;
        let value = if self.big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        };
        usize::try_from(value).map_err(|_| Malformed)
    }

    fn put_u16(self, data: &mut [u8], at: usize, value: u16) {
        let bytes = if self.big_endian {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        };
        data[at..at + 2].copy_from_slice(&bytes);
    }

    /// Offsets of the IFDs chained from the header (IFD0, IFD1, …).
    fn ifds(self, data: &[u8]) -> Result<Vec<usize>, Malformed> {
        let mut ifds = Vec::new();
        let mut next = self.u32(data, 4)?;
        while next != 0 && ifds.len() < MAX_IFDS {
            if ifds.contains(&next) {
                return Err(Malformed);
            }
            ifds.push(next);
            let count = usize::from(self.u16(data, next)?);
            next = self.u32(data, next + 2 + 12 * count)?;
        }
        Ok(ifds)
    }

    /// The value of the first entry tagged `tag` in the IFD at `ifd`, as an
    /// offset, with the entry's index.
    fn find(self, data: &[u8], ifd: usize, tag: u16) -> Result<Option<(usize, usize)>, Malformed> {
        let count = usize::from(self.u16(data, ifd)?);
        for i in 0..count {
            let entry = ifd + 2 + 12 * i;
            if self.u16(data, entry)? == tag {
                return Ok(Some((i, self.u32(data, entry + 8)?)));
            }
        }
        Ok(None)
    }
}

/// Bytes per value of a TIFF field type.
fn type_size(field_type: u16) -> Result<usize, Malformed> {
    match field_type {
        1 | 2 | 6 | 7 => Ok(1),
        3 | 8 => Ok(2),
        4 | 9 | 11 | 13 => Ok(4),
        5 | 10 | 12 => Ok(8),
        _ => Err(Malformed),
    }
}

/// Whether TIFF-structured EXIF holds GPS coordinates.
fn exif_has_gps(tiff: &[u8]) -> bool {
    let has = || -> Result<bool, Malformed> {
        let t = Tiff::of(tiff)?;
        for ifd in t.ifds(tiff)? {
            if let Some((_, gps)) = t.find(tiff, ifd, GPS_POINTER)?
                && (t.find(tiff, gps, GPS_LATITUDE)?.is_some()
                    || t.find(tiff, gps, GPS_LONGITUDE)?.is_some())
            {
                return Ok(true);
            }
        }
        Ok(false)
    };
    has().unwrap_or(false)
}

/// Remove the GPS IFD from TIFF-structured EXIF, in place and without moving
/// anything: its entries and values are zeroed, and the pointer to it is
/// taken out of IFD0 (the entries after it shift up by one slot). Returns
/// whether there was a GPS IFD.
fn drop_gps(tiff: &mut [u8]) -> Result<bool, Malformed> {
    let t = Tiff::of(tiff)?;
    let mut changed = false;
    for ifd in t.ifds(tiff)? {
        let Some((index, gps)) = t.find(tiff, ifd, GPS_POINTER)? else {
            continue;
        };
        zero_ifd(t, tiff, gps)?;

        // Unlink: shift the later entries and the next-IFD offset up one slot.
        let count = usize::from(t.u16(tiff, ifd)?);
        let entry = ifd + 2 + 12 * index;
        let end = ifd + 2 + 12 * count + 4;
        if end > tiff.len() {
            return Err(Malformed);
        }
        tiff.copy_within(entry + 12..end, entry);
        tiff[end - 12..end].fill(0);
        let fewer = u16::try_from(count - 1).map_err(|_| Malformed)?;
        t.put_u16(tiff, ifd, fewer);
        changed = true;
    }
    Ok(changed)
}

/// Zero an IFD's entries, its next-IFD offset and the values stored outside it.
fn zero_ifd(t: Tiff, tiff: &mut [u8], ifd: usize) -> Result<(), Malformed> {
    let count = usize::from(t.u16(tiff, ifd)?);
    let end = ifd + 2 + 12 * count + 4;
    if end > tiff.len() {
        return Err(Malformed);
    }
    for i in 0..count {
        let entry = ifd + 2 + 12 * i;
        let size = type_size(t.u16(tiff, entry + 2)?)?
            .checked_mul(t.u32(tiff, entry + 4)?)
            .ok_or(Malformed)?;
        if size > 4 {
            let at = t.u32(tiff, entry + 8)?;
            let value_end = at.checked_add(size).ok_or(Malformed)?;
            tiff.get_mut(at..value_end).ok_or(Malformed)?.fill(0);
        }
    }
    tiff[ifd..end].fill(0);
    Ok(())
}

// ---------------------------------------------------------------------------
// JPEG
// ---------------------------------------------------------------------------

const APP1: u8 = 0xE1;
const APP2: u8 = 0xE2;
const COM: u8 = 0xFE;
const EXIF_HEADER: &[u8] = b"Exif\0\0";
const XMP_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const XMP_EXTENSION_HEADER: &[u8] = b"http://ns.adobe.com/xmp/extension/\0";

/// A marker segment before the image data: `data[start..end]` is the whole
/// segment, marker included.
struct Segment {
    marker: u8,
    start: usize,
    end: usize,
}

impl Segment {
    fn payload<'a>(&self, data: &'a [u8]) -> &'a [u8] {
        &data[(self.start + 4).min(self.end)..self.end]
    }

    fn is_exif(&self, data: &[u8]) -> bool {
        self.marker == APP1 && self.payload(data).starts_with(EXIF_HEADER)
    }

    fn is_xmp(&self, data: &[u8]) -> bool {
        let payload = self.payload(data);
        self.marker == APP1
            && (payload.starts_with(XMP_HEADER) || payload.starts_with(XMP_EXTENSION_HEADER))
    }
}

/// The segments up to the first scan, and where the rest of the file starts.
fn jpeg_segments(data: &[u8]) -> (Vec<Segment>, usize) {
    let mut segments = Vec::new();
    let mut pos = 2;
    while pos + 4 <= data.len() && data[pos] == 0xFF {
        let marker = data[pos + 1];
        match marker {
            // Fill byte before a marker.
            0xFF => pos += 1,
            // Start of scan or end of image: the rest is picture data.
            0xDA | 0xD9 => break,
            // Markers without a length.
            0x01 | 0xD0..=0xD7 => {
                segments.push(Segment {
                    marker,
                    start: pos,
                    end: pos + 2,
                });
                pos += 2;
            }
            _ => {
                let len = usize::from(u16::from_be_bytes([data[pos + 2], data[pos + 3]]));
                let end = pos + 2 + len;
                if len < 2 || end > data.len() {
                    break;
                }
                segments.push(Segment {
                    marker,
                    start: pos,
                    end,
                });
                pos = end;
            }
        }
    }
    (segments, pos)
}

fn jpeg_has_location(data: &[u8]) -> bool {
    jpeg_segments(data).0.iter().any(|s| {
        let payload = s.payload(data);
        (s.is_exif(data) && exif_has_gps(&payload[EXIF_HEADER.len()..]))
            || (s.is_xmp(data) && mentions_gps(payload))
    })
}

fn scrub_jpeg(data: &[u8], metadata: Metadata) -> Vec<u8> {
    let (segments, rest) = jpeg_segments(data);
    let xmp_has_gps = segments
        .iter()
        .any(|s| s.is_xmp(data) && mentions_gps(s.payload(data)));
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..2]);
    // A multi-picture (MPF) segment locates the extra pictures after the end of
    // the main one relative to itself, so nothing after it may change size.
    let mut after_mpf = false;
    for segment in &segments {
        let bytes = &data[segment.start..segment.end];
        let payload = segment.payload(data);
        let keep = match metadata {
            Metadata::Keep => true,
            Metadata::NoLocation if segment.is_exif(data) => {
                let mut cleaned = bytes.to_vec();
                if drop_gps(&mut cleaned[4 + EXIF_HEADER.len()..]).is_ok() {
                    out.extend_from_slice(&cleaned);
                    continue;
                }
                // EXIF we can't make sense of might hide a location anywhere.
                false
            }
            Metadata::NoLocation => !(xmp_has_gps && segment.is_xmp(data)),
            // JFIF, ICC profiles and MPF, and Adobe's colour transform flag
            // stay; EXIF, XMP, IPTC, comments and the rest go.
            Metadata::None => !matches!(segment.marker, APP1 | 0xE3..=0xED | 0xEF | COM),
        };
        if keep {
            out.extend_from_slice(bytes);
        } else if after_mpf {
            out.extend_from_slice(&bytes[..4]);
            out.resize(out.len() + bytes.len() - 4, 0);
        }
        if segment.marker == APP2 && payload.starts_with(b"MPF\0") {
            after_mpf = true;
        }
    }
    out.extend_from_slice(&data[rest..]);
    out
}

// ---------------------------------------------------------------------------
// PNG
// ---------------------------------------------------------------------------

/// A chunk: `data[start..end]` is the whole chunk, length and CRC included.
struct Chunk {
    kind: [u8; 4],
    start: usize,
    end: usize,
}

impl Chunk {
    fn body<'a>(&self, data: &'a [u8]) -> &'a [u8] {
        &data[self.start + 8..self.end - 4]
    }

    fn is_text(&self) -> bool {
        matches!(&self.kind, b"tEXt" | b"zTXt" | b"iTXt")
    }
}

/// The chunks up to and including `IEND`, and where anything after them starts.
fn png_chunks(data: &[u8]) -> (Vec<Chunk>, usize) {
    let mut chunks = Vec::new();
    let mut pos = PNG_MAGIC.len();
    while pos + 12 <= data.len() {
        let len = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        let Some(end) = usize::try_from(len)
            .ok()
            .and_then(|len| (pos + 12).checked_add(len))
            .filter(|&end| end <= data.len())
        else {
            break;
        };
        let kind = [data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]];
        chunks.push(Chunk {
            kind,
            start: pos,
            end,
        });
        pos = end;
        if &kind == b"IEND" {
            break;
        }
    }
    (chunks, pos)
}

fn png_chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let len = u32::try_from(body.len()).expect("EXIF in one chunk is far below 4 GiB");
    let mut chunk = Vec::with_capacity(body.len() + 12);
    chunk.extend_from_slice(&len.to_be_bytes());
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(body);
    chunk.extend_from_slice(&crc32(&chunk[4..]).to_be_bytes());
    chunk
}

/// CRC-32 as PNG uses it (IEEE, reflected).
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// What a text chunk holds, by its keyword.
#[derive(Debug, PartialEq, Eq)]
enum TextKind {
    /// EXIF as ImageMagick (and djxl) store it: a hex dump.
    RawExif,
    Xmp,
    Other,
}

/// The keyword and text of a `tEXt`, `zTXt` or `iTXt` chunk body, inflated.
fn png_text(kind: &[u8; 4], body: &[u8]) -> Option<(TextKind, Vec<u8>)> {
    let nul = body.iter().position(|&b| b == 0)?;
    let (keyword, rest) = (&body[..nul], &body[nul + 1..]);
    let text_kind = match keyword {
        b"Raw profile type exif" | b"Raw profile type APP1" => TextKind::RawExif,
        b"XML:com.adobe.xmp" | b"Raw profile type xmp" => TextKind::Xmp,
        _ => TextKind::Other,
    };
    let inflate = |bytes: &[u8]| {
        miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(bytes, INFLATE_LIMIT).ok()
    };
    let text = match kind {
        b"tEXt" => rest.to_vec(),
        b"zTXt" => inflate(rest.get(1..)?)?,
        _ => {
            // iTXt: compressed flag, method, language\0, translated keyword\0, text.
            let (&compressed, rest) = rest.split_first()?;
            let rest = rest.get(1..)?;
            let lang = rest.iter().position(|&b| b == 0)?;
            let rest = &rest[lang + 1..];
            let translated = rest.iter().position(|&b| b == 0)?;
            let text = &rest[translated + 1..];
            if compressed == 1 {
                inflate(text)?
            } else {
                text.to_vec()
            }
        }
    };
    Some((text_kind, text))
}

/// The bytes of an ImageMagick raw profile: `\n<name>\n<length>\n<hex…>`.
fn parse_raw_profile(text: &[u8]) -> Option<Vec<u8>> {
    let mut words = text
        .split(u8::is_ascii_whitespace)
        .filter(|w| !w.is_empty());
    let _name = words.next()?;
    let len: usize = std::str::from_utf8(words.next()?).ok()?.parse().ok()?;
    let hex: Vec<u8> = words.flatten().copied().collect();
    let digit = |c: u8| {
        char::from(c)
            .to_digit(16)
            .and_then(|d| u8::try_from(d).ok())
    };
    let bytes: Option<Vec<u8>> = hex
        .chunks(2)
        .take(len)
        .map(|pair| match pair {
            [hi, lo] => Some(digit(*hi)? << 4 | digit(*lo)?),
            _ => None,
        })
        .collect();
    bytes.filter(|b| b.len() == len)
}

/// TIFF-structured EXIF from a raw profile, which may start with `Exif\0\0`.
fn raw_profile_exif(text: &[u8]) -> Option<Vec<u8>> {
    let mut bytes = parse_raw_profile(text)?;
    if bytes.starts_with(EXIF_HEADER) {
        bytes.drain(..EXIF_HEADER.len());
    }
    Some(bytes)
}

fn png_has_location(data: &[u8]) -> bool {
    png_chunks(data).0.iter().any(|chunk| {
        let body = chunk.body(data);
        if &chunk.kind == b"eXIf" {
            return exif_has_gps(body);
        }
        if !chunk.is_text() {
            return false;
        }
        match png_text(&chunk.kind, body) {
            Some((TextKind::RawExif, text)) => {
                raw_profile_exif(&text).is_some_and(|e| exif_has_gps(&e))
            }
            Some((TextKind::Xmp, text)) => mentions_gps(&text),
            _ => false,
        }
    })
}

fn scrub_png(data: &[u8], metadata: Metadata) -> Vec<u8> {
    let (chunks, rest) = png_chunks(data);
    let mut has_exif_chunk = chunks.iter().any(|c| &c.kind == b"eXIf");
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..PNG_MAGIC.len()]);
    for chunk in &chunks {
        let bytes = &data[chunk.start..chunk.end];
        let body = chunk.body(data);
        let metadata_chunk = &chunk.kind == b"eXIf" || chunk.is_text() || &chunk.kind == b"tIME";
        if metadata == Metadata::Keep || !metadata_chunk {
            out.extend_from_slice(bytes);
            continue;
        }
        if metadata == Metadata::None {
            continue;
        }
        // All but location.
        if &chunk.kind == b"eXIf" {
            let mut exif = body.to_vec();
            match drop_gps(&mut exif) {
                Ok(true) => out.extend_from_slice(&png_chunk(b"eXIf", &exif)),
                Ok(false) => out.extend_from_slice(bytes),
                Err(Malformed) => {}
            }
            continue;
        }
        if !chunk.is_text() {
            out.extend_from_slice(bytes);
            continue;
        }
        match png_text(&chunk.kind, body) {
            Some((TextKind::Other, _)) => out.extend_from_slice(bytes),
            Some((TextKind::Xmp, text)) if !mentions_gps(&text) => out.extend_from_slice(bytes),
            Some((TextKind::RawExif, text)) => {
                let Some(mut exif) = raw_profile_exif(&text) else {
                    continue;
                };
                match drop_gps(&mut exif) {
                    Ok(false) => out.extend_from_slice(bytes),
                    // Rewritten as a plain eXIf chunk, which needs no compressor.
                    Ok(true) if !has_exif_chunk => {
                        out.extend_from_slice(&png_chunk(b"eXIf", &exif));
                        has_exif_chunk = true;
                    }
                    Ok(true) | Err(Malformed) => {}
                }
            }
            // XMP naming a place, or text we can't read.
            _ => {}
        }
    }
    out.extend_from_slice(&data[rest..]);
    out
}

// ---------------------------------------------------------------------------
// JPEG XL
// ---------------------------------------------------------------------------

fn jxl_has_location(data: &[u8]) -> bool {
    let mut pos = 0;
    for _ in 0..256 {
        let Some(header) = data.get(pos..pos + 8) else {
            return false;
        };
        let size = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        let kind = &header[4..8];
        let (body_start, end) = match size {
            0 => (pos + 8, data.len()),
            1 => {
                let Some(large) = data.get(pos + 8..pos + 16) else {
                    return false;
                };
                let large = u64::from_be_bytes(large.try_into().unwrap_or_default());
                let Some(end) = usize::try_from(large).ok().and_then(|l| pos.checked_add(l)) else {
                    return false;
                };
                (pos + 16, end)
            }
            _ => (pos + 8, pos + usize::try_from(size).unwrap_or(usize::MAX)),
        };
        let Some(body) = data.get(body_start..end) else {
            return false;
        };
        if jxl_box_has_location(kind, body) {
            return true;
        }
        if end <= pos {
            return false;
        }
        pos = end;
    }
    false
}

fn jxl_box_has_location(kind: &[u8], body: &[u8]) -> bool {
    match kind {
        // A 4-byte offset to the TIFF header, then the EXIF.
        b"Exif" => body
            .get(..4)
            .map(|o| u32::from_be_bytes([o[0], o[1], o[2], o[3]]) as usize)
            .and_then(|offset| body.get(4 + offset..))
            .is_some_and(exif_has_gps),
        b"xml " => mentions_gps(body),
        // Brotli-compressed box: the inner type, then the compressed body.
        b"brob" => {
            let Some((inner, compressed)) = body.split_at_checked(4) else {
                return false;
            };
            if inner == b"brob" {
                return false;
            }
            let mut decompressed = Vec::new();
            let read = brotli_decompressor::Decompressor::new(compressed, 4096)
                .take(INFLATE_LIMIT as u64)
                .read_to_end(&mut decompressed);
            read.is_ok() && jxl_box_has_location(inner, &decompressed)
        }
        _ => false,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const APP0: u8 = 0xE0;
    const APP14: u8 = 0xEE;
    const MAKE: &[u8] = b"CatTestMake\0";
    /// 52°31'12.34"N as EXIF rationals, to find in the output.
    const LATITUDE: [u32; 6] = [52, 1, 31, 1, 1234, 100];

    fn rationals(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// Little-endian IFD with out-of-line values right after it.
    fn ifd(entries: &[(u16, u16, u32, Vec<u8>)], base: usize, next: u32) -> Vec<u8> {
        let head = 2 + 12 * entries.len() + 4;
        let mut out = u16::try_from(entries.len()).unwrap().to_le_bytes().to_vec();
        let mut extra: Vec<u8> = Vec::new();
        for (tag, kind, count, value) in entries {
            out.extend(tag.to_le_bytes());
            out.extend(kind.to_le_bytes());
            out.extend(count.to_le_bytes());
            if value.len() <= 4 {
                let mut inline = value.clone();
                inline.resize(4, 0);
                out.extend(inline);
            } else {
                out.extend(
                    u32::try_from(base + head + extra.len())
                        .unwrap()
                        .to_le_bytes(),
                );
                extra.extend(value);
            }
        }
        out.extend(next.to_le_bytes());
        out.extend(extra);
        out
    }

    /// TIFF-structured EXIF with a camera make, an orientation and, if `gps`,
    /// a GPS IFD with coordinates.
    pub(crate) fn exif(gps: bool) -> Vec<u8> {
        let ifd0 = |gps_at: Option<u32>| {
            let mut entries = vec![
                (0x010F, 2, MAKE.len() as u32, MAKE.to_vec()),
                (0x0112, 3, 1, 1u16.to_le_bytes().to_vec()),
            ];
            if let Some(at) = gps_at {
                entries.push((GPS_POINTER, 4, 1, at.to_le_bytes().to_vec()));
            }
            entries
        };
        let mut tiff = b"II*\0".to_vec();
        tiff.extend(8u32.to_le_bytes());
        if !gps {
            tiff.extend(ifd(&ifd0(None), 8, 0));
            return tiff;
        }
        let gps_at = 8 + ifd(&ifd0(Some(0)), 8, 0).len();
        tiff.extend(ifd(&ifd0(Some(u32::try_from(gps_at).unwrap())), 8, 0));
        tiff.extend(ifd(
            &[
                (1, 2, 2, b"N\0".to_vec()),
                (GPS_LATITUDE, 5, 3, rationals(&LATITUDE)),
                (3, 2, 2, b"E\0".to_vec()),
                (GPS_LONGITUDE, 5, 3, rationals(&[13, 1, 24, 1, 1817, 100])),
            ],
            gps_at,
            0,
        ));
        tiff
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0xFF, marker];
        out.extend(u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
        out.extend(payload);
        out
    }

    /// A JPEG-shaped file: SOI, the given segments, then fake scan data.
    pub(crate) fn jpeg(segments: &[Vec<u8>]) -> Vec<u8> {
        let mut out = JPEG_MAGIC.to_vec();
        for s in segments {
            out.extend(s);
        }
        out.extend([0xFF, 0xDA, 0x00, 0x02, 0x12, 0x34, 0xFF, 0xD9]);
        out
    }

    /// `jpeg` with an EXIF segment (holding a location if `gps`) after SOI.
    pub(crate) fn with_exif(jpeg: &[u8], gps: bool) -> Vec<u8> {
        [&jpeg[..2], &exif_segment(gps), &jpeg[2..]].concat()
    }

    pub(crate) fn mentions_make(data: &[u8]) -> bool {
        contains(data, MAKE)
    }

    fn exif_segment(gps: bool) -> Vec<u8> {
        segment(APP1, &[EXIF_HEADER, &exif(gps)].concat())
    }

    #[test]
    fn finds_and_drops_gps_in_exif() {
        let mut tiff = exif(true);
        let before = tiff.len();
        assert!(exif_has_gps(&tiff));
        assert!(drop_gps(&mut tiff).expect("well formed"));
        assert_eq!(tiff.len(), before, "nothing moves");
        assert!(!exif_has_gps(&tiff));
        assert!(!contains(&tiff, &rationals(&LATITUDE)));
        assert!(contains(&tiff, MAKE));
        // IFD0 is one entry shorter and still readable.
        let t = Tiff::of(&tiff).unwrap();
        assert_eq!(t.u16(&tiff, 8).unwrap(), 2);
        assert_eq!(t.ifds(&tiff).unwrap(), [8]);
        assert!(!drop_gps(&mut tiff).expect("still well formed"));

        assert!(!exif_has_gps(&exif(false)));
        assert!(drop_gps(&mut b"nonsense".to_vec()).is_err());
    }

    #[test]
    fn jpeg_without_location_keeps_its_layout() {
        let icc = segment(APP2, b"ICC_PROFILE\0\x01\x01data");
        let original = jpeg(&[segment(APP0, b"JFIF\0"), exif_segment(true), icc.clone()]);
        assert!(jpeg_has_location(&original));

        let cleaned = scrub_jpeg(&original, Metadata::NoLocation);
        assert_eq!(cleaned.len(), original.len());
        assert!(!jpeg_has_location(&cleaned));
        assert!(contains(&cleaned, MAKE));
        assert!(contains(&cleaned, &icc));
        assert!(cleaned.ends_with(&[0xFF, 0xDA, 0x00, 0x02, 0x12, 0x34, 0xFF, 0xD9]));

        let unchanged = jpeg(&[exif_segment(false)]);
        assert!(!jpeg_has_location(&unchanged));
        assert_eq!(scrub_jpeg(&unchanged, Metadata::NoLocation), unchanged);
    }

    #[test]
    fn jpeg_xmp_naming_a_place_is_dropped() {
        let xmp = |body: &str| segment(APP1, &[XMP_HEADER, body.as_bytes()].concat());
        let placed = xmp("<rdf exif:GPSLatitude=\"52,31.2N\"/>");
        let plain = xmp("<rdf xmp:Rating=\"5\"/>");
        assert!(jpeg_has_location(&jpeg(std::slice::from_ref(&placed))));
        assert!(!jpeg_has_location(&jpeg(std::slice::from_ref(&plain))));
        let cleaned = scrub_jpeg(&jpeg(std::slice::from_ref(&placed)), Metadata::NoLocation);
        assert_eq!(cleaned, jpeg(&[]));
        let kept = jpeg(&[plain]);
        assert_eq!(scrub_jpeg(&kept, Metadata::NoLocation), kept);
    }

    #[test]
    fn jpeg_without_metadata_keeps_colour_segments() {
        let jfif = segment(APP0, b"JFIF\0");
        let icc = segment(APP2, b"ICC_PROFILE\0\x01\x01data");
        let adobe = segment(APP14, b"Adobe\0");
        let dqt = segment(0xDB, &[0; 65]);
        let original = jpeg(&[
            jfif.clone(),
            exif_segment(true),
            segment(APP1, &[XMP_HEADER, b"<x/>"].concat()),
            segment(0xED, b"Photoshop 3.0\0"),
            segment(COM, b"hello"),
            icc.clone(),
            adobe.clone(),
            dqt.clone(),
        ]);
        assert_eq!(
            scrub_jpeg(&original, Metadata::None),
            jpeg(&[jfif, icc, adobe, dqt])
        );
    }

    #[test]
    fn jpeg_segments_after_mpf_are_blanked_not_removed() {
        let mpf = segment(APP2, b"MPF\0II*\0");
        let comment = segment(COM, b"secret");
        let original = jpeg(&[exif_segment(true), mpf.clone(), comment.clone()]);
        let cleaned = scrub_jpeg(&original, Metadata::None);
        let blank = [&comment[..4], &[0; 6]].concat();
        assert_eq!(cleaned, jpeg(&[mpf, blank]));
    }

    fn png(chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut out = PNG_MAGIC.to_vec();
        out.extend(png_chunk(b"IHDR", &[0; 13]));
        for c in chunks {
            out.extend(c);
        }
        out.extend(png_chunk(b"IDAT", b"pixels"));
        out.extend(png_chunk(b"IEND", b""));
        out
    }

    /// The way djxl and ImageMagick store EXIF: a zlib-compressed hex dump.
    fn raw_profile_chunk(tiff: &[u8]) -> Vec<u8> {
        let hex: String = [EXIF_HEADER, tiff]
            .concat()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let text = format!("\nexif\n{:8}\n{hex}\n", tiff.len() + EXIF_HEADER.len());
        let mut body = b"Raw profile type exif\0\0".to_vec();
        body.extend(miniz_oxide::deflate::compress_to_vec_zlib(
            text.as_bytes(),
            6,
        ));
        png_chunk(b"zTXt", &body)
    }

    #[test]
    fn crc_matches_the_png_spec() {
        // The IEND chunk every PNG ends with.
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }

    #[test]
    fn png_exif_loses_only_the_location() {
        let original = png(&[png_chunk(b"eXIf", &exif(true))]);
        assert!(png_has_location(&original));
        let cleaned = scrub_png(&original, Metadata::NoLocation);
        assert!(!png_has_location(&cleaned));
        assert!(contains(&cleaned, MAKE));
        let mut expected = exif(true);
        drop_gps(&mut expected).unwrap();
        assert_eq!(cleaned, png(&[png_chunk(b"eXIf", &expected)]));
    }

    #[test]
    fn png_raw_exif_profile_becomes_a_clean_exif_chunk() {
        let original = png(&[raw_profile_chunk(&exif(true))]);
        assert!(png_has_location(&original));
        let cleaned = scrub_png(&original, Metadata::NoLocation);
        let mut expected = exif(true);
        drop_gps(&mut expected).unwrap();
        assert_eq!(cleaned, png(&[png_chunk(b"eXIf", &expected)]));

        let plain = png(&[raw_profile_chunk(&exif(false))]);
        assert!(!png_has_location(&plain));
        assert_eq!(scrub_png(&plain, Metadata::NoLocation), plain);
    }

    #[test]
    fn png_without_metadata_keeps_colour_chunks() {
        let iccp = png_chunk(b"iCCP", b"icc\0\0data");
        let original = png(&[
            iccp.clone(),
            png_chunk(b"eXIf", &exif(true)),
            png_chunk(b"tEXt", b"Comment\0hi"),
            png_chunk(b"tIME", &[0; 7]),
        ]);
        assert_eq!(scrub_png(&original, Metadata::None), png(&[iccp]));
    }

    #[test]
    fn jxl_boxes_are_searched_even_when_compressed() {
        let exif_box = |tiff: &[u8]| [&0u32.to_be_bytes()[..], tiff].concat();
        assert!(jxl_box_has_location(b"Exif", &exif_box(&exif(true))));
        assert!(!jxl_box_has_location(b"Exif", &exif_box(&exif(false))));
        assert!(jxl_box_has_location(b"xml ", b"<exif:GPSLongitude>"));

        let mut file = crate::jxl::CONTAINER_MAGIC.to_vec();
        let body = exif_box(&exif(true));
        file.extend(u32::try_from(body.len() + 8).unwrap().to_be_bytes());
        file.extend(b"Exif");
        file.extend(&body);
        assert!(jxl_has_location(&file));
        assert!(!jxl_has_location(&crate::jxl::CONTAINER_MAGIC));
    }

    #[test]
    fn scrubs_files_and_refuses_other_formats() {
        let dir = tempfile::tempdir().expect("temp dir");
        let input = dir.path().join("in.jpg");
        let output = dir.path().join("out.jpg");
        std::fs::write(&input, jpeg(&[exif_segment(true)])).unwrap();
        assert!(has_location(&input));
        let args = step_args(&input, &output, Metadata::NoLocation);
        run_step(&args).expect("scrub");
        assert!(!has_location(&output));
        assert!(contains(&std::fs::read(&output).unwrap(), MAKE));
        // Never overwrites.
        assert!(matches!(run_step(&args), Err(Error::Io { .. })));

        let gif = dir.path().join("in.gif");
        std::fs::write(&gif, b"GIF89a").unwrap();
        assert!(!has_location(&gif));
        let err = scrub_file(&gif, &dir.path().join("out.gif"), Metadata::None);
        assert!(matches!(err, Err(Error::Unsupported { .. })));
    }
}
