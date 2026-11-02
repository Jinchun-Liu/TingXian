//! Recoverable cleanup of interview history only. Profile/model/configuration
//! data are never selected. A failed Trash operation must not clear live data.
use serde::Serialize;
use serde_json::{json, Map, Value};
use sqlx::{Column, Row, SqlitePool, TypeInfo, ValueRef};
use std::{fs, path::{Path, PathBuf}};

const TITLE: &str = "Interview practice";
const TABLES: &[&str] = &["transcripts", "transcript_chunks", "summary_processes", "meeting_notes", "meetings"];

#[derive(Debug, Serialize)]
pub struct ClearReport {
    pub archive_path: String,
    pub recording_folders: usize,
    pub database_meetings: usize,
}

#[cfg(target_os = "macos")]
pub fn move_to_trash(path: &Path) -> Result<PathBuf, String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    objc2::rc::autoreleasepool(|_| {
        let value = path.to_str().ok_or("废纸篓路径不是有效 UTF-8")?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(value));
        let mut result = None;
        NSFileManager::defaultManager().trashItemAtURL_resultingItemURL_error(&url, Some(&mut result))
            .map_err(|e| format!("无法移入 macOS 废纸篓，已停止清理：{e}"))?;
        let destination = result.and_then(|u| u.path()).ok_or("已请求移入废纸篓，但无法核对目标路径；请检查废纸篓")?;
        let destination = PathBuf::from(destination.to_string());
        if path.exists() || !destination.exists() { return Err("无法核对废纸篓移动结果，已停止清理".into()); }
        Ok(destination)
    })
}

#[cfg(not(target_os = "macos"))]
pub fn move_to_trash(_path: &Path) -> Result<PathBuf, String> {
    Err("当前清理功能需要 macOS 废纸篓；未删除任何记录".into())
}

fn owned_folders(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut owned = vec![];
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_name().to_string_lossy().starts_with("Interview practice_") { continue; }
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(format!("面试记录路径不是普通目录，已停止清理：{}", path.display()));
        }
        let metadata = path.join("metadata.json");
        let file = fs::symlink_metadata(&metadata).map_err(|e| format!("无法核对记录归属 {}：{e}", path.display()))?;
        if !file.is_file() || file.file_type().is_symlink() { return Err("记录元数据不是普通文件，已停止清理".into()); }
        let value: Value = serde_json::from_slice(&fs::read(&metadata).map_err(|e| e.to_string())?)
            .map_err(|e| format!("记录元数据无效，已停止清理：{e}"))?;
        if value.get("meeting_name").and_then(Value::as_str) == Some(TITLE) { owned.push(path); }
    }
    owned.sort();
    Ok(owned)
}

fn rows_to_json(rows: Vec<sqlx::sqlite::SqliteRow>) -> Result<Vec<Value>, String> {
    rows.into_iter().map(|row| {
        let mut object = Map::new();
        for column in row.columns() {
            let i = column.ordinal();
            let raw = row.try_get_raw(i).map_err(|e| e.to_string())?;
            let value = if raw.is_null() { Value::Null } else {
                match raw.type_info().name() {
                    "INTEGER" => json!(row.try_get::<i64,_>(i).map_err(|e| e.to_string())?),
                    "REAL" => json!(row.try_get::<f64,_>(i).map_err(|e| e.to_string())?),
                    "BLOB" => json!({"sqlite_blob_bytes": row.try_get::<Vec<u8>,_>(i).map_err(|e| e.to_string())?}),
                    _ => json!(row.try_get::<String,_>(i).map_err(|e| e.to_string())?),
                }
            };
            object.insert(column.name().to_owned(), value);
        }
        Ok(Value::Object(object))
    }).collect()
}

fn restore_folders(container: &Path, moved: &[PathBuf]) -> Result<(), String> {
    for original in moved.iter().rev() {
        let stored = container.join("recordings").join(original.file_name().ok_or("无效记录目录")?);
        if original.exists() { return Err(format!("恢复位置已被占用，记录仍在 {}", stored.display())); }
        fs::rename(&stored, original).map_err(|e| format!("自动恢复失败；记录保留在 {}：{e}", stored.display()))?;
    }
    Ok(())
}

