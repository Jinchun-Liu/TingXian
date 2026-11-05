//! Explicit, immutable local snapshots. Reading never starts capture or an AI request.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Row,SqlitePool};
const MAX_SNAPSHOT:usize=16*1024*1024;
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Current { pub question:String,pub answer:String }
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelfEdit {pub id:String,pub text:String}
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputDrafts {pub question:String,pub confirmed_answer:String,pub self_edit:Option<SelfEdit>}
impl InputDrafts {
    fn has_content(&self)->bool {!self.question.is_empty() || !self.confirmed_answer.is_empty() || self.self_edit.is_some()}
}
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub version:u32,pub history:Vec<Value>,pub transcripts:Vec<Value>,pub spoken_dialogue:Vec<Value>,
    pub current:Current,pub calls:u32,pub usage:String,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub usage_ledger:Option<Value>,
    // Additive to version 1: old saved sessions remain readable and unchanged.
    // Drafts never enter the spoken dialogue or confirmed answers implicitly.
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub input_drafts:Option<InputDrafts>,
}
#[derive(Debug,Serialize,Deserialize)]
pub struct SavedSession {pub id:i64,pub saved_at:String,pub turns:i64,pub calls:i64}
#[derive(Serialize)]
pub struct SessionPage {pub sessions:Vec<SavedSession>,pub next_before:Option<i64>}
pub async fn init(pool:&SqlitePool)->Result<(),String> {
    sqlx::query("CREATE TABLE IF NOT EXISTS interview_sessions (id INTEGER PRIMARY KEY AUTOINCREMENT,saved_at TEXT NOT NULL,turns INTEGER NOT NULL,calls INTEGER NOT NULL,payload TEXT NOT NULL,sha256 TEXT NOT NULL)")
        .execute(pool).await.map_err(|e|e.to_string())?;Ok(())
}
fn valid_text(v:&Value,key:&str)->bool {v.get(key).and_then(Value::as_str).is_some()}
fn validate(snapshot:&Snapshot)->Result<(),String> {
    if snapshot.version!=1 || (snapshot.history.is_empty() && !snapshot.input_drafts.as_ref().is_some_and(InputDrafts::has_content)) || snapshot.history.len()>10000 || snapshot.transcripts.len()>120 || snapshot.spoken_dialogue.len()>120 {return Err("会话版本或记录数量无效；未保存或替换当前记录".into());}
    let mut ids=std::collections::HashSet::new();
    for t in &snapshot.history {
        if !["remote","self","manual"].contains(&t["role"].as_str().unwrap_or("")) || !["listening","queued","generating","done","error","cancelled","recorded"].contains(&t["status"].as_str().unwrap_or(""))
            || !["id","question","suggestion","time","timing"].iter().all(|k|valid_text(t,k)) || !t["sources"].is_array()
            || !(t["confirmed_answer"].is_null() || t["confirmed_answer"].is_string()) || !ids.insert(t["id"].as_str().unwrap_or("")) {
            return Err("会话发言结构无效；未保存或替换当前记录".into());
        }
    }
    for t in snapshot.transcripts.iter().chain(&snapshot.spoken_dialogue) {
        if !valid_text(t,"text") || !valid_text(t,"source") || !valid_text(t,"timestamp") || t["sequence_id"].as_u64().is_none() || t["is_partial"].as_bool().is_none()
            || t["audio_start_time"].as_f64().is_none() || t["audio_end_time"].as_f64().is_none() {return Err("会话转写结构无效".into());}
    }
    if let Some(usage)=&snapshot.usage_ledger {validate_usage(usage,snapshot.calls)?;}
    if serde_json::to_vec(snapshot).map_err(|e|e.to_string())?.len()>MAX_SNAPSHOT {return Err("会话超过16 MiB，请导出保存；没有截断内容".into());}
    Ok(())
}
fn validate_usage(value:&Value,calls:u32)->Result<(),String> {
    let invalid=||"会话用量记录格式无效，当前记录保留".to_string();
    if value["version"].as_u64()!=Some(1){return Err(invalid());}
    let mut accepted=value["legacy_calls"].as_u64().filter(|n|*n<=u32::MAX as u64).ok_or_else(invalid)?;
    let requests=value["requests"].as_array().filter(|a|a.len()<=20000).ok_or_else(invalid)?;
    let mut seen=std::collections::HashSet::new();
    for r in requests {
        let id=r["question_id"].as_str().filter(|s|!s.is_empty() && s.chars().count()<=1000).ok_or_else(invalid)?;
        let revision=r["revision"].as_u64().filter(|n|*n<=u32::MAX as u64).ok_or_else(invalid)?;
        let attempts=r["attempts"].as_u64().filter(|n|*n<=2).ok_or_else(invalid)?;
        let was_accepted=r["accepted"].as_bool().ok_or_else(invalid)?;
        if !seen.insert((id,revision)) || !["pending","done","error","cancelled"].contains(&r["state"].as_str().unwrap_or("")){return Err(invalid());}
        match r.get("total_tokens") {
            Some(Value::Null)=>{},
            Some(v) if attempts>0 && v.as_u64().is_some_and(|n|n<=9_007_199_254_740_991)=>{},
            _=>return Err(invalid()),
        }
        if was_accepted {accepted+=1;}
    }
    if accepted!=calls as u64{return Err("会话调用数与用量记录不一致，当前记录保留".into());}
    Ok(())
}
fn scrub(value:&mut Value,depth:usize)->Result<(),String> {
    if depth>32 {return Err("会话结构过深".into());}
    match value {
        Value::Array(items)=>for v in items {scrub(v,depth+1)?;},
        Value::Object(items)=>{
            if items.keys().any(|k|["key","api_key","apiKey","authorization","password","secret"].contains(&k.as_str())) {return Err("会话不能包含凭证或配置字段".into());}
            for v in items.values_mut(){scrub(v,depth+1)?;}
        },_=>{}
    }Ok(())
}
pub async fn save(pool:&SqlitePool,snapshot:Snapshot,confirmed:bool)->Result<SavedSession,String> {
    save_with_key(pool,snapshot,confirmed,"").await
}
pub async fn save_with_key(pool:&SqlitePool,snapshot:Snapshot,confirmed:bool,key:&str)->Result<SavedSession,String> {
    if !confirmed {return Err("请明确选择保存本次会话".into());}
    validate(&snapshot)?;
    let mut data=serde_json::to_value(&snapshot).map_err(|e|e.to_string())?;
    scrub(&mut data,0)?;
    interview_core::redact_json_credentials(&mut data,Some(key))?;
    let payload=serde_json::to_string(&data).map_err(|e|e.to_string())?;
    if payload.len()>MAX_SNAPSHOT{return Err("会话超过16 MiB；没有截断内容".into());}
    init(pool).await?;
    let mut tx=pool.begin().await.map_err(|e|e.to_string())?;
    let (count,bytes):(i64,i64)=sqlx::query_as("SELECT count(*),coalesce(sum(length(CAST(payload AS BLOB))),0) FROM interview_sessions").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    if count>=200 || bytes+payload.len() as i64>128*1024*1024 {return Err("已保存会话达到200份或128 MiB，请先导出并清理历史；未覆盖旧记录".into());}
    let saved_at=chrono::Utc::now().to_rfc3339();let turns=snapshot.history.len() as i64;let calls=snapshot.calls as i64;
    let row=sqlx::query("INSERT INTO interview_sessions(saved_at,turns,calls,payload,sha256) VALUES(?,?,?,?,?)")
        .bind(&saved_at).bind(turns).bind(calls).bind(&payload).bind(interview_core::digest(payload.as_bytes()))
        .execute(&mut *tx).await.map_err(|e|e.to_string())?;
    tx.commit().await.map_err(|e|e.to_string())?;
    Ok(SavedSession{id:row.last_insert_rowid(),saved_at,turns,calls})
}
#[derive(Serialize)]
pub struct ExportReport {pub content:String,pub credentials_redacted:bool}
pub fn export(mut snapshot:Value,key:&str)->Result<ExportReport,String> {
    // Export remains available beyond the local snapshot limit, without silent
    // truncation. No database, capture or model operation is involved.
    const MAX_EXPORT:usize=64*1024*1024;
    if serde_json::to_vec(&snapshot).map_err(|_|"导出序列化失败")?.len()>MAX_EXPORT {return Err("导出超过64 MiB；没有截断内容".into());}
    let credentials_redacted=interview_core::redact_json_credentials(&mut snapshot,Some(key))?;
    let content=serde_json::to_string_pretty(&snapshot).map_err(|_|"导出序列化失败")?;
    if content.len()>MAX_EXPORT{return Err("导出超过64 MiB；没有截断内容".into());}
    Ok(ExportReport {content,credentials_redacted})
}
pub async fn list(pool:&SqlitePool,before:Option<i64>)->Result<SessionPage,String> {
    init(pool).await?;
    let rows=sqlx::query("SELECT id,saved_at,turns,calls FROM interview_sessions WHERE (? IS NULL OR id<?) ORDER BY id DESC LIMIT 21")
        .bind(before).bind(before).fetch_all(pool).await.map_err(|e|e.to_string())?;
    let more=rows.len()>20;let sessions:Vec<_>=rows.into_iter().take(20).map(|r|SavedSession{id:r.get("id"),saved_at:r.get("saved_at"),turns:r.get("turns"),calls:r.get("calls")}).collect();
    let next_before=if more{sessions.last().map(|s|s.id)}else{None};Ok(SessionPage{sessions,next_before})
}
pub async fn load(pool:&SqlitePool,id:i64)->Result<Snapshot,String> {
    init(pool).await?;
    let row=sqlx::query("SELECT payload,sha256 FROM interview_sessions WHERE id=?").bind(id).fetch_optional(pool).await.map_err(|e|e.to_string())?.ok_or("保存记录不存在，可能已清空历史")?;
    let payload:String=row.get("payload");let hash:String=row.get("sha256");
    if payload.len()>MAX_SNAPSHOT || interview_core::digest(payload.as_bytes())!=hash {return Err("会话内容校验失败，保留当前窗口及原记录".into());}
    let snapshot:Snapshot=serde_json::from_str(&payload).map_err(|_|"会话格式无效")?;validate(&snapshot)?;Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn snapshot()->Snapshot {
        let mut result:Snapshot=serde_json::from_value(json!({"version":1,"history":[{"role":"self","id":"m1","question":"我会使用幂等键","suggestion":"","confirmed_answer":null,"status":"done","sources":[],"time":"00:01","timing":""},{"role":"remote","id":"r1","question":"怎么测试？","suggestion":"建议回答：验证重复请求只写一次。","confirmed_answer":null,"status":"done","sources":[],"time":"00:02","timing":""}],"transcripts":[],"spoken_dialogue":[],"current":{"question":"怎么测试？","answer":"建议回答"},"calls":1,"usage":"未知"})).unwrap();
        result.history[1]["sources"]=json!([{ "source_id":"S-test","path":"synthetic.md","location":"paragraph 1","text":"synthetic source","kind":"technical","allow_external":true,"hash":"test-hash" }]);
        result.history[1]["fact_version"]=3.into();
        result.history[1]["confirmed_answer"]="我会测试重试次数和最终写入行数".into();
        result.history[1]["latency"]=json!({"version":1,"audio_end_basis":"unavailable","stages_ms":{"submitted":0,"answer_done":600},"request_attempts":1,"terminal":"done"});
        let spoken=json!({"sequence_id":1,"source":"Microphone","text":"我会使用幂等键","timestamp":"00:01","is_partial":false,"audio_start_time":0,"audio_end_time":1,"capture_session":4,"microphone_epoch":2,"confidence":null});
        result.transcripts.push(spoken.clone());result.spoken_dialogue.push(spoken);
        result
    }
    async fn pool()->SqlitePool {SqlitePool::connect("sqlite::memory:").await.unwrap()}
    #[tokio::test]
    async fn observed_usage_round_trips_and_legacy_stays_unknown() {
        let db=pool().await;
        let legacy=save(&db,snapshot(),true).await.unwrap();
        assert!(load(&db,legacy.id).await.unwrap().usage_ledger.is_none());
        let mut current=snapshot();current.calls=3;
        let ledger=json!({"version":1,"legacy_calls":1,"requests":[
            {"question_id":"observed-a","revision":0,"accepted":true,"attempts":2,"total_tokens":27,"state":"done"},
            {"question_id":"observed-b","revision":1,"accepted":true,"attempts":1,"total_tokens":null,"state":"cancelled"},
            {"question_id":"rejected-c","revision":2,"accepted":false,"attempts":0,"total_tokens":null,"state":"error"}
        ]});
        current.usage_ledger=Some(ledger.clone());
        let saved=save(&db,current,true).await.unwrap();
        assert_eq!(load(&db,saved.id).await.unwrap().usage_ledger,Some(ledger.clone()));
        assert!(validate_usage(&ledger,2).is_err());
        for (key,bad) in [("attempts",json!(3)),("revision",json!(-1)),("total_tokens",json!("27")),("total_tokens",json!(-2)),("total_tokens",json!(9_007_199_254_740_992u64)),("accepted",json!("true"))] {
            let mut invalid=ledger.clone();invalid["requests"][0][key]=bad;assert!(validate_usage(&invalid,3).is_err());
        }
        let mut duplicate=ledger.clone();let first=duplicate["requests"][0].clone();duplicate["requests"].as_array_mut().unwrap().push(first);
        assert!(validate_usage(&duplicate,4).is_err());
        let mut no_send=ledger.clone();no_send["requests"][0]["attempts"]=json!(0);assert!(validate_usage(&no_send,3).is_err());
        assert_eq!(list(&db,None).await.unwrap().sessions.len(),2);
    }
    #[tokio::test]
    async fn only_explicit_save_and_immutable_versions_survive_database_reopen() {
        let folder=std::env::temp_dir().join(format!("meetily-saved-session-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&folder).unwrap();
        let path=folder.join("session.sqlite");let options=sqlx::sqlite::SqliteConnectOptions::new().filename(&path).create_if_missing(true);
        let db=SqlitePool::connect_with(options.clone()).await.unwrap();
        assert!(save(&db,snapshot(),false).await.is_err());assert!(list(&db,None).await.unwrap().sessions.is_empty());
        let first=save(&db,snapshot(),true).await.unwrap();let mut changed=snapshot();changed.history[1]["suggestion"]="新建议".into();
        let second=save(&db,changed,true).await.unwrap();assert!(second.id>first.id);db.close().await;
        let reopened=SqlitePool::connect_with(options).await.unwrap();
        let restored=load(&reopened,first.id).await.unwrap();
        assert_eq!(serde_json::to_value(&restored).unwrap(),serde_json::to_value(snapshot()).unwrap(),"dialogue, answers, sources, fact version, timings, confirmations and usage must all survive reopen");
        assert_eq!(load(&reopened,second.id).await.unwrap().history[1]["suggestion"],"新建议");
        reopened.close().await; // Synthetic artifacts retained for inspection.
    }
    #[tokio::test]
    async fn malformed_tampered_and_secret_payloads_are_not_silently_restored() {
        let db=pool().await;let mut s=snapshot();s.version=2;assert!(save(&db,s,true).await.is_err());
        let mut s=snapshot();s.history[0]["api_key"]="synthetic".into();assert!(save(&db,s,true).await.is_err());
        let mut s=snapshot();s.history[1]["id"]="m1".into();assert!(save(&db,s,true).await.is_err());
        let mut s=snapshot();let fake=format!("sk-{}","synthetic_test_only".repeat(2));s.history[0]["question"]=fake.clone().into();
        let row=save(&db,s,true).await.unwrap();let restored=load(&db,row.id).await.unwrap();assert!(!serde_json::to_string(&restored).unwrap().contains(&fake));
        sqlx::query("UPDATE interview_sessions SET payload='{}' WHERE id=?").bind(row.id).execute(&db).await.unwrap();
        assert!(load(&db,row.id).await.unwrap_err().contains("校验失败"));
    }
    #[tokio::test]
    async fn listing_uses_stable_cursor_without_loading_dialogue() {
        let db=pool().await;for _ in 0..21 {save(&db,snapshot(),true).await.unwrap();}
        let first=list(&db,None).await.unwrap();assert_eq!(first.sessions.len(),20);let second=list(&db,first.next_before).await.unwrap();
        assert_eq!(second.sessions.len(),1);assert!(second.next_before.is_none());assert!(first.sessions.iter().all(|s|s.id!=second.sessions[0].id));
        assert!(!serde_json::to_string(&first).unwrap().contains("幂等键"));
    }
    #[tokio::test]
    async fn save_and_export_hide_keys_adjacent_to_chinese_and_keep_full_technical_history() {
        let fake=format!("sk-{}","synthetic01234567".repeat(2));let configured="synthetic-configured-token";
        let mut s=snapshot();s.history[0]["question"]=format!("测试凭证为{fake}；null、0与空字符串不能混用").into();
        s.history[1]["sources"][0]["text"]=configured.into();s.current.answer=format!("{configured}；DemoProject、pytest、JSON Schema");
        let db=pool().await;let saved=save_with_key(&db,s.clone(),true,configured).await.unwrap();
        let loaded=load(&db,saved.id).await.unwrap();let stored=serde_json::to_string(&loaded).unwrap();
        assert!(!stored.contains(&fake));assert!(!stored.contains(configured));assert!(stored.contains("null、0与空字符串不能混用"));
        let report=export(serde_json::json!({"snapshot":s,"values":[null,0,false,""]}),configured).unwrap();
        assert!(report.credentials_redacted);assert!(!report.content.contains(&fake));assert!(!report.content.contains(configured));
        let actual:Value=serde_json::from_str(&report.content).unwrap();assert_eq!(actual["values"],serde_json::json!([null,0,false,""]));
        assert_eq!(actual["snapshot"],serde_json::to_value(&loaded).unwrap());
        assert_eq!(loaded.history[1]["latency"],s.history[1]["latency"]);assert_eq!(loaded.transcripts,s.transcripts);
        assert!(!export(serde_json::json!({"answer":"简短回答"}),configured).unwrap().credentials_redacted);
        let mut bad=serde_json::Map::new();bad.insert(fake,Value::Null);
        assert!(export(Value::Object(bad),configured).is_err());
    }
}
