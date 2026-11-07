//! Atomic, immutable checkpoints for memory versions and attempt accounting.
//! Raw revisions stay in the existing journal; each checkpoint pins its prefix.
use crate::{compaction::{Attempt,Scheduler},memory_store,session_memory::{Dependencies,JournalRecord,Ledger,MemoryChunk},Result};
use serde::{Deserialize,Serialize};
use sqlx::{SqlitePool,SqliteConnection};

const MAX_PAYLOAD:usize=8*1024*1024;
const TOTAL_BYTES:i64=128*1024*1024;
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {pub session_id:String,pub epoch:u64,pub revision:u64,pub last_record:u64,pub sha256:String}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Boundary {pub session_id:String,pub epoch:u64,pub last_record:u64,pub last_sha256:String,pub checkpoint:Option<Reference>}
impl Boundary {
    pub fn capture(ledger:&Ledger,checkpoint:Option<Reference>)->Result<Self>{Ok(Self{session_id:ledger.session_id().into(),epoch:ledger.epoch(),last_record:ledger.journal().len() as u64,last_sha256:crate::digest(&serde_json::to_vec(&ledger.journal().last()).map_err(|e|e.to_string())?),checkpoint})}
    pub fn validate(&self)->Result<()> {if self.session_id.is_empty()||self.session_id.len()>160||!self.session_id.bytes().all(|c|c.is_ascii_alphanumeric()||b"-_:./".contains(&c))||self.epoch==0||self.last_record>20_000||self.last_sha256.len()!=64||!self.last_sha256.bytes().all(|c|c.is_ascii_hexdigit())||self.checkpoint.as_ref().is_some_and(|r|r.session_id!=self.session_id||r.epoch!=self.epoch||r.last_record>self.last_record||r.revision==0||r.revision>128||r.sha256.len()!=64){return Err("会话记忆原文边界无效".into());}Ok(())}
}
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub schema_version:u32,pub session_id:String,pub epoch:u64,pub last_record:u64,
    pub context_version:u64,pub dependencies:Dependencies,pub missing_older_history:bool,
    pub chunks:Vec<MemoryChunk>,pub attempts:Vec<Attempt>,
}
impl Checkpoint {
    pub fn capture(ledger:&Ledger,scheduler:&Scheduler)->Self {Self {schema_version:1,session_id:ledger.session_id().into(),epoch:ledger.epoch(),last_record:ledger.journal().len() as u64,context_version:ledger.version(),dependencies:ledger.dependencies().clone(),missing_older_history:ledger.missing_older_history(),chunks:ledger.chunks().to_vec(),attempts:scheduler.attempts().to_vec()}}
    fn restore(&self,journal:&[JournalRecord])->Result<(Ledger,Scheduler)> {
        if self.schema_version!=1||self.last_record!=journal.len() as u64 {return Err("会话记忆检查点版本或原文数量不一致".into());}
        let ledger=Ledger::restore(&self.session_id,self.epoch,self.dependencies.clone(),self.missing_older_history,journal,&self.chunks,self.context_version)?;
        // Verify every attempt's source input, including failed/interrupted ones.
        // An accounting row is never evidence that its candidate was committed.
        let originals:std::collections::BTreeMap<_,_>=journal.iter().map(|r|((r.event.id.as_str(),r.event.revision),&r.event)).collect();
        for a in &self.attempts {
            if a.snapshot.session_id!=self.session_id||a.snapshot.epoch!=self.epoch||a.snapshot.events.len()>64||a.snapshot.events.is_empty()||a.snapshot.base_context_version>self.context_version {return Err("压缩任务账本跨越会话或版本".into());}
            for e in &a.snapshot.events {if originals.get(&(e.id.as_str(),e.revision))!=Some(&e) {return Err("压缩账本引用缺少原文".into());}}
            if a.status==crate::compaction::Status::Committed&&!self.chunks.iter().any(|c|c.snapshot==a.snapshot) {return Err("已晋级任务缺少记忆版本".into());}
        }
        Ok((ledger,Scheduler::restore(self.attempts.clone())?))
    }
}
pub(crate) async fn init(pool:&SqlitePool)->Result<()> {
    sqlx::query("CREATE TABLE IF NOT EXISTS tingxian_memory_checkpoints(session_id TEXT NOT NULL,revision INTEGER NOT NULL,epoch INTEGER NOT NULL,last_record INTEGER NOT NULL,context_version INTEGER NOT NULL,payload TEXT NOT NULL,sha256 TEXT NOT NULL,PRIMARY KEY(session_id,revision))").execute(pool).await.map_err(|e|e.to_string())?;Ok(())
}
async fn journal(connection:&mut SqliteConnection,id:&str,last:u64)->Result<Vec<JournalRecord>> {
    if last>20_000 {return Err("检查点原文范围越界".into());}
    let rows=sqlx::query_as::<_,(i64,String,String)>("SELECT record_sequence,payload,sha256 FROM tingxian_memory_events WHERE session_id=? AND record_sequence<=? ORDER BY record_sequence").bind(id).bind(last as i64).fetch_all(&mut *connection).await.map_err(|e|e.to_string())?;
    if rows.len()!=last as usize {return Err("检查点原文覆盖不完整".into());}
    rows.into_iter().enumerate().map(|(i,(seq,payload,hash))|{
        if payload.len()>84*1024||seq!=i as i64+1||crate::digest(payload.as_bytes())!=hash {return Err("检查点原文校验失败".into());}
        let r:JournalRecord=serde_json::from_str(&payload).map_err(|_|"检查点原文格式错误")?;
        if r.record_sequence!=seq as u64||r.event.session_id!=id {return Err("检查点原文身份不匹配".into());}Ok(r)
    }).collect()
}
/// The short publication lock belongs to the native memory owner, not capture.
/// Network execution never holds that lock. expected_previous is a real CAS.
pub async fn save(pool:&SqlitePool,checkpoint:&Checkpoint,expected_previous:u64)->Result<Reference> {
    let payload=serde_json::to_string(checkpoint).map_err(|e|e.to_string())?;
    if payload.len()>MAX_PAYLOAD||expected_previous>=128 {return Err("会话记忆达到8 MiB/128版上限，请导出并新建会话".into());}
    let sha=crate::digest(payload.as_bytes());let revision=expected_previous+1;
    let mut tx=pool.begin().await.map_err(|e|e.to_string())?;
    let epoch:i64=sqlx::query_scalar("SELECT epoch+1 FROM tingxian_recovery_epoch WHERE id=1").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    let last:Option<(i64,i64)>=sqlx::query_as("SELECT epoch,last_record FROM tingxian_memory_sessions WHERE id=?").bind(&checkpoint.session_id).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
    if last!=Some((epoch,checkpoint.last_record as i64))||checkpoint.epoch!=epoch as u64 {return Err("会话原文已更新或清理，请重新组装检查点；旧版本保留".into());}
    let previous:i64=sqlx::query_scalar("SELECT coalesce(max(revision),0) FROM tingxian_memory_checkpoints WHERE session_id=?").bind(&checkpoint.session_id).fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    if previous as u64!=expected_previous {return Err("会话记忆已有新版本，旧写入未覆盖".into());}
    let raw=journal(&mut tx,&checkpoint.session_id,checkpoint.last_record).await?;
    checkpoint.restore(&raw)?;
    let bytes:i64=sqlx::query_scalar("SELECT coalesce(sum(length(CAST(payload AS BLOB))),0) FROM tingxian_memory_checkpoints").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    if bytes.saturating_add(payload.len() as i64)>TOTAL_BYTES {return Err("会话记忆检查点达到128 MiB，请导出并清理历史".into());}
    sqlx::query("INSERT INTO tingxian_memory_checkpoints VALUES(?,?,?,?,?,?,?)").bind(&checkpoint.session_id).bind(revision as i64).bind(epoch).bind(checkpoint.last_record as i64).bind(checkpoint.context_version as i64).bind(&payload).bind(&sha).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e|e.to_string())?;Ok(Reference{session_id:checkpoint.session_id.clone(),epoch:checkpoint.epoch,revision,last_record:checkpoint.last_record,sha256:sha})
}
pub async fn latest(pool:&SqlitePool,id:&str)->Result<Option<Reference>> {
    let row:Option<(i64,i64,i64,String)>=sqlx::query_as("SELECT epoch,revision,last_record,sha256 FROM tingxian_memory_checkpoints WHERE session_id=? ORDER BY revision DESC LIMIT 1").bind(id).fetch_optional(pool).await.map_err(|e|e.to_string())?;
    Ok(row.map(|(epoch,revision,last_record,sha256)|Reference{session_id:id.into(),epoch:epoch as u64,revision:revision as u64,last_record:last_record as u64,sha256}))
}
/// Restores the exact checkpoint prefix. Later raw events must be explicitly
/// replayed for crash recovery, or forked for restoring an older saved session.
pub async fn load(pool:&SqlitePool,reference:&Reference)->Result<(Ledger,Scheduler)> {
    let mut tx=pool.begin().await.map_err(|e|e.to_string())?;
    let epoch:i64=sqlx::query_scalar("SELECT epoch+1 FROM tingxian_recovery_epoch WHERE id=1").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    if reference.epoch!=epoch as u64||reference.revision==0||reference.revision>128 {return Err("会话记忆已清空或版本无效".into());}
    let (payload,sha): (String,String)=sqlx::query_as("SELECT payload,sha256 FROM tingxian_memory_checkpoints WHERE session_id=? AND revision=?").bind(&reference.session_id).bind(reference.revision as i64).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?.ok_or("会话记忆检查点不存在")?;
    if payload.len()>MAX_PAYLOAD||crate::digest(payload.as_bytes())!=sha||sha!=reference.sha256 {return Err("会话记忆检查点校验失败".into());}
    let c:Checkpoint=serde_json::from_str(&payload).map_err(|_|"会话记忆检查点格式无效")?;
    if c.session_id!=reference.session_id||c.epoch!=reference.epoch||c.last_record!=reference.last_record {return Err("会话记忆检查点归属或覆盖不匹配".into());}
    let raw=journal(&mut tx,&reference.session_id,c.last_record).await?;c.restore(&raw)
}

