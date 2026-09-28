use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;
use walkdir::WalkDir;

#[cfg(not(target_os = "macos"))]
use tauri::menu::{AboutMetadata, PredefinedMenuItem, Submenu};
use tauri::menu::{Menu, MenuItem};
use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

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

/// macOS が勝手に作るファイル・フォルダかどうか。
fn is_macos_junk_name(name: &str) -> bool {
    name == ".DS_Store" || name == "__MACOSX" || name.starts_with("._")
}

/// Add directory to ZIP
fn add_directory_to_zip<W: Write + io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    source_dir: &Path,
    prefix: &str,
) -> io::Result<()> {
    let options = get_zip_options();

    // filter_entry で枝ごと落とす。エントリを 1 つずつ弾くだけだと、`__MACOSX`
    // 自身はスキップされてもその中身は walk され、ZIP に入ってしまう。
    // depth 0 (ドロップされたフォルダ自身) は名前に関わらず対象にする。
    let walker = WalkDir::new(source_dir).into_iter().filter_entry(|e| {
        e.depth() == 0
            || e.file_name()
                .to_str()
                .map(|name| !is_macos_junk_name(name))
                .unwrap_or(true)
    });

    for entry in walker.filter_map(|e| e.ok()) {
        let path = entry.path();

        let relative_path = path.strip_prefix(source_dir)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        // Build path within ZIP. Join components with `/` explicitly: ZIP entry
        // names must use `/` per the spec, but `to_string_lossy()` would keep
        // Windows `\` separators.
        let relative_str = relative_path
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let zip_path = if prefix.is_empty() {
            relative_str
        } else {
            format!("{}/{}", prefix, relative_str)
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
/// ZIP entry names use `/` as the separator (per the spec), but some Windows
/// tools write `\`-separated names, and on Windows `PathBuf::push` treats `\`
/// as a separator too — so a `..\foo` segment must not survive as a single
/// "literal" component. We split on both separators and reject `..` in either
/// form. On Windows, components containing `:` are also rejected: a `C:`
/// prefix makes `push` replace the entire path with a drive-relative one.
/// On other platforms `:` is an ordinary filename character (e.g. timestamps)
/// and must not cause entries to be skipped.
fn safe_extract_path(root: &Path, entry_name: &str) -> Option<std::path::PathBuf> {
    let mut path = root.to_path_buf();
    for component in entry_name.split(['/', '\\']) {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." || (cfg!(windows) && component.contains(':')) {
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
        if let Some(base) = name.rsplit(['/', '\\']).next() {
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

/// Licenses of the libraries bundled into the app. THIRD-PARTY-NOTICES.txt at the
/// repository root is generated by `pnpm notices` and embedded at build time.
const THIRD_PARTY_NOTICES: &str = include_str!("../../THIRD-PARTY-NOTICES.txt");

/// Menu item id / window label of the Third-Party Licenses window.
const LICENSES_ID: &str = "licenses";

#[tauri::command]
fn third_party_notices() -> &'static str {
    THIRD_PARTY_NOTICES
}

/// Open the Third-Party Licenses window, or bring it to the front if it is already open.
/// The window is destroyed when closed, so each open creates it again.
fn show_licenses_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(win) = app.get_webview_window(LICENSES_ID) {
        win.show()?;
        return win.set_focus();
    }
    WebviewWindowBuilder::new(app, LICENSES_ID, WebviewUrl::App("licenses".into()))
        .title("Third-Party Licenses")
        .inner_size(640.0, 560.0)
        .min_inner_size(400.0, 300.0)
        .center()
        .build()?;
    Ok(())
}

/// The app menu. On macOS it is Tauri's default menu with "Third-Party Licenses" placed
/// right below "About arcvault" in the app menu. Windows had no menu bar before, so it
/// gets only a Help menu holding About and Third-Party Licenses.
fn build_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let licenses = MenuItem::with_id(app, LICENSES_ID, "Third-Party Licenses", true, None::<&str>)?;

    #[cfg(target_os = "macos")]
    {
        let menu = Menu::default(app)?;
        // Menu::default puts the app menu first, with About at position 0.
        let items = menu.items()?;
        if let Some(app_menu) = items.first().and_then(|item| item.as_submenu()) {
            app_menu.insert(&licenses, 1)?;
        }
        Ok(menu)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let about = PredefinedMenuItem::about(
            app,
            Some("About ArcVault"),
            Some(AboutMetadata {
                name: Some("ArcVault".into()),
                version: Some(app.package_info().version.to_string()),
                ..Default::default()
            }),
        )?;
        let help = Submenu::with_items(app, "Help", true, &[&about, &licenses])?;
        Menu::with_items(app, &[&help])
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .menu(build_menu)
        .on_menu_event(|app, event| {
            if event.id() == LICENSES_ID {
                // Creating a webview window from a menu event handler can deadlock on
                // Windows (see WebviewWindowBuilder::new), so do it off that thread.
                let app = app.clone();
                std::thread::spawn(move || {
                    if let Err(e) = show_licenses_window(&app) {
                        eprintln!("arcvault: failed to show the licenses window: {e}");
                    }
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            zip_folder,
            zip_files,
            unzip_archive,
            get_downloads_dir,
            get_desktop_dir,
            get_parent_dir,
            third_party_notices,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト専用の一時ディレクトリを作る。
    ///
    /// 名前を固定すると、同時に走った別の cargo test や、たまたま同名の
    /// ディレクトリを持っていたローカル環境の中身を消してしまう。
    /// プロセス ID と連番で必ず一意にする。
    fn temp_test_dir(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "arcvault_test_{}_{}_{}",
            label,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 配布物に入る直接依存の crate 名を Cargo.toml から拾う。`[dependencies]` と
    /// `[target.'cfg(..)'.dependencies]` の `name = ...` の 1 行書式だけを見る
    /// (`[dependencies.foo]` 形式は拾えない)。build / dev 依存は配布物に入らないので除く。
    /// 配布しないターゲット (Linux) 専用の target 依存を足すと、about.toml の targets の
    /// 外なので notices に載らず、このテストが落ちる。その時はここで除外する。
    fn direct_rust_dependencies(manifest: &str) -> Vec<String> {
        let mut section = String::new();
        let mut names = Vec::new();
        for line in manifest.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                section = line.to_string();
                continue;
            }
            let shipped = section == "[dependencies]"
                || (section.starts_with("[target.") && section.ends_with(".dependencies]"));
            if !shipped || line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((name, _)) = line.split_once('=') {
                names.push(name.trim().to_string());
            }
        }
        names
    }

    /// Windows の checkout は autocrlf で CRLF になるので、行末を揃えてから読む
    /// (`"\nimporters:\n"` のような改行込みの検索が外れる)。
    fn lf(text: &str) -> String {
        text.replace("\r\n", "\n")
    }

    /// THIRD-PARTY-NOTICES.txt の "Used by:" ブロックに並ぶ (package 名, version)。
    /// 生成物のエントリは区切り線 → `License: ...` → 空行 → "Used by:" の並びなので、
    /// その並びだけをブロックとして読む (ライセンス本文に同じ字面があっても数えない)。
    /// npm 側と Rust 側は "# Rust crates" の見出しで分かれている。
    fn packages_in_notices(notices: &str, section: &str) -> Vec<(String, String)> {
        let text = match section {
            "npm" => notices.split("# Rust crates").next(),
            "rust" => notices.split("# Rust crates").nth(1),
            _ => None,
        }
        .expect("the notices file has a Rust crates heading");
        let separator = "=".repeat(80);
        let mut packages = Vec::new();
        let mut in_block = false;
        let mut after_separator = false;
        let mut after_license = false;
        for line in text.lines() {
            if in_block {
                let mut words = line.strip_prefix("  ").unwrap_or("").split(' ');
                match (words.next(), words.next()) {
                    (Some(name), Some(version)) if !name.is_empty() => {
                        packages.push((name.to_string(), version.to_string()));
                    }
                    _ => in_block = false,
                }
                continue;
            }
            in_block = after_license && line == "Used by:";
            after_license = (after_separator && line.starts_with("License: "))
                || (after_license && line.is_empty());
            after_separator = line == separator;
        }
        packages
    }

    /// Cargo.lock の [[package]] ブロック。(name, version, dependencies の行)。
    fn locked_rust_packages(lock: &str) -> Vec<(String, String, Vec<String>)> {
        lock.split("[[package]]")
            .skip(1)
            .map(|block| {
                let field = |key: &str| {
                    block
                        .lines()
                        .find_map(|line| line.strip_prefix(key))
                        .map(|rest| rest.trim().trim_matches('"').to_string())
                        .unwrap_or_default()
                };
                let deps = block
                    .lines()
                    .filter_map(|line| line.strip_prefix(" \""))
                    .map(|line| line.trim_end_matches("\",").to_string())
                    .collect();
                (field("name = "), field("version = "), deps)
            })
            .collect()
    }

    /// Cargo.lock が arcvault の直接依存 `name` に選んだ version。同じ crate が 2 つ以上の
    /// version で入っている時は、ルートの dependencies に `name version` の形で書かれる。
    fn resolved_rust_version(lock: &[(String, String, Vec<String>)], name: &str) -> String {
        let root = lock
            .iter()
            .find(|(crate_name, _, _)| crate_name == "arcvault")
            .expect("Cargo.lock has the arcvault package");
        let entry = root
            .2
            .iter()
            .find(|dep| *dep == name || dep.starts_with(&format!("{name} ")))
            .unwrap_or_else(|| panic!("{name} is not a dependency of arcvault in Cargo.lock"));
        // 同名同 version で source が違う時は `name version (source)` になるので 2 語目だけ
        match entry.split(' ').nth(1) {
            Some(version) => version.to_string(),
            None => {
                let mut versions = lock
                    .iter()
                    .filter(|(crate_name, _, _)| crate_name == name)
                    .map(|(_, version, _)| version.clone());
                let version = versions.next().expect("the crate is in Cargo.lock");
                assert!(
                    versions.next().is_none(),
                    "{name} has several versions in Cargo.lock"
                );
                version
            }
        }
    }

    /// pnpm-lock.yaml の importers の `.` (このプロジェクト) が `group`
    /// (dependencies / devDependencies) に選んだ (name, version)。
    /// `name:` → `specifier:` → `version:` の 3 行で並ぶ。
    fn resolved_npm_versions(lock: &str, group: &str) -> Vec<(String, String)> {
        let importer = lock
            .split("\nimporters:\n")
            .nth(1)
            .expect("pnpm-lock.yaml has an importers section")
            .split("\npackages:\n")
            .next()
            .expect("importers come before packages");
        let header = format!("    {group}:");
        let mut resolved = Vec::new();
        let mut name = String::new();
        let mut in_group = false;
        for line in importer.lines() {
            if line.starts_with("    ") && !line.starts_with("     ") {
                in_group = line == header;
                continue;
            }
            if !in_group {
                continue;
            }
            if let Some(key) = line
                .strip_prefix("      ")
                .filter(|rest| !rest.starts_with(' '))
            {
                name = key.trim_end_matches(':').trim_matches('\'').to_string();
            } else if let Some(version) = line.strip_prefix("        version: ") {
                // peer 依存の括弧は notices の version には無い
                let version = version.split('(').next().unwrap_or(version).trim();
                resolved.push((name.clone(), version.to_string()));
            }
        }
        resolved
    }

    /// devDependencies のまま runtime として bundle される package
    /// (scripts/generate-third-party-notices.sh の BUNDLED_RUNTIME のうち、直接依存のもの)。
    const BUNDLED_DEV_DEPENDENCIES: [&str; 2] = ["svelte", "@sveltejs/kit"];

    /// 直接依存が、Cargo.lock が選んだ version で載っているか。名前だけだと、上げた依存の
    /// 旧 version が推移依存として残っている時に通ってしまう。
    #[test]
    fn third_party_notices_list_every_direct_rust_dependency() {
        // Arrange
        let deps = direct_rust_dependencies(&lf(include_str!("../Cargo.toml")));
        assert!(deps.contains(&"zip".to_string()), "parsed deps: {deps:?}");
        let lock = locked_rust_packages(&lf(include_str!("../Cargo.lock")));
        let listed = packages_in_notices(&lf(THIRD_PARTY_NOTICES), "rust");
        assert!(listed.len() > 100, "parsed notices: {listed:?}");

        // Act
        let missing: Vec<(String, String)> = deps
            .iter()
            .map(|name| (name.clone(), resolved_rust_version(&lock, name)))
            .filter(|entry| !listed.contains(entry))
            .collect();

        // Assert
        assert!(
            missing.is_empty(),
            "not in THIRD-PARTY-NOTICES.txt (run `pnpm notices`): {missing:?}"
        );
    }

    /// 載っている crate の version が Cargo.lock と食い違えば、依存を上げたのに
    /// `pnpm notices` を流していない。
    #[test]
    fn third_party_notices_match_cargo_lock_versions() {
        // Arrange
        let lock = locked_rust_packages(&lf(include_str!("../Cargo.lock")));
        let listed = packages_in_notices(&lf(THIRD_PARTY_NOTICES), "rust");
        assert!(listed.len() > 100, "parsed notices: {listed:?}");

        // Act
        let stale: Vec<&(String, String)> = listed
            .iter()
            .filter(|(name, version)| !lock.iter().any(|(n, v, _)| n == name && v == version))
            .collect();

        // Assert
        assert!(
            stale.is_empty(),
            "not in Cargo.lock (run `pnpm notices`): {stale:?}"
        );
    }

    /// npm の直接依存 (dependencies と、bundle される devDependencies) が pnpm-lock.yaml の
    /// 選んだ version で載っているか。notices の npm 側は node_modules の package.json から
    /// 書くので、lock を更新して install と再生成を忘れると古い version のまま残る。
    #[test]
    fn third_party_notices_list_every_npm_runtime_dependency() {
        // Arrange
        let lock = lf(include_str!("../../pnpm-lock.yaml"));
        let mut expected = resolved_npm_versions(&lock, "dependencies");
        assert!(
            expected.iter().any(|(name, _)| name == "@tauri-apps/api"),
            "parsed lock importer: {expected:?}"
        );
        let dev = resolved_npm_versions(&lock, "devDependencies");
        for name in BUNDLED_DEV_DEPENDENCIES {
            let entry = dev
                .iter()
                .find(|(dev_name, _)| dev_name == name)
                .unwrap_or_else(|| panic!("{name} is not a devDependency in pnpm-lock.yaml"));
            expected.push(entry.clone());
        }
        let listed = packages_in_notices(&lf(THIRD_PARTY_NOTICES), "npm");

        // Act
        let missing: Vec<&(String, String)> = expected
            .iter()
            .filter(|entry| !listed.contains(entry))
            .collect();

        // Assert
        assert!(
            missing.is_empty(),
            "not in THIRD-PARTY-NOTICES.txt (run `pnpm notices`): {missing:?}"
        );
    }

    /// notices の npm 側の全 package が pnpm-lock.yaml の packages にその version で
    /// あるか (推移依存の esm-env も含む)。
    #[test]
    fn third_party_notices_match_pnpm_lock_versions() {
        // Arrange
        let lock = lf(include_str!("../../pnpm-lock.yaml"));
        let listed = packages_in_notices(&lf(THIRD_PARTY_NOTICES), "npm");
        assert!(listed.len() >= 8, "parsed notices: {listed:?}");

        // Act
        let stale: Vec<&(String, String)> = listed
            .iter()
            .filter(|(name, version)| {
                let plain = format!("\n  {name}@{version}:");
                let quoted = format!("\n  '{name}@{version}':");
                !lock.contains(&plain) && !lock.contains(&quoted)
            })
            .collect();

        // Assert
        assert!(
            stale.is_empty(),
            "not in pnpm-lock.yaml (run `pnpm install` and `pnpm notices`): {stale:?}"
        );
    }

    /// Windows の checkout (CRLF) でも同じ結果になること。
    #[test]
    fn notices_parsers_accept_crlf() {
        // Arrange
        let notices = "x\r\n# Rust crates\r\n\r\n".to_string()
            + &"=".repeat(80)
            + "\r\nLicense: MIT\r\n\r\nUsed by:\r\n  serde 1.0.0 (u)\r\n\r\ntext\r\n";
        let lock = "lockfileVersion: '9.0'\r\nimporters:\r\n  .:\r\n    dependencies:\r\n      '@tauri-apps/api':\r\n        specifier: ^2\r\n        version: 2.9.1\r\npackages:\r\n";

        // Act
        let listed = packages_in_notices(&lf(&notices), "rust");
        let resolved = resolved_npm_versions(&lf(lock), "dependencies");

        // Assert
        assert_eq!(listed, vec![("serde".to_string(), "1.0.0".to_string())]);
        assert_eq!(
            resolved,
            vec![("@tauri-apps/api".to_string(), "2.9.1".to_string())]
        );
    }

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
        let dir = temp_test_dir("unique_dir");

        // Non-existent path is returned unchanged.
        let missing = dir.join("foo");
        assert_eq!(get_unique_dir_path(&missing), missing);

        // For an existing folder, `_N` is appended to the whole name without
        // inventing a `.zip` extension or splitting on dots.
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

    #[test]
    fn rejects_backslash_zip_slip_paths() {
        // On Windows, `PathBuf::push` treats `\` as a separator, so
        // backslash-separated traversal must be rejected as well.
        let root = Path::new("/tmp/extract");
        assert!(safe_extract_path(root, "..\\escape.txt").is_none());
        assert!(safe_extract_path(root, "a\\..\\..\\b.txt").is_none());
        assert!(safe_extract_path(root, "a/..\\..\\b.txt").is_none());
        // Backslash-separated (non-traversal) names now become nested dirs.
        assert_eq!(
            safe_extract_path(root, "sub\\file.txt"),
            Some(Path::new("/tmp/extract/sub/file.txt").to_path_buf())
        );
    }

    #[cfg(windows)]
    #[test]
    fn rejects_drive_prefixes_on_windows() {
        // A `C:` component would make `PathBuf::push` replace the whole path.
        let root = Path::new("C:\\extract");
        assert!(safe_extract_path(root, "C:\\evil.txt").is_none());
        assert!(safe_extract_path(root, "C:/evil.txt").is_none());
    }

    #[test]
    fn unique_output_path_appends_a_counter_before_the_extension() {
        let dir = temp_test_dir("unique_output");

        // Non-existent path is returned unchanged.
        let missing = dir.join("foo.zip");
        assert_eq!(get_unique_output_path(&missing), missing);

        let existing = dir.join("archive.zip");
        std::fs::write(&existing, b"").unwrap();
        assert_eq!(get_unique_output_path(&existing), dir.join("archive_1.zip"));

        // Keeps counting up while the candidate is taken.
        std::fs::write(dir.join("archive_1.zip"), b"").unwrap();
        assert_eq!(get_unique_output_path(&existing), dir.join("archive_2.zip"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_directory_to_zip_excludes_macos_junk() {
        // Windows で開いたときに .DS_Store や ._ ファイルが見えないことが
        // このアプリの売りなので、除外は落とせない。
        let dir = temp_test_dir("junk_exclusion");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::create_dir_all(dir.join("__MACOSX")).unwrap();
        std::fs::write(dir.join("keep.txt"), b"keep").unwrap();
        std::fs::write(dir.join(".DS_Store"), b"junk").unwrap();
        std::fs::write(dir.join("._keep.txt"), b"junk").unwrap();
        std::fs::write(dir.join("sub/nested.txt"), b"nested").unwrap();
        std::fs::write(dir.join("__MACOSX/x"), b"junk").unwrap();

        let mut buffer = io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            add_directory_to_zip(&mut zip, &dir, "").unwrap();
            zip.finish().unwrap();
        }

        let mut archive = zip::ZipArchive::new(io::Cursor::new(buffer.into_inner())).unwrap();
        let mut names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        names.sort();

        assert_eq!(names, vec!["keep.txt", "sub/", "sub/nested.txt"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_directory_to_zip_uses_forward_slashes_and_the_prefix() {
        let dir = temp_test_dir("zip_prefix");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/nested.txt"), b"nested").unwrap();

        let mut buffer = io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            add_directory_to_zip(&mut zip, &dir, "top").unwrap();
            zip.finish().unwrap();
        }

        let mut archive = zip::ZipArchive::new(io::Cursor::new(buffer.into_inner())).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();

        // ZIP のエントリ名は仕様上つねに `/` 区切り (Windows でも `\` にしない)
        assert!(names.iter().all(|n| !n.contains('\\')), "{:?}", names);
        assert!(names.contains(&"top/sub/nested.txt".to_string()), "{:?}", names);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stored_files_are_deflated() {
        // 無圧縮や未対応の方式にすると Windows の標準機能で開けなくなる
        let dir = temp_test_dir("compression");
        std::fs::write(dir.join("a.txt"), b"hello hello hello hello").unwrap();

        let mut buffer = io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            add_directory_to_zip(&mut zip, &dir, "").unwrap();
            zip.finish().unwrap();
        }

        let mut archive = zip::ZipArchive::new(io::Cursor::new(buffer.into_inner())).unwrap();
        let entry = archive.by_name("a.txt").unwrap();
        assert_eq!(entry.compression(), CompressionMethod::Deflated);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(windows))]
    #[test]
    fn keeps_colon_names_on_unix() {
        // `:` is an ordinary filename character outside Windows; entries like
        // timestamped logs must not be skipped.
        let root = Path::new("/tmp/extract");
        assert_eq!(
            safe_extract_path(root, "logs/2026-07-22T12:00:00.txt"),
            Some(Path::new("/tmp/extract/logs/2026-07-22T12:00:00.txt").to_path_buf())
        );
    }
}
