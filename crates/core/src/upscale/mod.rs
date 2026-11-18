//! AI upscaling: Real-ESRGAN models run by `realesrgan-ncnn-vulkan` on the
//! graphics card, as an optional download.
//!
//! Nothing here ships with the app. The user installs it from the app's
//! settings (or with `catconvert upscaler install`), which fetches:
//!
//! - **our build of the upscaler**, `catconvert-upscaler`, attached to the
//!   CatConvert release with the same version (see [`downloads`]), checked
//!   against the SHA-256 compiled into this build;
//! - **the model weights, straight from xinntao's official Real-ESRGAN
//!   release**, checked against the SHA-256s below. CatConvert never hosts
//!   them. `realesr-general-x4v3` is only published for PyTorch, so it is
//!   converted to ncnn's format on the user's machine (see [`pth`]).
//!
//! The same files can be installed from disk for machines without internet
//! ([`install_files`]). [`Upscaler::locate`] finds the result, or a tool named
//! by [`UPSCALER_ENV`], or `realesrgan-ncnn-vulkan` on `PATH`.
//!
//! [`crate::plan`] turns [`crate::ConvertOptions::ai`] into three steps for a
//! still picture: ffmpeg decodes, turns and crops it into a PNG, the upscaler
//! enlarges that 4×, and ffmpeg scales the result to the exact size asked for
//! with Lanczos and encodes it. Videos and animations keep using the classic
//! scaler.

mod install;
mod pth;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use install::{
    Download, DownloadKind, UnpinnedDownload, UpscalerStatus, can_download, downloads,
    install_files, licenses, matches_checksum, status, uninstall, unpinned_download,
};

/// Environment variable naming an upscaler binary (our
/// `catconvert-upscaler` or upstream `realesrgan-ncnn-vulkan`) to use
/// before the installed one. Its models are taken from the installed
/// folder, or a `models` folder next to the binary.
pub const UPSCALER_ENV: &str = "CATCONVERT_UPSCALER";

/// File name of our build of the upscaler, without `.exe`.
pub(crate) const UPSCALER_NAME: &str = "catconvert-upscaler";
/// Upstream's name for the same program, looked for on `PATH`.
const UPSTREAM_NAME: &str = "realesrgan-ncnn-vulkan";
/// Folder of the installed models inside the install folder.
pub(crate) const MODELS_DIR: &str = "models";
/// The desktop app's identifier, which names its data folder.
const APP_ID: &str = "dev.jotrorox.catconvert";

/// What the plan reports when a conversion asks for AI upscaling that isn't
/// installed.
pub(crate) const NOT_INSTALLED: &str = "AI upscaling is not installed";

/// What every model enlarges by in its one pass. Real-ESRGAN's docs warn
/// that asking a 4× model for less gives poor results, and upstream's 2× and
/// 3× animevideov3 files are the 4× network with a bicubic shrink at the end,
/// so other sizes are made from the 4× result with Lanczos.
pub const MAX_AI_SCALE: u32 = 4;

/// Largest picture the upscaler is asked to write: 8192 × 8192 pixels, about
/// 200 MB of RGB in memory. A bigger result is enlarged from a smaller copy
/// of the picture instead (see [`crate::plan`]).
pub const MAX_AI_PIXELS: u64 = 8192 * 8192;

/// A Real-ESRGAN model.
///
/// In JSON: `"general"`, `"photo"`, `"anime"` or `"anime-fast"`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AiModel {
    /// `realesr-general-x4v3`: photos and real-world pictures. Small and
    /// quick, with natural detail.
    #[default]
    General,
    /// `realesrgan-x4plus`: photos, with the most detail, but several times
    /// slower than [`AiModel::General`].
    Photo,
    /// `realesrgan-x4plus-anime`: drawings, illustrations and anime.
    Anime,
    /// `realesr-animevideov3`: anime, very quick.
    AnimeFast,
}

impl AiModel {
    /// Every model, in the order the UI lists them.
    pub const ALL: [Self; 4] = [Self::General, Self::Photo, Self::Anime, Self::AnimeFast];

