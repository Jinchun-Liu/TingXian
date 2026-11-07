//! Program-observed dependencies for derived profiles. Never supplied by an LLM.
use crate::{Index, Result};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};
use sqlx::Row;

pub(crate) const ASSERTIONS_SQL:&str="(SELECT json_group_array(json_object('id',x.record_id,'revision',x.revision)) FROM (SELECT a.record_id,a.revision FROM current_record_assertions a WHERE a.path=d.path AND a.source_hash=d.hash AND a.source_kind=d.kind AND a.root=CASE WHEN d.path GLOB '.背景/*' THEN '背景资料库' ELSE COALESCE((SELECT value FROM settings WHERE key='root'),'') END AND EXISTS(SELECT 1 FROM chunks c WHERE c.id=a.source_id AND c.path=d.path) ORDER BY a.record_id) x)";
pub(crate) const REVIEW_SQL:&str="(SELECT json_object('confirmed',m.confirmed,'confirmed_at',m.confirmed_at,'reviewed_hash',m.reviewed_hash,'valid_from',m.valid_from,'valid_until',m.valid_until) FROM document_metadata m WHERE m.path=d.path AND m.root=CASE WHEN d.path GLOB '.背景/*' THEN '背景资料库' ELSE COALESCE((SELECT value FROM settings WHERE key='root'),'') END)";
// Normalize the stored object's key order: serde_json feature unification in a
// desktop host must not invalidate an otherwise identical persisted binding.
const STORED_REVIEW_SQL:&str="CASE WHEN json_type(dep.value,'$.review')='object' THEN json_object('confirmed',json_extract(dep.value,'$.review.confirmed'),'confirmed_at',json_extract(dep.value,'$.review.confirmed_at'),'reviewed_hash',json_extract(dep.value,'$.review.reviewed_hash'),'valid_from',json_extract(dep.value,'$.review.valid_from'),'valid_until',json_extract(dep.value,'$.review.valid_until')) END";
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
pub struct ReviewBinding {pub reviewed_hash:String,pub confirmed:i64,pub confirmed_at:Option<String>,pub valid_from:Option<String>,pub valid_until:Option<String>}
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
pub struct AssertionVersion {pub id:String,pub revision:i64}
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
pub struct Dependency {
    pub path:String,pub root:String,pub hash:String,pub kind:String,pub allow_external:bool,
    pub assertions:Vec<AssertionVersion>,pub review:Option<ReviewBinding>,
}
impl Dependency {
    pub fn id(&self)->String {crate::digest(&serde_json::to_vec(self).unwrap())}
}
pub(crate) fn legacy_invalid_sql()->String {
    format!("(EXISTS(SELECT 1 FROM background_library x WHERE x.key GLOB 'excluded/*' AND json_extract(x.value,'$.excluded')=1) OR EXISTS(SELECT 1 FROM record_assertions) OR EXISTS(SELECT 1 FROM documents d WHERE NOT {}))",crate::source_policy::CURRENT_SQL)
}
pub(crate) fn available_sql()->String {
    let current=crate::source_policy::CURRENT_SQL;let excluded=crate::source_policy::EXCLUDED_SQL;
    format!("(CASE WHEN EXISTS(SELECT 1 FROM background_library WHERE key='profile_dependencies') THEN EXISTS(SELECT 1 FROM background_library p JOIN background_library b ON b.key='profile' WHERE p.key='profile_dependencies' AND json_extract(p.value,'$.format')='profile-dependencies-v1' AND json_extract(p.value,'$.profile')=json_extract(b.value,'$.profile') AND json_array_length(p.value,'$.sources')>0 AND NOT EXISTS(SELECT 1 FROM json_each(p.value,'$.sources') dep WHERE NOT EXISTS(SELECT 1 FROM documents d WHERE d.path=json_extract(dep.value,'$.path') AND d.hash=json_extract(dep.value,'$.hash') AND d.kind=json_extract(dep.value,'$.kind') AND d.allow_external=json_extract(dep.value,'$.allow_external') AND CASE WHEN d.path GLOB '.背景/*' THEN '背景资料库' ELSE COALESCE((SELECT value FROM settings WHERE key='root'),'') END=json_extract(dep.value,'$.root') AND {current} AND NOT {excluded} AND {ASSERTIONS_SQL}=json_extract(dep.value,'$.assertions') AND {REVIEW_SQL} IS {STORED_REVIEW_SQL}))) ELSE NOT {} END)",legacy_invalid_sql())
}
pub(crate) async fn snapshot(tx:&mut sqlx::Transaction<'_,sqlx::Sqlite>,path:Option<&str>)->Result<Vec<Dependency>> {
    let current=crate::source_policy::CURRENT_SQL;let excluded=crate::source_policy::EXCLUDED_SQL;
    // The single-path branch records even currently ineligible dependencies so
    // applying a draft cannot silently make an excluded source eligible again.
    // Keep the exact-path predicate separate: an OR against the whole-library
    // filter prevents SQLite from using the document primary key on each import.
    let filter=match path {Some(_)=>"d.path=?".to_owned(),None=>format!("d.allow_external=1 AND {current} AND NOT {excluded}")};
    let sql=format!("SELECT d.*,CASE WHEN d.path GLOB '.背景/*' THEN '背景资料库' ELSE COALESCE((SELECT value FROM settings WHERE key='root'),'') END AS root,{ASSERTIONS_SQL} AS assertions,{REVIEW_SQL} AS review FROM documents d WHERE d.path!='.背景/综合背景' AND {filter} AND EXISTS(SELECT 1 FROM chunks c WHERE c.path=d.path) ORDER BY root,d.path");
    let mut query=sqlx::query(&sql);
    if let Some(path)=path {query=query.bind(path);}
    let rows=query.fetch_all(&mut **tx).await.map_err(|e|e.to_string())?;
    rows.into_iter().map(|r|Ok(Dependency {path:r.get("path"),root:r.get("root"),hash:r.get("hash"),kind:r.get("kind"),allow_external:r.get("allow_external"),review:r.get::<Option<String>,_>("review").map(|v|serde_json::from_str(&v).map_err(|e|e.to_string())).transpose()?,assertions:serde_json::from_str(&r.get::<String,_>("assertions")).map_err(|e|e.to_string())?})).collect()
}
pub(crate) async fn publish(tx:&mut sqlx::Transaction<'_,sqlx::Sqlite>,bundle:&crate::background::Bundle,observed:&[Dependency])->Result<()> {
    if bundle.documents.is_empty() {
        // Editing wording preserves the old dependency bindings, including any
        // stale bindings. A manual summary edit cannot certify new source data.
        let raw:Option<String>=sqlx::query_scalar("SELECT value FROM background_library WHERE key='profile_dependencies'").fetch_optional(&mut **tx).await.map_err(|e|e.to_string())?;
        if let Some(raw)=raw {
            let mut value:Value=serde_json::from_str(&raw).map_err(|e|e.to_string())?;
            value["profile"]=json!(bundle.profile);
            sqlx::query("UPDATE background_library SET value=? WHERE key='profile_dependencies'").bind(value.to_string()).execute(&mut **tx).await.map_err(|e|e.to_string())?;
        }
        return Ok(());
    }
    let mut deps=observed.to_vec();
    for doc in &bundle.documents {
        let path=format!(".背景/{}/{}",doc.id,doc.name);
        let mut source=snapshot(tx,Some(&path)).await?.into_iter().next().ok_or("整理来源未进入索引")?;
        // Raw document records do not incorporate manual claims. Only a
        // separately observed identical source can certify those revisions.
        source.assertions.clear();
        if !deps.iter().any(|d|d.path==source.path&&d.root==source.root&&d.hash==source.hash&&d.kind==source.kind&&d.allow_external==source.allow_external) {deps.push(source);}
    }
    deps.sort_by_key(Dependency::id);deps.dedup();
    let value=json!({"format":"profile-dependencies-v1","profile":bundle.profile,"sources":deps});
    sqlx::query("INSERT OR REPLACE INTO background_library VALUES('profile_dependencies',?)").bind(value.to_string()).execute(&mut **tx).await.map_err(|e|e.to_string())?;
    Ok(())
}
impl Index {
    pub async fn profile_available(&self)->Result<bool> {
        sqlx::query_scalar(&format!("SELECT {}",available_sql())).fetch_one(&self.pool).await.map_err(|e|e.to_string())
    }
    pub async fn background_dependencies(&self)->Result<Vec<Dependency>> {
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        snapshot(&mut tx,None).await
    }
}
