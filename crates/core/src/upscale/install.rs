//! Installing the upscaler and its models, from the internet or from files.
//!
//! Three files make a complete install, each recognised by its SHA-256:
//!
//! | File                                    | From                         | Gives                          |
//! | --------------------------------------- | ---------------------------- | ------------------------------ |
//! | `catconvert-upscaler-<target>.zip`      | the CatConvert release       | the upscaler and its licenses  |
//! | `realesrgan-ncnn-vulkan-20220424-ubuntu.zip` | the Real-ESRGAN release | x4plus, x4plus-anime, animevideov3 |
//! | `realesr-general-x4v3.pth`              | the Real-ESRGAN release      | general-x4v3, converted here   |
//!
//! Only `models/` is taken from upstream's zip; its program and samples are
//! left alone.

use std::borrow::Cow;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{AiModel, MODELS_DIR, UPSCALER_NAME, Upscaler, UpscalerOrigin, installed_models, pth};
use crate::{Error, Result};

/// The official release the models come from.
const UPSTREAM_RELEASE: &str = "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0";
/// Upstream's ncnn build for Linux, the smallest of the three: its
/// `models/` folder is the same in all of them.
const MODELS_ZIP: &str = "realesrgan-ncnn-vulkan-20220424-ubuntu.zip";
const MODELS_ZIP_SHA256: &str = "e5aa6eb131234b87c0c51f82b89390f5e3e642b7b70f2b9bbe95b6a285a40c96";
const MODELS_ZIP_SIZE: u64 = 46_931_474;
/// `realesr-general-x4v3`, only published as PyTorch weights.
const GENERAL_PTH: &str = "realesr-general-x4v3.pth";
const GENERAL_PTH_SHA256: &str = "8dc7edb9ac80ccdc30c3a5dca6616509367f05fbc184ad95b731f05bece96292";
const GENERAL_PTH_SIZE: u64 = 4_885_111;

/// The model files taken from [`MODELS_ZIP`] (as `models/<name>`), with their
/// SHA-256. Its 2× and 3× animevideov3 files are left out: they are the 4×
/// network with a bicubic shrink, which CatConvert does with Lanczos instead.
const ZIP_MODELS: [(&str, &str); 6] = [
    (
        "realesr-animevideov3-x4.bin",
        "548a36f9c3f4ab8da56cd3b13badf23968bee207b396dad14d04b830e5f2ab2d",
    ),
    (
        "realesr-animevideov3-x4.param",
        "850a248e7c14c27e5bd8cf7265113a9441036a7db63963bb8aa5169d788a435e",
    ),
    (
        "realesrgan-x4plus-anime.bin",
        "fe01c269cfd10cdef8e018ab66ebe750cf79c7af4d1f9c16c737e1295229bacc",
    ),
    (
        "realesrgan-x4plus-anime.param",
        "2b8fb6e0ae4d2d85704ca08c119a2f5ea40add4f2ecd512eb7f4cd44b6127ed4",
    ),
    (
        "realesrgan-x4plus.bin",
        "713ee713b0353afaa27976f0563a64a5043bd70b9bd8936c2e26e25ebcdbcddf",
    ),
    (
        "realesrgan-x4plus.param",
        "35330ececcea33b6c397a72548e788d5d53becee4734c50b7fada36e89f10a86",
    ),
];

/// SHA-256 of what [`pth::compact_to_ncnn`] makes of [`GENERAL_PTH`]: the
/// `.param` and the `.bin`. The conversion is exact, so anything else is a bug.
const GENERAL_CONVERTED: (&str, &str) = (
    "49211143467730a613159ed76416c8f31b65aa9931650080a479afebe8337ec5",
    "8fdd6052f8931091164a570ee7b5c5b2d1dcd991112d464eb9ae1a5b52ece553",
);

/// Where CatConvert's releases live.
const RELEASES: &str = "https://github.com/Jotrorox/catconvert/releases/download";

/// The build of the upscaler that runs here, as named in release assets, or
/// `None` where there is none.
const PACK_TARGET: Option<&str> = if cfg!(target_os = "macos") {
    Some("universal-apple-darwin")
} else if cfg!(all(windows, target_arch = "x86_64")) {
    Some("x86_64-pc-windows-msvc")
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some("x86_64-unknown-linux-gnu")
} else {
    None
};

