use std::path::Path;
use std::process::Command;
use serde::{Deserialize, Serialize};

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

/// ditto コマンドでフォルダを Zip 圧縮
/// include_parent: true の場合、フォルダ自体を含める（--keepParent）
#[tauri::command]
fn zip_folder(
    folder_path: &str,
    output_dir: &str,
    include_parent: bool,
) -> ZipResult {
    // パスを正規化（シンボリックリンクや .. を解決）
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

    let output_path_str = match output_path.to_str() {
        Some(s) => s,
        None => return ZipResult::error("出力パスに無効な文字が含まれています"),
    };

    let folder_str = match folder.to_str() {
        Some(s) => s,
        None => return ZipResult::error("フォルダパスに無効な文字が含まれています"),
    };

    let mut args = vec!["-c", "-k", "--sequesterRsrc"];
    if include_parent {
        args.push("--keepParent");
    }
    args.push(folder_str);
    args.push(output_path_str);

    match Command::new("ditto").args(&args).output() {
        Ok(output) => {
            if output.status.success() {
                ZipResult::success(output_path.to_string_lossy().to_string())
            } else {
                ZipResult::error(String::from_utf8_lossy(&output.stderr).to_string())
            }
        }
        Err(e) => ZipResult::error(e.to_string()),
    }
}

/// 複数ファイルを一時フォルダにコピーしてから Zip 圧縮
#[tauri::command]
fn zip_files(
    file_paths: Vec<String>,
    output_dir: &str,
    archive_name: &str,
) -> ZipResult {
    use std::fs;

    if file_paths.is_empty() {
        return ZipResult::error("ファイルが指定されていません");
    }

    // 出力先ディレクトリを正規化
    let output_dir_path = match canonicalize_path(output_dir) {
        Ok(p) => p,
        Err(e) => return ZipResult::error(e),
    };

    // 一時フォルダを作成
    let temp_dir = std::env::temp_dir().join(format!("arcvault_{}", std::process::id()));
    let staging_dir = temp_dir.join(archive_name);

    if let Err(e) = fs::create_dir_all(&staging_dir) {
        return ZipResult::error(format!("一時フォルダの作成に失敗: {}", e));
    }

    // ファイルを一時フォルダにコピー
    for file_path in &file_paths {
        // 各ファイルパスを正規化
        let src = match canonicalize_path(file_path) {
            Ok(p) => p,
            Err(_) => continue, // 存在しないファイルはスキップ
        };

        let file_name = src.file_name().unwrap_or_default();
        let dest = staging_dir.join(file_name);

        if src.is_dir() {
            if let Err(e) = copy_dir_recursive(&src, &dest) {
                let _ = fs::remove_dir_all(&temp_dir);
                return ZipResult::error(format!("フォルダのコピーに失敗: {}", e));
            }
        } else if let Err(e) = fs::copy(&src, &dest) {
            let _ = fs::remove_dir_all(&temp_dir);
            return ZipResult::error(format!("ファイルのコピーに失敗: {}", e));
        }
    }

    let base_output_path = output_dir_path.join(format!("{}.zip", archive_name));
    let output_path = get_unique_output_path(&base_output_path);

    // ditto で圧縮
    let result = Command::new("ditto")
        .args(["-c", "-k", "--sequesterRsrc", "--keepParent"])
        .arg(&staging_dir)
        .arg(&output_path)
        .output();

    // 一時フォルダを削除
    let _ = fs::remove_dir_all(&temp_dir);

    match result {
        Ok(output) => {
            if output.status.success() {
                ZipResult::success(output_path.to_string_lossy().to_string())
            } else {
                ZipResult::error(String::from_utf8_lossy(&output.stderr).to_string())
            }
        }
        Err(e) => ZipResult::error(e.to_string()),
    }
}

/// ディレクトリを再帰的にコピー
fn copy_dir_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    use std::fs;

    fs::create_dir_all(dest)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest_path = dest.join(entry.file_name());

        if path.is_dir() {
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            fs::copy(&path, &dest_path)?;
        }
    }

    Ok(())
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
