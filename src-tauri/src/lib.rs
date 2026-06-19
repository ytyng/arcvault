use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;
use walkdir::WalkDir;

#[derive(Debug, Serialize, Deserialize)]
pub struct ZipResult {
    pub success: bool,
    pub output_path: Option<String>,
    pub error: Option<String>,
    /// True when the archive is encrypted and a (correct) password is required.
    /// Lets the frontend prompt for a password instead of treating it as a fatal error.
    pub needs_password: bool,
}

impl ZipResult {
    fn success(path: String) -> Self {
        Self {
            success: true,
            output_path: Some(path),
            error: None,
            needs_password: false,
        }
    }

    fn error(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            output_path: None,
            error: Some(msg.into()),
            needs_password: false,
        }
    }

    /// Error that indicates the frontend should prompt the user for a password.
    fn needs_password(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            output_path: None,
            error: Some(msg.into()),
            needs_password: true,
        }
    }
}

/// Canonicalize path and verify existence
fn canonicalize_path(path: &str) -> Result<std::path::PathBuf, String> {
    let p = Path::new(path);
    p.canonicalize()
        .map_err(|e| format!("Failed to resolve path: {}", e))
}

/// Returns path with sequential number if file with same name exists
fn get_unique_output_path(base_path: &Path) -> std::path::PathBuf {
    if !base_path.exists() {
        return base_path.to_path_buf();
    }

    let parent = base_path.parent().unwrap_or(Path::new("."));
    let stem = base_path.file_stem().and_then(|s| s.to_str()).unwrap_or("archive");
    let extension = base_path.extension().and_then(|e| e.to_str()).unwrap_or("zip");

    let mut counter = 1;
    loop {
        let new_name = format!("{}_{}.{}", stem, counter, extension);
        let new_path = parent.join(&new_name);
        if !new_path.exists() {
            return new_path;
        }
        counter += 1;
    }
}

/// Returns a unique directory path by appending `_N` to the whole folder name.
///
/// Unlike `get_unique_output_path` (which is file-oriented and splits off an
/// extension), this keeps the name intact — extracting `foo.zip` into an
/// existing `foo/` yields `foo_1`, not `foo_1.zip`, and stems containing dots
/// (e.g. `archive.backup`) are not rewritten.
fn get_unique_dir_path(base_path: &Path) -> std::path::PathBuf {
    if !base_path.exists() {
        return base_path.to_path_buf();
    }

    let parent = base_path.parent().unwrap_or(Path::new("."));
    let name = base_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("extracted");

    let mut counter = 1;
    loop {
        let new_path = parent.join(format!("{}_{}", name, counter));
        if !new_path.exists() {
            return new_path;
        }
        counter += 1;
    }
}

/// ZIP compression options
fn get_zip_options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6))
}

/// Add directory to ZIP
fn add_directory_to_zip<W: Write + io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    source_dir: &Path,
    prefix: &str,
) -> io::Result<()> {
    let options = get_zip_options();

    for entry in WalkDir::new(source_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();

        // Exclude .DS_Store and __MACOSX
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name == ".DS_Store" || name == "__MACOSX" || name.starts_with("._") {
                continue;
            }
        }

        let relative_path = path.strip_prefix(source_dir)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        // Build path within ZIP
        let zip_path = if prefix.is_empty() {
            relative_path.to_string_lossy().to_string()
        } else {
            format!("{}/{}", prefix, relative_path.to_string_lossy())
        };

        // Skip empty path (root)
        if zip_path.is_empty() {
            continue;
        }

        if path.is_dir() {
            // Add directory entry (append / at end)
            let dir_path = if zip_path.ends_with('/') {
                zip_path
            } else {
                format!("{}/", zip_path)
            };
            zip.add_directory(&dir_path, options.clone())?;
        } else {
            // Add file
            zip.start_file(&zip_path, options.clone())?;
            let mut file = File::open(path)?;
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)?;
            zip.write_all(&buffer)?;
        }
    }

    Ok(())
}