/// Fresh local journal (including legacy missing-history marker), no paid work.
pub async fn load_journal(pool:&SqlitePool,session:&memory_store::Session)->Result<Ledger> {
    let mut ledger=Ledger::new(&session.id,session.epoch,session.dependencies.clone(),session.missing_older_history)?;
    let mut after=0;
    loop {let records=memory_store::page(pool,&session.id,session.epoch,after,64).await?;if records.is_empty(){break;}for r in records {if r.record_sequence>session.last_record {return Err("原文在恢复中变化，请重试".into());}after=r.record_sequence;ledger.append(r.event)?;}}
    if after!=session.last_record {return Err("原文在恢复中变化，请重试".into());}Ok(ledger)
}

/// Restore an immutable saved prefix into an independent writable branch in a
/// single transaction. Inherited accounting is retained, never re-executed.
pub async fn load_boundary(pool:&SqlitePool,boundary:&Boundary)->Result<(Ledger,Scheduler)> {
    boundary.validate()?;
    let mut tx=pool.begin().await.map_err(|e|e.to_string())?;
    let epoch:i64=sqlx::query_scalar("SELECT epoch+1 FROM tingxian_recovery_epoch WHERE id=1").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    if epoch as u64!=boundary.epoch{return Err("会话原文边界已过期".into());}
    let raw=journal(&mut tx,&boundary.session_id,boundary.last_record).await?;
    if crate::digest(&serde_json::to_vec(&raw.last()).map_err(|e|e.to_string())?)!=boundary.last_sha256{return Err("会话保存边界与原文不一致".into());}
    let (deps,missing):(String,i64)=sqlx::query_as("SELECT dependencies,missing_older FROM tingxian_memory_sessions WHERE id=? AND epoch=?").bind(&boundary.session_id).bind(epoch).fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e|e.to_string())?;
    let (mut ledger,scheduler)=if let Some(r)=&boundary.checkpoint{load(pool,r).await?}else{(Ledger::new(&boundary.session_id,boundary.epoch,serde_json::from_str(&deps).map_err(|_|"会话依赖无效")?,missing!=0)?,Scheduler::default())};
    for r in raw.iter().skip(ledger.journal().len()){ledger.append(r.event.clone())?;}
    Ok((ledger,scheduler))
}
pub async fn fork(pool:&SqlitePool,reference:&Reference,id:&str,dependencies:Dependencies)->Result<(Ledger,Scheduler,Reference)> {
    let (old,_)=load(pool,reference).await?;let b=Boundary::capture(&old,Some(reference.clone()))?;fork_boundary(pool,&b,id,dependencies).await
}
pub async fn fork_boundary(pool:&SqlitePool,reference:&Boundary,id:&str,dependencies:Dependencies)->Result<(Ledger,Scheduler,Reference)> {
    let (old,scheduler)=load_boundary(pool,reference).await?;
    let mut ledger=old.fork(id,reference.epoch)?;
    let mut attempts=scheduler.attempts().to_vec();
    for a in &mut attempts {
        a.snapshot.session_id=id.into();a.snapshot.epoch=reference.epoch;
        for e in &mut a.snapshot.events {e.session_id=id.into();e.epoch=reference.epoch;}
        a.id=crate::digest(format!("{}:{}",a.snapshot.id()?,a.config_version).as_bytes());
    }
    let scheduler=Scheduler::restore(attempts)?;ledger.update_dependencies(dependencies)?;
    let checkpoint=Checkpoint::capture(&ledger,&scheduler);checkpoint.restore(ledger.journal())?;
    let payload=serde_json::to_string(&checkpoint).map_err(|e|e.to_string())?;
    if payload.len()>MAX_PAYLOAD{return Err("恢复记忆超过8 MiB，原会话保留".into());}
    let records:Vec<_>=ledger.journal().iter().map(|r|serde_json::to_string(r).map(|p|(r,p)).map_err(|e|e.to_string())).collect::<Result<_>>()?;
    let bytes=records.iter().map(|(_,p)|p.len() as i64).sum::<i64>();
    let mut tx=pool.begin().await.map_err(|e|e.to_string())?;
    let epoch:i64=sqlx::query_scalar("SELECT epoch+1 FROM tingxian_recovery_epoch WHERE id=1").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    if epoch as u64!=reference.epoch{return Err("恢复期间历史已清空，未回填旧会话".into());}
    let (count,total):(i64,i64)=sqlx::query_as("SELECT count(*),coalesce(sum(bytes),0) FROM tingxian_memory_sessions").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    let cp_bytes:i64=sqlx::query_scalar("SELECT coalesce(sum(length(CAST(payload AS BLOB))),0) FROM tingxian_memory_checkpoints").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    if count>=200||bytes>32*1024*1024||total.saturating_add(bytes)>128*1024*1024||cp_bytes.saturating_add(payload.len() as i64)>TOTAL_BYTES{return Err("恢复分支达到本地保留上限，原会话保留".into());}
    sqlx::query("INSERT INTO tingxian_memory_sessions VALUES(?,?,?,?,?,?,?)").bind(id).bind(epoch).bind(serde_json::to_string(ledger.dependencies()).map_err(|e|e.to_string())?).bind(i64::from(ledger.missing_older_history())).bind(ledger.journal().len() as i64).bind(ledger.next_sequence() as i64-1).bind(bytes).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    for (r,p) in records {
        let e=&r.event;
        sqlx::query("INSERT INTO tingxian_memory_events VALUES(?,?,?,?,?,?,?)").bind(id).bind(r.record_sequence as i64).bind(&e.id).bind(e.sequence as i64).bind(e.revision as i64).bind(&p).bind(crate::digest(p.as_bytes())).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        sqlx::query("INSERT INTO tingxian_memory_latest VALUES(?,?,?,?,?) ON CONFLICT(session_id,event_id) DO UPDATE SET revision=excluded.revision,record_sequence=excluded.record_sequence").bind(id).bind(&e.id).bind(e.sequence as i64).bind(e.revision as i64).bind(r.record_sequence as i64).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    }
    let sha=crate::digest(payload.as_bytes());
    sqlx::query("INSERT INTO tingxian_memory_checkpoints VALUES(?,1,?,?,?,?,?)").bind(id).bind(epoch).bind(checkpoint.last_record as i64).bind(checkpoint.context_version as i64).bind(payload).bind(&sha).execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e|e.to_string())?;
    let r=Reference{session_id:id.into(),epoch:reference.epoch,revision:1,last_record:checkpoint.last_record,sha256:sha};Ok((ledger,scheduler,r))
}
