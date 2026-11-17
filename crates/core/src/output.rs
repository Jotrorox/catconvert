//! Where a result is saved and what it is called.
//!
//! By default the result goes next to the input under a free name: `clip.mp4`,
//! then `clip 2.mp4`, … An [`OutputTarget`] can pick another folder, another
//! name or a name pattern such as `{name}-{format}` for batches, and whether a
//! file already using that name is kept (the new one gets a number) or
//! replaced.
//!
//! Names are checked against the strictest rules (Windows'), on every
//! platform, so files keep working when they are copied elsewhere.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::convert::{ConvertOptions, OutputFormat};
use crate::{Error, MediaInfo, MediaKind, Result};

/// What to do when the chosen name is already taken.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Collision {
    /// Keep the file that is there and number the new one: `clip 2.mp4`, …
    #[default]
    KeepBoth,
    /// Replace the file that is there once the new one is finished. The
    /// original input and the outputs of queued jobs are never replaced: the
    /// new file gets a number instead.
    Overwrite,
}

/// Where to save a result and under which name.
///
/// The default saves next to the input under the usual name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OutputTarget {
    /// Folder to save in; `None` is the input's folder. It must exist.
    pub folder: Option<PathBuf>,
    /// File name without the extension, which always follows the format.
    /// May hold placeholders (see [`PLACEHOLDERS`]), which makes it a pattern
    /// for batches. `None` or blank is the usual name.
    pub name: Option<String>,
    pub collision: Collision,
}

impl OutputTarget {
    /// Whether this is the default: the input's folder, the usual name.
    #[must_use]
    pub fn is_default(&self) -> bool {
        self.folder.is_none() && self.name.as_deref().is_none_or(|n| n.trim().is_empty())
    }
}

/// Placeholders a name pattern can use, with what they stand for.
pub const PLACEHOLDERS: [(&str, &str); 6] = [
    ("{name}", "the original's name, without its extension"),
    ("{format}", "the new format's extension, e.g. mp4"),
    ("{width}", "the new width in pixels"),
    ("{height}", "the new height in pixels"),
    ("{date}", "today's date, e.g. 2026-05-17"),
    ("{time}", "where a saved frame is in the video, e.g. 0m12s"),
];

/// Where the result would go, for showing before it is saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputPreview {
    /// Where the result would be written right now.
    pub path: PathBuf,
    /// The chosen name is taken by a file that [`Collision::Overwrite`]
    /// would replace. With [`Collision::KeepBoth`], `path` has a number.
    pub exists: bool,
}

/// Everything needed to name one result: where it goes by default, what it
/// may not write over, its usual name and the placeholders' values.
#[derive(Debug, Clone, PartialEq)]
pub struct Naming {
    /// Folder used when the target names none.
    pub dir: PathBuf,
    /// Files never written over: the input(s).
    pub inputs: Vec<PathBuf>,
    /// The usual name, without extension.
    pub stem: String,
    pub format: OutputFormat,
    /// `{name}`.
    pub name: String,
    /// `{width}` and `{height}`, when the result has a picture of known size.
    pub size: Option<(u32, u32)>,
    /// `{date}`.
    pub date: String,
    /// `{time}`, for saved frames.
    pub time: Option<String>,
}

impl Naming {
    /// Naming a result of `format` made from `input`, called like it.
    #[must_use]
    pub fn new(input: &Path, format: OutputFormat) -> Self {
        let name = file_stem(input);
        Self {
            dir: input.parent().map(Path::to_path_buf).unwrap_or_default(),
            inputs: vec![input.to_path_buf()],
            stem: name.clone(),
            format,
            name,
            size: None,
            date: today(),
            time: None,
        }
    }

    /// Naming the conversion of `input` with `options`.
    #[must_use]
    pub fn conversion(input: &MediaInfo, options: &ConvertOptions) -> Self {
        Self {
            size: crate::convert::result_dims(input, options),
            ..Self::new(&input.path, options.format)
        }
    }