/// Record of what was installed, next to the upscaler.
const RECORD: &str = "installed.json";
/// The models' license and credits, written next to them.
const MODELS_LICENSE: &str = "LICENSE-Real-ESRGAN.txt";
const MODELS_LICENSE_TEXT: &str = include_str!("MODELS-LICENSE.txt");
/// The upscaler's own license texts, in our zip.
const UPSCALER_LICENSES: &str = "THIRD-PARTY-LICENSES.txt";

/// Largest file taken out of a zip: well above the biggest model (33 MB).
const MAX_ENTRY_BYTES: u64 = 256 * 1024 * 1024;

/// The SHA-256 of this version's upscaler download for this platform,
/// compiled in by the release build (`CATCONVERT_UPSCALER_SHA256`). Local
/// builds have none: they take the upscaler from a file, or download it
/// with the weaker check of [`unpinned_download`].
fn pinned_pack_sha256() -> Option<&'static str> {
    option_env!("CATCONVERT_UPSCALER_SHA256")
        .map(str::trim)
        .filter(|sha| sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// One of the files an install needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadKind {
    /// Our build of the upscaler, from the CatConvert release.
    Upscaler,
    /// Upstream's ncnn zip, for its `models/`.
    Models,
    /// `realesr-general-x4v3.pth`, converted on install.
    GeneralModel,
}

/// Where to fetch one of the files an install needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Download {
    pub kind: DownloadKind,
    pub url: String,
    /// The name to save it under, which [`install_files`] doesn't care about.
    pub file_name: String,
    /// Expected SHA-256, lower-case hex.
    pub sha256: String,
    /// Size in bytes, where known in advance.
    pub size: Option<u64>,
}

/// Whether this build can download the upscaler: only release builds know
/// the SHA-256 to check it against.
#[must_use]
pub fn can_download() -> bool {
    PACK_TARGET.is_some() && pinned_pack_sha256().is_some()
}

/// Where a build without a compiled-in SHA-256 can fetch the upscaler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnpinnedDownload {
    pub url: String,
    /// The name to save it under.
    pub file_name: String,
    /// The `.sha256` published next to it, for [`matches_checksum`].
    pub checksum_url: String,
}

/// The upscaler for a build made from source, which has no SHA-256 of it
/// compiled in: the zip in the release of this version, and the checksum
/// published with it. That checksum comes from the same place as the file, so
/// it shows a damaged download but not a swapped one; such builds already
/// take any upscaler zip the user picks (see [`install_files`]). `None` in
/// release builds, which check against their own SHA-256 ([`downloads`]), and
/// where no upscaler is built for this platform.
#[must_use]
pub fn unpinned_download() -> Option<UnpinnedDownload> {
    if pinned_pack_sha256().is_some() {
        return None;
    }
    let file_name = format!("{UPSCALER_NAME}-{}.zip", PACK_TARGET?);
    let url = format!("{RELEASES}/v{}/{file_name}", env!("CARGO_PKG_VERSION"));
    Some(UnpinnedDownload {
        checksum_url: format!("{url}.sha256"),
        url,
        file_name,
    })
}

/// Whether `file` has the SHA-256 that `checksum` starts with: the text of a
/// `.sha256` file as `shasum` writes it, the hash and then the file's name.
///
/// # Errors
///
/// [`Error::Io`] if the file can't be read.
pub fn matches_checksum(file: &Path, checksum: &str) -> Result<bool> {
    let expected = checksum.split_whitespace().next().unwrap_or_default();
    let well_formed = expected.len() == 64 && expected.bytes().all(|b| b.is_ascii_hexdigit());
    Ok(well_formed && sha256_file(file)?.eq_ignore_ascii_case(expected))
}

