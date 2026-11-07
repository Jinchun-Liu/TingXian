//! Source withdrawal is a current permission, not a historical snapshot.
//! Uses the existing library table; originals and archived extracts are retained.
use crate::{digest, FactCard, Index, Result};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;

pub const EXCLUDED_SQL: &str = "EXISTS (SELECT 1 FROM background_library x WHERE x.key GLOB 'excluded/*' AND json_extract(x.value,'$.excluded')=1 AND json_extract(x.value,'$.root')=CASE WHEN d.path GLOB '.背景/*' THEN '背景资料库' ELSE COALESCE((SELECT value FROM settings WHERE key='root'),'') END AND json_extract(x.value,'$.path')=d.path)";
// Until the summary is regenerated with a dependency map, fail closed: it may
// contain any revoked source. It remains visible for local historical review.
pub const SUMMARY_BLOCKED_SQL: &str = "(d.path='.背景/综合背景' AND EXISTS (SELECT 1 FROM background_library x WHERE x.key GLOB 'excluded/*' AND json_extract(x.value,'$.excluded')=1))";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceExclusion {
    pub path: String,
    pub expected_root: String,
    pub expected_hash: String,
    pub excluded: bool,
}

impl Index {
    pub async fn has_withdrawn_sources(&self) -> Result<bool> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM background_library WHERE key GLOB 'excluded/*' AND json_extract(value,'$.excluded')=1)")
            .fetch_one(&self.pool).await.map_err(|e| e.to_string())
    }
    pub async fn request_history(&self, history: &[crate::Turn]) -> Result<Vec<crate::Turn>> {
        let mut turns=history.to_vec();
        if self.has_withdrawn_sources().await? {
            // Suggestions may quote any withdrawn source and have no dependency
            // graph. Actual utterances remain actual user-supplied context.
            for turn in &mut turns {turn.suggested_answer=None;}
        }
        Ok(turns)
    }
    pub async fn exclude_source(&self, update: SourceExclusion) -> Result<()> {
        let _gate = self.gate.lock().await;
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        let hash: Option<String> = sqlx::query_scalar("SELECT hash FROM documents WHERE path=?")
            .bind(&update.path).fetch_optional(&mut *tx).await.map_err(|e| e.to_string())?;
        let root: String = sqlx::query_scalar("SELECT value FROM settings WHERE key='root'")
            .fetch_optional(&mut *tx).await.map_err(|e| e.to_string())?.unwrap_or_default();
        let root = crate::background::doc_root(&update.path, &root);
        if hash.as_deref() != Some(&update.expected_hash) || root != update.expected_root {
            return Err("来源已变化，请刷新后重新选择；未改变检索范围".into());
        }
        let key = format!("excluded/{}", digest(&serde_json::to_vec(&(root, &update.path)).unwrap()));
        let value = json!({"root":root,"path":update.path,"hash_at_change":update.expected_hash,
            "excluded":update.excluded,"updated_at":chrono::Utc::now().to_rfc3339()});
        sqlx::query("INSERT OR REPLACE INTO background_library(key,value) VALUES(?,?)")
            .bind(key).bind(value.to_string()).execute(&mut *tx).await.map_err(|e| e.to_string())?;
        sqlx::query("UPDATE settings SET value=CAST(value AS INTEGER)+1 WHERE key='version'")
            .execute(&mut *tx).await.map_err(|e| e.to_string())?;
        tx.commit().await.map_err(|e| e.to_string())
    }

    /// The saved card is still available to the editor/history. A new request
    /// cannot reuse its copied facts when an attached source has been withdrawn.
    pub async fn request_fact_card(&self) -> Result<FactCard> {
        let facts = self.fact_card().await?;
        for source in &facts.sources {
            let row = sqlx::query(&format!("SELECT d.hash,CASE WHEN d.path GLOB '.背景/*' THEN '背景资料库' ELSE COALESCE((SELECT value FROM settings WHERE key='root'),'') END AS root,{EXCLUDED_SQL} AS excluded FROM documents d WHERE d.path=?"))
                .bind(&source.reference.path).fetch_optional(&self.pool).await.map_err(|e| e.to_string())?;
            if !row.is_some_and(|r| !r.get::<bool,_>("excluded") && r.get::<String,_>("hash")==source.reference.hash && r.get::<String,_>("root")==source.reference.root) {
                return Err("事实卡引用的来源已撤回或更新，请先核对事实卡；本次未发送".into());
            }
        }
        Ok(facts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn withdrawal_blocks_search_summary_worker_and_copied_fact_without_deleting_history() {
        let dir=tempfile::tempdir().unwrap().keep();
        let index=Index::open(&dir.join("test.sqlite")).await.unwrap();
        let doc=crate::background::Doc{id:"a".repeat(64),name:"synthetic.txt".into(),hash:"b".repeat(64),records:vec![crate::background::Record{kind:"personal".into(),location:"行 1".into(),quote:"虚构项目使用幂等键".into(),note:"本人参与".into()}]};
        let bundle=crate::background::Bundle{format:"meetily-background-v1".into(),documents:vec![doc],profile:crate::background::Profile{overview:"虚构项目概述涉及幂等键".into(),role:"测试开发".into(),aliases:String::new()},settings:None};
        index.apply_background(&bundle,&index.setting("version").await.unwrap(),true,json!({}),false).await.unwrap();
        let e=index.search("幂等键", "").await.unwrap().into_iter().find(|e|e.path!=".背景/综合背景").unwrap();
        index.correct_facts(crate::FactCorrection{expected_version:0,text:e.text.clone(),source_note:"来源行1".into(),sources:vec![crate::FactSourceRef{source_id:e.source_id.clone(),root:e.root.clone(),path:e.path.clone(),hash:e.hash.clone()}],reason:"本人核对".into(),confirmed:true}).await.unwrap();
        assert!(index.request_fact_card().await.is_ok());
        let set=|excluded|SourceExclusion{path:e.path.clone(),expected_root:e.root.clone(),expected_hash:e.hash.clone(),excluded};
        let old_version=index.setting("version").await.unwrap();
        index.exclude_source(set(true)).await.unwrap();
        assert_ne!(index.setting("version").await.unwrap(),old_version);
        assert!(index.search("幂等键", "").await.unwrap().is_empty());
        assert!(index.background_inputs(false).await.unwrap().is_empty());
        assert!(index.background_role().await.unwrap().is_none());
        assert!(index.request_fact_card().await.is_err());
        let history=[crate::Turn{question:"问题".into(),confirmed_answer:Some("本人实际说过的话".into()),suggested_answer:Some("旧AI草稿含来源内容".into())}];
        let filtered=index.request_history(&history).await.unwrap();
        assert!(filtered[0].suggested_answer.is_none());assert_eq!(filtered[0].confirmed_answer,history[0].confirmed_answer);
        assert_eq!(index.fact_card().await.unwrap().text,e.text);
        assert!(index.documents().await.unwrap().iter().any(|d|d.path==e.path&&d.excluded));
        assert!(!index.document_segments(&e.path,&e.root,&e.hash).await.unwrap().is_empty());
        let mut stale=set(false);stale.expected_hash="c".repeat(64);
        assert!(index.exclude_source(stale).await.is_err());
        assert!(index.search("幂等键", "").await.unwrap().is_empty());
        index.exclude_source(set(false)).await.unwrap();
        assert!(!index.search("幂等键", "").await.unwrap().is_empty());
        assert!(index.request_fact_card().await.is_ok());
    }
}