    /// Naming the frame of `input` at `at_secs` saved as a PNG:
    /// `clip-frame-0m12s` unless a name is chosen.
    #[must_use]
    pub fn frame(input: &MediaInfo, options: &ConvertOptions, at_secs: f64) -> Self {
        let still = ConvertOptions {
            format: OutputFormat::Png,
            ..options.clone()
        };
        let stamp = frame_stamp(at_secs);
        let naming = Self::conversion(input, &still);
        Self {
            stem: format!("{}-frame-{stamp}", naming.name),
            time: Some(stamp),
            ..naming
        }
    }

    /// Naming an animation made from `pictures`, next to the first one,
    /// called `name` unless a name is chosen.
    #[must_use]
    pub fn sequence(pictures: &[MediaInfo], options: &ConvertOptions, name: String) -> Self {
        let first = pictures.first();
        let path = first.map_or_else(|| Path::new(""), |info| info.path.as_path());
        Self {
            inputs: pictures.iter().map(|info| info.path.clone()).collect(),
            stem: name.clone(),
            name,
            size: first.and_then(|info| crate::convert::result_dims(info, options)),
            ..Self::new(path, options.format)
        }
    }

    /// The file name, without extension, that `target` asks for.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] if the pattern uses an unknown placeholder or
    /// one without a value here, or makes a name that isn't allowed.
    pub fn chosen_stem(&self, target: &OutputTarget) -> Result<String> {
        match target.name.as_deref().map(str::trim) {
            Some(pattern) if !pattern.is_empty() => self.expand(pattern),
            _ => Ok(self.stem.clone()),
        }
    }

    /// Where the result of `target` goes now: the chosen name, or with a
    /// number when it is taken and not to be replaced. `taken` rules out
    /// paths reserved but not yet on disk.
    ///
    /// # Errors
    ///
    /// As [`Naming::chosen_stem`], and [`Error::Unsupported`] if the chosen
    /// folder doesn't exist.
    pub fn resolve(&self, target: &OutputTarget, taken: impl Fn(&Path) -> bool) -> Result<PathBuf> {
        self.preview(target, taken).map(|preview| preview.path)
    }

    /// Like [`Naming::resolve`], also telling whether the chosen name is
    /// taken by a file that could be replaced.
    ///
    /// # Errors
    ///
    /// As [`Naming::resolve`].
    pub fn preview(
        &self,
        target: &OutputTarget,
        taken: impl Fn(&Path) -> bool,
    ) -> Result<OutputPreview> {
        let dir = match &target.folder {
            Some(folder) if !folder.is_dir() => {
                return Err(Error::unsupported(format!(
                    "the folder {} isn't there",
                    folder.display()
                )));
            }
            Some(folder) => folder.clone(),
            None => self.dir.clone(),
        };
        let stem = self.chosen_stem(target)?;
        let ext = self.format.extension();
        let off_limits = |p: &Path| self.inputs.iter().any(|input| input == p) || taken(p);
        let exact = dir.join(format!("{stem}.{ext}"));
        let replaceable = exact.is_file() && !off_limits(&exact);
        if replaceable && target.collision == Collision::Overwrite {
            return Ok(OutputPreview {
                path: exact,
                exists: true,
            });
        }
        Ok(OutputPreview {
            path: free_in(&dir, &stem, ext, off_limits),
            exists: replaceable,
        })
    }

    /// Fill in a name pattern and check the result.
    fn expand(&self, pattern: &str) -> Result<String> {
        let mut out = String::new();
        let mut rest = pattern;
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            let word = after
                .find('}')
                .map(|close| &after[..close])
                .filter(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_alphanumeric()));
            let Some(word) = word else {
                out.push('{');
                rest = after;
                continue;
            };
            out.push_str(&clean(&self.value(word)?));
            rest = &after[word.len() + 1..];
        }
        out.push_str(rest);
        let stem = strip_media_extension(out.trim());
        check_stem(stem, self.format)?;
        Ok(stem.to_owned())
    }

    fn value(&self, word: &str) -> Result<String> {
        let missing = |what: &str| Error::unsupported(format!("{{{word}}} only works for {what}"));
        Ok(match word.to_ascii_lowercase().as_str() {
            "name" => self.name.clone(),
            "format" => self.format.extension().to_owned(),
            "width" => self
                .size
                .ok_or_else(|| missing("photos and videos"))?
                .0
                .to_string(),
            "height" => self
                .size
                .ok_or_else(|| missing("photos and videos"))?
                .1
                .to_string(),
            "date" => self.date.clone(),
            "time" => self.time.clone().ok_or_else(|| missing("saved frames"))?,
            _ => {
                let known: Vec<&str> = PLACEHOLDERS.iter().map(|(p, _)| *p).collect();
                return Err(Error::unsupported(format!(
                    "{{{word}}} isn't a placeholder CatConvert knows. Use {}",
                    known.join(", ")
                )));
            }
        })
    }
}