/// Every file a complete install fetches, the upscaler only where
/// [`can_download`].
#[must_use]
pub fn downloads() -> Vec<Download> {
    let pack = PACK_TARGET.zip(pinned_pack_sha256()).map(|(target, sha)| {
        let file_name = format!("{UPSCALER_NAME}-{target}.zip");
        Download {
            kind: DownloadKind::Upscaler,
            url: format!("{RELEASES}/v{}/{file_name}", env!("CARGO_PKG_VERSION")),
            file_name,
            sha256: sha.to_ascii_lowercase(),
            size: None,
        }
    });
    pack.into_iter()
        .chain([
            Download {
                kind: DownloadKind::Models,
                url: format!("{UPSTREAM_RELEASE}/{MODELS_ZIP}"),
                file_name: MODELS_ZIP.to_owned(),
                sha256: MODELS_ZIP_SHA256.to_owned(),
                size: Some(MODELS_ZIP_SIZE),
            },
            Download {
                kind: DownloadKind::GeneralModel,
                url: format!("{UPSTREAM_RELEASE}/{GENERAL_PTH}"),
                file_name: GENERAL_PTH.to_owned(),
                sha256: GENERAL_PTH_SHA256.to_owned(),
                size: Some(GENERAL_PTH_SIZE),
            },
        ])
        .collect()
}

/// The state of AI upscaling, for a settings page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpscalerStatus {
    /// The install folder looked at.
    pub dir: PathBuf,
    /// The upscaler that conversions use, if any.
    pub binary: Option<PathBuf>,
    /// Where that upscaler was found.
    pub origin: Option<UpscalerOrigin>,
    /// The models it can use.
    pub models: Vec<AiModel>,
    /// What the install folder still lacks.
    pub missing: Vec<DownloadKind>,
    /// The installed upscaler is not the one this version of CatConvert
    /// comes with, so downloading again updates it.
    pub outdated: bool,
    /// See [`can_download`].
    pub can_download: bool,
    /// Bytes still to download for a complete install, as far as known.
    pub download_bytes: u64,
}

/// What is installed in `dir`, and which upscaler conversions would use.
#[must_use]
pub fn status(dir: &Path) -> UpscalerStatus {
    let found = Upscaler::locate(Some(dir));
    let has_binary = crate::ffmpeg::executable(dir, UPSCALER_NAME).is_some();
    let models = installed_models(&dir.join(MODELS_DIR));
    let mut missing = Vec::new();
    if !has_binary {
        missing.push(DownloadKind::Upscaler);
    }
    if [AiModel::Photo, AiModel::Anime, AiModel::AnimeFast]
        .into_iter()
        .any(|model| !models.contains(model))
    {
        missing.push(DownloadKind::Models);
    }
    if !models.contains(AiModel::General) {
        missing.push(DownloadKind::GeneralModel);
    }
    let outdated = has_binary
        && pinned_pack_sha256().is_some_and(|pinned| {
            read_record(dir)
                .upscaler
                .is_none_or(|sha| !sha.eq_ignore_ascii_case(pinned))
        });
    let download_bytes = downloads()
        .iter()
        .filter(|d| missing.contains(&d.kind))
        .filter_map(|d| d.size)
        .sum();
    UpscalerStatus {
        dir: dir.to_path_buf(),
        binary: found.as_ref().map(|u| u.binary().to_path_buf()),
        origin: found.as_ref().map(Upscaler::origin),
        models: found
            .map(|u| u.models().iter().collect())
            .unwrap_or_default(),
        missing,
        outdated,
        can_download: can_download(),
        download_bytes,
    }
}

/// What [`RECORD`] holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Record {
    /// SHA-256 of the upscaler zip that was installed.
    upscaler: Option<String>,
}

