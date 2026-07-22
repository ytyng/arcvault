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

# Release build (triggers GitHub Actions, uploads to GitHub Releases)
pnpm release
```

## Release

- `.github/workflows/release.yml` builds macOS (universal dmg, ad-hoc signed) and
  Windows (unsigned NSIS exe) via `tauri-apps/tauri-action`, publishing to the
  `v<version>` GitHub Release. Trigger is `workflow_dispatch` only (no auto build on push).
- `pnpm release` (`scripts/release.sh`) triggers the workflow with `gh workflow run`
  and watches it. Bump `version` in `src-tauri/tauri.conf.json` and `package.json` first.
- Note: the script is named `release`, not `publish`, because `pnpm publish` is a
  built-in pnpm command (npm registry publish) and cannot be shadowed by a script.

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