    /// The short name used in JSON and on the command line, e.g. `"general"`.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Photo => "photo",
            Self::Anime => "anime",
            Self::AnimeFast => "anime-fast",
        }
    }

    /// The model with this [`AiModel::id`].
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|model| model.id() == id)
    }

    /// What the model is for, for menus.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::General => "Photos",
            Self::Photo => "Photos, most detail (slow)",
            Self::Anime => "Drawings and anime",
            Self::AnimeFast => "Anime, quick",
        }
    }

    /// Upstream's name of the model, which `-n` takes.
    #[must_use]
    pub fn upstream_name(self) -> &'static str {
        match self {
            Self::General => "realesr-general-x4v3",
            Self::Photo => "realesrgan-x4plus",
            Self::Anime => "realesrgan-x4plus-anime",
            Self::AnimeFast => "realesr-animevideov3",
        }
    }

    /// File name of the model's weights, without extension. The upscaler
    /// looks for animevideov3 with the scale appended, as `-x4`, and for
    /// every other model under its plain name.
    fn stem(self) -> String {
        match self {
            Self::AnimeFast => format!("{}-x{MAX_AI_SCALE}", self.upstream_name()),
            _ => self.upstream_name().to_owned(),
        }
    }

    /// Every file the model needs in the models folder. The upscaler crashes
    /// on a missing one, so [`Upscaler::locate`] only offers complete models.
    pub(crate) fn files(self) -> [String; 2] {
        let stem = self.stem();
        [format!("{stem}.param"), format!("{stem}.bin")]
    }

    /// Rough seconds per megapixel of the picture going in, on a fast and on
    /// a slow graphics card. An Apple M5 takes about 2.3 s (general), 16 s
    /// (photo), 6.6 s (anime) and 1.8 s (anime-fast); a fast desktop card
    /// about half that, and a laptop's built-in graphics several times more.
    fn secs_per_megapixel(self) -> (f64, f64) {
        match self {
            Self::General => (1.2, 9.0),
            Self::Photo => (8.0, 64.0),
            Self::Anime => (3.3, 26.0),
            Self::AnimeFast => (0.9, 7.0),
        }
    }
}

/// How long enlarging a picture of `megapixels` (going in) with `model`
/// roughly takes, in seconds: a range from a fast to a slow graphics card.
#[must_use]
pub fn estimate_secs(model: AiModel, megapixels: f64) -> (f64, f64) {
    let (fast, slow) = model.secs_per_megapixel();
    let mp = megapixels.max(0.0);
    // Starting the GPU and loading the model takes a moment of its own.
    (0.5 + fast * mp, 2.0 + slow * mp)
}

/// A set of [`AiModel`]s, e.g. the ones that are installed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct AiModels(u8);

impl AiModels {
    /// No models.
    pub const NONE: Self = Self(0);

    fn bit(model: AiModel) -> u8 {
        match model {
            AiModel::General => 1,
            AiModel::Photo => 2,
            AiModel::Anime => 4,
            AiModel::AnimeFast => 8,
        }
    }

    /// Whether `model` is in the set.
    #[must_use]
    pub fn contains(self, model: AiModel) -> bool {
        self.0 & Self::bit(model) != 0
    }

    /// The set with `model` added.
    #[must_use]
    pub fn with(self, model: AiModel) -> Self {
        Self(self.0 | Self::bit(model))
    }

    /// Whether the set is empty.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The models in the set, in [`AiModel::ALL`] order.
    pub fn iter(self) -> impl Iterator<Item = AiModel> {
        AiModel::ALL.into_iter().filter(move |&m| self.contains(m))
    }
}

impl FromIterator<AiModel> for AiModels {
    fn from_iter<I: IntoIterator<Item = AiModel>>(iter: I) -> Self {
        iter.into_iter().fold(Self::NONE, Self::with)
    }
}

/// Where the upscaler in use was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpscalerOrigin {
    /// Installed by CatConvert (see [`install_files`]).
    Installed,
    /// Named by [`UPSCALER_ENV`].
    Env,
    /// `realesrgan-ncnn-vulkan` found on `PATH`.
    Path,
}

/// An upscaler binary and the models it can use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upscaler {
    binary: PathBuf,
    models_dir: PathBuf,
    models: AiModels,
    origin: UpscalerOrigin,
}

impl Upscaler {
    /// Find the upscaler: the binary named by [`UPSCALER_ENV`], then the one
    /// installed in `installed` (see [`default_dir`]), then
    /// `realesrgan-ncnn-vulkan` on `PATH`. Models come from `installed`, or a
    /// `models` folder next to the binary. `None` if no binary is found, or
    /// no model is installed for it.
    #[must_use]
    pub fn locate(installed: Option<&Path>) -> Option<Self> {
        let env = std::env::var_os(UPSCALER_ENV)
            .map(PathBuf::from)
            .filter(|path| path.is_file())
            .map(|path| (path, UpscalerOrigin::Env));
        let own = installed
            .and_then(|dir| crate::ffmpeg::executable(dir, UPSCALER_NAME))
            .map(|path| (path, UpscalerOrigin::Installed));
        let path = || {
            std::env::var_os("PATH")
                .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
                .unwrap_or_default()
                .into_iter()
                .find_map(|dir| crate::ffmpeg::executable(&dir, UPSTREAM_NAME))
                .map(|path| (path, UpscalerOrigin::Path))
        };
        let (binary, origin) = env.or(own).or_else(path)?;
        let candidates = installed
            .map(|dir| dir.join(MODELS_DIR))
            .into_iter()
            .chain(binary.parent().map(|dir| dir.join(MODELS_DIR)));
        candidates
            .map(|dir| (installed_models(&dir), dir))
            .find(|(models, _)| !models.is_empty())
            .map(|(models, models_dir)| Self {
                binary,
                models_dir,
                models,
                origin,
            })
    }