fn read_record(dir: &Path) -> Record {
    std::fs::read(dir.join(RECORD))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Install from downloaded files into `dir`: any of the three files in the
/// module docs, recognised by their SHA-256 whatever they are called. Each
/// file installs its part, replacing an older copy; the rest stays.
///
/// Returns what was installed, in the order of `files`.
///
/// # Errors
///
/// [`Error::Unsupported`] for a file that isn't one of them (or is damaged),
/// [`Error::Io`] if something can't be read or written. Nothing is changed
/// in `dir` unless every file is good.
pub fn install_files(dir: &Path, files: &[PathBuf]) -> Result<Vec<DownloadKind>> {
    std::fs::create_dir_all(dir).map_err(io_error(dir))?;
    let staging = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(dir)
        .map_err(io_error(dir))?;
    let models = staging.path().join(MODELS_DIR);
    std::fs::create_dir(&models).map_err(io_error(&models))?;

    let mut installed = Vec::new();
    let mut record = read_record(dir);
    for file in files {
        let sha = sha256_file(file)?;
        let kind = identify(file, &sha)?;
        match kind {
            DownloadKind::Models | DownloadKind::GeneralModel => {
                if kind == DownloadKind::Models {
                    unpack_models(file, &models)?;
                } else {
                    convert_general(file, &models)?;
                }
                write_file(&models.join(MODELS_LICENSE), MODELS_LICENSE_TEXT.as_bytes())?;
            }
            DownloadKind::Upscaler => {
                unpack_upscaler(file, staging.path())?;
                record.upscaler = Some(sha);
            }
        }
        installed.push(kind);
    }

    // Everything checked out: move it into place.
    let target_models = dir.join(MODELS_DIR);
    std::fs::create_dir_all(&target_models).map_err(io_error(&target_models))?;
    for (from_dir, to_dir) in [
        (&models, &target_models),
        (&staging.path().to_path_buf(), &dir.to_path_buf()),
    ] {
        for entry in std::fs::read_dir(from_dir).map_err(io_error(from_dir))? {
            let entry = entry.map_err(io_error(from_dir))?;
            let from = entry.path();
            if from.is_dir() {
                continue;
            }
            let to = to_dir.join(entry.file_name());
            std::fs::rename(&from, &to).map_err(io_error(&to))?;
        }
    }
    if installed.contains(&DownloadKind::Upscaler) {
        let json = serde_json::to_vec_pretty(&record).unwrap_or_default();
        let path = dir.join(RECORD);
        std::fs::write(&path, json).map_err(io_error(&path))?;
    }
    Ok(installed)
}

/// The license texts of what is installed in `dir`: the upscaler's and the
/// models', for an app's open-source notices. `None` if nothing is installed.
#[must_use]
pub fn licenses(dir: &Path) -> Option<String> {
    let texts: Vec<String> = [
        dir.join(UPSCALER_LICENSES),
        dir.join(MODELS_DIR).join(MODELS_LICENSE),
    ]
    .iter()
    .filter_map(|path| std::fs::read_to_string(path).ok())
    .collect();
    (!texts.is_empty()).then(|| texts.join("\n\n"))
}

/// Remove everything [`install_files`] put in `dir`.
///
/// # Errors
///
/// [`Error::Io`] if the folder can't be removed.
pub fn uninstall(dir: &Path) -> Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(Error::Io {
            path: dir.to_path_buf(),
            source: err,
        }),
        _ => Ok(()),
    }
}

