use crate::{Index, Result, Segment};
use chrono::{Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all="snake_case")]
pub enum Confirmation { Unreviewed, Confirmed, ContentChanged }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all="snake_case")]
pub enum Validity { Unknown, Current, Expired, NotYetValid, ContentChanged, Invalid }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentStatus {
    pub confirmation: Confirmation,
    pub validity: Validity,
    pub confirmed_at: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub updated_at: Option<String>,
}
impl Default for DocumentStatus {
    fn default()->Self {Self {confirmation:Confirmation::Unreviewed,validity:Validity::Unknown,confirmed_at:None,valid_from:None,valid_until:None,updated_at:None}}
}
#[derive(Debug, Clone, Serialize)]
pub struct DocumentRecord {
    pub path: String, pub root: String, pub hash: String, pub kind: String, pub allow_external: bool,
    pub chunks: i64, pub metadata: DocumentStatus,
}
#[derive(Debug, Clone, Deserialize)]
pub struct DocumentUpdate {
    pub path: String, pub expected_root: String, pub expected_hash: String, pub confirmed: bool,
    pub valid_from: Option<String>, pub valid_until: Option<String>,
}

pub(crate) const META_COLUMNS:&str="m.reviewed_hash AS meta_hash,m.confirmed AS meta_confirmed,m.confirmed_at AS meta_confirmed_at,m.valid_from AS meta_from,m.valid_until AS meta_until,m.updated_at AS meta_updated";
pub(crate) const META_JOIN:&str="LEFT JOIN document_metadata m ON m.path=d.path AND m.root=CASE WHEN d.path GLOB '.背景/*' THEN '背景资料库' ELSE COALESCE((SELECT value FROM settings WHERE key='root'),'') END";
fn date(value:&str)->Result<NaiveDate> {
    if value.len()!=10 {return Err("有效日期须为 YYYY-MM-DD".into());}
    let parsed=NaiveDate::parse_from_str(value,"%Y-%m-%d").map_err(|_|"有效日期无效，请检查年月日")?;
    if parsed.format("%Y-%m-%d").to_string()!=value {return Err("有效日期须为 YYYY-MM-DD".into());}Ok(parsed)
}
fn bounds(from:Option<&str>,until:Option<&str>)->Result<(Option<NaiveDate>,Option<NaiveDate>)> {
    let start=from.map(date).transpose()?;let end=until.map(date).transpose()?;
    if start.zip(end).is_some_and(|(a,b)|a>b) {return Err("有效期结束日期不能早于开始日期".into());}Ok((start,end))
}
pub(crate) fn status(row:&sqlx::sqlite::SqliteRow, hash:&str, today:NaiveDate)->DocumentStatus {
    let reviewed:Option<String>=row.get("meta_hash");
    let mut result=DocumentStatus {confirmed_at:row.get("meta_confirmed_at"),valid_from:row.get("meta_from"),valid_until:row.get("meta_until"),updated_at:row.get("meta_updated"),..Default::default()};
    let Some(reviewed)=reviewed else {return result;};
    if reviewed!=hash {result.confirmation=Confirmation::ContentChanged;result.validity=Validity::ContentChanged;return result;}
    if row.get::<Option<bool>,_>("meta_confirmed").unwrap_or(false) {result.confirmation=Confirmation::Confirmed;}
    result.validity=match bounds(result.valid_from.as_deref(),result.valid_until.as_deref()) {
        Err(_)=>Validity::Invalid,
        Ok((None,None))=>Validity::Unknown,
        Ok((Some(start),_)) if today<start=>Validity::NotYetValid,
        Ok((_,Some(end))) if today>end=>Validity::Expired,
        _=>Validity::Current,
    };result
}
impl Index {
    pub async fn documents(&self)->Result<Vec<DocumentRecord>> {
        let rows=sqlx::query(&format!("SELECT d.*,CASE WHEN d.path GLOB '.背景/*' THEN '背景资料库' ELSE COALESCE((SELECT value FROM settings WHERE key='root'),'') END AS current_root,(SELECT count(*) FROM chunks c WHERE c.path=d.path) AS chunk_count,{META_COLUMNS} FROM documents d {META_JOIN} ORDER BY d.path"))
            .fetch_all(&self.pool).await.map_err(|e|e.to_string())?;
        let today=Local::now().date_naive();
        Ok(rows.into_iter().map(|r| {let hash:String=r.get("hash");let metadata=status(&r,&hash,today);DocumentRecord {path:r.get("path"),root:r.get("current_root"),hash,kind:r.get("kind"),allow_external:r.get("allow_external"),chunks:r.get("chunk_count"),metadata}}).collect())
    }
    pub async fn update_document(&self, update:DocumentUpdate)->Result<()> {
        bounds(update.valid_from.as_deref(),update.valid_until.as_deref())?;
        let _gate=self.gate.lock().await;
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let hash:Option<String>=sqlx::query_scalar("SELECT hash FROM documents WHERE path=?").bind(&update.path).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
        if hash.as_deref()!=Some(update.expected_hash.as_str()) {return Err("资料已更新或退出索引，请刷新后重新核对；未保存旧版本确认".into());}
        let root:String=sqlx::query_scalar("SELECT value FROM settings WHERE key='root'").fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?.unwrap_or_default();
        let root=crate::background::doc_root(&update.path,&root).to_owned();
        if root!=update.expected_root {return Err("资料根目录已改变，请刷新后重新核对；未保存旧目录确认".into());}
        let now=Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis,true);
        sqlx::query("INSERT OR REPLACE INTO document_metadata(path,root,reviewed_hash,confirmed,confirmed_at,valid_from,valid_until,updated_at) VALUES(?,?,?,?,?,?,?,?)")
            .bind(update.path).bind(root).bind(update.expected_hash).bind(update.confirmed).bind(if update.confirmed {Some(now.clone())} else {None})
            .bind(update.valid_from).bind(update.valid_until).bind(now).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        sqlx::query("UPDATE settings SET value=CAST(value AS INTEGER)+1 WHERE key='version'").execute(&mut *tx).await.map_err(|e|e.to_string())?;
        tx.commit().await.map_err(|e|e.to_string())?;Ok(())
    }
    pub async fn document_segments(&self,path:&str,expected_root:&str,expected_hash:&str)->Result<Vec<Segment>> {
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let hash:Option<String>=sqlx::query_scalar("SELECT hash FROM documents WHERE path=?").bind(path).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
        if hash.as_deref()!=Some(expected_hash) {return Err("资料已更新或退出索引，请刷新后查看".into());}
        let root:String=sqlx::query_scalar("SELECT value FROM settings WHERE key='root'").fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?.unwrap_or_default();
        if crate::background::doc_root(path,&root)!=expected_root {return Err("资料根目录已改变，请刷新后查看".into());}
        let rows=sqlx::query("SELECT location,text FROM chunks WHERE path=? ORDER BY rowid").bind(path).fetch_all(&mut *tx).await.map_err(|e|e.to_string())?;
        Ok(rows.into_iter().map(|r|Segment {location:r.get("location"),text:r.get("text")}).collect())
    }
}

/// Relevance remains the primary key; provenance only breaks equal BM25
/// scores, including duplicate contents from several versions/sources.
pub(crate) fn provenance_rank(kind:&str,status:&DocumentStatus)->(u8,u8,u8) {
    let kind=match kind {"evidence"=>0,"personal"=>1,"job"=>2,"technical"=>3,_=>4};
    let confirmation=match status.confirmation {Confirmation::Confirmed=>0,Confirmation::Unreviewed=>1,Confirmation::ContentChanged=>2};
    let validity=match status.validity {Validity::Current=>0,Validity::Unknown=>1,Validity::Expired|Validity::NotYetValid=>2,_=>3};
    (kind,confirmation,validity)
}
