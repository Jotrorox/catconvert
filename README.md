# CatConvert

An offline photo, video and audio file converter. Nothing leaves your device.

CatConvert is a desktop app for macOS, Windows and Linux built with
[Tauri 2](https://v2.tauri.app), a [SolidJS](https://www.solidjs.com) + TypeScript
front end and a Rust core library. iOS and Android shells come later, through
Tauri mobile or native Swift/Kotlin apps over the UniFFI bindings.

> **Status: early.** Conversion runs through the `ffmpeg`/`ffprobe` installed on
> your machine (found on `PATH`, in Homebrew's folders, or via `CATCONVERT_FFMPEG`).
> You can drop files, trim, resize, rotate, mirror, change frame rate, quality and
> format, drop the sound, and watch a serial queue with live progress. Results are
> saved next to the original and never overwrite anything.

## Quick start

You need [mise](https://mise.jdx.dev) and [rustup](https://rustup.rs). Everything
else (Node, pnpm, actionlint, ...) is pinned in `mise.toml`; Rust is pinned in
`rust-toolchain.toml` and installed by rustup automatically.

```sh
mise install     # install pinned tools
mise run dev     # install JS deps and launch the desktop app
```

Install ffmpeg first (`brew install ffmpeg`, `winget install ffmpeg` or your
distro's package), then drop a video on the kitty. Narrow the window below 720px
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
├── scripts/release.mjs     # version bump + tag (used by `mise run release`)
└── .github/                # CI, release and Dependabot config
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
mise run cli -- convert clip.mov --to mp3            # pull the audio
mise run cli -- convert clip.mov --to mp4 --rotate 90 --mute --dry-run  # print the ffmpeg command
```

## CI and releases

- **CI** (`.github/workflows/ci.yml`): `mise run check` on Ubuntu, plus unsigned
  desktop builds on macOS, Windows and Ubuntu 22.04 (bundles uploaded as
  workflow artifacts for 7 days).
- **Release** (`.github/workflows/release.yml`): pushing a `v*` tag builds a
  universal macOS `.dmg`, Windows `.msi`/`.exe`, Linux `.AppImage`/`.deb` and
  the `catconvert` CLI for each OS, and attaches everything to a **draft**
  GitHub Release.

```sh
mise run release 0.2.0
git push origin main --follow-tags
```

### Signing (optional)

Without these secrets, releases are built unsigned. Add them as secrets of the
`release` environment (_Settings → Environments → release_, restricted to `v*`
tags), e.g. `gh secret set APPLE_ID --env release`:

| Secret                         | Purpose                                                  |
| ------------------------------ | -------------------------------------------------------- |
| `APPLE_CERTIFICATE`            | Base64 of the exported "Developer ID Application" `.p12` |
| `APPLE_CERTIFICATE_PASSWORD`   | Password of that `.p12`                                  |
| `APPLE_SIGNING_IDENTITY`       | e.g. `Developer ID Application: Your Name (TEAMID)`      |
| `APPLE_ID`                     | Apple ID email used for notarization                     |
| `APPLE_PASSWORD`               | App-specific password for that Apple ID                  |
| `APPLE_TEAM_ID`                | 10-character team ID                                     |
| `WINDOWS_CERTIFICATE`          | Base64 of a code-signing `.pfx`                          |
| `WINDOWS_CERTIFICATE_PASSWORD` | Password of that `.pfx`                                  |

macOS signing needs the first three; notarization additionally needs the Apple
ID trio. With notarization enabled, both the app and the `.dmg` are notarized
and stapled. See [Tauri: macOS code signing](https://v2.tauri.app/distribute/sign/macos/)
and [Windows code signing](https://v2.tauri.app/distribute/sign/windows/).

## Mobile (later)

The Tauri crate keeps the `#[cfg_attr(mobile, tauri::mobile_entry_point)]` entry
point and builds as `staticlib`/`cdylib`, so `pnpm tauri ios init` /
`pnpm tauri android init` (run in `apps/desktop`) can be added later.
Alternatively, native shells can use `catconvert-ffi` through UniFFI.

## License

[BSD-3-Clause](LICENSE)
