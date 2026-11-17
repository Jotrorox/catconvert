//! Where finished conversions go: next to the original, into one folder, or
//! in the original's place.
//!
//! Replacing is careful about the original. The conversion writes a hidden
//! file next to it (`.clip.converting.mp4`); only once that is finished and
//! not empty does the original go to the Trash (or, where there is none, get
//! renamed to `clip.orig.mov`) and the new file take its name. A failed or
//! cancelled conversion leaves the original as it was.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::convert::{OutputFormat, file_stem, frame_stem, free_path_in, output_path};
use crate::sequence::{sequence_name, sequence_output_path};
use crate::{Error, MediaInfo, Result};

/// Where the result of a conversion goes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Destination {
    /// Next to the original, under a free name: `clip.mp4`, `clip 2.mp4`, …
    #[default]
    NextToInput,
    /// Into `path`, under a free name there. With `by_kind`, into its
    /// `Videos`, `Pictures` or `Audio` subfolder, by what the output is.
    Folder {
        path: PathBuf,
        #[serde(default)]
        by_kind: bool,
    },
    /// In the original's place: the original goes to the Trash once the
    /// result is safely written. A new extension gives a new name
    /// (`clip.mov` becomes `clip.mp4`). Only for conversions of one file.
    ReplaceInput,
}

impl Destination {
    /// The folder an output of `format` made from `input` goes in.
    #[must_use]
    pub fn folder_for(&self, input: &Path, format: OutputFormat) -> PathBuf {
        match self {
            Self::Folder {
                path,
                by_kind: true,
            } => path.join(kind_folder(format)),
            Self::Folder {
                path,
                by_kind: false,
            } => path.clone(),
            Self::NextToInput | Self::ReplaceInput => {
                input.parent().map_or_else(PathBuf::new, Path::to_path_buf)
            }
        }
    }

    /// Fails for a folder that can't be one: empty or relative.
    fn check(&self) -> Result<()> {
        match self {
            Self::Folder { path, .. } if path.as_os_str().is_empty() => Err(Error::unsupported(
                "choose an output folder in Settings first",
            )),
            Self::Folder { path, .. } if !path.is_absolute() => Err(Error::unsupported(format!(
                "the output folder must be a full path, not {}",
                path.display()
            ))),
            _ => Ok(()),
        }
    }
}

/// Subfolder of a [`Destination::Folder`] with `by_kind` for `format`.
/// Animations count as pictures.
fn kind_folder(format: OutputFormat) -> &'static str {
    use OutputFormat as F;
    match format {
        F::Mp4 | F::Webm | F::Mov | F::Mkv => "Videos",
        F::Mp3 | F::M4a | F::Wav | F::Flac | F::Ogg => "Audio",
        F::Gif | F::Png | F::Jpg | F::Webp | F::Avif | F::Jxl => "Pictures",
    }
}

/// Where a conversion writes, and where its result ends up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The file the plan writes.
    pub written: PathBuf,
    /// Where the result ends up: `written`, unless it replaces the original.
    pub output: PathBuf,
    /// The original the result replaces, if it does.
    pub replaces: Option<PathBuf>,
}

impl Target {
    fn at(path: PathBuf) -> Self {
        Self {
            written: path.clone(),
            output: path,
            replaces: None,
        }
    }

    /// Paths to keep other jobs from picking while this one is queued: what
    /// it writes, where that goes and the original it replaces.
    pub(crate) fn paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.written.clone()];
        for path in [Some(&self.output), self.replaces.as_ref()]
            .into_iter()
            .flatten()
        {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
        paths
    }
}

