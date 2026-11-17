# CatConvert

An offline photo, video and audio file converter. Nothing leaves your device.

CatConvert is a desktop app for macOS, Windows and Linux built with
[Tauri 2](https://v2.tauri.app), a [SolidJS](https://www.solidjs.com) + TypeScript
front end and a Rust core library. iOS and Android shells come later, through
Tauri mobile or native Swift/Kotlin apps over the UniFFI bindings.

## Features

- **Photo, video and audio** in, any sensible format out: trim, crop (free or
  1:1, 9:16, 16:9), resize, rotate, mirror, change frame rate, quality and
  format, or drop the sound. Even out loudness and fade audio in and out.
- **Pictures to animation:** drop several pictures to make a GIF, animated WebP
  or JPEG XL, or an MP4 slideshow. Reorder them in a filmstrip, set how long each
  shows, play forward, reversed or as a boomerang, and fit other shapes with
  black bars or by cropping. GIFs and animated JXL get the same speed, reverse
  and boomerang controls.
- **JPEG XL**, still and animated: lossless or lossy, an effort setting and
  progressive files. Unedited JPEGs are repacked losslessly and can be restored
  bit for bit.
- **Frame by frame:** step through a video and save any frame as a PNG
  (←/→ and S). HDR video, which iPhones record by default, is tone-mapped when it
  becomes a picture or GIF.
- **Privacy:** remove location data and keep the rest of the metadata.
- **Size estimate** before you convert, and a serial queue with live progress.
  Results are saved next to the original and never overwrite anything.
- **Works from other apps:** photos, videos and songs offer CatConvert under
  "Open With" (plus "Send to" on Windows), macOS can add a "Convert with
  CatConvert" Finder Quick Action, and finished files drag straight out of the
  Queue and History.
- **Batch and recipes:** convert many files at once with saved settings.
- **Auto-updates** from GitHub Releases, signed with the updater key.

Conversion runs through `ffmpeg`/`ffprobe` and libjxl's tools. Release builds
ship their own, so there is nothing to install. Development builds use an
installed ffmpeg (found on `PATH`, in Homebrew's folders, or via
`CATCONVERT_FFMPEG`, which always wins) and libjxl (`brew install jpeg-xl`, or
`CATCONVERT_LIBJXL`). An ffmpeg built with libjxl works as a JPEG XL fallback.

## Download

Get the latest `.dmg`, `.msi`/`.exe`, `.AppImage` or `.deb` from
[Releases](https://github.com/Jotrorox/catconvert/releases/latest).

## Quick start

You need [mise](https://mise.jdx.dev) and [rustup](https://rustup.rs). Everything
else (Node, pnpm, actionlint, ...) is pinned in `mise.toml`; Rust is pinned in
`rust-toolchain.toml` and installed by rustup automatically.

```sh
mise install     # install pinned tools
mise run dev     # install JS deps and launch the desktop app
```

Development builds use your installed ffmpeg, so install it first (`brew install
ffmpeg`, `winget install ffmpeg` or your distro's package), then drop a video on
the kitty. To build the app with its own ffmpeg inside, see
[Bundled media tools](CONTRIBUTING.md#bundled-media-tools). Narrow the window below 720px
to see the phone layout.

On Linux, install Tauri's system packages first; see
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/#linux).

## Layout

```
catconvert/
├── Cargo.toml              # Cargo workspace: crates/* + apps/desktop/src-tauri
├── rust-toolchain.toml     # pinned Rust toolchain (rustup)
├── mise.toml               # pinned tools + tasks
├── package.json            # pnpm workspace root, scripts delegate to apps/desktop
├── pnpm-workspace.yaml
├── biome.json              # lint + format for TS/JS/JSON/CSS
├── crates/
│   ├── core/               # catconvert-core: pure Rust, no platform code
│   ├── ffi/                # catconvert-ffi: UniFFI exports for Swift/Kotlin shells
│   └── cli/                # catconvert binary (clap)
├── apps/
│   └── desktop/            # SolidJS + Vite front end (src/)
│       └── src-tauri/      # Tauri shell: thin commands over catconvert-core
├── scripts/
│   ├── release.mjs         # version bump + tag (used by `mise run release`)
│   └── media-tools/        # static ffmpeg/libjxl build + pinned sources
└── .github/                # CI, release, media-tools and Dependabot config
```

All logic lives in `catconvert-core`. The Tauri commands, the CLI and the FFI
crate are thin adapters, and the front end talks to Tauri only through the typed
wrappers in `apps/desktop/src/lib/api.ts`.

## Tasks

Run from anywhere in the repo with `mise run <task>` (`mise tasks` lists them).

| Task        | What it does                                                            |
| ----------- | ----------------------------------------------------------------------- |
| `setup`     | `pnpm install` (frozen lockfile in CI)                                  |
| `dev`       | Run the desktop app in dev mode (hot reload)                            |
| `build`     | Production desktop bundle for the current OS (`target/release/bundle/`) |
| `cli`       | Run the CLI: `mise run cli -- convert clip.mov --to gif`                |
| `fmt`       | `cargo fmt` + `biome format`                                            |
| `fmt-check` | Formatting check without writing                                        |
| `lint`      | `cargo clippy -D warnings`, `biome check`, `tsc --noEmit`, `actionlint` |
| `test`      | `cargo test --workspace` + `vitest run`                                 |
| `check`     | `fmt-check` + `lint` + `test` (what CI runs)                            |
| `release`   | `mise run release 0.2.0`: bump versions, commit, tag `v0.2.0` (no push) |

## CLI

```sh
mise run cli -- probe clip.mov                       # duration, size, codecs as JSON
mise run cli -- convert clip.mov --to gif --start 1 --end 4 --size 480
mise run cli -- convert photo.jpg --to jxl --lossless             # repack JPEG, restorable
mise run cli -- convert clip.mov --to jxl --size 480 --effort 9    # animated JPEG XL
mise run cli -- convert clip.mov --to mp3            # pull the audio
mise run cli -- convert clip.mov --to mp4 --crop 9:16 --size 1080  # vertical, centred
mise run cli -- convert clip.mov --to mp4 --rotate 90 --mute --dry-run  # print the ffmpeg command
mise run cli -- animate cat*.jpg --to gif --frame 0.25 --direction boomerang  # pictures → animation
```

## CI and releases

- **CI** (`.github/workflows/ci.yml`): `mise run check` on Ubuntu, plus unsigned
  desktop builds on macOS, Windows and Ubuntu 22.04 (bundles uploaded as
  workflow artifacts for 7 days).
- **Release** (`.github/workflows/release.yml`): pushing a `v*` tag builds a
  universal macOS `.dmg`, Windows `.msi`/`.exe`, Linux `.AppImage`/`.deb` and
  the `catconvert` CLI for each OS, and attaches everything to a **draft**
  GitHub Release. The desktop bundles include ffmpeg and libjxl's tools, built
  by `.github/workflows/media-tools.yml`, and the release gets their complete
  source as `catconvert-<tag>-media-tools-source.tar.xz`. The CLI uses an
  installed ffmpeg.

```sh
mise run release 0.2.0
git push origin main --follow-tags
```

### Signing (optional)

Without these secrets, releases are built unsigned. Add them as secrets of the
`release` environment (_Settings → Environments → release_, restricted to `v*`
tags), e.g. `gh secret set APPLE_ID --env release`:

| Secret                               | Purpose                                                  |
| ------------------------------------ | -------------------------------------------------------- |
| `APPLE_CERTIFICATE`                  | Base64 of the exported "Developer ID Application" `.p12` |
| `APPLE_CERTIFICATE_PASSWORD`         | Password of that `.p12`                                  |
| `APPLE_SIGNING_IDENTITY`             | e.g. `Developer ID Application: Your Name (TEAMID)`      |
| `APPLE_ID`                           | Apple ID email used for notarization                     |
| `APPLE_PASSWORD`                     | App-specific password for that Apple ID                  |
| `APPLE_TEAM_ID`                      | 10-character team ID                                     |
| `WINDOWS_CERTIFICATE`                | Base64 of a code-signing `.pfx`                          |
| `WINDOWS_CERTIFICATE_PASSWORD`       | Password of that `.pfx`                                  |
| `TAURI_SIGNING_PRIVATE_KEY`          | Updater signing key (contents of the `.key` file)        |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password of that key                                     |

macOS signing needs the first three; notarization additionally needs the Apple
ID trio. With notarization enabled, both the app and the `.dmg` are notarized
and stapled. See [Tauri: macOS code signing](https://v2.tauri.app/distribute/sign/macos/)
and [Windows code signing](https://v2.tauri.app/distribute/sign/windows/).

### Auto-updates

The app checks
`https://github.com/Jotrorox/catconvert/releases/latest/download/latest.json`
at launch and from _Settings → Updates_, and only installs bundles signed with
the updater key whose public half is `plugins.updater.pubkey` in
`apps/desktop/src-tauri/tauri.conf.json`. With `TAURI_SIGNING_PRIVATE_KEY` set,
the release workflow builds signed updater bundles (`.app.tar.gz`, `.AppImage`,
the Windows installers) and attaches a `latest.json` covering every platform.
Installed apps see a release once it is published (drafts and pre-releases are
not `latest`), and the release assets must be downloadable without signing in,
so the repository (or wherever the endpoint points) has to be public.

Keep the private key and its password backed up: without them, existing
installs can never be updated again. To rotate the key, ship one release that
contains the new public key, signed with the old key, before switching.
A new pair comes from `pnpm tauri signer generate -w ~/.tauri/catconvert.key`
(run in `apps/desktop`).

## Mobile (later)

The Tauri crate keeps the `#[cfg_attr(mobile, tauri::mobile_entry_point)]` entry
point and builds as `staticlib`/`cdylib`, so `pnpm tauri ios init` /
`pnpm tauri android init` (run in `apps/desktop`) can be added later.
Alternatively, native shells can use `catconvert-ffi` through UniFFI.

## License

CatConvert is [BSD-3-Clause](LICENSE). The desktop app bundles ffmpeg (built
with x264) and ffprobe, which are GPL-2.0-or-later, and libjxl's tools; they run
as separate programs. Their licenses are listed in the app (Settings →
Open-source licenses), and each release includes their source code.
