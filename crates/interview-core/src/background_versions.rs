//! Local provenance only: archived extracts never enter search or model inputs.
use crate::{background::{Bundle,Doc},digest,Index,Result};
use serde_json::{json,Value};
use sqlx::{Row,Sqlite,Transaction};
use std::collections::HashMap;

const PREFIX:&str="revision/";
const LIMIT:i64=64*1024*1024;
fn hex(value:&str)->bool{value.len()==64&&value.bytes().all(|b|b.is_ascii_hexdigit()&&!b.is_ascii_uppercase())}
pub(crate) async fn archive(tx:&mut Transaction<'_,Sqlite>,doc:&Doc,version:&str,recorded:bool)->Result<()> {
    let raw=serde_json::to_string(doc).map_err(|e|e.to_string())?;
    let key=format!("{PREFIX}{}/{}",doc.id,digest(raw.as_bytes()));
    let entry=json!({"document":doc,"index_version":version,"recorded_at":if recorded{Some(chrono::Utc::now().to_rfc3339())}else{None}}).to_string();
    sqlx::query("INSERT OR IGNORE INTO background_library(key,value) VALUES(?,?)").bind(key).bind(entry).execute(&mut **tx).await.map_err(|e|e.to_string())?;
    Ok(())
}
pub(crate) async fn check_limit(tx:&mut Transaction<'_,Sqlite>)->Result<()> {
    let bytes:i64=sqlx::query_scalar("SELECT coalesce(sum(length(CAST(value AS BLOB))),0) FROM background_library WHERE key GLOB 'revision/*'").fetch_one(&mut **tx).await.map_err(|e|e.to_string())?;
    if bytes>LIMIT{return Err("来源版本记录达到64 MiB上限，本次未应用；请先导出并整理资料库，旧资料已保留".into());}Ok(())
}
async fn current(tx:&mut Transaction<'_,Sqlite>,source:&str,expected:&str)->Result<Doc>{
    if !hex(source)||!hex(expected){return Err("来源版本标识无效".into());}
    let raw:Option<String>=sqlx::query_scalar("SELECT value FROM background_library WHERE key=?").bind(source).fetch_optional(&mut **tx).await.map_err(|e|e.to_string())?;
    let doc:Doc=serde_json::from_str(&raw.ok_or("来源已不在当前资料库")?).map_err(|_|"来源资料无效")?;
    let hash=digest(&serde_json::to_vec(&doc).map_err(|e|e.to_string())?);
    let active:Option<String>=sqlx::query_scalar("SELECT hash FROM documents WHERE path=?").bind(format!(".背景/{}/{}",doc.id,doc.name)).fetch_optional(&mut **tx).await.map_err(|e|e.to_string())?;
    if hash!=expected||active.as_deref()!=Some(expected){return Err("来源已变化，请刷新后查看版本".into());}Ok(doc)
}
impl Index {
    pub async fn background_changes(&self,bundle:&Bundle,version:&str)->Result<Value>{
        bundle.validate()?;let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let active:String=sqlx::query_scalar("SELECT value FROM settings WHERE key='version'").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
        if active!=version{return Err("资料库已变化，请重新核对草稿".into());}
        let rows=sqlx::query("SELECT b.key,json_extract(b.value,'$.name') AS name,json_extract(b.value,'$.hash') AS source_hash,d.hash AS extraction_hash FROM background_library b JOIN documents d ON d.path='.背景/'||b.key||'/'||json_extract(b.value,'$.name') WHERE length(b.key)=64 AND b.key NOT GLOB '*[^a-f0-9]*'")
            .fetch_all(&mut *tx).await.map_err(|e|e.to_string())?;
        let mut existing=HashMap::new();let mut hashes:HashMap<String,Vec<(String,String)>>=HashMap::new();
        for row in rows{
            let id:String=row.get("key");let hash:String=row.get("source_hash");let name:String=row.get("name");
            hashes.entry(hash.clone()).or_default().push((id.clone(),name));
            existing.insert(id,(hash,row.get::<String,_>("extraction_hash")));
        }
        let mut changes=vec![];
        for doc in &bundle.documents{
            let hash=digest(&serde_json::to_vec(doc).map_err(|e|e.to_string())?);
            let state=match existing.get(&doc.id){None=>"new",Some((old,_)) if old!=&doc.hash=>"source_changed",Some((_,old)) if old!=&hash=>"extraction_changed",_=>"unchanged"};
            let identical=hashes.get(&doc.hash).into_iter().flatten().filter(|(id,_)|id!=&doc.id).map(|(_,name)|name.clone()).collect::<Vec<_>>();
            changes.push(json!({"id":doc.id,"name":doc.name,"state":state,"identical_count":identical.len(),"identical_sources":identical.into_iter().take(3).collect::<Vec<_>>()}));
        }
        Ok(json!(changes))
    }
    pub async fn background_previous(&self,source:&str,version:&str,offset:usize)->Result<Value>{
        if !hex(source)||offset>16000{return Err("来源或分页无效".into());}
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let active:String=sqlx::query_scalar("SELECT value FROM settings WHERE key='version'").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
        if active!=version{return Err("资料库已变化，请重新核对草稿".into());}
        let raw:Option<String>=sqlx::query_scalar("SELECT value FROM background_library WHERE key=?").bind(source).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
        let Some(raw)=raw else{return Ok(Value::Null)};
        let doc:Doc=serde_json::from_str(&raw).map_err(|_|"先前来源资料无效")?;
        Ok(json!({"name":doc.name,"source_hash":doc.hash,"rows":doc.records.iter().skip(offset).take(20).collect::<Vec<_>>(),"more":offset+20<doc.records.len()}))
    }
    pub async fn background_versions(&self,source:&str,expected:&str,offset:usize)->Result<Value>{
        if offset>100000{return Err("分页无效".into());}
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;let doc=current(&mut tx,source,expected).await?;
        let rows=sqlx::query("SELECT key,json_extract(value,'$.document.name') AS name,json_extract(value,'$.document.hash') AS source_hash,json_extract(value,'$.recorded_at') AS recorded_at,json_array_length(value,'$.document.records') AS count FROM background_library WHERE key GLOB ? ORDER BY CAST(json_extract(value,'$.index_version') AS INTEGER) DESC,key DESC LIMIT 21 OFFSET ?")
            .bind(format!("{PREFIX}{source}/*")).bind(offset as i64).fetch_all(&mut *tx).await.map_err(|e|e.to_string())?;
        let more=rows.len()>20;
        let mut result=rows.into_iter().take(20).map(|r|{
            let key:String=r.get("key");let revision=key.rsplit('/').next().unwrap_or("");
            json!({"revision":revision,"name":r.get::<String,_>("name"),"source_hash":r.get::<String,_>("source_hash"),"recorded_at":r.get::<Option<String>,_>("recorded_at"),"count":r.get::<i64,_>("count"),"current":revision==expected})
        }).collect::<Vec<_>>();
        if result.is_empty()&&offset==0{result.push(json!({"revision":expected,"name":doc.name,"source_hash":doc.hash,"recorded_at":null,"count":doc.records.len(),"current":true}));}
        Ok(json!({"rows":result,"more":more}))
    }
    pub async fn background_version_records(&self,source:&str,expected:&str,revision:&str,offset:usize)->Result<Value>{
        if !hex(revision)||offset>16000{return Err("版本或分页无效".into());}
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;let latest=current(&mut tx,source,expected).await?;
        let doc=if revision==expected{latest}else{
            let raw:Option<String>=sqlx::query_scalar("SELECT value FROM background_library WHERE key=?").bind(format!("{PREFIX}{source}/{revision}")).fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
            let value:Value=serde_json::from_str(&raw.ok_or("未找到来源版本")?).map_err(|_|"来源版本不可读")?;
            let doc:Doc=serde_json::from_value(value["document"].clone()).map_err(|_|"来源版本无效")?;
            if doc.id!=source||digest(&serde_json::to_vec(&doc).map_err(|e|e.to_string())?)!=revision{return Err("来源版本校验失败".into());}doc
        };
        Ok(json!({"rows":doc.records.iter().skip(offset).take(20).collect::<Vec<_>>(),"more":offset+20<doc.records.len(),"current":revision==expected}))
    }
}
