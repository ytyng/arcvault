# ArcVault - AI Agent Development Guide

## Project Overview

ArcVault is a macOS desktop app for creating Windows-compatible ZIP files.
Built with Tauri 2 + SvelteKit 5 + Tailwind 4.

## Architecture

```
arcvault/
├── src/                    # SvelteKit frontend
│   ├── routes/+page.svelte # Main UI
│   └── app.html            # HTML template
├── src-tauri/              # Tauri/Rust backend
│   ├── src/lib.rs          # Tauri command definitions
│   ├── Cargo.toml          # Rust dependencies
│   └── capabilities/       # Tauri permissions
└── static/                 # Static files
```

## Important Technical Decisions

### ZIP Compression Method

- Uses Rust `zip` crate instead of `ditto` command
- Reason: `ditto` does not set UTF-8 Language Encoding Flag (bit 11), causing garbled Japanese filenames on Windows
- The `zip` crate automatically sets this flag

### Tauri Commands

Defined in `src-tauri/src/lib.rs`:

- `zip_folder`: Compress a single folder to ZIP
- `zip_files`: Compress multiple files/folders to ZIP
- `unzip_archive`: Extract a ZIP (Archive/Extract tabbed UI). Decodes Shift-JIS (CP932) filenames to avoid mojibake on Windows-made ZIPs, supports password-protected archives (ZipCrypto + AES), and can optionally convert Shift-JIS text files to UTF-8
- `get_downloads_dir`: Get Downloads folder path
- `get_desktop_dir`: Get Desktop folder path
- `get_parent_dir`: Get parent directory path

### Excluded Files

Automatically excluded from ZIP:
- `.DS_Store`
- `__MACOSX`
- Files starting with `._`

## Development Commands

```bash
# Development server
pnpm tauri dev

# Frontend build only
pnpm build

# Rust backend build only
cd src-tauri && cargo build --release

# Full app build
pnpm tauri build

# Icon generation
pnpm tauri icon /path/to/icon.png

# Release: bump the version and push it; the push triggers the GitHub Actions build
pnpm release
```

## Release

- `.github/workflows/release.yml` runs on every push to `main` (and on `workflow_dispatch`
  as a retry). A `plan` job reads `src-tauri/tauri.conf.json` and asks the GitHub API whether
  `v<version>` is already a published Release: 404 means release, 200 means nothing to do,
  anything else fails the run rather than guessing. Only then do `test`, `build` and `publish`
  run. The decision is "is this version released", not "did the diff touch the version", so
  squash / rebase / direct push all behave the same, and a failed release is retried by
  pushing the fix (no path filter for the same reason).
- `build` makes macOS (universal dmg, Developer ID signed + notarized) and Windows (unsigned
  NSIS exe) via `tauri-apps/tauri-action` into a draft Release; `publish` flips the draft to
  published only after both legs succeed. A draft left by a failed run is reused by the
  re-run of the same version.
- `pull_request` runs only the `test` job (`plan` is skipped, so `build` / `publish` are
  skipped with it). The PR concurrency group is per commit so PR checks never queue behind
  a release; releases serialise in one group with `queue: max` so a burst of pushes cannot
  drop a version.
- macOS signing: a macOS-only step imports the Developer ID `.p12` (from `APPLE_CERTIFICATE`
  / `APPLE_CERTIFICATE_PASSWORD` secrets) into a throwaway keychain; then tauri-action signs
  and notarizes using `APPLE_SIGNING_IDENTITY` / `APPLE_ID` / `APPLE_PASSWORD` (app-specific)
  / `APPLE_TEAM_ID`. `tauri.conf.json` keeps `signingIdentity: "-"` (ad-hoc) for local builds;
  `APPLE_SIGNING_IDENTITY` overrides it in CI (tauri-cli precedence: env wins over config), so
  CI produces a Developer ID build. The Apple env vars are only passed on the macOS matrix leg.
  Missing secrets make the macOS job fail.
- `tauri-action` is pinned to a commit SHA (not `@v0`) because it receives the Apple signing
  secrets; a moved tag would be a supply-chain risk.
- `pnpm release [patch|minor|major]` (`scripts/release.sh`, default `patch`) bumps the
  version in `src-tauri/tauri.conf.json` and `package.json`, commits `chore: release
  vX.Y.Z`, pushes to main, then finds the run started by that push (by head SHA) and
  watches it. It refuses to run unless the tree is clean and `HEAD == origin/main`. The
  push is what releases; editing the version by hand and pushing does the same thing.
- `package.json` `version` is cosmetic here (tauri-action reads the version from
  `tauri.conf.json`), but the script keeps both in sync.
- Note: the script is named `release`, not `publish`, because `pnpm publish` is a
  built-in pnpm command (npm registry publish) and cannot be shadowed by a script.
- Homebrew: the cask lives in `ytyng/homebrew-tap` (`brew install --cask ytyng/tap/arcvault`).
  The tap updates itself hourly from the latest published Release, so nothing here pushes
  to it and no tap token is needed in this repository.

## Svelte 5 Runes

This project uses Svelte 5. State management uses Runes:

```svelte
let files = $state<string[]>([]);
let isDragging = $state(false);
```

## Tauri Plugins

Plugins in use:
- `tauri-plugin-store`: Settings persistence
- `tauri-plugin-dialog`: Native save dialog
- `tauri-plugin-opener`: Open files/URLs

When adding new plugins:
1. Add dependency to `src-tauri/Cargo.toml`
2. Initialize plugin in `run()` at `src-tauri/src/lib.rs`
3. Add permission to `src-tauri/capabilities/default.json`
4. Add npm package to `package.json`

## Testing

Unit tests are not yet implemented.

## Notes

- Path separator uses `/` (macOS-only app)
- Tauri commands are called from frontend using `invoke`
- Settings are saved to `settings.json` via `@tauri-apps/plugin-store`
