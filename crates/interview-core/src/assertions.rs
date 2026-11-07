//! Manual claims are versioned separately from the immutable imported excerpt.
//! FTS is a rebuildable derivative; a claim only applies to its exact library,
//! source hash, chunk and category. Old revisions are explicit history only.
use crate::{digest, FactSource, FactSourceRef, Index, Result};
use serde::{Deserialize, Serialize};
use sqlx::Row;

pub(crate) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS record_assertions(record_id TEXT NOT NULL, revision INTEGER NOT NULL, source_id TEXT NOT NULL, root TEXT NOT NULL, path TEXT NOT NULL, source_hash TEXT NOT NULL, source_kind TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(record_id,revision));
CREATE INDEX IF NOT EXISTS record_assertion_source ON record_assertions(source_id,path,root,source_hash,source_kind);
CREATE VIEW IF NOT EXISTS current_record_assertions AS SELECT a.* FROM record_assertions a WHERE a.revision=(SELECT MAX(b.revision) FROM record_assertions b WHERE b.record_id=a.record_id);
CREATE VIRTUAL TABLE IF NOT EXISTS assertion_search USING fts5(id UNINDEXED,tokens);";
// Expects aliases c (chunk), d (document); exclusion/validity is checked by the caller.
pub(crate) const JOIN: &str = "LEFT JOIN current_record_assertions a ON a.source_id=c.id AND a.path=d.path AND a.source_hash=d.hash AND a.source_kind=d.kind AND a.root=CASE WHEN d.path GLOB '.背景/*' THEN '背景资料库' ELSE COALESCE((SELECT value FROM settings WHERE key='root'),'') END";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all="snake_case")]
pub enum Activity { Unknown, Planned, InProgress, Completed }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserAssertion {
    pub record_id: String, pub revision: i64, pub previous_version: Option<i64>,
    pub origin: String, pub claim: String, pub activity: Activity,
    pub reason: String, pub recorded_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssertionRecord { #[serde(flatten)] pub assertion: UserAssertion, pub source: FactSource }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssertionUpdate {
    pub source: FactSourceRef, pub expected_revision: i64, pub claim: String,
    pub activity: Activity, pub reason: String, pub confirmed: bool,
}
#[derive(Serialize)]
pub struct RecordSegment { pub source_id: String, pub location: String, pub text: String, pub user_assertion: Option<UserAssertion> }
#[derive(Serialize)]
pub struct AssertionHistory { pub versions: Vec<AssertionRecord>, pub next_before: Option<i64> }
#[derive(Serialize)]
pub struct DocumentAssertionHistory { pub versions: Vec<AssertionRecord>, pub next_offset: Option<i64> }
pub(crate) fn parse(value: Option<String>) -> Result<Option<UserAssertion>> {
    value.map(|v| serde_json::from_str::<AssertionRecord>(&v).map(|r|r.assertion).map_err(|e|format!("本人修订记录不可读：{e}"))).transpose()
}
fn record_id(source: &FactSourceRef) -> String {
    digest(&serde_json::to_vec(&(source.root.as_str(),source.path.as_str(),source.source_id.as_str(),source.hash.as_str())).unwrap())
}
impl Index {
    /// Rebuild only the derived FTS cache, preserving every authoritative revision.
    pub async fn rebuild_assertion_search(&self)->Result<()> {
        let _gate=self.gate.lock().await;
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let rows:Vec<String>=sqlx::query_scalar("SELECT value FROM current_record_assertions").fetch_all(&mut *tx).await.map_err(|e|e.to_string())?;
        sqlx::query("DELETE FROM assertion_search").execute(&mut *tx).await.map_err(|e|e.to_string())?;
        for raw in rows {
            let record:AssertionRecord=serde_json::from_str(&raw).map_err(|e|e.to_string())?;
            sqlx::query("INSERT INTO assertion_search VALUES(?,?)").bind(record.assertion.record_id).bind(self.tokens(&record.assertion.claim).join(" ")).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        }
        tx.commit().await.map_err(|e|e.to_string())
    }
    pub async fn revise_record(&self, update: AssertionUpdate) -> Result<AssertionRecord> {
        if !update.confirmed {return Err("请确认本次本人声明；未保存".into());}
        if update.claim.trim().is_empty() || update.claim.chars().count()>1200 {return Err("本人声明须为 1–1200 字符".into());}
        if update.reason.trim().is_empty() || update.reason.chars().count()>300 {return Err("请填写不超过 300 字符的修订依据".into());}
        if crate::contains_credential(&update.claim) || crate::contains_credential(&update.reason) {return Err("声明含凭证格式，未保存".into());}
        let _gate=self.gate.lock().await;
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let excluded=crate::source_policy::EXCLUDED_SQL;
        let row=sqlx::query(&format!("SELECT c.path,c.location,c.text,d.hash,d.kind,CASE WHEN d.path GLOB '.背景/*' THEN '背景资料库' ELSE COALESCE((SELECT value FROM settings WHERE key='root'),'') END AS root,{excluded} AS excluded FROM chunks c JOIN documents d ON c.path=d.path WHERE c.id=?"))
            .bind(&update.source.source_id).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?.ok_or("来源已退出索引，请刷新后修订")?;
        if row.get::<String,_>("path")!=update.source.path || row.get::<String,_>("hash")!=update.source.hash || row.get::<String,_>("root")!=update.source.root {return Err("来源版本或资料库已变化，请刷新后重新核对；未覆盖记录".into());}
        if row.get::<bool,_>("excluded") || update.source.path==".背景/综合背景" {return Err("请在参与检索的原始资料片段上修订".into());}
        let id=record_id(&update.source);
        let previous:i64=sqlx::query_scalar("SELECT COALESCE(MAX(revision),0) FROM record_assertions WHERE record_id=?").bind(&id).fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
        if update.expected_revision!=previous {return Err("这条声明已有新版本，请加载后比较；未覆盖记录".into());}
        let revision=previous.checked_add(1).ok_or("修订版本已达上限")?;
        let source=FactSource {reference:update.source,location:row.get("location"),text:row.get("text"),kind:row.get("kind"),user_assertion:None};
        let record=AssertionRecord {assertion:UserAssertion {record_id:id.clone(),revision,previous_version:if previous==0 {None}else{Some(previous)},origin:"user_asserted".into(),claim:update.claim.trim().into(),activity:update.activity,reason:update.reason.trim().into(),recorded_at:chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis,true)},source};
        sqlx::query("INSERT INTO record_assertions VALUES(?,?,?,?,?,?,?,?)").bind(&id).bind(revision).bind(&record.source.reference.source_id).bind(&record.source.reference.root).bind(&record.source.reference.path).bind(&record.source.reference.hash).bind(&record.source.kind).bind(serde_json::to_string(&record).map_err(|e|e.to_string())?).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        sqlx::query("DELETE FROM assertion_search WHERE id=?").bind(&id).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        sqlx::query("INSERT INTO assertion_search VALUES(?,?)").bind(&id).bind(self.tokens(&record.assertion.claim).join(" ")).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        sqlx::query("UPDATE settings SET value=CAST(value AS INTEGER)+1 WHERE key='version'").execute(&mut *tx).await.map_err(|e|e.to_string())?;
        tx.commit().await.map_err(|e|e.to_string())?;
        Ok(record)
    }
    pub async fn document_records(&self,path:&str,root:&str,hash:&str)->Result<Vec<RecordSegment>> {
        let _gate=self.gate.lock().await;
        self.document_segments(path,root,hash).await?; // same identity contract as legacy preview
        let rows=sqlx::query(&format!("SELECT c.id,c.location,c.text,a.value FROM chunks c JOIN documents d ON c.path=d.path {JOIN} WHERE c.path=? ORDER BY c.rowid")).bind(path).fetch_all(&self.pool).await.map_err(|e|e.to_string())?;
        rows.into_iter().map(|r|Ok(RecordSegment {source_id:r.get("id"),location:r.get("location"),text:r.get("text"),user_assertion:parse(r.get("value"))?})).collect()
    }
    pub async fn assertion_history(&self,source:FactSourceRef,before:Option<i64>)->Result<AssertionHistory> {
        // Explicit historical identity is allowed even after a source changes.
        let rows:Vec<String>=sqlx::query_scalar("SELECT value FROM record_assertions WHERE record_id=? AND (? IS NULL OR revision<?) ORDER BY revision DESC LIMIT 21").bind(record_id(&source)).bind(before).bind(before).fetch_all(&self.pool).await.map_err(|e|e.to_string())?;
        let more=rows.len()>20;
        let versions=rows.into_iter().take(20).map(|v|serde_json::from_str::<AssertionRecord>(&v).map_err(|e|e.to_string())).collect::<Result<Vec<_>>>()?;
        let next_before=if more {versions.last().map(|r|r.assertion.revision)}else{None};
        Ok(AssertionHistory {versions,next_before})
    }
    pub async fn has_manual_assertions(&self)->Result<bool> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM record_assertions)").fetch_one(&self.pool).await.map_err(|e|e.to_string())
    }
    pub async fn document_assertion_history(&self,path:&str,root:&str,offset:i64)->Result<DocumentAssertionHistory> {
        if !(0..=100_000).contains(&offset) {return Err("历史分页位置无效".into());}
        let rows:Vec<String>=sqlx::query_scalar("SELECT value FROM record_assertions WHERE path=? AND root=? ORDER BY rowid DESC LIMIT 21 OFFSET ?")
            .bind(path).bind(root).bind(offset).fetch_all(&self.pool).await.map_err(|e|e.to_string())?;
        let next_offset=if rows.len()>20 {Some(offset+20)}else{None};
        let versions=rows.into_iter().take(20).map(|v|serde_json::from_str(&v).map_err(|e|e.to_string())).collect::<Result<Vec<_>>>()?;
        Ok(DocumentAssertionHistory {versions,next_offset})
    }
}
