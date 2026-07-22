# ArcVault

![](./src-tauri/icons/128x128@2x.png)

A macOS desktop application for creating Windows-compatible ZIP files.

## Screenshots

| Archive | Extract |
| --- | --- |
| ![Archive tab: drop files or folders to create a ZIP](./docs/images/screenshot-archive.png) | ![Extract tab: drop .zip files to extract](./docs/images/screenshot-extract.png) |

## Features

- **Windows Compatible**: Sets UTF-8 Language Encoding Flag (bit 11) to prevent garbled filenames on Windows
- **Drag & Drop**: Simply drop folders or files to create ZIP archives
- **Instant Folder Compression**: Single folder drops are immediately compressed
- **Multiple Files Support**: Combine multiple files/folders into one ZIP
- **Flexible Output Location**: Choose from source location, Downloads, Desktop, or custom path
- **macOS Junk Exclusion**: Automatically excludes `.DS_Store`, `__MACOSX`, and `._` prefixed files
- **Extraction**: Drop a `.zip` to extract it, with Shift-JIS (CP932) filenames decoded without mojibake
- **Password Protected Archives**: Prompts for a password when the archive is encrypted
- **Optional Text Conversion**: Converts Shift-JIS text files to UTF-8 during extraction

## Download

Prebuilt binaries are available on the [latest GitHub Release](https://github.com/ytyng/arcvault/releases/latest):

- **macOS**: `arcvault_x.x.x_universal.dmg` (Intel / Apple Silicon universal)
- **Windows**: `arcvault_x.x.x_x64-setup.exe` (NSIS installer)

> **Note (macOS)**: The app is ad-hoc signed and not notarized. On first launch,
> right-click the app and choose "Open", or remove the quarantine attribute:
> `xattr -dr com.apple.quarantine /Applications/arcvault.app`
>
> **Note (Windows)**: The installer is unsigned, so SmartScreen may warn you.
> Click "More info" → "Run anyway".

## Tech Stack

- **Frontend**: SvelteKit 5 + Tailwind CSS 4
- **Backend**: Tauri 2 (Rust)
- **ZIP Compression**: Rust zip crate (Deflate compression)

## Development Setup

### Requirements

- Node.js 20+
- pnpm
- Rust (rustup)
- Xcode Command Line Tools

### Installation

```bash
pnpm install
```

### Development Server

```bash
pnpm tauri dev
```

### Build

```bash
pnpm build
pnpm tauri build
```

### Release

Releases are built by GitHub Actions (`.github/workflows/release.yml`) and
uploaded to GitHub Releases. The workflow is triggered manually, not on push:

```bash
# 1. Bump "version" in src-tauri/tauri.conf.json (and package.json)
# 2. Commit and push to main
# 3. Trigger the build (requires authenticated gh CLI)
pnpm release
```

This builds a macOS universal dmg (ad-hoc signed, not notarized) and an
unsigned Windows NSIS installer, then publishes them to the `v<version>` Release.

## Usage

1. Launch the app
2. Drag & drop folders or files onto the window
3. Folders are immediately compressed to ZIP
4. Files are added to a list; click "Create Zip" to compress

### Settings

- **Output Location**: Source location / Downloads / Desktop / Custom
- **Include parent folder**: When ON, preserves folder structure inside ZIP

## License

MIT