/// `{stem}.{ext}` in `dir`, or `{stem} 2.{ext}`, `{stem} 3.{ext}`, …,
/// whichever is first neither on disk nor `taken`.
pub(crate) fn free_in(dir: &Path, stem: &str, ext: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    (1..)
        .map(|n| {
            let name = if n == 1 {
                format!("{stem}.{ext}")
            } else {
                format!("{stem} {n}.{ext}")
            };
            dir.join(name)
        })
        .find(|candidate| !candidate.exists() && !taken(candidate))
        .expect("an unbounded range always yields a free name")
}

pub(crate) fn file_stem(input: &Path) -> String {
    input
        .file_stem()
        .map_or_else(|| "converted".into(), |s| s.to_string_lossy().into_owned())
}

/// `0m12s`, or `1h02m03s` past an hour.
pub(crate) fn frame_stamp(at_secs: f64) -> String {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let whole = at_secs.max(0.0).floor() as u64;
    let (h, m, s) = (whole / 3600, whole / 60 % 60, whole % 60);
    if h > 0 {
        format!("{h}h{m:02}m{s:02}s")
    } else {
        format!("{m}m{s:02}s")
    }
}

/// Today's local date as `2026-05-17`.
fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Characters Windows doesn't allow in file names (macOS and Linux only
/// refuse `/`).
const FORBIDDEN: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Names Windows keeps for devices, whatever the extension.
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Longest file name most file systems take, in bytes.
const MAX_NAME_BYTES: usize = 255;