/// Compress folder to ZIP
/// include_parent: if true, include the folder itself (like --keepParent)
#[tauri::command]
async fn zip_folder(
    folder_path: String,
    output_dir: String,
    include_parent: bool,
) -> ZipResult {
    // Canonicalize path
    let folder = match canonicalize_path(&folder_path) {
        Ok(p) => p,
        Err(e) => return ZipResult::error(e),
    };

    if !folder.is_dir() {
        return ZipResult::error("The specified path is not a folder");
    }

    let output_dir_path = match canonicalize_path(&output_dir) {
        Ok(p) => p,
        Err(e) => return ZipResult::error(e),
    };

    let folder_name = folder.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("archive");

    let base_output_path = output_dir_path.join(format!("{}.zip", folder_name));
    let output_path = get_unique_output_path(&base_output_path);

    // Create ZIP file
    let file = match File::create(&output_path) {
        Ok(f) => f,
        Err(e) => return ZipResult::error(format!("Failed to create ZIP file: {}", e)),
    };

    let mut zip = zip::ZipWriter::new(file);

    // Prefix (folder_name if include_parent is true)
    let prefix = if include_parent { folder_name } else { "" };

    if let Err(e) = add_directory_to_zip(&mut zip, &folder, prefix) {
        return ZipResult::error(format!("Failed to compress: {}", e));
    }

    if let Err(e) = zip.finish() {
        return ZipResult::error(format!("Failed to finalize ZIP: {}", e));
    }

    match output_path.to_str() {
        Some(s) => ZipResult::success(s.to_string()),
        None => ZipResult::error("Output path contains invalid characters"),
    }
}

/// Compress multiple files to ZIP
#[tauri::command]
async fn zip_files(
    file_paths: Vec<String>,
    output_dir: String,
    archive_name: String,
) -> ZipResult {
    if file_paths.is_empty() {
        return ZipResult::error("No files specified");
    }

    // Canonicalize output directory
    let output_dir_path = match canonicalize_path(&output_dir) {
        Ok(p) => p,
        Err(e) => return ZipResult::error(e),
    };

    let base_output_path = output_dir_path.join(format!("{}.zip", archive_name));
    let output_path = get_unique_output_path(&base_output_path);

    // Create ZIP file
    let file = match File::create(&output_path) {
        Ok(f) => f,
        Err(e) => return ZipResult::error(format!("Failed to create ZIP file: {}", e)),
    };

    let mut zip = zip::ZipWriter::new(file);
    let options = get_zip_options();

    // Add files to ZIP
    for file_path in &file_paths {
        let src = match canonicalize_path(file_path) {
            Ok(p) => p,
            Err(_) => continue, // Skip non-existent files
        };

        let file_name = src.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");

        // Exclude .DS_Store etc.
        if file_name == ".DS_Store" || file_name.starts_with("._") {
            continue;
        }

        // Path within archive: archive_name/filename
        let zip_path = format!("{}/{}", archive_name, file_name);

        if src.is_dir() {
            // Recursively add directory
            if let Err(e) = add_directory_to_zip(&mut zip, &src, &zip_path) {
                return ZipResult::error(format!("Failed to add directory: {}", e));
            }
        } else {
            // Add file
            if let Err(e) = zip.start_file(&zip_path, options.clone()) {
                return ZipResult::error(format!("Failed to create file entry: {}", e));
            }

            let mut src_file = match File::open(&src) {
                Ok(f) => f,
                Err(e) => return ZipResult::error(format!("Failed to open file: {}", e)),
            };

            let mut buffer = Vec::new();
            if let Err(e) = src_file.read_to_end(&mut buffer) {
                return ZipResult::error(format!("Failed to read file: {}", e));
            }

            if let Err(e) = zip.write_all(&buffer) {
                return ZipResult::error(format!("Failed to write file: {}", e));
            }
        }
    }

    if let Err(e) = zip.finish() {
        return ZipResult::error(format!("Failed to finalize ZIP: {}", e));
    }

    match output_path.to_str() {
        Some(s) => ZipResult::success(s.to_string()),
        None => ZipResult::error("Output path contains invalid characters"),
    }
}

/// Decode a ZIP entry's raw filename bytes into a String.
///
/// Windows ZIP tools (7-Zip, WinRAR, etc.) often store Japanese filenames in
/// Shift-JIS (CP932) without setting the UTF-8 Language Encoding Flag (bit 11).
/// When that flag is absent the bytes are not UTF-8, so we fall back to
/// decoding them as Shift-JIS to avoid mojibake.
fn decode_zip_name(raw: &[u8]) -> String {
    match std::str::from_utf8(raw) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let (decoded, _, _) = encoding_rs::SHIFT_JIS.decode(raw);
            decoded.into_owned()
        }
    }
}

