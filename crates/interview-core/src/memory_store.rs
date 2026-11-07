//! Incremental raw-event journal in the host's EXISTING session SQLite pool.
//! UI eviction must wait for append's commit acknowledgement. No network here.
use crate::{session_memory::{Dependencies,JournalRecord,Ledger},Result};
use serde::{Deserialize,Serialize};
use sqlx::{Row,SqliteConnection,SqlitePool};

pub const TABLES:&[&str]=&["tingxian_memory_checkpoints","tingxian_memory_events","tingxian_memory_latest","tingxian_memory_sessions"];
const SESSION_BYTES:i64=32*1024*1024;
const TOTAL_BYTES:i64=128*1024*1024;
const SCHEMA:&str="CREATE TABLE IF NOT EXISTS tingxian_memory_sessions(id TEXT PRIMARY KEY,epoch INTEGER NOT NULL,dependencies TEXT NOT NULL,missing_older INTEGER NOT NULL,last_record INTEGER NOT NULL,last_event INTEGER NOT NULL,bytes INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS tingxian_memory_events(session_id TEXT NOT NULL,record_sequence INTEGER NOT NULL,event_id TEXT NOT NULL,sequence INTEGER NOT NULL,revision INTEGER NOT NULL,payload TEXT NOT NULL,sha256 TEXT NOT NULL,PRIMARY KEY(session_id,record_sequence));
CREATE TABLE IF NOT EXISTS tingxian_memory_latest(session_id TEXT NOT NULL,event_id TEXT NOT NULL,sequence INTEGER NOT NULL,revision INTEGER NOT NULL,record_sequence INTEGER NOT NULL,PRIMARY KEY(session_id,event_id),UNIQUE(session_id,sequence));";

#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct Session {pub id:String,pub epoch:u64,pub dependencies:Dependencies,pub missing_older_history:bool,pub last_record:u64,pub last_event:u64,pub bytes:u64}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub struct Ack {pub record_sequence:u64,pub sha256:String}

