//! Local recovery checkpoints, separate from explicitly saved sessions.
//! Epoch fencing makes a late write after history cleanup fail atomically.
use serde::Serialize;
use serde_json::Value;
use sqlx::{Row, SqliteConnection, SqlitePool};

const MAX_PAYLOAD: usize = 16 * 1024 * 1024;
const MAX_TOTAL: i64 = 128 * 1024 * 1024;
#[derive(Debug, Serialize)]
pub struct DraftSummary {
    pub id: String, pub revision: i64, pub updated_at: String,
    pub turns: i64, pub calls: i64,
}
#[derive(Debug, Serialize)]
pub struct DraftList { pub epoch: i64, pub drafts: Vec<DraftSummary> }

pub async fn init(pool: &SqlitePool) -> Result<(), String> {
    sqlx::query("CREATE TABLE IF NOT EXISTS tingxian_recovery_epoch (id INTEGER PRIMARY KEY CHECK(id=1), epoch INTEGER NOT NULL)")
        .execute(pool).await.map_err(|e| e.to_string())?;
    sqlx::query("INSERT OR IGNORE INTO tingxian_recovery_epoch(id,epoch) VALUES(1,0)")
        .execute(pool).await.map_err(|e| e.to_string())?;
    sqlx::query("CREATE TABLE IF NOT EXISTS tingxian_recovery_drafts (id TEXT PRIMARY KEY, revision INTEGER NOT NULL, updated_at TEXT NOT NULL, turns INTEGER NOT NULL, calls INTEGER NOT NULL, payload TEXT NOT NULL, sha256 TEXT NOT NULL)")
        .execute(pool).await.map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn list(pool: &SqlitePool) -> Result<DraftList, String> {
    init(pool).await?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let epoch = sqlx::query_scalar("SELECT epoch FROM tingxian_recovery_epoch WHERE id=1")
        .fetch_one(&mut *tx).await.map_err(|e| e.to_string())?;
    // Metadata only. No transcript body is read just to show recovery availability.
    let rows = sqlx::query("SELECT id,revision,updated_at,turns,calls FROM tingxian_recovery_drafts ORDER BY updated_at DESC,id DESC")
        .fetch_all(&mut *tx).await.map_err(|e| e.to_string())?;
    let drafts = rows.into_iter().map(|r| DraftSummary {id:r.get("id"), revision:r.get("revision"), updated_at:r.get("updated_at"), turns:r.get("turns"), calls:r.get("calls")}).collect();
    Ok(DraftList {epoch, drafts})
}

pub async fn write(pool: &SqlitePool, id: &str, epoch: i64, revision: i64, mut data: Value, key: &str) -> Result<(), String> {
    if id.is_empty() || id.len()>80 || !id.bytes().all(|c|c.is_ascii_alphanumeric() || c==b'-') || epoch<0 || revision<=0 || revision>9_007_199_254_740_991 {
        return Err("恢复草稿标识或版本无效".into());
    }
    let history = data["snapshot"]["history"].as_array().ok_or("恢复草稿缺少会话")?;
    let turns = history.len() as i64;
    let calls = data["snapshot"]["calls"].as_u64().filter(|n|*n<=u32::MAX as u64).ok_or("恢复草稿调用数无效")? as i64;
    if data["version"]!=1 || data["snapshot"]["version"]!=1 || turns>10000 || !data["composer"]["question"].is_string() || !data["composer"]["confirmed_answer"].is_string() {
        return Err("恢复草稿格式无效".into());
    }
    if serde_json::to_vec(&data).map_err(|e|e.to_string())?.len()>MAX_PAYLOAD {return Err("恢复草稿超过16 MiB，请手动导出；原草稿保留".into());}
    crate::redact_json_credentials(&mut data, Some(key))?;
    let payload = serde_json::to_string(&data).map_err(|e|e.to_string())?;
    if payload.len()>MAX_PAYLOAD {return Err("恢复草稿超过16 MiB；原草稿保留".into());}
    init(pool).await?;
    let mut tx = pool.begin().await.map_err(|e|e.to_string())?;
    let active_epoch:i64 = sqlx::query_scalar("SELECT epoch FROM tingxian_recovery_epoch WHERE id=1").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    if epoch!=active_epoch {return Err("历史已清理，过期草稿写入已拦截".into());}
    let old:Option<i64> = sqlx::query_scalar("SELECT revision FROM tingxian_recovery_drafts WHERE id=?").bind(id).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
    if old.is_some_and(|r|r>=revision) {return Err("过期草稿修订已拦截".into());}
    let (count,bytes):(i64,i64)=sqlx::query_as("SELECT count(*),coalesce(sum(length(CAST(payload AS BLOB))),0) FROM tingxian_recovery_drafts WHERE id<>?").bind(id).fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    if count>=200 || bytes+payload.len() as i64>MAX_TOTAL {return Err("恢复草稿达到200份或128 MiB，请先恢复/导出并清理历史；原草稿保留".into());}
    sqlx::query("INSERT INTO tingxian_recovery_drafts(id,revision,updated_at,turns,calls,payload,sha256) VALUES(?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,updated_at=excluded.updated_at,turns=excluded.turns,calls=excluded.calls,payload=excluded.payload,sha256=excluded.sha256")
        .bind(id).bind(revision).bind(chrono::Utc::now().to_rfc3339()).bind(turns).bind(calls).bind(&payload).bind(crate::digest(payload.as_bytes()))
        .execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e|e.to_string())
}

