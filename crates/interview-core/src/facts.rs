//! User-confirmed fact-card revisions. Never populated from AI or speech.
use crate::{Index, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::Row;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FactSourceRef {
    pub source_id: String,
    pub root: String,
    pub path: String,
    pub hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FactSource {
    #[serde(flatten)] pub reference: FactSourceRef,
    pub location: String,
    pub text: String,
    pub kind: String,
    #[serde(default)] pub user_assertion: Option<crate::assertions::UserAssertion>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactCard {
    pub fact_id: String,
    pub version: i64,
    pub previous_version: Option<i64>,
    pub text: String,
    pub source_note: String,
    pub sources: Vec<FactSource>,
    pub reason: String,
    pub confirmed_at: Option<String>,
    pub recorded_at: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct FactCorrection {
    pub expected_version: i64,
    pub text: String,
    pub source_note: String,
    pub sources: Vec<FactSourceRef>,
    pub reason: String,
    pub confirmed: bool,
}
#[derive(Debug, Serialize)]
pub struct FactHistory {
    pub versions: Vec<FactCard>,
    pub next_before: Option<i64>,
}
fn card(row:sqlx::sqlite::SqliteRow)->Result<FactCard> {
    let sources:String=row.get("sources_json");
    Ok(FactCard {fact_id:"personal-fact-card".into(),version:row.get("version"),previous_version:row.get("previous_version"),text:row.get("text"),source_note:row.get("source_note"),sources:serde_json::from_str(&sources).map_err(|e|format!("事实来源记录不可读：{e}"))?,reason:row.get("reason"),confirmed_at:row.get("confirmed_at"),recorded_at:row.get("recorded_at")})
}
impl Index {
    pub async fn fact_card(&self)->Result<FactCard> {
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let latest=card(sqlx::query("SELECT * FROM fact_revisions ORDER BY version DESC LIMIT 1").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?)?;
        let text:Option<String>=sqlx::query_scalar("SELECT value FROM settings WHERE key='facts'").fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
        if latest.text!=text.unwrap_or_default() {return Err("事实卡与版本记录不一致，已停止使用，请先核对资料库".into());}
        Ok(latest)
    }
    pub async fn fact_history(&self,before:Option<i64>)->Result<FactHistory> {
        let rows=sqlx::query("SELECT * FROM fact_revisions WHERE (? IS NULL OR version < ?) ORDER BY version DESC LIMIT 21")
            .bind(before).bind(before).fetch_all(&self.pool).await.map_err(|e|e.to_string())?;
        let more=rows.len()>20;
        let versions=rows.into_iter().take(20).map(card).collect::<Result<Vec<_>>>()?;
        let next_before=if more {versions.last().map(|v|v.version)} else {None};
        Ok(FactHistory {versions,next_before})
    }
    pub async fn correct_facts(&self, update:FactCorrection)->Result<FactCard> {
        if !update.confirmed {return Err("请先明确确认本次事实更正；未保存".into());}
        if update.text.chars().count()>2200 {return Err("事实卡超过2200字符，请精简且保留限制".into());}
        if [&update.text,&update.reason,&update.source_note].iter().any(|text|crate::contains_credential(text)){return Err("事实或来源说明含API凭证格式，未保存；请先移除凭证".into());}
        if update.reason.trim().is_empty() || update.reason.chars().count()>300 {return Err("请填写不超过300字符的更正原因".into());}
        if update.source_note.trim().is_empty() || update.source_note.chars().count()>1200 {return Err("请填写不超过1200字符的具体来源与定位，或本人直接核对的依据".into());}
        if update.sources.len()>6 {return Err("事实来源最多选择6个片段".into());}
        let _gate=self.gate.lock().await;
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let previous=card(sqlx::query("SELECT * FROM fact_revisions ORDER BY version DESC LIMIT 1").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?)?;
        if previous.version!=update.expected_version {return Err("事实卡已有新版本，请加载最新版本并比较后重新确认；未覆盖任何记录".into());}
        let text:Option<String>=sqlx::query_scalar("SELECT value FROM settings WHERE key='facts'").fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
        if previous.text!=text.unwrap_or_default() {return Err("事实卡与版本记录不一致，未保存".into());}
        let root:String=sqlx::query_scalar("SELECT value FROM settings WHERE key='root'").fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?.unwrap_or_default();
        let mut sources=vec![];let mut ids=std::collections::HashSet::new();
        for source in update.sources {
            if !ids.insert(source.source_id.clone()) {return Err("事实来源重复，请重新选择".into());}
            if source.root!=crate::background::doc_root(&source.path,&root) {return Err("来源目录已改变，请重新检索并核对事实来源".into());}
            let row=sqlx::query(&format!("SELECT c.path,c.location,c.text,d.hash,d.kind,a.value FROM chunks c JOIN documents d ON d.path=c.path {} WHERE c.id=?",crate::assertions::JOIN))
                .bind(&source.source_id).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?.ok_or("事实来源已退出索引，请重新检索")?;
            let path:String=row.get("path");let hash:String=row.get("hash");let kind:String=row.get("kind");
            if path!=source.path || hash!=source.hash {return Err("来源内容已改变，请重新检索并核对事实来源".into());}
            if !["personal","evidence"].contains(&kind.as_str()) {return Err("练习、岗位与技术资料不能作为个人经历证明；请使用原始证据或本人直接核对的依据".into());}
            // Store the indexed snapshot, never text or kind supplied by the UI.
            sources.push(FactSource {reference:source,location:row.get("location"),text:row.get("text"),kind,user_assertion:crate::assertions::parse(row.get("value"))?});
        }
        let version=previous.version.checked_add(1).ok_or("事实版本已达上限")?;
        let now=Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis,true);
        let next=FactCard {fact_id:previous.fact_id,version,previous_version:Some(previous.version),text:update.text,source_note:update.source_note.trim().into(),sources,reason:update.reason.trim().into(),confirmed_at:Some(now.clone()),recorded_at:now};
        sqlx::query("INSERT INTO fact_revisions(version,previous_version,text,source_note,sources_json,reason,confirmed_at,recorded_at) VALUES(?,?,?,?,?,?,?,?)")
            .bind(next.version).bind(next.previous_version).bind(&next.text).bind(&next.source_note).bind(serde_json::to_string(&next.sources).map_err(|e|e.to_string())?).bind(&next.reason).bind(&next.confirmed_at).bind(&next.recorded_at)
            .execute(&mut *tx).await.map_err(|e|e.to_string())?;
        sqlx::query("INSERT OR REPLACE INTO settings(key,value) VALUES('facts',?)").bind(&next.text).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        tx.commit().await.map_err(|e|e.to_string())?;Ok(next)
    }
}