    /// An upscaler at explicit paths, e.g. for tests.
    #[must_use]
    pub fn at(binary: impl Into<PathBuf>, models_dir: impl Into<PathBuf>) -> Self {
        let models_dir = models_dir.into();
        Self {
            binary: binary.into(),
            models: installed_models(&models_dir),
            models_dir,
            origin: UpscalerOrigin::Env,
        }
    }

    /// Path of the upscaler binary.
    #[must_use]
    pub fn binary(&self) -> &Path {
        &self.binary
    }

    /// Folder the models are loaded from (`-m`).
    #[must_use]
    pub fn models_dir(&self) -> &Path {
        &self.models_dir
    }

    /// The models whose files are all there.
    #[must_use]
    pub fn models(&self) -> AiModels {
        self.models
    }

    /// Where the binary was found.
    #[must_use]
    pub fn origin(&self) -> UpscalerOrigin {
        self.origin
    }
}

/// The models whose files are all in `dir`.
pub(crate) fn installed_models(dir: &Path) -> AiModels {
    AiModel::ALL
        .into_iter()
        .filter(|model| model.files().iter().all(|file| dir.join(file).is_file()))
        .collect()
}

/// Where the desktop app installs the upscaler, and where the CLI looks for
/// it: `upscaler` in the app's data folder (`~/Library/Application Support/
/// dev.jotrorox.catconvert` on macOS, `%APPDATA%\dev.jotrorox.catconvert` on
/// Windows, `$XDG_DATA_HOME/dev.jotrorox.catconvert` or
/// `~/.local/share/dev.jotrorox.catconvert` elsewhere).
#[must_use]
pub fn default_dir() -> Option<PathBuf> {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home()?.join("Library/Application Support")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .or_else(|| Some(home()?.join(".local/share")))?
    };
    Some(base.join(APP_ID).join("upscaler"))
}

/// The completion in a progress line of the upscaler, which prints
/// `12.50%` on stderr per finished tile.
pub(crate) fn parse_progress(line: &str) -> Option<f32> {
    let percent: f32 = line.trim().strip_suffix('%')?.parse().ok()?;
    (percent.is_finite() && (0.0..=100.0).contains(&percent)).then_some(percent / 100.0)
}

