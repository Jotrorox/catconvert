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
