//! Recoverable cleanup of interview history only. Profile/model/configuration
//! data are never selected. A failed Trash operation must not clear live data.
use serde::{Deserialize,Serialize};
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
    pub saved_sessions: usize,
    pub staged_archives: usize,
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

#[cfg(windows)]
pub fn move_to_trash(path: &Path) -> Result<PathBuf, String> {
    // The uniquely named archive is recycled before committing any database deletion.
    // Windows shell parsing names must resolve to readable physical files so the
    // existing hash verification and rollback remain effective.
    let original = path.canonicalize().map_err(|e| e.to_string())?;
    trash::delete(path).map_err(|e| format!("无法移入 Windows 回收站，已停止清理：{e}"))?;
    let items = trash::os_limited::list().map_err(|e| format!("已请求回收，但无法核对回收站，未清除数据库：{e}"))?;
    for item in items {
        let same_name = item.name == original.file_name().ok_or("无效归档名")?;
        // Windows canonical paths may carry a verbatim prefix; compare the parent
        // through the filesystem instead of comparing their spelling.
        let same_parent = item.original_parent.canonicalize().ok() == original.parent().map(|p| p.to_path_buf());
        if same_name && same_parent {
            let destination = PathBuf::from(item.id);
            if !path.exists() && destination.is_dir() { return Ok(destination); }
        }
    }
    Err("无法核对 Windows 回收站中的归档，未清除数据库；请检查回收站".into())
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn move_to_trash(_path: &Path) -> Result<PathBuf, String> {
    Err("当前平台尚未接入可恢复清理；未删除任何记录".into())
}

const ARCHIVE_PREFIX:&str="Meetily-历史清理-";
const ARCHIVE_SCOPE:&str="Interview practice history only; profile and settings excluded";
#[derive(Deserialize)]
struct ArchiveIdentity {version:u32,scope:String}
#[derive(Default)]
struct OwnedFolders {recordings:Vec<PathBuf>,archives:Vec<PathBuf>,unfinished:usize}
#[derive(Debug,Serialize)]
pub struct ResidueStatus {pub recording_folders:usize,pub unfinished_folders:usize,pub staged_archives:usize,pub database_meetings:usize,pub saved_sessions:i64}
fn owned_folders(root:&Path)->Result<OwnedFolders,String> {
    let mut owned=OwnedFolders::default();
    let root_meta=match fs::symlink_metadata(root){Ok(m)=>m,Err(e) if e.kind()==std::io::ErrorKind::NotFound=>return Ok(owned),Err(e)=>return Err(e.to_string())};
    if root_meta.file_type().is_symlink() || !root_meta.is_dir(){return Err("记录根目录不是普通目录，已停止检查/清理".into());}
    for entry in fs::read_dir(root).map_err(|e|e.to_string())? {
        let entry=entry.map_err(|e|e.to_string())?;let name=entry.file_name();let name=name.to_string_lossy();
        let archive=name.starts_with(ARCHIVE_PREFIX);
        if !archive && !name.starts_with("Interview practice_"){continue;}
        let path=entry.path();let meta=fs::symlink_metadata(&path).map_err(|e|e.to_string())?;
        if meta.file_type().is_symlink() || !meta.is_dir(){return Err(format!("面试残留路径不是普通目录，未清理：{}",path.display()));}
        let identity=path.join(if archive{"history.json"}else{"metadata.json"});
        let meta=fs::symlink_metadata(&identity).map_err(|e|format!("无法核对记录归属 {}：{e}",path.display()))?;
        if !meta.is_file() || meta.file_type().is_symlink(){return Err("记录归属文件不是普通文件，未清理".into());}
        if archive {
            // Deserialize only identity fields; never materialize saved dialogue
            // merely to count an interrupted cleanup's staging directory.
            if meta.len()>256*1024*1024 {return Err("历史残留归档过大，请人工检查；未清理".into());}
            let id:ArchiveIdentity=serde_json::from_reader(std::io::BufReader::new(fs::File::open(&identity).map_err(|e|e.to_string())?)).map_err(|e|format!("历史残留归档身份无效：{e}"))?;
            if id.version!=1 || id.scope!=ARCHIVE_SCOPE {return Err("历史残留归档归属不匹配，未清理".into());}
            owned.archives.push(path);
        }else{
            if meta.len()>1024*1024{return Err("面试记录元数据过大，未清理".into());}
            let value:Value=serde_json::from_reader(fs::File::open(&identity).map_err(|e|e.to_string())?).map_err(|e|format!("记录元数据无效：{e}"))?;
            if value.get("meeting_name").and_then(Value::as_str)==Some(TITLE){
                if value.get("status").and_then(Value::as_str)!=Some("completed"){owned.unfinished+=1;}
                owned.recordings.push(path);
            }
        }
    }
    owned.recordings.sort();owned.archives.sort();Ok(owned)
}
fn select_meetings(rows:Vec<sqlx::sqlite::SqliteRow>,folders:&[PathBuf])->Result<Vec<String>,String> {
    let mut selected=vec![];
    for meeting in rows {
        let title:String=meeting.try_get("title").map_err(|e|e.to_string())?;
        let folder:Option<String>=meeting.try_get("folder_path").map_err(|e|e.to_string())?;
        let matches_folder=folder.as_ref().is_some_and(|p|folders.contains(&PathBuf::from(p)));
        if title==TITLE || matches_folder {
            if let Some(path)=folder.filter(|p|!p.is_empty()) {
                let path=PathBuf::from(path);
                if path.exists() && !folders.contains(&path){return Err(format!("数据库中的录音目录未通过归属核对，已停止检查/清理：{}",path.display()));}
            }
            selected.push(meeting.try_get::<String,_>("id").map_err(|e|e.to_string())?);
        }
    }Ok(selected)
}
pub async fn residue_status(pool:&SqlitePool,recordings:&Path)->Result<ResidueStatus,String> {
    let root=match fs::symlink_metadata(recordings) {
        Ok(m) if m.file_type().is_symlink()=>return Err("记录根目录是符号链接，未检查或清理".into()),
        Ok(_)=>recordings.canonicalize().map_err(|e|e.to_string())?,
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>recordings.to_path_buf(),
        Err(e)=>return Err(e.to_string()),
    };
    let folders=owned_folders(&root)?;
    let rows=sqlx::query("SELECT id,title,folder_path FROM meetings").fetch_all(pool).await.map_err(|e|e.to_string())?;
    let selected=select_meetings(rows,&folders.recordings)?;
    let exists:i64=sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='interview_sessions'").fetch_one(pool).await.map_err(|e|e.to_string())?;
    let saved_sessions=if exists>0{sqlx::query_scalar("SELECT count(*) FROM interview_sessions").fetch_one(pool).await.map_err(|e|e.to_string())?}else{0};
    Ok(ResidueStatus{recording_folders:folders.recordings.len(),unfinished_folders:folders.unfinished,staged_archives:folders.archives.len(),database_meetings:selected.len(),saved_sessions})
}

pub(crate) fn rows_to_json(rows: Vec<sqlx::sqlite::SqliteRow>) -> Result<Vec<Value>, String> {
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
    clear_scoped_history_with(pool,recordings,snapshot,true,trash).await
}
/// Temporary records only: explicitly saved snapshots remain usable.
pub async fn clear_temporary_history_with<F>(pool:&SqlitePool,recordings:&Path,snapshot:Value,trash:F)->Result<ClearReport,String>
where F:FnOnce(&Path)->Result<PathBuf,String> {
    clear_scoped_history_with(pool,recordings,snapshot,false,trash).await
}
async fn clear_scoped_history_with<F>(pool:&SqlitePool,recordings:&Path,snapshot:Value,include_saved:bool,trash:F)->Result<ClearReport,String>
where F:FnOnce(&Path)->Result<PathBuf,String> {
    let snapshot_bytes = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
    if snapshot_bytes.len() > 32 * 1024 * 1024 { return Err("本次历史超过 32 MiB，请先导出后分批清理".into()); }
    fs::create_dir_all(recordings).map_err(|e| e.to_string())?;
    if fs::symlink_metadata(recordings).map_err(|e| e.to_string())?.file_type().is_symlink() {
        return Err("录音目录是符号链接，已停止清理".into());
    }
    let root = recordings.canonicalize().map_err(|e| e.to_string())?;
    let owned=owned_folders(&root)?;
    let recording_folders=owned.recordings.len();let staged_archives=owned.archives.len();
    if include_saved {crate::interview_sessions::init(pool).await?;}
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let meetings=sqlx::query("SELECT id,title,folder_path FROM meetings").fetch_all(&mut *tx).await.map_err(|e|e.to_string())?;
    let selected=select_meetings(meetings,&owned.recordings)?;
    let folders:Vec<_>=owned.recordings.into_iter().chain(owned.archives).collect();
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
    let saved=if include_saved {rows_to_json(sqlx::query("SELECT * FROM interview_sessions ORDER BY id").fetch_all(&mut *tx).await.map_err(|e|e.to_string())?)?}else{vec![]};
    let saved_sessions=saved.len();
    database.insert("interview_sessions".into(),json!(saved));
    // Stage on the recordings volume so moving audio never needs copy+delete.
    let staging = root.join(format!("Meetily-历史清理-{}-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"), uuid::Uuid::new_v4()));
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    fs::create_dir(staging.join("recordings")).map_err(|e| e.to_string())?;
    let archive = json!({"version":1,"created_at":chrono::Utc::now().to_rfc3339(),
        "scope":ARCHIVE_SCOPE,"includes_saved_sessions":include_saved,
        "original_recording_paths":folders,"database_rows":database,"session_and_recovery":snapshot});
    let archive_path = staging.join("history.json");
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&archive_path).map_err(|e| e.to_string())?;
    use std::io::Write;
    let archive_bytes=serde_json::to_vec_pretty(&archive).map_err(|e|e.to_string())?;
    let archive_hash=interview_core::digest(&archive_bytes);
    file.write_all(&archive_bytes).map_err(|e|e.to_string())?;
    drop(archive_bytes);
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
    let verification=fs::read(trashed.join("history.json")).map(|bytes|interview_core::digest(&bytes)==archive_hash);
    if !matches!(verification,Ok(true)) {
        let restore=restore_folders(&trashed,&moved);
        return Err(format!("废纸篓历史备份校验失败，未清空数据库；录音恢复结果：{restore:?}"));
    }
    let removal = async {
        for &table in TABLES {
            let key = if table == "meetings" { "id" } else { "meeting_id" };
            for id in &selected {
                sqlx::query(&format!("DELETE FROM {table} WHERE {key} = ?"))
                    .bind(id).execute(&mut *tx).await?;
            }
        }
        if include_saved {sqlx::query("DELETE FROM interview_sessions").execute(&mut *tx).await?;}
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
    Ok(ClearReport { archive_path: trashed.to_string_lossy().into_owned(), recording_folders, database_meetings: selected.len(), saved_sessions, staged_archives })
}