/// A placeholder's value made safe for a file name: characters Windows
/// refuses become `_`.
fn clean(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if FORBIDDEN.contains(&c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// `stem` without a trailing media extension (`clip.mov` → `clip`): the
/// format decides the extension. Other dots stay (`v1.2`).
fn strip_media_extension(stem: &str) -> &str {
    match stem.rsplit_once('.') {
        Some((base, ext))
            if !base.trim().is_empty() && MediaKind::from_extension(ext) != MediaKind::Unknown =>
        {
            base.trim_end()
        }
        _ => stem,
    }
}

/// Check a chosen name (without extension) against the rules of every
/// platform CatConvert runs on.
///
/// # Errors
///
/// [`Error::Unsupported`] saying what is wrong with it.
pub fn check_stem(stem: &str, format: OutputFormat) -> Result<()> {
    if stem.trim().is_empty() {
        return Err(Error::unsupported("the file name is empty"));
    }
    if let Some(c) = stem.chars().find(|c| FORBIDDEN.contains(c)) {
        return Err(Error::unsupported(format!(
            "file names can't contain {c} (nor any of < > : \" / \\ | ? *)"
        )));
    }
    if stem.starts_with('.') {
        return Err(Error::unsupported(
            "file names can't start with a dot, which would hide the file",
        ));
    }
    if stem.chars().any(char::is_control) {
        return Err(Error::unsupported(
            "file names can't contain control characters",
        ));
    }
    let device = stem.split('.').next().unwrap_or_default().trim_end();
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(device)) {
        return Err(Error::unsupported(format!(
            "{device} is a name Windows keeps for itself; pick another"
        )));
    }
    if stem.len() + 1 + format.extension().len() > MAX_NAME_BYTES {
        return Err(Error::unsupported("the file name is too long"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naming(dir: &Path) -> Naming {
        Naming {
            date: "2026-05-17".into(),
            size: Some((1280, 720)),
            ..Naming::new(&dir.join("clip.mov"), OutputFormat::Mp4)
        }
    }

    fn named(name: &str) -> OutputTarget {
        OutputTarget {
            name: Some(name.into()),
            ..OutputTarget::default()
        }
    }

    #[test]
    fn the_default_is_next_to_the_input() {
        let dir = tempfile::tempdir().expect("temp dir");
        let naming = naming(dir.path());
        let target = OutputTarget::default();
        assert!(target.is_default());
        assert_eq!(
            naming.resolve(&target, |_| false).expect("resolve"),
            dir.path().join("clip.mp4")
        );
        // Blank names count as none.
        assert!(named("  ").is_default());
        assert_eq!(
            naming.resolve(&named("  "), |_| false).expect("resolve"),
            dir.path().join("clip.mp4")
        );
    }

    #[test]
    fn saves_in_the_chosen_folder_under_the_chosen_name() {
        let dir = tempfile::tempdir().expect("temp dir");
        let elsewhere = tempfile::tempdir().expect("temp dir");
        let target = OutputTarget {
            folder: Some(elsewhere.path().to_path_buf()),
            name: Some("holiday".into()),
            ..OutputTarget::default()
        };
        assert!(!target.is_default());
        assert_eq!(
            naming(dir.path())
                .resolve(&target, |_| false)
                .expect("resolve"),
            elsewhere.path().join("holiday.mp4")
        );
    }

    #[test]
    fn a_missing_folder_is_refused() {
        let dir = tempfile::tempdir().expect("temp dir");
        let target = OutputTarget {
            folder: Some(dir.path().join("gone")),
            ..OutputTarget::default()
        };
        let err = naming(dir.path())
            .resolve(&target, |_| false)
            .expect_err("no folder");
        assert!(err.to_string().contains("isn't there"), "{err}");
    }

    #[test]
    fn patterns_fill_in_placeholders() {
        let dir = tempfile::tempdir().expect("temp dir");
        let naming = naming(dir.path());
        let stem = |pattern: &str| naming.chosen_stem(&named(pattern));
        assert_eq!(stem("{name}-{format}").expect("ok"), "clip-mp4");
        assert_eq!(stem("{name}_{height}p").expect("ok"), "clip_720p");
        assert_eq!(
            stem("{name}_{width}x{height}").expect("ok"),
            "clip_1280x720"
        );
        assert_eq!(stem("{name} ({date})").expect("ok"), "clip (2026-05-17)");
        // Placeholders ignore case; braces around anything else stay.
        assert_eq!(stem("{NAME} {x y} {").expect("ok"), "clip {x y} {");
        assert!(
            stem("{size}")
                .expect_err("unknown")
                .to_string()
                .contains("{name}")
        );
        assert!(
            stem("{time}")
                .expect_err("not a frame")
                .to_string()
                .contains("frames")
        );

        let song = Naming {
            size: None,
            ..naming.clone()
        };
        assert!(song.chosen_stem(&named("{width}")).is_err());
    }

    #[test]
    fn frames_have_their_own_name_and_time() {
        let input = MediaInfo {
            width: Some(1920),
            height: Some(1080),
            ..crate::media::tests::sample("/videos/clip.mov", MediaKind::Video)
        };
        let naming = Naming::frame(&input, &ConvertOptions::default(), 3723.9);
        assert_eq!(naming.stem, "clip-frame-1h02m03s");
        assert_eq!(naming.format, OutputFormat::Png);
        assert_eq!(naming.size, Some((1920, 1080)));
        assert_eq!(
            naming.chosen_stem(&named("{name} at {time}")).expect("ok"),
            "clip at 1h02m03s"
        );
    }

    #[test]
    fn placeholder_values_are_made_safe() {
        let dir = tempfile::tempdir().expect("temp dir");
        let naming = Naming {
            name: "a:b|c?".into(),
            ..naming(dir.path())
        };
        assert_eq!(naming.chosen_stem(&named("{name}")).expect("ok"), "a_b_c_");
    }

    #[test]
    fn names_windows_refuses_are_refused_everywhere() {
        let dir = tempfile::tempdir().expect("temp dir");
        let naming = naming(dir.path());
        for bad in [
            "a<b", "a>b", "a:b", "a\"b", "a/b", "a\\b", "a|b", "a?b", "a*b", "a\tb",
        ] {
            assert!(naming.chosen_stem(&named(bad)).is_err(), "{bad:?} passed");
        }
        for reserved in ["CON", "nul", "Com1", "lpt9", "aux.backup", "PRN "] {
            let err = naming.chosen_stem(&named(reserved)).expect_err(reserved);
            assert!(err.to_string().contains("Windows"), "{err}");
        }
        assert!(naming.chosen_stem(&named(&"x".repeat(260))).is_err());
        // Look-alikes are fine.
        for fine in ["console", "COM10", "nullish", "a.b", "über", "猫"] {
            assert!(naming.chosen_stem(&named(fine)).is_ok(), "{fine:?} refused");
        }
    }

    #[test]
    fn the_format_decides_the_extension() {
        let dir = tempfile::tempdir().expect("temp dir");
        let naming = naming(dir.path());
        let stem = |pattern: &str| naming.chosen_stem(&named(pattern)).expect("ok");
        assert_eq!(stem("holiday.mp4"), "holiday");
        assert_eq!(stem("holiday.MOV"), "holiday");
        assert_eq!(stem("{name}.webm"), "clip");
        // Not a media extension: part of the name.
        assert_eq!(stem("v1.2"), "v1.2");
        assert_eq!(stem("notes.txt"), "notes.txt");
        // Only an extension is not a name, and would hide the file.
        assert!(naming.chosen_stem(&named(".mp4")).is_err());
    }

    #[test]
    fn taken_names_get_a_number_unless_replaced() {
        let dir = tempfile::tempdir().expect("temp dir");
        let naming = naming(dir.path());
        std::fs::write(dir.path().join("holiday.mp4"), b"old").expect("write");

        let keep = named("holiday");
        let preview = naming.preview(&keep, |_| false).expect("preview");
        assert_eq!(preview.path, dir.path().join("holiday 2.mp4"));
        assert!(preview.exists);

        let replace = OutputTarget {
            collision: Collision::Overwrite,
            ..keep
        };
        let preview = naming.preview(&replace, |_| false).expect("preview");
        assert_eq!(preview.path, dir.path().join("holiday.mp4"));
        assert!(preview.exists);

        // A name a queued job will write is never replaced.
        let reserved = dir.path().join("holiday.mp4");
        let preview = naming
            .preview(&replace, |p| p == reserved)
            .expect("preview");
        assert_eq!(preview.path, dir.path().join("holiday 2.mp4"));
        assert!(!preview.exists);

        // Free names are used as they are.
        let preview = naming.preview(&named("fresh"), |_| false).expect("preview");
        assert_eq!(preview.path, dir.path().join("fresh.mp4"));
        assert!(!preview.exists);
    }

    #[test]
    fn the_original_is_never_replaced() {
        let dir = tempfile::tempdir().expect("temp dir");
        let input = dir.path().join("clip.mp4");
        std::fs::write(&input, b"original").expect("write");
        let naming = Naming::new(&input, OutputFormat::Mp4);
        let target = OutputTarget {
            collision: Collision::Overwrite,
            ..OutputTarget::default()
        };
        let preview = naming.preview(&target, |_| false).expect("preview");
        assert_eq!(preview.path, dir.path().join("clip 2.mp4"));
        assert!(!preview.exists);
    }

    #[test]
    fn targets_read_camel_case_and_fill_gaps() {
        let target: OutputTarget =
            serde_json::from_value(serde_json::json!({ "name": "x", "collision": "overwrite" }))
                .expect("parse");
        assert_eq!(target.folder, None);
        assert_eq!(target.name.as_deref(), Some("x"));
        assert_eq!(target.collision, Collision::Overwrite);
        let empty: OutputTarget = serde_json::from_value(serde_json::json!({})).expect("parse");
        assert_eq!(empty, OutputTarget::default());
        assert_eq!(
            serde_json::to_value(Collision::KeepBoth).expect("json"),
            serde_json::json!("keepBoth")
        );
    }
}