/// The callback permits a real Trash integration test and a deterministic
/// failure test. Production always passes move_to_trash; no delete fallback.
pub async fn clear_history_with<F>(pool: &SqlitePool, recordings: &Path, snapshot: Value, trash: F) -> Result<ClearReport, String>
where F: FnOnce(&Path) -> Result<PathBuf, String> {
    let snapshot_bytes = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
    if snapshot_bytes.len() > 32 * 1024 * 1024 { return Err("本次历史超过 32 MiB，请先导出后分批清理".into()); }
    fs::create_dir_all(recordings).map_err(|e| e.to_string())?;
    if fs::symlink_metadata(recordings).map_err(|e| e.to_string())?.file_type().is_symlink() {
        return Err("录音目录是符号链接，已停止清理".into());
    }
    let root = recordings.canonicalize().map_err(|e| e.to_string())?;
    let folders = owned_folders(&root)?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let meetings = sqlx::query("SELECT * FROM meetings").fetch_all(&mut *tx).await.map_err(|e| e.to_string())?;
    let mut selected = vec![];
    for meeting in meetings {
        let title: String = meeting.try_get("title").map_err(|e| e.to_string())?;
        let folder: Option<String> = meeting.try_get("folder_path").map_err(|e| e.to_string())?;
        let matches_folder = folder.as_ref().map(|p| folders.contains(&PathBuf::from(p))).unwrap_or(false);
        if title == TITLE || matches_folder {
            if let Some(path) = folder.filter(|p| !p.is_empty()) {
                let path = PathBuf::from(path);
                if path.exists() && !folders.contains(&path) {
                    return Err(format!("数据库中的录音目录未通过归属核对，已停止清理：{}", path.display()));
                }
            }
            selected.push(meeting.try_get::<String,_>("id").map_err(|e| e.to_string())?);
        }
    }
    let mut database = Map::new();
    for &table in TABLES {
        let key = if table == "meetings" { "id" } else { "meeting_id" };
        let mut rows = vec![];
        for id in &selected {
            rows.extend(sqlx::query(&format!("SELECT * FROM {table} WHERE {key} = ?"))
                .bind(id).fetch_all(&mut *tx).await.map_err(|e| e.to_string())?);
        }
        database.insert(table.into(), json!(rows_to_json(rows)?));
    }
    // Stage on the recordings volume so moving audio never needs copy+delete.
    let staging = root.join(format!("Meetily-历史清理-{}-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"), uuid::Uuid::new_v4()));
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    fs::create_dir(staging.join("recordings")).map_err(|e| e.to_string())?;
    let archive = json!({"version":1,"created_at":chrono::Utc::now().to_rfc3339(),
        "scope":"Interview practice history only; profile and settings excluded",
        "original_recording_paths":folders,"database_rows":database,"session_and_recovery":snapshot});
    let archive_path = staging.join("history.json");
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&archive_path).map_err(|e| e.to_string())?;
    use std::io::Write;
    file.write_all(&serde_json::to_vec_pretty(&archive).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    let mut moved = vec![];
    for folder in &folders {
        let destination = staging.join("recordings").join(folder.file_name().ok_or("无效文件名")?);
        if let Err(error) = fs::rename(folder, destination) {
            let restore = restore_folders(&staging, &moved);
            return Err(format!("移动记录失败，未清空数据库：{error}。恢复结果：{restore:?}"));
        }
        moved.push(folder.clone());
    }
    let trashed = match trash(&staging) {
        Ok(path) => path,
        Err(error) => {
            let restore = restore_folders(&staging, &moved);
            return Err(format!("{error}；未清空数据库。恢复结果：{restore:?}；历史备份仍在 {}", staging.display()));
        }
    };
    let removal = async {
        for &table in TABLES {
            let key = if table == "meetings" { "id" } else { "meeting_id" };
            for id in &selected {
                sqlx::query(&format!("DELETE FROM {table} WHERE {key} = ?"))
                    .bind(id).execute(&mut *tx).await?;
            }
        }
        Ok::<(),sqlx::Error>(())
    }.await;
    if let Err(error) = removal {
        let _ = tx.rollback().await;
        let restore = restore_folders(&trashed, &moved);
        return Err(format!("数据库清理失败：{error}；录音恢复结果：{restore:?}；完整备份在 {}", trashed.display()));
    }
    if let Err(error) = tx.commit().await {
        let restore = restore_folders(&trashed, &moved);
        return Err(format!("数据库提交失败：{error}；录音恢复结果：{restore:?}；完整备份在 {}", trashed.display()));
    }
    Ok(ClearReport { archive_path: trashed.to_string_lossy().into_owned(), recording_folders: moved.len(), database_meetings: selected.len() })
}
