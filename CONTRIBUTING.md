# Contributing to CatConvert

Thanks for helping out! This document covers the local workflow and conventions.

## Setup

1. Install [mise](https://mise.jdx.dev) and [rustup](https://rustup.rs).
2. `mise install` installs the pinned Node, pnpm and linters. rustup picks up
   `rust-toolchain.toml` on the first `cargo` call. Don't install these tools
   globally; if something is missing, add it to `mise.toml`.
3. `mise run dev` starts the desktop app.

Only macOS is needed for local development. Windows and Linux builds come from CI.

## Before you push

```sh
mise run fmt     # format everything
mise run check   # what CI runs: fmt-check, clippy -D warnings, biome, tsc, tests
```

## Where code goes

- **Logic** goes in `crates/core` (`catconvert-core`). It must stay free of
  platform, UI and FFI code.
- **Adapters** stay thin: Tauri commands (`apps/desktop/src-tauri`), the CLI
  (`crates/cli`) and UniFFI exports (`crates/ffi`) only convert types and call
  the core.
- **Front end ↔ backend** calls go through `apps/desktop/src/lib/api.ts`. Add a
  command there with its types instead of calling `invoke`/`listen` directly.
- Add tests next to the code: Rust unit tests in the same file, Vitest tests as
  `*.test.ts(x)` beside the component.

## Bundled media tools

Release builds ship `ffmpeg`, `ffprobe` and libjxl's `cjxl`, `djxl` and
`jxlinfo` inside the app as Tauri sidecars, named `catconvert-ffmpeg` and so on.
`scripts/media-tools/build.sh` builds them statically from the sources pinned in
`scripts/media-tools/sources.lock`. The app looks for tools in this order:
`CATCONVERT_FFMPEG` (and `CATCONVERT_LIBJXL`), then the bundled copies, then
`PATH` and Homebrew.

- `mise run dev` and `mise run build` don't include them and use your installed
  ffmpeg.
- `mise run media-tools` builds them for this machine into
  `apps/desktop/src-tauri/binaries` (gitignored). It takes a while and needs
  cmake, meson, ninja, nasm and pkg-config
  (`brew install cmake meson ninja nasm pkgconf`).
  Then `mise run build-bundled` builds the app with them inside.
- To update a component, change its version, URL and SHA-256 in `sources.lock`
  together and open a PR. `.github/workflows/media-tools.yml` builds all three
  platforms when that file changes, and fails if a binary links a non-system
  library, an encoder the app needs is missing, or the tools outgrow their size
  budget.

Licensing: ffmpeg with x264 and x265 is GPL-2.0-or-later. CatConvert stays BSD-3-Clause
because it only runs these tools as separate processes. Don't link ffmpeg's
libraries into the app. Every release must carry the matching source archive,
which the release workflow attaches automatically.

### Optional AI upscaler

AI upscaling runs `catconvert-upscaler`, our build of
[realesrgan-ncnn-vulkan](https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan). It
isn't in the installers: the app downloads it on request from the release of its
own version, as `catconvert-upscaler-<target>.zip`, and checks it against a
SHA-256 compiled into the app and the CLI (`CATCONVERT_UPSCALER_SHA256`, set by
the release workflow).

- `scripts/upscaler/build.sh` builds it from the sources pinned in
  `scripts/upscaler/sources.lock` (realesrgan-ncnn-vulkan, ncnn with its
  glslang, libwebp, and MoltenVK on macOS), with the small patches in
  `scripts/upscaler/patches/`; each patch says why it exists. Upstream's
  command line and messages stay unchanged.
- ncnn's built-in Vulkan loader opens the GPU driver at runtime, so the binary
  links only OS libraries: no Vulkan loader, no OpenMP and no C++ runtime
  (static CRT on Windows, static libstdc++ on Linux, MoltenVK linked in on
  macOS). No Vulkan SDK is needed to build it. Because of MoltenVK, the macOS
  build needs macOS 12.
- `mise run upscaler` builds it for this machine into
  `target/upscaler/dist/<target>`. It needs cmake and ninja
  (`brew install cmake ninja`); on Windows, run it in Git Bash with Visual
  Studio's C++ tools instead of ninja. `build.sh pack <dir> <file.zip>` makes
  the zip the app installs.
- To update a component, change its version, URL and SHA-256 in `sources.lock`
  together and check that the patches still apply. `.github/workflows/upscaler.yml`
  builds all three platforms when anything in `scripts/upscaler` changes. It
  fails if the binary links a non-system library or outgrows its size budget,
  and on Linux it upscales a test image with Mesa's software Vulkan driver.
- Releases sign and notarize the macOS binary, then attach the three zips,
  their `.sha256` files and `catconvert-<version>-upscaler-source.tar.xz`.

Licensing: realesrgan-ncnn-vulkan is MIT (its license also covers the
realsr-ncnn-vulkan code it is based on), ncnn is BSD-3-Clause with a few
permissively licensed parts, glslang is BSD-3-Clause and other permissive
licenses, libwebp is BSD-3-Clause, and MoltenVK is Apache-2.0. Each pack carries
these texts in `THIRD-PARTY-LICENSES.txt`. The Real-ESRGAN model weights are
never in our pack or anywhere in our releases: the app fetches them from
xinntao's official releases.

## Opening files from other apps

Files reach a running or starting app in two ways. macOS sends
`RunEvent::Opened`. Windows and Linux start the program with the paths as
arguments, and `tauri-plugin-single-instance` passes a second launch's arguments
to the running app. Both go through `src-tauri/src/incoming.rs`, which holds the
files until the front end subscribes (`onOpenedFiles` in `api.ts`).

| Where   | Entry point                    | Configured in                                               |
| ------- | ------------------------------ | ----------------------------------------------------------- |
| macOS   | Open With, Dock icon           | `tauri.macos.conf.json`, `Info.plist` (types macOS lacks)   |
| macOS   | Finder Quick Action (Settings) | `quick-action/` templates, `src/quick_action.rs`            |
| Windows | Open with, Send to             | `windows/hooks.nsh` (NSIS), `windows/shell.wxs` (MSI)       |
| Linux   | Open With                      | `tauri.linux.conf.json` (MIME types in the `.desktop` file) |

Windows doesn't use Tauri's `fileAssociations`: it would make CatConvert the
default app for every listed type. The installers only add it to "Open with".

When the core learns a new input format (`MediaKind::from_extension`), add it to
all of these lists and to the file picker in `api.ts`.

Dragging finished files out uses `tauri-plugin-drag`, which `dragFiles` in
`api.ts` calls directly (the plugin's npm package isn't needed).

## Commit messages

We use [Conventional Commits](https://www.conventionalcommits.org):

```
<type>(<optional scope>): <summary>

<optional body>
```

Common types: `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`,
`chore`. Useful scopes: `core`, `ffi`, `cli`, `desktop`, `ui`, `release`.
Mark breaking changes with `!` (`feat(core)!: ...`) or a `BREAKING CHANGE:` footer.

Examples:

```
feat(core): detect HEIC images by extension
fix(desktop): ignore duplicate drops of the same file
ci: cache the pnpm store on Windows
```

## Releases

Maintainers run `mise run release <version>`, which bumps the version in
`Cargo.toml`, `tauri.conf.json` and both `package.json` files, commits
`chore(release): v<version>` and creates the tag. Pushing the tag triggers the
release workflow, which creates a draft GitHub Release to review and publish.
Publishing it is what ships the update: installed apps read `latest.json` from
the latest published release (see "Auto-updates" in the README).
