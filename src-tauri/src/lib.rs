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
}

impl ZipResult {
    fn success(path: String) -> Self {
        Self {
            success: true,
            output_path: Some(path),
            error: None,
        }
    }

    fn error(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            output_path: None,
            error: Some(msg.into()),
        }
    }
}

/// パスを正規化し、存在確認を行う
fn canonicalize_path(path: &str) -> Result<std::path::PathBuf, String> {
    let p = Path::new(path);
    p.canonicalize()
        .map_err(|e| format!("パスの解決に失敗: {}", e))
}

/// 同名ファイルが存在する場合は連番を付与したパスを返す
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

/// ZIP圧縮オプション
fn get_zip_options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6))
}

/// ディレクトリをZIPに追加
fn add_directory_to_zip<W: Write + io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    source_dir: &Path,
    prefix: &str,
) -> io::Result<()> {
    let options = get_zip_options();

    for entry in WalkDir::new(source_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();

        // .DS_Store や __MACOSX を除外
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name == ".DS_Store" || name == "__MACOSX" || name.starts_with("._") {
                continue;
            }
        }

        let relative_path = path.strip_prefix(source_dir)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        // ZIP内でのパスを構築
        let zip_path = if prefix.is_empty() {
            relative_path.to_string_lossy().to_string()
        } else {
            format!("{}/{}", prefix, relative_path.to_string_lossy())
        };

        // 空のパス（ルート）はスキップ
        if zip_path.is_empty() {
            continue;
        }

        if path.is_dir() {
            // ディレクトリエントリを追加（末尾に / を付ける）
            let dir_path = if zip_path.ends_with('/') {
                zip_path
            } else {
                format!("{}/", zip_path)
            };
            zip.add_directory(&dir_path, options.clone())?;
        } else {
            // ファイルを追加
            zip.start_file(&zip_path, options.clone())?;
            let mut file = File::open(path)?;
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)?;
            zip.write_all(&buffer)?;
        }
    }

    Ok(())
}

/// フォルダを Zip 圧縮
/// include_parent: true の場合、フォルダ自体を含める（--keepParent相当）
#[tauri::command]
fn zip_folder(
    folder_path: &str,
    output_dir: &str,
    include_parent: bool,
) -> ZipResult {
    // パスを正規化
    let folder = match canonicalize_path(folder_path) {
        Ok(p) => p,
        Err(e) => return ZipResult::error(e),
    };

    if !folder.is_dir() {
        return ZipResult::error("指定されたパスはフォルダではありません");
    }

    let output_dir_path = match canonicalize_path(output_dir) {
        Ok(p) => p,
        Err(e) => return ZipResult::error(e),
    };

    let folder_name = folder.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("archive");

    let base_output_path = output_dir_path.join(format!("{}.zip", folder_name));
    let output_path = get_unique_output_path(&base_output_path);

    // ZIPファイルを作成
    let file = match File::create(&output_path) {
        Ok(f) => f,
        Err(e) => return ZipResult::error(format!("ZIPファイルの作成に失敗: {}", e)),
    };

    let mut zip = zip::ZipWriter::new(file);

    // プレフィックス（include_parent が true なら folder_name）
    let prefix = if include_parent { folder_name } else { "" };

    if let Err(e) = add_directory_to_zip(&mut zip, &folder, prefix) {
        return ZipResult::error(format!("ZIP圧縮に失敗: {}", e));
    }

    if let Err(e) = zip.finish() {
        return ZipResult::error(format!("ZIPの完了に失敗: {}", e));
    }

    match output_path.to_str() {
        Some(s) => ZipResult::success(s.to_string()),
        None => ZipResult::error("出力パスに無効な文字が含まれています"),
    }
}

/// 複数ファイルを Zip 圧縮
#[tauri::command]
fn zip_files(
    file_paths: Vec<String>,
    output_dir: &str,
    archive_name: &str,
) -> ZipResult {
    if file_paths.is_empty() {
        return ZipResult::error("ファイルが指定されていません");
    }

    // 出力先ディレクトリを正規化
    let output_dir_path = match canonicalize_path(output_dir) {
        Ok(p) => p,
        Err(e) => return ZipResult::error(e),
    };

    let base_output_path = output_dir_path.join(format!("{}.zip", archive_name));
    let output_path = get_unique_output_path(&base_output_path);

    // ZIPファイルを作成
    let file = match File::create(&output_path) {
        Ok(f) => f,
        Err(e) => return ZipResult::error(format!("ZIPファイルの作成に失敗: {}", e)),
    };

    let mut zip = zip::ZipWriter::new(file);
    let options = get_zip_options();

    // ファイルをZIPに追加
    for file_path in &file_paths {
        let src = match canonicalize_path(file_path) {
            Ok(p) => p,
            Err(_) => continue, // 存在しないファイルはスキップ
        };

        let file_name = src.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");

        // .DS_Store などを除外
        if file_name == ".DS_Store" || file_name.starts_with("._") {
            continue;
        }

        // アーカイブ内でのパス: archive_name/filename
        let zip_path = format!("{}/{}", archive_name, file_name);

        if src.is_dir() {
            // ディレクトリの場合は再帰的に追加
            if let Err(e) = add_directory_to_zip(&mut zip, &src, &zip_path) {
                return ZipResult::error(format!("ディレクトリの追加に失敗: {}", e));
            }
        } else {
            // ファイルを追加
            if let Err(e) = zip.start_file(&zip_path, options.clone()) {
                return ZipResult::error(format!("ファイルエントリの作成に失敗: {}", e));
            }

            let mut src_file = match File::open(&src) {
                Ok(f) => f,
                Err(e) => return ZipResult::error(format!("ファイルのオープンに失敗: {}", e)),
            };

            let mut buffer = Vec::new();
            if let Err(e) = src_file.read_to_end(&mut buffer) {
                return ZipResult::error(format!("ファイルの読み込みに失敗: {}", e));
            }

            if let Err(e) = zip.write_all(&buffer) {
                return ZipResult::error(format!("ファイルの書き込みに失敗: {}", e));
            }
        }
    }

    if let Err(e) = zip.finish() {
        return ZipResult::error(format!("ZIPの完了に失敗: {}", e));
    }

    match output_path.to_str() {
        Some(s) => ZipResult::success(s.to_string()),
        None => ZipResult::error("出力パスに無効な文字が含まれています"),
    }
}

/// ダウンロードフォルダのパスを取得
#[tauri::command]
fn get_downloads_dir() -> Option<String> {
    dirs::download_dir().map(|p| p.to_string_lossy().to_string())
}

/// デスクトップフォルダのパスを取得
#[tauri::command]
fn get_desktop_dir() -> Option<String> {
    dirs::desktop_dir().map(|p| p.to_string_lossy().to_string())
}

/// パスの親ディレクトリを取得
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
            get_downloads_dir,
            get_desktop_dir,
            get_parent_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