pub async fn load(pool:&SqlitePool,id:&str)->Result<Value,String> {
    init(pool).await?;
    let row=sqlx::query("SELECT payload,sha256 FROM tingxian_recovery_drafts WHERE id=?").bind(id).fetch_optional(pool).await.map_err(|e|e.to_string())?.ok_or("恢复草稿不存在，可能已清理")?;
    let payload:String=row.get("payload");let hash:String=row.get("sha256");
    if payload.len()>MAX_PAYLOAD || crate::digest(payload.as_bytes())!=hash {return Err("恢复草稿校验失败，原记录与当前窗口均保留".into());}
    serde_json::from_str(&payload).map_err(|_|"恢复草稿格式无效，原记录保留".into())
}

/// Only call inside the same transaction that deletes already-archived history.
pub async fn clear_archived(connection:&mut SqliteConnection)->Result<(),String> {
    sqlx::query("DELETE FROM tingxian_recovery_drafts").execute(&mut *connection).await.map_err(|e|e.to_string())?;
    sqlx::query("UPDATE tingxian_recovery_epoch SET epoch=epoch+1 WHERE id=1").execute(connection).await.map_err(|e|e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn data(text:&str)->Value {json!({"version":1,"snapshot":{"version":1,"history":[{"suggestion":text}],"calls":2},"composer":{"question":"未发送的问题","confirmed_answer":"本人输入尚未确认"}})}
    #[tokio::test]
    async fn checkpoints_reopen_and_late_revisions_cannot_erase_generated_text() {
        let dir=tempfile::tempdir().unwrap();let options=sqlx::sqlite::SqliteConnectOptions::new().filename(dir.path().join("recovery.sqlite")).create_if_missing(true);
        let pool=SqlitePool::connect_with(options.clone()).await.unwrap();
        write(&pool,"draft-1",0,1,data("部分正文"),"").await.unwrap();
        write(&pool,"draft-1",0,3,data("较新的完整正文"),"").await.unwrap();
        assert!(write(&pool,"draft-1",0,2,data("迟到的旧正文"),"").await.is_err());
        pool.close().await;
        let reopened=SqlitePool::connect_with(options).await.unwrap();
        assert_eq!(load(&reopened,"draft-1").await.unwrap(),data("较新的完整正文"));
        let rows=list(&reopened).await.unwrap();assert_eq!(rows.drafts.len(),1);assert_eq!(rows.drafts[0].revision,3);
        assert!(!serde_json::to_string(&rows).unwrap().contains("完整正文"));
    }
    #[tokio::test]
    async fn clear_commit_fences_late_writes_but_failed_clear_preserves_every_draft() {
        let pool=SqlitePool::connect("sqlite::memory:").await.unwrap();
        write(&pool,"draft-old",0,1,data("before"),"").await.unwrap();
        let mut tx=pool.begin().await.unwrap();clear_archived(&mut tx).await.unwrap();tx.rollback().await.unwrap();
        assert_eq!(load(&pool,"draft-old").await.unwrap(),data("before"));assert_eq!(list(&pool).await.unwrap().epoch,0);
        let mut tx=pool.begin().await.unwrap();clear_archived(&mut tx).await.unwrap();tx.commit().await.unwrap();
        assert!(write(&pool,"draft-old",0,9,data("late"),"").await.is_err());
        assert!(load(&pool,"draft-old").await.is_err());
        write(&pool,"draft-new",1,1,data("fresh"),"").await.unwrap();
        assert_eq!(list(&pool).await.unwrap().drafts.len(),1);
    }
    #[tokio::test]
    async fn corrupted_or_oversize_checkpoint_does_not_replace_last_readable_copy() {
        let pool=SqlitePool::connect("sqlite::memory:").await.unwrap();
        write(&pool,"one",0,1,data("retained"),"").await.unwrap();
        assert!(write(&pool,"one",0,2,data(&"x".repeat(MAX_PAYLOAD)),"").await.is_err());
        assert_eq!(load(&pool,"one").await.unwrap(),data("retained"));
        sqlx::query("UPDATE tingxian_recovery_drafts SET payload='{}' WHERE id='one'").execute(&pool).await.unwrap();
        assert!(load(&pool,"one").await.unwrap_err().contains("校验"));
        assert_eq!(list(&pool).await.unwrap().drafts.len(),1);
    }
    #[tokio::test]
    async fn new_window_never_overwrites_previous_window_and_secrets_are_redacted() {
        let pool=SqlitePool::connect("sqlite::memory:").await.unwrap();
        write(&pool,"previous",0,1,data("previous answer"),"").await.unwrap();
        let key="synthetic-secret-that-is-not-an-api-key";
        write(&pool,"current",0,1,data(key),key).await.unwrap();
        assert_eq!(load(&pool,"previous").await.unwrap(),data("previous answer"));
        assert!(!load(&pool,"current").await.unwrap().to_string().contains(key));
        assert_eq!(list(&pool).await.unwrap().drafts.len(),2);
    }
}