/// Which of the install files `file` is, by its SHA-256 `sha`.
fn identify(file: &Path, sha: &str) -> Result<DownloadKind> {
    if sha == MODELS_ZIP_SHA256 {
        return Ok(DownloadKind::Models);
    }
    if sha == GENERAL_PTH_SHA256 {
        return Ok(DownloadKind::GeneralModel);
    }
    match pinned_pack_sha256() {
        Some(pinned) if sha.eq_ignore_ascii_case(pinned) => return Ok(DownloadKind::Upscaler),
        // Local builds can't know the hash of a build they didn't make, so
        // they take any upscaler zip the user picks.
        None if is_upscaler_zip(file) => return Ok(DownloadKind::Upscaler),
        _ => {}
    }
    let name = file.file_name().map_or_else(
        || file.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let looks_like_ours = name.starts_with(UPSCALER_NAME);
    Err(Error::unsupported(if looks_like_ours {
        format!(
            "{name} isn't the upscaler made for this version of CatConvert ({}), or the download \
             is damaged",
            env!("CARGO_PKG_VERSION")
        )
    } else {
        format!(
            "{name} isn't one of the files AI upscaling installs from ({UPSCALER_NAME}-…zip, \
             {MODELS_ZIP} or {GENERAL_PTH}), or it is damaged"
        )
    }))
}

/// Whether `file` is a zip that holds our upscaler.
fn is_upscaler_zip(file: &Path) -> bool {
    let binary = format!("{UPSCALER_NAME}{}", std::env::consts::EXE_SUFFIX);
    File::open(file)
        .ok()
        .and_then(|f| zip::ZipArchive::new(f).ok())
        .is_some_and(|zip| {
            zip.file_names()
                .flatten()
                .any(|name| name == binary.as_str())
        })
}

/// Take the models out of upstream's zip into `models`, checking each.
fn unpack_models(file: &Path, models: &Path) -> Result<()> {
    let mut zip = open_zip(file)?;
    for (name, sha) in ZIP_MODELS {
        let entry = format!("models/{name}");
        let bytes = read_entry(&mut zip, &entry, file)?;
        if hex_sha256(&bytes) != sha {
            return Err(damaged(file));
        }
        write_file(&models.join(name), &bytes)?;
    }
    Ok(())
}

/// Convert `realesr-general-x4v3.pth` into ncnn files in `models`.
fn convert_general(file: &Path, models: &Path) -> Result<()> {
    let bytes = std::fs::read(file).map_err(io_error(file))?;
    let converted = pth::compact_to_ncnn(&bytes).map_err(|reason| {
        Error::unsupported(format!("couldn't convert {}: {reason}", file.display()))
    })?;
    if (
        hex_sha256(converted.param.as_bytes()).as_str(),
        hex_sha256(&converted.bin).as_str(),
    ) != GENERAL_CONVERTED
    {
        return Err(Error::unsupported(format!(
            "converting {} gave an unexpected result",
            file.display()
        )));
    }
    let [param, bin] = AiModel::General.files();
    write_file(&models.join(param), converted.param.as_bytes())?;
    write_file(&models.join(bin), &converted.bin)
}

/// Take the upscaler and its license files out of our zip into `dir`.
fn unpack_upscaler(file: &Path, dir: &Path) -> Result<()> {
    let binary = format!("{UPSCALER_NAME}{}", std::env::consts::EXE_SUFFIX);
    let mut zip = open_zip(file)?;
    let names: Vec<String> = zip.file_names().flatten().map(Cow::into_owned).collect();
    if !names.contains(&binary) {
        return Err(Error::unsupported(format!(
            "{} has no {binary} for this computer",
            file.display()
        )));
    }
    for name in names {
        // Only plain files at the top: nothing can land outside `dir`.
        let plain = !name.is_empty()
            && !name.starts_with('.')
            && !name.contains(['/', '\\', ':'])
            && name != RECORD;
        if !plain {
            continue;
        }
        let bytes = read_entry(&mut zip, &name, file)?;
        let path = dir.join(&name);
        write_file(&path, &bytes)?;
        if name == binary {
            make_executable(&path)?;
        }
    }
    Ok(())
}

fn open_zip(file: &Path) -> Result<zip::ZipArchive<File>> {
    let f = File::open(file).map_err(io_error(file))?;
    zip::ZipArchive::new(f).map_err(|_| damaged(file))
}

fn read_entry(zip: &mut zip::ZipArchive<File>, name: &str, file: &Path) -> Result<Vec<u8>> {
    let entry = zip.by_name(name).map_err(|_| damaged(file))?;
    if entry.size() > MAX_ENTRY_BYTES {
        return Err(damaged(file));
    }
    let mut bytes = Vec::new();
    entry
        .take(MAX_ENTRY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| damaged(file))?;
    if bytes.len() as u64 > MAX_ENTRY_BYTES {
        return Err(damaged(file));
    }
    Ok(bytes)
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = File::create(path).map_err(io_error(path))?;
    f.write_all(bytes).map_err(io_error(path))?;
    f.sync_all().map_err(io_error(path))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(io_error(path))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn damaged(file: &Path) -> Error {
    Error::unsupported(format!(
        "{} is damaged or incomplete: download it again",
        file.display()
    ))
}

/// The SHA-256 of a file, as lower-case hex.
///
/// # Errors
///
/// [`Error::Io`] if the file can't be read.
pub(crate) fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(io_error(path))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf).map_err(io_error(path))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex_sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn io_error(path: &Path) -> impl Fn(std::io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    fn zip_with(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(File::create(path).expect("create"));
        for (name, bytes) in entries {
            zip.start_file(
                *name,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
            )
            .expect("entry");
            zip.write_all(bytes).expect("write");
        }
        zip.finish().expect("finish");
    }

    #[test]
    fn hashes_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("abc");
        std::fs::write(&path, b"abc").expect("write");
        assert_eq!(
            sha256_file(&path).expect("hash"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn checks_a_file_against_a_published_checksum() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("abc");
        std::fs::write(&path, b"abc").expect("write");
        let sha = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(matches_checksum(&path, &format!("{sha}  abc\n")).expect("hash"));
        assert!(matches_checksum(&path, &sha.to_ascii_uppercase()).expect("hash"));
        let other = sha.replace('b', "c");
        assert!(!matches_checksum(&path, &format!("{other}  abc\n")).expect("hash"));
        // Not a checksum at all, e.g. an error page.
        assert!(!matches_checksum(&path, "").expect("hash"));
        assert!(!matches_checksum(&path, "<html>Not Found</html>").expect("hash"));
    }

    #[test]
    fn only_builds_without_a_pinned_upscaler_download_it_unpinned() {
        let unpinned = unpinned_download();
        assert_eq!(
            unpinned.is_some(),
            PACK_TARGET.is_some() && pinned_pack_sha256().is_none()
        );
        if let Some(download) = unpinned {
            assert!(download.url.starts_with(RELEASES));
            assert!(download.url.ends_with(&download.file_name));
            assert_eq!(download.checksum_url, format!("{}.sha256", download.url));
        }
    }

    #[test]
    fn lists_downloads_from_the_official_release() {
        let all = downloads();
        let models = all
            .iter()
            .find(|d| d.kind == DownloadKind::Models)
            .expect("models");
        assert!(
            models
                .url
                .starts_with("https://github.com/xinntao/Real-ESRGAN/releases/")
        );
        assert_eq!(models.size, Some(MODELS_ZIP_SIZE));
        assert!(all.iter().any(|d| d.kind == DownloadKind::GeneralModel));
        assert_eq!(
            all.iter().any(|d| d.kind == DownloadKind::Upscaler),
            can_download()
        );
    }

    #[test]
    fn refuses_unknown_and_damaged_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let install = dir.path().join("upscaler");
        let junk = dir.path().join("holiday.jpg");
        std::fs::write(&junk, b"not a model").expect("write");
        let err = install_files(&install, &[junk]).expect_err("unknown");
        assert!(err.to_string().contains("isn't one of the files"), "{err}");
        // Nothing half-installed is left behind.
        assert!(status(&install).models.is_empty());
        assert_eq!(std::fs::read_dir(&install).expect("dir").count(), 0);
    }

    #[test]
    fn installs_an_upscaler_zip_in_local_builds() {
        if pinned_pack_sha256().is_some() {
            return;
        }
        let dir = tempfile::tempdir().expect("temp dir");
        let install = dir.path().join("upscaler");
        let binary = format!("{UPSCALER_NAME}{}", std::env::consts::EXE_SUFFIX);
        let pack = dir.path().join("catconvert-upscaler-test.zip");
        zip_with(
            &pack,
            &[
                (binary.as_str(), b"#!/bin/sh\n"),
                ("THIRD-PARTY-LICENSES.txt", b"MIT"),
                ("../escape.txt", b"no"),
                ("sub/dir.txt", b"no"),
            ],
        );
        let installed = install_files(&install, &[pack]).expect("install");
        assert_eq!(installed, [DownloadKind::Upscaler]);
        assert!(install.join(&binary).is_file());
        assert!(install.join("THIRD-PARTY-LICENSES.txt").is_file());
        assert!(!dir.path().join("escape.txt").exists());
        assert!(!install.join("sub").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(install.join(&binary))
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111);
        }
        let state = status(&install);
        assert!(!state.missing.contains(&DownloadKind::Upscaler));
        assert!(state.missing.contains(&DownloadKind::Models));
        // No staging folders are left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&install)
            .expect("dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".install-"))
            .collect();
        assert!(leftovers.is_empty());

        uninstall(&install).expect("remove");
        assert!(!install.exists());
        uninstall(&install).expect("removing twice is fine");
    }

    #[test]
    fn a_zip_without_the_upscaler_is_not_one() {
        let dir = tempfile::tempdir().expect("temp dir");
        let other = dir.path().join("catconvert-upscaler-other.zip");
        zip_with(&other, &[("readme.txt", b"hi")]);
        assert!(!is_upscaler_zip(&other));
        let err = install_files(&dir.path().join("u"), &[other]).expect_err("refused");
        assert!(err.to_string().contains("isn't the upscaler"), "{err}");
    }
}