/// Extensions treated as text files for optional encoding conversion.
const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "py", "json", "sql", "h", "m", "c", "cpp", "cc", "cxx", "hpp",
    "hh", "mm", "swift", "java", "kt", "rs", "go", "rb", "pl", "php", "js", "ts",
    "jsx", "tsx", "css", "scss", "less", "html", "htm", "xml", "yaml", "yml",
    "toml", "ini", "cfg", "conf", "csv", "tsv", "sh", "bash", "zsh", "bat",
    "ps1", "r", "lua", "vim", "tex", "log", "srt", "vtt", "properties", "env",
];

/// Returns true if the filename has a text-file extension.
fn is_text_file(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| {
            let lower = ext.to_ascii_lowercase();
            TEXT_EXTENSIONS.contains(&lower.as_str())
        })
        .unwrap_or(false)
}

/// Convert file content to UTF-8.
///
/// If the bytes are already valid UTF-8 they are returned unchanged. Otherwise
/// they are assumed to be Shift-JIS and re-encoded as UTF-8.
fn convert_text_to_utf8(bytes: &[u8]) -> Vec<u8> {
    if std::str::from_utf8(bytes).is_ok() {
        return bytes.to_vec();
    }
    let (decoded, _, _) = encoding_rs::SHIFT_JIS.decode(bytes);
    decoded.into_owned().into_bytes()
}

/// Safely join a ZIP entry name onto an extraction root, preventing Zip Slip.
/// Returns None if the entry would escape the root (e.g. contains `..`).
///
/// ZIP entry names use `/` as the separator (per the spec), so we only split on
/// `/`. A backslash-separated segment (e.g. `..\foo` from some Windows tools)
/// stays a single component here; since this app is macOS-only and `\` is an
/// ordinary filename character there, it cannot traverse directories — the
/// segment is written verbatim as one literal path component.
fn safe_extract_path(root: &Path, entry_name: &str) -> Option<std::path::PathBuf> {
    let mut path = root.to_path_buf();
    for component in entry_name.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            return None;
        }
        path.push(component);
    }
    Some(path)
}

/// Extract a ZIP archive, decoding Shift-JIS filenames and optionally
/// converting Shift-JIS text files to UTF-8.
///
/// - `password`: optional password for encrypted archives
/// - `convert_text_encoding`: if true, text files are converted to UTF-8
///
/// This is an `async` command so Tauri runs it off the main (UI) thread; a
/// synchronous command would block the WebView and freeze the UI during
/// extraction of large archives.
#[tauri::command]
async fn unzip_archive(
    zip_path: String,
    output_dir: String,
    password: Option<String>,
    convert_text_encoding: bool,
) -> ZipResult {
    let zip = match canonicalize_path(&zip_path) {
        Ok(p) => p,
        Err(e) => return ZipResult::error(e),
    };

    let output_dir_path = match canonicalize_path(&output_dir) {
        Ok(p) => p,
        Err(e) => return ZipResult::error(e),
    };

    let file = match File::open(&zip) {
        Ok(f) => f,
        Err(e) => return ZipResult::error(format!("Failed to open archive: {}", e)),
    };

    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(e) => return ZipResult::error(format!("Not a valid ZIP archive: {}", e)),
    };

    // Extract into a folder named after the archive (made unique if it exists)
    let stem = zip
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("extracted");
    let extract_root = get_unique_dir_path(&output_dir_path.join(stem));

    if let Err(e) = std::fs::create_dir_all(&extract_root) {
        return ZipResult::error(format!("Failed to create output folder: {}", e));
    }

    let password = password.filter(|p| !p.is_empty());

    for i in 0..archive.len() {
        let mut entry = match &password {
            Some(p) => archive.by_index_decrypt(i, p.as_bytes()),
            None => archive.by_index(i),
        };

        let entry = match entry.as_mut() {
            Ok(e) => e,
            Err(zip::result::ZipError::InvalidPassword) => {
                let _ = std::fs::remove_dir_all(&extract_root);
                return ZipResult::needs_password("Incorrect password");
            }
            Err(zip::result::ZipError::UnsupportedArchive(
                zip::result::ZipError::PASSWORD_REQUIRED,
            )) => {
                let _ = std::fs::remove_dir_all(&extract_root);
                return ZipResult::needs_password("This archive is password protected.");
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&extract_root);
                return ZipResult::error(format!("Failed to read archive entry: {}", e));
            }
        };

        let name = decode_zip_name(entry.name_raw());

        // Skip macOS metadata entries
        if name.contains("__MACOSX/") || name.starts_with("__MACOSX") {
            continue;
        }
        if let Some(base) = name.rsplit('/').next() {
            if base == ".DS_Store" || base.starts_with("._") {
                continue;
            }
        }

        let out_path = match safe_extract_path(&extract_root, &name) {
            Some(p) => p,
            None => continue, // Reject Zip Slip paths
        };

        if entry.is_dir() {
            if let Err(e) = std::fs::create_dir_all(&out_path) {
                let _ = std::fs::remove_dir_all(&extract_root);
                return ZipResult::error(format!("Failed to create directory: {}", e));
            }
            continue;
        }

        if let Some(parent) = out_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                let _ = std::fs::remove_dir_all(&extract_root);
                return ZipResult::error(format!("Failed to create directory: {}", e));
            }
        }

        let mut buffer = Vec::new();
        if let Err(e) = entry.read_to_end(&mut buffer) {
            let _ = std::fs::remove_dir_all(&extract_root);
            return ZipResult::error(format!("Failed to read file from archive: {}", e));
        }

        let data = if convert_text_encoding && is_text_file(&name) {
            convert_text_to_utf8(&buffer)
        } else {
            buffer
        };

        if let Err(e) = std::fs::write(&out_path, &data) {
            let _ = std::fs::remove_dir_all(&extract_root);
            return ZipResult::error(format!("Failed to write file: {}", e));
        }
    }

    match extract_root.to_str() {
        Some(s) => ZipResult::success(s.to_string()),
        None => ZipResult::error("Output path contains invalid characters"),
    }
}