/// The graphics cards the upscaler lists while starting, e.g.
/// `[0 Apple M5]  queueC=0[1] …` → `Apple M5`.
pub(crate) fn gpu_names(stderr: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in stderr.lines() {
        let Some(rest) = line.trim_start().strip_prefix('[') else {
            continue;
        };
        let Some((inside, _)) = rest.split_once(']') else {
            continue;
        };
        let Some((index, name)) = inside.split_once(' ') else {
            continue;
        };
        if index.parse::<u32>().is_ok() && !name.trim().is_empty() {
            let name = name.trim().to_owned();
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// A message for the user from the upscaler's error output, if it says
/// something recognisable.
pub(crate) fn friendly_error(stderr: &str) -> Option<String> {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("invalid gpu device") || lower.contains("vkcreateinstance failed") {
        return Some(no_gpu_message().to_owned());
    }
    if lower.contains("fopen") || lower.contains("load_param") || lower.contains("load_model") {
        return Some(
            "the AI model files are missing or damaged: reinstall AI upscaling in the settings"
                .to_owned(),
        );
    }
    if lower.contains("decode image") {
        return Some("the upscaler couldn't read the picture".to_owned());
    }
    if lower.contains("encode image") {
        return Some("the upscaler couldn't write the enlarged picture".to_owned());
    }
    stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && parse_progress(line).is_none())
        .rfind(|line| !line.starts_with('['))
        .map(|line| format!("AI upscaling failed: {line}"))
}

/// What to tell the user when there is no graphics card the upscaler can use.
pub(crate) fn no_gpu_message() -> &'static str {
    "this computer has no graphics card that supports AI upscaling (it needs Vulkan): \
     use classic upscaling instead"
}

/// What to tell the user when the upscaler made a black picture.
pub(crate) const BLACK_OUTPUT: &str = "AI upscaling made a black picture, which usually means \
     the graphics driver is too old: update it, or use classic upscaling";

/// A small picture to try the upscaler on (see [`crate::Ffmpeg::check_upscaler`]).
pub(crate) const PROBE_PNG: &[u8] = include_bytes!("probe.png");

/// Whether the upscaler works on this computer, from a test run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum GpuCheck {
    /// It enlarged a test picture, on `gpu` if it said which.
    Ready { gpu: Option<String> },
    /// There is no graphics card it can use.
    NoGpu,
    /// It ran, but the result was black: usually an old graphics driver.
    BlackOutput { gpu: Option<String> },
    /// It failed for another reason.
    Failed { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_name_their_files_like_upstream() {
        assert_eq!(
            AiModel::Photo.files(),
            ["realesrgan-x4plus.param", "realesrgan-x4plus.bin"]
        );
        assert_eq!(
            AiModel::AnimeFast.files(),
            [
                "realesr-animevideov3-x4.param",
                "realesr-animevideov3-x4.bin",
            ]
        );
        for model in AiModel::ALL {
            assert_eq!(AiModel::from_id(model.id()), Some(model));
            assert_eq!(
                serde_json::to_value(model).expect("json"),
                serde_json::json!(model.id())
            );
        }
        assert_eq!(AiModel::from_id("x4plus"), None);
    }

    #[test]
    fn model_sets() {
        let set: AiModels = [AiModel::Photo, AiModel::AnimeFast].into_iter().collect();
        assert!(set.contains(AiModel::Photo) && !set.contains(AiModel::General));
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            [AiModel::Photo, AiModel::AnimeFast]
        );
        assert!(AiModels::NONE.is_empty() && !set.is_empty());
    }

    #[test]
    fn reads_progress_gpus_and_errors() {
        assert_eq!(parse_progress("25.00%"), Some(0.25));
        assert_eq!(parse_progress(" 100.00%\r"), Some(1.0));
        assert_eq!(parse_progress("[0 Apple M5]  queueC=0[1]"), None);
        assert_eq!(parse_progress("150%"), None);

        let log = "[0 Apple M5]  queueC=0[1]  queueG=0[1]  queueT=0[1]\n\
                   [0 Apple M5]  bugsbn1=0  bugbilz=0\n\
                   [1 NVIDIA GeForce RTX 3060]  queueC=2[8]\n0.00%\n25.00%\n";
        assert_eq!(gpu_names(log), ["Apple M5", "NVIDIA GeForce RTX 3060"]);
        assert!(gpu_names("0.00%\nsomething\n").is_empty());

        assert_eq!(
            friendly_error("[0 Apple M5]  queueC=0[1]\ninvalid gpu device\n").as_deref(),
            Some(no_gpu_message())
        );
        assert!(
            friendly_error("fopen models/x.param failed\n")
                .is_some_and(|m| m.contains("reinstall"))
        );
        assert!(
            friendly_error("decode image /tmp/a.png failed\n")
                .is_some_and(|m| m.contains("couldn't read"))
        );
        assert_eq!(
            friendly_error("0.00%\nout of memory\n25.00%\n").as_deref(),
            Some("AI upscaling failed: out of memory")
        );
        assert_eq!(friendly_error("0.00%\n"), None);
    }

    #[test]
    fn estimates_grow_with_size_and_model() {
        let (fast, slow) = estimate_secs(AiModel::General, 1.0);
        assert!(fast < slow);
        assert!(estimate_secs(AiModel::Photo, 1.0).0 > fast);
        assert!(estimate_secs(AiModel::General, 4.0).1 > slow);
    }

    #[test]
    fn locates_an_installed_upscaler_and_its_models() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert_eq!(Upscaler::locate(Some(dir.path())), None);
        let binary = dir
            .path()
            .join(format!("{UPSCALER_NAME}{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&binary, b"").expect("write");
        let models = dir.path().join(MODELS_DIR);
        std::fs::create_dir(&models).expect("mkdir");
        // A binary without models is no use.
        if std::env::var_os(UPSCALER_ENV).is_none() {
            assert_eq!(Upscaler::locate(Some(dir.path())), None);
        }
        for file in AiModel::Photo.files() {
            std::fs::write(models.join(file), b"").expect("write");
        }
        // Half a model doesn't count.
        std::fs::write(models.join("realesrgan-x4plus-anime.param"), b"").expect("write");
        if std::env::var_os(UPSCALER_ENV).is_none() {
            let found = Upscaler::locate(Some(dir.path())).expect("found");
            assert_eq!(found.binary(), binary);
            assert_eq!(found.models_dir(), models);
            assert_eq!(found.origin(), UpscalerOrigin::Installed);
            assert_eq!(found.models().iter().collect::<Vec<_>>(), [AiModel::Photo]);
        }
    }

    #[test]
    fn default_dir_is_named_after_the_app() {
        if let Some(dir) = default_dir() {
            assert!(dir.ends_with(Path::new(APP_ID).join("upscaler")), "{dir:?}");
        }
    }
}