/// Where converting `input` to `format` goes for `destination`.
///
/// Names that exist or that `taken` rules out are skipped. Replacing keeps
/// the original's exact name when the extension stays (ignoring case), and
/// otherwise takes the first free `{stem}.{ext}` next to it.
///
/// # Errors
///
/// [`Error::Unsupported`] for a folder that is empty or not a full path.
pub fn output_target(
    input: &Path,
    format: OutputFormat,
    destination: &Destination,
    taken: impl Fn(&Path) -> bool,
) -> Result<Target> {
    destination.check()?;
    let (stem, ext) = (file_stem(input), format.extension());
    Ok(match destination {
        Destination::NextToInput => Target::at(output_path(input, format, taken)),
        Destination::Folder { .. } => {
            let dir = destination.folder_for(input, format);
            Target::at(free_path_in(&dir, input, &stem, ext, taken))
        }
        Destination::ReplaceInput => {
            let dir = destination.folder_for(input, format);
            let same = input
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(ext));
            let output = if same {
                input.to_path_buf()
            } else {
                free_path_in(&dir, input, &stem, ext, &taken)
            };
            let written = free_path_in(&dir, input, &format!(".{stem}.converting"), ext, |p| {
                p == output || taken(p)
            });
            Target {
                written,
                output,
                replaces: Some(input.to_path_buf()),
            }
        }
    })
}

/// Where an animation made from `inputs` goes for `destination`.
///
/// # Errors
///
/// [`Error::Unsupported`] for [`Destination::ReplaceInput`]: one file can't
/// take the place of several pictures. Also for a folder that can't be one.
pub fn sequence_target(
    inputs: &[MediaInfo],
    format: OutputFormat,
    destination: &Destination,
    taken: impl Fn(&Path) -> bool,
) -> Result<Target> {
    destination.check()?;
    match destination {
        Destination::NextToInput => Ok(Target::at(sequence_output_path(inputs, format, taken))),
        Destination::Folder { .. } => {
            let first = inputs
                .first()
                .map_or_else(|| Path::new(""), |info| &info.path);
            let dir = destination.folder_for(first, format);
            let name = sequence_name(inputs);
            Ok(Target::at(free_path_in(
                &dir,
                first,
                &name,
                format.extension(),
                |p| inputs.iter().any(|info| info.path == p) || taken(p),
            )))
        }
        Destination::ReplaceInput => Err(Error::unsupported(
            "an animation can't replace the pictures it's made from; save it next to them or in \
             the output folder",
        )),
    }
}

/// Where the frame of `input` at `at_secs` goes for `destination`: into the
/// output folder, or next to the video. A frame never replaces its video.
///
/// # Errors
///
/// [`Error::Unsupported`] for a folder that can't be one.
pub fn frame_target(
    input: &Path,
    at_secs: f64,
    destination: &Destination,
    taken: impl Fn(&Path) -> bool,
) -> Result<PathBuf> {
    destination.check()?;
    let format = OutputFormat::Png;
    let dir = match destination {
        Destination::Folder { .. } => destination.folder_for(input, format),
        _ => Destination::NextToInput.folder_for(input, format),
    };
    Ok(free_path_in(
        &dir,
        input,
        &frame_stem(input, at_secs),
        format.extension(),
        taken,
    ))
}

/// Create the folder `output` goes in, if it isn't there yet.
pub(crate) fn ensure_folder(output: &Path) -> Result<()> {
    match output.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => {
            std::fs::create_dir_all(dir).map_err(|source| Error::Io {
                path: dir.to_path_buf(),
                source,
            })
        }
        _ => Ok(()),
    }
}

/// What became of the original a conversion replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "to", rename_all = "camelCase")]
pub enum Replaced {
    /// It is in the Trash (Recycle Bin).
    Trash,
    /// There was no Trash to put it in, so it was renamed to `path`.
    Renamed { path: PathBuf },
}

/// Put the finished `target.written` in place of `target.replaces`.
///
/// Checks the new file isn't empty, moves the original to the Trash (or
/// renames it to `clip.orig.mov` if that fails), then renames the new file
/// to `target.output`, or to the next free name should something have taken
/// that meanwhile. Returns where the result ended up.
///
/// # Errors
///
/// [`Error::Ffmpeg`] when the new file is empty or the original can't be
/// moved out of the way; the new file is removed and the original left
/// untouched. Also when the new file can't be renamed after all; the
/// message says where both files are then.
pub(crate) fn replace_original(
    target: &Target,
    taken: impl Fn(&Path) -> bool,
) -> Result<(PathBuf, Replaced)> {
    swap_in(target, taken, move_to_trash)
}