/// Get Downloads folder path
#[tauri::command]
fn get_downloads_dir() -> Option<String> {
    dirs::download_dir().map(|p| p.to_string_lossy().to_string())
}

/// Get Desktop folder path
#[tauri::command]
fn get_desktop_dir() -> Option<String> {
    dirs::desktop_dir().map(|p| p.to_string_lossy().to_string())
}

/// Get parent directory of path
#[tauri::command]
fn get_parent_dir(path: &str) -> Option<String> {
    Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            zip_folder,
            zip_files,
            unzip_archive,
            get_downloads_dir,
            get_desktop_dir,
            get_parent_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_shift_jis_filename_without_mojibake() {
        // "日本語.txt" encoded as Shift-JIS (CP932)
        let (sjis_bytes, _, _) = encoding_rs::SHIFT_JIS.encode("日本語.txt");
        let decoded = decode_zip_name(&sjis_bytes);
        assert_eq!(decoded, "日本語.txt");
    }

    #[test]
    fn decodes_utf8_filename_unchanged() {
        let decoded = decode_zip_name("写真.png".as_bytes());
        assert_eq!(decoded, "写真.png");
    }

    #[test]
    fn detects_text_files_by_extension() {
        assert!(is_text_file("readme.md"));
        assert!(is_text_file("main.py"));
        assert!(is_text_file("DATA.JSON"));
        assert!(is_text_file("header.h"));
        assert!(is_text_file("View.m"));
        assert!(!is_text_file("photo.png"));
        assert!(!is_text_file("archive.zip"));
        assert!(!is_text_file("noext"));
    }

    #[test]
    fn converts_shift_jis_text_to_utf8() {
        let (sjis_bytes, _, _) = encoding_rs::SHIFT_JIS.encode("こんにちは世界");
        let converted = convert_text_to_utf8(&sjis_bytes);
        assert_eq!(String::from_utf8(converted).unwrap(), "こんにちは世界");
    }

    #[test]
    fn leaves_valid_utf8_text_unchanged() {
        let original = "already utf-8 テキスト".as_bytes();
        let converted = convert_text_to_utf8(original);
        assert_eq!(converted, original);
    }

    #[test]
    fn unique_dir_path_keeps_name_without_extension() {
        // Non-existent path is returned unchanged.
        let base = Path::new("/tmp/arcvault-nonexistent-xyz/foo");
        assert_eq!(get_unique_dir_path(base), base.to_path_buf());

        // For an existing folder, `_N` is appended to the whole name without
        // inventing a `.zip` extension or splitting on dots.
        let dir = std::env::temp_dir().join("arcvault_unique_dir_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let existing = dir.join("archive.backup");
        std::fs::create_dir_all(&existing).unwrap();

        let unique = get_unique_dir_path(&existing);
        assert_eq!(unique, dir.join("archive.backup_1"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_zip_slip_paths() {
        let root = Path::new("/tmp/extract");
        assert!(safe_extract_path(root, "../escape.txt").is_none());
        assert!(safe_extract_path(root, "a/../../b.txt").is_none());
        assert_eq!(
            safe_extract_path(root, "sub/dir/file.txt"),
            Some(Path::new("/tmp/extract/sub/dir/file.txt").to_path_buf())
        );
    }
}
