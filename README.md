# ArcVault

![](./src-tauri/icons/128x128@2x.png)

A macOS desktop application for creating Windows-compatible ZIP files.

![](/documents/images/flashcap-20260130-110557.png)

## Features

- **Windows Compatible**: Sets UTF-8 Language Encoding Flag (bit 11) to prevent garbled filenames on Windows
- **Drag & Drop**: Simply drop folders or files to create ZIP archives
- **Instant Folder Compression**: Single folder drops are immediately compressed
- **Multiple Files Support**: Combine multiple files/folders into one ZIP
- **Flexible Output Location**: Choose from source location, Downloads, Desktop, or custom path
- **macOS Junk Exclusion**: Automatically excludes `.DS_Store`, `__MACOSX`, and `._` prefixed files

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