/// [`replace_original`] with `trash` to move the original to the Trash.
fn swap_in(
    target: &Target,
    taken: impl Fn(&Path) -> bool,
    trash: impl FnOnce(&Path) -> std::result::Result<(), String>,
) -> Result<(PathBuf, Replaced)> {
    let Some(original) = &target.replaces else {
        return Ok((target.written.clone(), Replaced::Trash));
    };
    let fail = |message: String| {
        let _ = std::fs::remove_file(&target.written);
        Err(Error::Ffmpeg { message })
    };

    match std::fs::metadata(&target.written) {
        Ok(meta) if meta.len() > 0 => {}
        Ok(_) => return fail("the converted file came out empty, so the original was kept".into()),
        Err(err) => {
            return fail(format!(
                "the converted file is missing ({err}), so the original was kept"
            ));
        }
    }
    // The new file takes over the original's permissions (e.g. read-only
    // for others), as an in-place edit would.
    if let Ok(meta) = std::fs::metadata(original) {
        let _ = std::fs::set_permissions(&target.written, meta.permissions());
    }

    let replaced = match trash(original) {
        Ok(()) => Replaced::Trash,
        Err(trash_err) => {
            let kept = kept_original_path(original, &taken);
            match std::fs::rename(original, &kept) {
                Ok(()) => Replaced::Renamed { path: kept },
                Err(err) => {
                    return fail(format!(
                        "couldn't move the original to the Trash ({trash_err}) or rename it \
                         ({err}), so it was kept"
                    ));
                }
            }
        }
    };

    // Something may have appeared at a new name while converting.
    let output = if &target.output != original && target.output.exists() {
        let dir = target.output.parent().unwrap_or_else(|| Path::new(""));
        let ext = target
            .output
            .extension()
            .map_or_else(String::new, |e| e.to_string_lossy().into_owned());
        free_path_in(dir, original, &file_stem(&target.output), &ext, &taken)
    } else {
        target.output.clone()
    };
    if let Err(err) = std::fs::rename(&target.written, &output) {
        let original_now = match &replaced {
            Replaced::Renamed { path } if std::fs::rename(path, original).is_ok() => {
                let _ = std::fs::remove_file(&target.written);
                return Err(Error::Ffmpeg {
                    message: format!(
                        "couldn't put the converted file in place ({err}), so the original was kept"
                    ),
                });
            }
            Replaced::Renamed { path } => path.display().to_string(),
            Replaced::Trash => "in the Trash".into(),
        };
        return Err(Error::Ffmpeg {
            message: format!(
                "couldn't put the converted file in place ({err}). It's at {}; the original is {}",
                target.written.display(),
                original_now
            ),
        });
    }
    Ok((output, replaced))
}

/// `clip.orig.mov` next to `original`, or `clip.orig 2.mov`, … if taken.
fn kept_original_path(original: &Path, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let dir = original.parent().unwrap_or_else(|| Path::new(""));
    let stem = format!("{}.orig", file_stem(original));
    match original.extension() {
        Some(ext) => free_path_in(dir, original, &stem, &ext.to_string_lossy(), taken),
        None => (1..)
            .map(|n| {
                dir.join(if n == 1 {
                    stem.clone()
                } else {
                    format!("{stem} {n}")
                })
            })
            .find(|p| !p.exists() && !taken(p))
            .expect("an unbounded range always yields a free name"),
    }
}

/// Move `path` to the Trash (Recycle Bin on Windows).
#[cfg(not(any(test, target_os = "android", target_os = "ios")))]
fn move_to_trash(path: &Path) -> std::result::Result<(), String> {
    #[allow(unused_mut)]
    let mut trash = trash::TrashContext::default();
    // Finder's way supports "Put Back" but needs permission to script
    // Finder; the system call needs none.
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        trash.set_delete_method(DeleteMethod::NsFileManager);
    }
    trash.delete(path).map_err(|err| err.to_string())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[cfg(not(test))]
fn move_to_trash(_path: &Path) -> std::result::Result<(), String> {
    Err("there is no Trash on this system".into())
}