#[cfg(test)]
mod residue_tests {
    use super::*;
    async fn pool()->SqlitePool {
        let pool=SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE meetings(id TEXT,title TEXT,folder_path TEXT)").execute(&pool).await.unwrap();pool
    }
    #[tokio::test]
    async fn metadata_only_inspection_counts_owned_and_unfinished_records_without_initializing_saved_sessions() {
        let root=std::env::temp_dir().join(format!("meetily-residue-{}",uuid::Uuid::new_v4()));
        let pool=pool().await;
        let empty=residue_status(&pool,&root).await.unwrap();assert_eq!(empty.recording_folders,0);assert!(!root.exists());
        fs::create_dir(&root).unwrap();let root=root.canonicalize().unwrap();
        for (name,title,status) in [("Interview practice_open",TITLE,"recording"),("Interview practice_done",TITLE,"completed"),("Interview practice_unrelated","Other","completed")] {
            let folder=root.join(name);fs::create_dir(&folder).unwrap();fs::write(folder.join("metadata.json"),json!({"meeting_name":title,"status":status}).to_string()).unwrap();
            fs::write(folder.join("transcripts.json"),"SENSITIVE_BODY_MUST_NOT_BE_PARSED_OR_RETURNED").unwrap();
        }
        sqlx::query("INSERT INTO meetings VALUES('owned',?,?),('other','Other',null)").bind(TITLE).bind(root.join("Interview practice_done").to_str().unwrap()).execute(&pool).await.unwrap();
        let stage=root.join(format!("{ARCHIVE_PREFIX}test"));fs::create_dir(&stage).unwrap();fs::write(stage.join("history.json"),json!({"version":1,"scope":ARCHIVE_SCOPE,"secret_body":"NOT_RETURNED"}).to_string()).unwrap();
        let found=residue_status(&pool,&root).await.unwrap();assert_eq!(found.recording_folders,2);assert_eq!(found.unfinished_folders,1);assert_eq!(found.database_meetings,1);assert_eq!(found.staged_archives,1);
        assert!(!serde_json::to_string(&found).unwrap().contains("BODY"));
        let saved_table:i64=sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE name='interview_sessions'").fetch_one(&pool).await.unwrap();assert_eq!(saved_table,0);
        assert!(stage.exists()); // Inspection never moves files or modifies rows.
    }
    #[tokio::test]
    async fn unknown_archive_identity_and_symlinks_fail_closed() {
        let root=std::env::temp_dir().join(format!("meetily-residue-invalid-{}",uuid::Uuid::new_v4()));fs::create_dir(&root).unwrap();
        let pool=pool().await;let stage=root.join(format!("{ARCHIVE_PREFIX}unrelated"));fs::create_dir(&stage).unwrap();
        fs::write(stage.join("history.json"),json!({"version":1,"scope":"not this application"}).to_string()).unwrap();
        assert!(residue_status(&pool,&root).await.unwrap_err().contains("归属"));assert!(stage.exists());
        #[cfg(unix)] {
            let link=root.with_extension("symlink");std::os::unix::fs::symlink(root.join("nonexistent"),&link).unwrap();
            assert!(residue_status(&pool,&link).await.unwrap_err().contains("符号链接"));
        }
    }
}