pub async fn init(pool:&SqlitePool)->Result<()> {crate::recovery::init(pool).await?;sqlx::raw_sql(SCHEMA).execute(pool).await.map_err(|e|e.to_string())?;crate::memory_checkpoint::init(pool).await?;Ok(())}
fn session(row:sqlx::sqlite::SqliteRow)->Result<Session>{
    let s=Session{id:row.try_get("id").map_err(|e|e.to_string())?,epoch:row.get::<i64,_>("epoch") as u64,dependencies:serde_json::from_str(row.get("dependencies")).map_err(|_|"会话依赖记录无效")?,missing_older_history:row.get::<i64,_>("missing_older")!=0,last_record:row.get::<i64,_>("last_record") as u64,last_event:row.get::<i64,_>("last_event") as u64,bytes:row.get::<i64,_>("bytes") as u64};
    if s.epoch>9_007_199_254_740_991||s.last_record>20_000||s.last_event>s.last_record||s.bytes>SESSION_BYTES as u64 {return Err("会话原文目录损坏或越界".into());}
    Ledger::new(&s.id,s.epoch,s.dependencies.clone(),s.missing_older_history)?;Ok(s)
}
fn decode(payload:&str,hash:&str)->Result<JournalRecord> {
    if payload.len()>84*1024||crate::digest(payload.as_bytes())!=hash {return Err("会话原文记录校验失败，保留原文件".into());}
    let record:JournalRecord=serde_json::from_str(payload).map_err(|_|"会话原文格式无效")?;record.event.validate()?;Ok(record)
}
/// Opening a legacy v1 snapshot must pass missing_older_history=true. It does
/// not manufacture events that were already evicted from its 120-entry ring.
pub async fn open(pool:&SqlitePool,id:&str,dependencies:Dependencies,missing_older_history:bool)->Result<Session> {
    init(pool).await?;
    let mut tx=pool.begin().await.map_err(|e|e.to_string())?;
    let epoch:i64=sqlx::query_scalar("SELECT epoch+1 FROM tingxian_recovery_epoch WHERE id=1").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    Ledger::new(id,epoch as u64,dependencies.clone(),missing_older_history)?;
    if let Some(row)=sqlx::query("SELECT * FROM tingxian_memory_sessions WHERE id=?").bind(id).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())? {
        let s=session(row)?;if s.epoch!=epoch as u64 {return Err("会话已过期，请新建会话".into());}return Ok(s);
    }
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM tingxian_memory_sessions").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    if count>=200 {return Err("会话事件库达到200份，请先导出并清理历史".into());}
    let dep=serde_json::to_string(&dependencies).map_err(|e|e.to_string())?;
    sqlx::query("INSERT INTO tingxian_memory_sessions VALUES(?,?,?,?,0,0,0)").bind(id).bind(epoch).bind(dep).bind(i64::from(missing_older_history)).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e|e.to_string())?;
    Ok(Session{id:id.into(),epoch:epoch as u64,dependencies,missing_older_history,last_record:0,last_event:0,bytes:0})
}
pub async fn append(pool:&SqlitePool,record:&JournalRecord)->Result<Ack> {
    let e=&record.event;e.validate()?;
    if record.record_sequence==0||record.record_sequence>20_000||e.epoch>i64::MAX as u64 {return Err("会话记录顺序超出限制".into());}
    let payload=serde_json::to_string(record).map_err(|e|e.to_string())?;let sha=crate::digest(payload.as_bytes());
    if payload.len()>84*1024 {return Err("会话记录过大，未写入".into());}
    let mut tx=pool.begin().await.map_err(|e|e.to_string())?;
    let epoch:i64=sqlx::query_scalar("SELECT epoch+1 FROM tingxian_recovery_epoch WHERE id=1").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    let s=session(sqlx::query("SELECT * FROM tingxian_memory_sessions WHERE id=?").bind(&e.session_id).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?.ok_or("会话已清空或尚未创建")?)?;
    if s.epoch!=e.epoch||epoch as u64!=e.epoch {return Err("历史已清空，迟到事件未写入".into());}
    if let Some((old,hash))=sqlx::query_as::<_,(String,String)>("SELECT payload,sha256 FROM tingxian_memory_events WHERE session_id=? AND record_sequence=?").bind(&e.session_id).bind(record.record_sequence as i64).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())? {
        if old!=payload||hash!=sha {return Err("同一记录顺序已被占用，未覆盖原文".into());}return Ok(Ack{record_sequence:record.record_sequence,sha256:sha});
    }
    if record.record_sequence!=s.last_record+1 {return Err("事件尚有未保存前序，不允许跳过".into());}
    let old=sqlx::query_as::<_,(String,String)>("SELECT e.payload,e.sha256 FROM tingxian_memory_latest l JOIN tingxian_memory_events e ON e.session_id=l.session_id AND e.record_sequence=l.record_sequence WHERE l.session_id=? AND l.event_id=?").bind(&e.session_id).bind(&e.id).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
    if let Some((payload,sha))=old {
        let previous=decode(&payload,&sha)?.event;
        if e.id!=previous.id||e.session_id!=previous.session_id||e.epoch!=previous.epoch||e.sequence!=previous.sequence||e.revision!=previous.revision+1||e.replaces_revision!=Some(previous.revision)||e.kind!=previous.kind||e.origin!=previous.origin||e.capture_session!=previous.capture_session||e.utterance_id!=previous.utterance_id||e.question!=previous.question {return Err("原文修订关系无效，未写入".into());}
    } else if e.sequence!=s.last_event+1||e.revision!=1||e.replaces_revision.is_some() {return Err("事件逻辑顺序或初始修订无效".into());}
    let bytes=payload.len() as i64;
    let total:i64=sqlx::query_scalar("SELECT coalesce(sum(bytes),0) FROM tingxian_memory_sessions").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    if s.bytes.saturating_add(bytes as u64)>SESSION_BYTES as u64||total.saturating_add(bytes)>TOTAL_BYTES {return Err("会话原文达到32 MiB/总128 MiB上限；未写入，请导出并新建会话".into());}
    sqlx::query("INSERT INTO tingxian_memory_events VALUES(?,?,?,?,?,?,?)").bind(&e.session_id).bind(record.record_sequence as i64).bind(&e.id).bind(e.sequence as i64).bind(e.revision as i64).bind(&payload).bind(&sha).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("INSERT INTO tingxian_memory_latest VALUES(?,?,?,?,?) ON CONFLICT(session_id,event_id) DO UPDATE SET revision=excluded.revision,record_sequence=excluded.record_sequence").bind(&e.session_id).bind(&e.id).bind(e.sequence as i64).bind(e.revision as i64).bind(record.record_sequence as i64).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    sqlx::query("UPDATE tingxian_memory_sessions SET last_record=?,last_event=max(last_event,?),bytes=bytes+? WHERE id=?").bind(record.record_sequence as i64).bind(e.sequence as i64).bind(bytes).bind(&e.session_id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e|e.to_string())?;Ok(Ack{record_sequence:record.record_sequence,sha256:sha})
}
/// Bounded pages; no whole-journal load for every live utterance.
pub async fn page(pool:&SqlitePool,id:&str,epoch:u64,after:u64,limit:u32)->Result<Vec<JournalRecord>> {
    if !(1..=64).contains(&limit)||after>20_000 {return Err("原文分页参数无效".into());}
    let mut tx=pool.begin().await.map_err(|e|e.to_string())?;
    let current:i64=sqlx::query_scalar("SELECT epoch+1 FROM tingxian_recovery_epoch WHERE id=1").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    let s=session(sqlx::query("SELECT * FROM tingxian_memory_sessions WHERE id=?").bind(id).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?.ok_or("会话原文不存在")?)?;
    if s.epoch!=epoch||current as u64!=epoch {return Err("旧会话分页已失效".into());}
    let rows=sqlx::query_as::<_,(String,String)>("SELECT payload,sha256 FROM tingxian_memory_events WHERE session_id=? AND record_sequence>? ORDER BY record_sequence LIMIT ?").bind(id).bind(after as i64).bind(i64::from(limit)).fetch_all(&mut *tx).await.map_err(|e|e.to_string())?;
    if rows.len()!=s.last_record.saturating_sub(after).min(u64::from(limit)) as usize {return Err("会话原文覆盖缺失，未声明恢复完整".into());}
    rows.into_iter().enumerate().map(|(i,(p,h))|{let r=decode(&p,&h)?;if r.event.session_id!=id||r.event.epoch!=epoch||r.record_sequence!=after+i as u64+1{return Err("原文页缺失或归属不一致".into());}Ok(r)}).collect()
}
/// Called ONLY after the same host transaction's archive has been independently
/// read back from Trash. recovery::clear_archived advances the global epoch.
pub async fn clear_archived(connection:&mut SqliteConnection)->Result<()> {
    for table in TABLES {sqlx::query(&format!("DELETE FROM {table}")).execute(&mut *connection).await.map_err(|e|e.to_string())?;}Ok(())
}