/// Tests keep out of the real Trash: originals get renamed instead.
#[cfg(test)]
fn move_to_trash(_path: &Path) -> std::result::Result<(), String> {
    Err("tests don't use the real Trash".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path, contents: &[u8]) {
        std::fs::write(path, contents).expect("write");
    }

    /// A Trash in `bin`, so tests don't fill the real one.
    fn bin(bin: &Path) -> impl FnOnce(&Path) -> std::result::Result<(), String> + '_ {
        move |path: &Path| {
            std::fs::create_dir_all(bin).map_err(|e| e.to_string())?;
            let name = path.file_name().expect("name");
            std::fs::rename(path, bin.join(name)).map_err(|e| e.to_string())
        }
    }

    fn no_trash(_: &Path) -> std::result::Result<(), String> {
        Err("no Trash here".into())
    }

    fn picture(path: &str) -> MediaInfo {
        crate::media::tests::sample(path, crate::MediaKind::Image)
    }

    #[test]
    fn destination_serializes_with_a_kind_tag() {
        let folder = Destination::Folder {
            path: "/out".into(),
            by_kind: true,
        };
        assert_eq!(
            serde_json::to_value(&folder).expect("serialize"),
            serde_json::json!({"kind": "folder", "path": "/out", "byKind": true})
        );
        let parsed: Destination =
            serde_json::from_value(serde_json::json!({"kind": "replaceInput"})).expect("parse");
        assert_eq!(parsed, Destination::ReplaceInput);
        let parsed: Destination =
            serde_json::from_value(serde_json::json!({"kind": "folder", "path": "/out"}))
                .expect("parse");
        assert_eq!(
            parsed,
            Destination::Folder {
                path: "/out".into(),
                by_kind: false
            }
        );
        // Options from before destinations existed still parse.
        let options: crate::ConvertOptions =
            serde_json::from_value(serde_json::json!({"format": "gif"})).expect("parse");
        assert_eq!(options.destination, Destination::NextToInput);
    }

    #[test]
    fn folder_outputs_avoid_existing_and_reserved_names() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (src, out) = (dir.path().join("src"), dir.path().join("out"));
        std::fs::create_dir_all(&src).expect("mkdir");
        let input = src.join("clip.mov");
        touch(&input, b"x");
        let folder = Destination::Folder {
            path: out.clone(),
            by_kind: false,
        };

        // The folder doesn't have to exist yet.
        let target = output_target(&input, OutputFormat::Mp4, &folder, |_| false).expect("target");
        assert_eq!(target, Target::at(out.join("clip.mp4")));

        std::fs::create_dir_all(&out).expect("mkdir");
        touch(&out.join("clip.mp4"), b"x");
        let reserved = out.join("clip 2.mp4");
        let target =
            output_target(&input, OutputFormat::Mp4, &folder, |p| p == reserved).expect("target");
        assert_eq!(target.output, out.join("clip 3.mp4"));

        // Same-named files from different folders don't collide either.
        let other = dir.path().join("elsewhere").join("clip.mov");
        let target = output_target(&other, OutputFormat::Mp4, &folder, |p| {
            p == reserved || p == out.join("clip 3.mp4")
        })
        .expect("target");
        assert_eq!(target.output, out.join("clip 4.mp4"));
    }

    #[test]
    fn folders_by_kind_sort_outputs_into_subfolders() {
        let folder = Destination::Folder {
            path: "/out".into(),
            by_kind: true,
        };
        let input = Path::new("/clips/cat.mov");
        let into = |format| folder.folder_for(input, format);
        assert_eq!(into(OutputFormat::Mp4), Path::new("/out/Videos"));
        assert_eq!(into(OutputFormat::Mp3), Path::new("/out/Audio"));
        assert_eq!(into(OutputFormat::Gif), Path::new("/out/Pictures"));
        assert_eq!(into(OutputFormat::Jxl), Path::new("/out/Pictures"));
        assert_eq!(
            Destination::NextToInput.folder_for(input, OutputFormat::Mp4),
            Path::new("/clips")
        );
    }

    #[test]
    fn folders_must_be_full_paths() {
        let input = Path::new("/clips/cat.mov");
        for path in ["", "relative/out"] {
            let folder = Destination::Folder {
                path: path.into(),
                by_kind: false,
            };
            assert!(output_target(input, OutputFormat::Mp4, &folder, |_| false).is_err());
        }
    }

    #[test]
    fn replacing_writes_a_hidden_file_first() {
        let dir = tempfile::tempdir().expect("temp dir");
        let input = dir.path().join("clip.mov");
        touch(&input, b"x");

        // Same extension (any case): the result takes the original's name.
        let target = output_target(
            &input,
            OutputFormat::Mov,
            &Destination::ReplaceInput,
            |_| false,
        )
        .expect("target");
        assert_eq!(target.output, input);
        assert_eq!(target.written, dir.path().join(".clip.converting.mov"));
        assert_eq!(target.replaces.as_deref(), Some(input.as_path()));

        let upper = dir.path().join("PHOTO.JPG");
        touch(&upper, b"x");
        let target = output_target(
            &upper,
            OutputFormat::Jpg,
            &Destination::ReplaceInput,
            |_| false,
        )
        .expect("target");
        assert_eq!(target.output, upper);

        // A new extension gets the first free name, without touching others.
        touch(&dir.path().join("clip.mp4"), b"someone else's");
        touch(&dir.path().join(".clip.converting.mp4"), b"leftover");
        let target = output_target(
            &input,
            OutputFormat::Mp4,
            &Destination::ReplaceInput,
            |_| false,
        )
        .expect("target");
        assert_eq!(target.output, dir.path().join("clip 2.mp4"));
        assert_eq!(target.written, dir.path().join(".clip.converting 2.mp4"));
        assert_eq!(
            target.paths(),
            [target.written.clone(), target.output.clone(), input.clone()]
        );
    }

    #[test]
    fn animations_never_replace_their_pictures() {
        let pictures = [picture("/p/cat 1.jpg"), picture("/p/cat 2.jpg")];
        assert!(
            sequence_target(
                &pictures,
                OutputFormat::Gif,
                &Destination::ReplaceInput,
                |_| false
            )
            .is_err()
        );
        let folder = Destination::Folder {
            path: "/out".into(),
            by_kind: true,
        };
        let target =
            sequence_target(&pictures, OutputFormat::Gif, &folder, |_| false).expect("target");
        assert_eq!(target.output, Path::new("/out/Pictures/cat.gif"));
    }

    #[test]
    fn frames_go_to_the_folder_or_next_to_the_video() {
        let input = Path::new("/clips/cat.mov");
        let next = frame_target(input, 12.5, &Destination::ReplaceInput, |_| false).expect("path");
        assert_eq!(next, Path::new("/clips/cat-frame-0m12s.png"));
        let folder = Destination::Folder {
            path: "/out".into(),
            by_kind: true,
        };
        let into = frame_target(input, 12.5, &folder, |_| false).expect("path");
        assert_eq!(into, Path::new("/out/Pictures/cat-frame-0m12s.png"));
    }

    #[test]
    fn replacing_with_a_new_extension_swaps_the_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let input = dir.path().join("clip.mov");
        touch(&input, b"original");
        let target = output_target(
            &input,
            OutputFormat::Mp4,
            &Destination::ReplaceInput,
            |_| false,
        )
        .expect("target");
        touch(&target.written, b"converted");

        let trash = dir.path().join("Trash");
        let (output, replaced) = swap_in(&target, |_| false, bin(&trash)).expect("replace");
        assert_eq!(replaced, Replaced::Trash);
        assert_eq!(output, dir.path().join("clip.mp4"));
        assert_eq!(std::fs::read(&output).expect("read"), b"converted");
        assert!(!target.written.exists());
        assert!(!input.exists(), "the original is out of the way");
        assert_eq!(
            std::fs::read(trash.join("clip.mov")).expect("read"),
            b"original"
        );
    }

    #[test]
    fn without_a_trash_the_original_is_kept_under_another_name() {
        let dir = tempfile::tempdir().expect("temp dir");
        let input = dir.path().join("clip.mov");
        touch(&input, b"original");
        let target = output_target(
            &input,
            OutputFormat::Mov,
            &Destination::ReplaceInput,
            |_| false,
        )
        .expect("target");
        touch(&target.written, b"converted");

        let (output, replaced) = swap_in(&target, |_| false, no_trash).expect("replace");
        let kept = dir.path().join("clip.orig.mov");
        assert_eq!(replaced, Replaced::Renamed { path: kept.clone() });
        assert_eq!(output, input);
        assert_eq!(std::fs::read(&input).expect("read"), b"converted");
        assert_eq!(std::fs::read(&kept).expect("read"), b"original");
    }

    #[test]
    fn replacing_with_the_same_extension_keeps_the_name() {
        let dir = tempfile::tempdir().expect("temp dir");
        let input = dir.path().join("photo.jpg");
        touch(&input, b"original");
        let target = output_target(
            &input,
            OutputFormat::Jpg,
            &Destination::ReplaceInput,
            |_| false,
        )
        .expect("target");
        touch(&target.written, b"converted");

        let trash = dir.path().join("Trash");
        let (output, _) = swap_in(&target, |_| false, bin(&trash)).expect("replace");
        assert_eq!(output, input);
        assert_eq!(
            std::fs::read(trash.join("photo.jpg")).expect("read"),
            b"original"
        );
        assert_eq!(std::fs::read(&input).expect("read"), b"converted");
        assert!(!target.written.exists());
    }

    #[test]
    fn a_name_taken_meanwhile_moves_the_result_along() {
        let dir = tempfile::tempdir().expect("temp dir");
        let input = dir.path().join("clip.mov");
        touch(&input, b"original");
        let target = output_target(
            &input,
            OutputFormat::Mp4,
            &Destination::ReplaceInput,
            |_| false,
        )
        .expect("target");
        touch(&target.written, b"converted");
        touch(&target.output, b"appeared while converting");

        let trash = dir.path().join("Trash");
        let (output, _) = swap_in(&target, |_| false, bin(&trash)).expect("replace");
        assert_eq!(output, dir.path().join("clip 2.mp4"));
        assert_eq!(
            std::fs::read(&target.output).expect("read"),
            b"appeared while converting"
        );
    }

    #[test]
    fn an_empty_result_leaves_the_original_alone() {
        let dir = tempfile::tempdir().expect("temp dir");
        let input = dir.path().join("clip.mov");
        touch(&input, b"original");
        let target = output_target(
            &input,
            OutputFormat::Mp4,
            &Destination::ReplaceInput,
            |_| false,
        )
        .expect("target");

        let trash = dir.path().join("Trash");
        // Missing altogether.
        assert!(swap_in(&target, |_| false, bin(&trash)).is_err());
        // Empty.
        touch(&target.written, b"");
        let err = swap_in(&target, |_| false, bin(&trash)).expect_err("empty");
        assert!(err.to_string().contains("empty"), "{err}");
        assert!(!target.written.exists(), "the empty file is cleaned up");
        assert_eq!(std::fs::read(&input).expect("read"), b"original");
        assert!(!trash.exists(), "nothing went to the Trash");
    }

    #[test]
    fn an_original_that_cant_be_moved_stays_put() {
        let dir = tempfile::tempdir().expect("temp dir");
        let input = dir.path().join("clip.mov");
        touch(&input, b"original");
        let target = Target {
            // The rename fallback fails: the original is "gone" already.
            replaces: Some(dir.path().join("vanished.mov")),
            ..output_target(
                &input,
                OutputFormat::Mp4,
                &Destination::ReplaceInput,
                |_| false,
            )
            .expect("target")
        };
        touch(&target.written, b"converted");
        let err = swap_in(&target, |_| false, no_trash).expect_err("can't move");
        assert!(err.to_string().contains("was kept"), "{err}");
        assert!(!target.written.exists());
        assert!(!target.output.exists());
        assert_eq!(std::fs::read(&input).expect("read"), b"original");
    }

    #[test]
    fn kept_originals_get_a_free_name() {
        let dir = tempfile::tempdir().expect("temp dir");
        let input = dir.path().join("clip.mov");
        touch(&dir.path().join("clip.orig.mov"), b"x");
        assert_eq!(
            kept_original_path(&input, |_| false),
            dir.path().join("clip.orig 2.mov")
        );
        let bare = dir.path().join("README");
        assert_eq!(
            kept_original_path(&bare, |_| false),
            dir.path().join("README.orig")
        );
    }
}
