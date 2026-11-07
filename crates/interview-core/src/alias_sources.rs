//! Local retrieval aliases retain the source snapshot that introduced each line.
use crate::{Index,Result};
use serde::{Deserialize,Serialize};
use serde_json::Value;
use std::collections::{BTreeMap,HashMap,HashSet};

const KEY:&str="alias_sources";
const FORMAT:&str="alias-sources-v1";
const MAX_METADATA_BYTES:usize=8*1024*1024;
type Tx<'a>=sqlx::Transaction<'a,sqlx::Sqlite>;
#[derive(Serialize,Deserialize)]
struct Lineage {format:String,lines:BTreeMap<String,String>,snapshots:BTreeMap<String,Value>}
impl Default for Lineage {
    fn default()->Self {Self {format:FORMAT.into(),lines:BTreeMap::new(),snapshots:BTreeMap::new()}}
}
#[derive(Debug,Serialize)]
pub struct AliasState {pub aliases:String,pub blocked_lines:Vec<String>}
impl AliasState {
    pub fn effective(&self)->String {
        let blocked:HashSet<_>=self.blocked_lines.iter().map(String::as_str).collect();
        self.aliases.lines().filter(|line|!blocked.contains(line.trim())).collect::<Vec<_>>().join("\n")
    }
}
fn lines(value:&str)->impl Iterator<Item=&str> {value.lines().map(str::trim).filter(|s|!s.is_empty())}
fn validate(value:&str)->Result<()> {
    if value.chars().count()>4000 {return Err("别名超过 4000 字，请先精简".into());}
    if crate::contains_credential(value) {return Err("别名含 API 凭证格式，未保存；请使用专用 Key 输入框".into());}
    Ok(())
}
async fn saved(tx:&mut Tx<'_>)->Result<String> {
    Ok(sqlx::query_scalar::<_,String>("SELECT value FROM settings WHERE key='aliases'")
        .fetch_optional(&mut **tx).await.map_err(|e|e.to_string())?.unwrap_or_else(||crate::DEFAULT_ALIASES.into()))
}
async fn read(tx:&mut Tx<'_>)->Result<Lineage> {
    let raw:Option<String>=sqlx::query_scalar("SELECT value FROM settings WHERE key=?").bind(KEY)
        .fetch_optional(&mut **tx).await.map_err(|e|e.to_string())?;
    let Some(raw)=raw else {return Ok(Lineage::default());};
    if raw.len()>MAX_METADATA_BYTES {return Err("别名来源记录过大，未用于检索".into());}
    let lineage:Lineage=serde_json::from_str(&raw).map_err(|_|"别名来源记录无效，未用于检索")?;
    if lineage.format!=FORMAT {return Err("别名来源版本不受支持，未用于检索".into());}
    Ok(lineage)
}
async fn write(tx:&mut Tx<'_>,mut lineage:Lineage,aliases:&str)->Result<()> {
    let active:HashSet<_>=lines(aliases).map(|s|crate::digest(s.as_bytes())).collect();
    lineage.lines.retain(|key,_|active.contains(key));
    let used:HashSet<_>=lineage.lines.values().collect();
    lineage.snapshots.retain(|key,_|used.contains(key));
    // One snapshot per import, not per alias. Bound accumulated old batches;
    // an oversized merge fails atomically without changing the library.
    let raw=serde_json::to_string(&lineage).map_err(|e|e.to_string())?;
    if raw.len()>MAX_METADATA_BYTES {return Err("别名来源记录超过 8 MB，请先精简旧别名或取消本次术语合并".into());}
    sqlx::query("INSERT OR REPLACE INTO settings VALUES(?,?)").bind(KEY).bind(raw)
        .execute(&mut **tx).await.map_err(|e|e.to_string())?;
    sqlx::query("INSERT OR REPLACE INTO settings VALUES('aliases',?)").bind(aliases)
        .execute(&mut **tx).await.map_err(|e|e.to_string())?;Ok(())
}
pub(crate) async fn merge(tx:&mut Tx<'_>,suggested:&str)->Result<()> {
    let old=saved(tx).await?;let mut lineage=read(tx).await?;
    let mut existing:HashSet<_>=lines(&old).map(String::from).collect();
    let added:Vec<_>=lines(suggested).filter(|s|existing.insert((*s).into())).collect();
    if added.is_empty() {return Ok(());}
    let snapshot=crate::preference_sources::capture(tx).await?;
    let key=crate::digest(snapshot.to_string().as_bytes());
    for line in &added {lineage.lines.insert(crate::digest(line.as_bytes()),key.clone());}
    lineage.snapshots.insert(key,snapshot);
    // Existing exact lines retain ownership, even when a later model repeats
    // them. It cannot silently certify an old line or capture a manual one.
    let aliases=if old.is_empty(){added.join("\n")}else{format!("{}\n{}",old.trim_end_matches(['\r','\n']),added.join("\n"))};
    validate(&aliases)?;write(tx,lineage,&aliases).await
}
impl Index {
    pub async fn alias_state(&self)->Result<AliasState> {
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let aliases=saved(&mut tx).await?;let lineage=read(&mut tx).await?;
        let mut checked=HashMap::new();let mut blocked_lines=vec![];let mut seen=HashSet::new();
        for line in lines(&aliases).filter(|line|seen.insert(*line)) {
            let Some(snapshot)=lineage.lines.get(&crate::digest(line.as_bytes())) else {continue;};
            let valid=if let Some(valid)=checked.get(snapshot){*valid}else{
                let valid=match lineage.snapshots.get(snapshot){Some(value)=>crate::preference_sources::current(&mut tx,value).await?,None=>false};
                checked.insert(snapshot.clone(),valid);valid
            };
            if !valid {blocked_lines.push(line.to_string());}
        }
        Ok(AliasState {aliases,blocked_lines})
    }
    pub async fn request_aliases(&self)->Result<String> {Ok(self.alias_state().await?.effective())}
    pub async fn set_aliases(&self,value:&str,expected:Option<&str>)->Result<()> {
        validate(value)?;let _gate=self.gate.lock().await;
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let previous=saved(&mut tx).await?;
        if expected.is_some_and(|expected|expected!=previous) {return Err("别名已变化，编辑草稿已保留；请重新读取后核对".into());}
        let lineage=read(&mut tx).await?;
        // Unchanged lines keep their binding through reorder, whitespace and
        // unrelated edits. New/changed lines are explicit manual input.
        write(&mut tx,lineage,value).await?;tx.commit().await.map_err(|e|e.to_string())
    }
    pub async fn confirm_manual_aliases(&self,selected:&[String],expected:&str)->Result<()> {
        if selected.is_empty()||selected.len()>100 {return Err("请选择 1 至 100 行要核对的别名".into());}
        let _gate=self.gate.lock().await;let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let aliases=saved(&mut tx).await?;
        if aliases!=expected {return Err("别名已变化，请重新读取后核对；未修改".into());}
        let active:HashSet<_>=lines(&aliases).collect();let mut seen=HashSet::new();let mut lineage=read(&mut tx).await?;
        for line in selected {
            if !active.contains(line.as_str())||!seen.insert(line) {return Err("别名行已变化或重复；未修改".into());}
            lineage.lines.remove(&crate::digest(line.as_bytes()));
        }
        write(&mut tx,lineage,&aliases).await?;tx.commit().await.map_err(|e|e.to_string())
    }
}
