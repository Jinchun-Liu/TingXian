//! Append-only namespace for reviewed machine-generated material. Original
//! quotes are retrieved as evidence; model interpretations add search terms.
use crate::{digest, contains_credential, Index, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;

pub const PREFIX: &str = ".背景/";
pub const BASELINE:u64 = 256*1024;
pub const ANSWER_CHARS:usize = 12_000;
const OUTPUT:u64 = crate::MAX_ANSWER_TOKENS as u64;
pub fn doc_root<'a>(path:&str,root:&'a str)->&'a str {if path.starts_with(PREFIX){"背景资料库"}else{root}}
#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record{pub kind:String,pub location:String,pub quote:String,pub note:String}
#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Doc{pub id:String,pub name:String,pub hash:String,pub records:Vec<Record>}
#[derive(Clone,Serialize,Deserialize,Default)]
#[serde(deny_unknown_fields)]
pub struct Profile{pub overview:String,pub role:String,pub aliases:String}
#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle{pub format:String,pub documents:Vec<Doc>,pub profile:Profile}

fn bounded(s:&str,n:usize,empty:bool)->bool {
    (empty||!s.trim().is_empty()) && s.chars().count()<=n && !contains_credential(s)
        && !s.chars().any(|c|c.is_control() && !matches!(c,'\n'|'\t'|'\r'))
}
impl Bundle {
    pub fn validate(&self)->Result<()> {
        if self.documents.is_empty(){return Err("资料包为空".into())}self.validate_fields()
    }
    fn validate_fields(&self)->Result<()> {
        let p=&self.profile;
        if self.format!="meetily-background-v1" || self.documents.len()>2000
            || !bounded(&p.overview,8000,false) || !bounded(&p.role,200,true) || !bounded(&p.aliases,2000,true) {return Err("背景资料包或设置格式无效".into())}
        let mut ids=std::collections::HashSet::new();
        for d in &self.documents {
            let hex=|s:&str|s.len()==64 && s.bytes().all(|b|b.is_ascii_hexdigit()&&!b.is_ascii_uppercase());
            if !hex(&d.id)||!hex(&d.hash)||!ids.insert(&d.id)||!bounded(&d.name,1024,false)||d.name.starts_with('/')
            ||d.name.split('/').any(|p|p.is_empty()||p.starts_with('.'))||d.records.is_empty()||d.records.len()>16000 {return Err("来源名称、标识或条目数量无效".into())}
            for r in &d.records {
                if !["personal","evidence","practice","job","technical"].contains(&r.kind.as_str())||!bounded(&r.note,450,false)||!bounded(&r.quote,650,false)||!bounded(&r.location,180,false){return Err("来源条目格式无效".into())}
            }
        }
        Ok(())
    }
}
impl Index {
    pub async fn background_view(&self)->Result<Value> {
        let row=sqlx::query("SELECT COUNT(*) AS chunks,COALESCE(SUM(length(CAST(c.text AS BLOB))+length(CAST(c.location AS BLOB))+length(CAST(c.path AS BLOB))+128),0) AS bytes FROM chunks c").fetch_one(&self.pool).await.map_err(|e|e.to_string())?;
        let docs:i64=sqlx::query_scalar("SELECT count(*) FROM documents").fetch_one(&self.pool).await.map_err(|e|e.to_string())?;
        let all:u64=row.get::<i64,_>("bytes") as u64;
        let state:Option<String>=sqlx::query_scalar("SELECT value FROM background_library WHERE key='profile'").fetch_optional(&self.pool).await.map_err(|e|e.to_string())?;
        let state:Value=state.map(|s|serde_json::from_str(&s)).transpose().map_err(|e|e.to_string())?.unwrap_or(Value::Null);
        // Text chat upper budget includes worst-case 4-byte Unicode, output,
        // role/framing overhead. Not a provider tokenizer or an image estimate.
        let rag=ANSWER_CHARS as u64*4+OUTPUT+2048;
        let full=rag+all;
        let measurement:Option<String>=sqlx::query_scalar("SELECT value FROM background_library WHERE key='measurement'").fetch_optional(&self.pool).await.map_err(|e|e.to_string())?;
        let measurement:Value=measurement.map(|s|serde_json::from_str(&s)).transpose().map_err(|e|e.to_string())?.unwrap_or(Value::Null);
        Ok(json!({"documents":docs,"chunks":row.get::<i64,_>("chunks"),"full_budget":full,"rag_budget":rag,
            "measurement":measurement,"baseline":BASELINE,"fits_full":full<=BASELINE,"state":state,"version":self.setting("version").await?}))
    }
    pub async fn background_inputs(&self,measure:bool)->Result<Vec<Value>>{
        // Only documents marked for sharing may participate in remote synthesis.
        // Use the already indexed selection, never reopen original JSON/YAML.
        let mut out=vec![];let mut bytes=0;let mut cursor=0;
        loop {
            let (next,batch)=self.background_input_page(measure,cursor).await?;
            if batch.is_empty(){break}
            bytes+=batch.iter().map(|i|i.to_string().len()).sum::<usize>();
            if bytes>16*1024*1024{return Err("既有可共享资料超过单批 16 MiB，请仅整理本次选择的文件".into())}
            out.extend(batch);cursor=next;
        }
        Ok(out)
    }
    pub async fn background_input_page(&self,measure:bool,cursor:i64)->Result<(i64,Vec<Value>)>{
        let rows=sqlx::query("SELECT c.rowid AS seq,c.path,c.location,c.text,d.kind FROM chunks c JOIN documents d ON d.path=c.path WHERE c.rowid>? AND d.allow_external=1 AND (? OR c.path!='.背景/综合背景') ORDER BY c.rowid LIMIT 256").bind(cursor).bind(measure).fetch_all(&self.pool).await.map_err(|e|e.to_string())?;
        let mut next=cursor;let mut out=vec![];
        for r in rows {
            let path:String=r.get("path");
            let (id,name)=path.strip_prefix(PREFIX).and_then(|p|p.split_once('/')).unwrap_or(("",path.as_str()));
            let item=json!({"id":id,"file":name,"kind":r.get::<String,_>("kind"),"location":r.get::<String,_>("location"),"quote":r.get::<String,_>("text")});
            if contains_credential(&item.to_string()){return Err("既有材料含凭证格式，未发送".into())}
            next=r.get("seq");out.push(item);
        }
        Ok((next,out))
    }
    pub async fn apply_background(&self,bundle:&Bundle,version:&str,share:bool,metrics:Value,merge_terms:bool)->Result<()>{
        bundle.validate()?;
        self.save_background(bundle,version,share,metrics,merge_terms).await
    }
    pub async fn edit_background(&self,profile:Profile,version:&str,share:bool,metrics:Value,merge_terms:bool)->Result<()>{
        let bundle=Bundle{format:"meetily-background-v1".into(),documents:vec![],profile};
        bundle.validate_fields()?;
        self.save_background(&bundle,version,share,metrics,merge_terms).await
    }
    async fn save_background(&self,bundle:&Bundle,version:&str,share:bool,metrics:Value,merge_terms:bool)->Result<()>{
        let _guard=self.gate.lock().await;
        let mut tx=self.pool.begin().await.map_err(|e|e.to_string())?;
        let current:String=sqlx::query_scalar("SELECT value FROM settings WHERE key='version'").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
        if current!=version{return Err("资料库已变化，请重新整理或重新打开资料包".into())}
        for doc in &bundle.documents {
            let prefix=format!("{PREFIX}{}/",doc.id);
            let old:Vec<String>=sqlx::query_scalar("SELECT path FROM documents WHERE substr(path,1,length(?))=?").bind(&prefix).bind(&prefix).fetch_all(&mut *tx).await.map_err(|e|e.to_string())?;
            for p in old {crate::index::remove_document(&mut tx,&p).await?;}
            let path=format!("{prefix}{}",doc.name);
            let first=&doc.records[0].kind;
            let kind=if doc.records.iter().all(|r|r.kind==*first){first.as_str()}else{"practice"};
            // A changed extraction also invalidates a previous source review.
            let hash=digest(&serde_json::to_vec(doc).map_err(|e|e.to_string())?);
            sqlx::query("INSERT INTO documents VALUES(?,?,?,?)").bind(&path).bind(&hash).bind(kind).bind(share).execute(&mut *tx).await.map_err(|e|e.to_string())?;
            for (i,r) in doc.records.iter().enumerate() {
                let id=format!("B{}",&digest(format!("{path}:{hash}:{i}").as_bytes())[..24]);
                let tokens=self.tokens(&format!("{} {} {}",doc.name,r.quote,r.note)).join(" ");
                sqlx::query("INSERT INTO chunks VALUES(?,?,?,?)").bind(&id).bind(&path).bind(&r.location).bind(&r.quote).execute(&mut *tx).await.map_err(|e|e.to_string())?;
                sqlx::query("INSERT INTO search VALUES(?,?)").bind(id).bind(tokens).execute(&mut *tx).await.map_err(|e|e.to_string())?;
            }
            sqlx::query("INSERT OR REPLACE INTO background_library VALUES(?,?)").bind(&doc.id).bind(serde_json::to_string(doc).map_err(|e|e.to_string())?).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        }
        let path=".背景/综合背景"; crate::index::remove_document(&mut tx,path).await?;
        let hash=digest(bundle.profile.overview.as_bytes());
        sqlx::query("INSERT INTO documents VALUES(?,?,'practice',?)").bind(path).bind(&hash).bind(share).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        let chars=bundle.profile.overview.chars().collect::<Vec<_>>();
        for (i,part) in chars.chunks(450).enumerate(){
            let text=part.iter().collect::<String>(); let id=format!("P{}:{i}",&hash[..20]);
            sqlx::query("INSERT INTO chunks VALUES(?,?,?,?)").bind(&id).bind(path).bind(format!("模型整理 {}",i+1)).bind(&text).execute(&mut *tx).await.map_err(|e|e.to_string())?;
            sqlx::query("INSERT INTO search VALUES(?,?)").bind(&id).bind(self.tokens(&text).join(" ")).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        }
        if merge_terms && !bundle.profile.aliases.trim().is_empty(){
            let old:Option<String>=sqlx::query_scalar("SELECT value FROM settings WHERE key='aliases'").fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
            let mut lines=old.as_deref().filter(|s|!s.is_empty()).unwrap_or(crate::DEFAULT_ALIASES).lines().collect::<Vec<_>>();
            for line in bundle.profile.aliases.lines().map(str::trim).filter(|s|!s.is_empty()){if !lines.contains(&line){lines.push(line)}}
            let aliases=lines.join("\n"); if aliases.chars().count()>4000{return Err("合并后术语超过 4000 字，请先精简".into())}
            sqlx::query("INSERT OR REPLACE INTO settings VALUES('aliases',?)").bind(aliases).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        }
        sqlx::query("INSERT OR REPLACE INTO background_library VALUES('profile',?)").bind(json!({"profile":bundle.profile,"share":share,"metrics":metrics}).to_string()).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        sqlx::query("UPDATE settings SET value=CAST(value AS INTEGER)+1 WHERE key='version'").execute(&mut *tx).await.map_err(|e|e.to_string())?;
        tx.commit().await.map_err(|e|e.to_string())
    }
    pub async fn background_role(&self)->Result<Option<String>>{
        let role:Option<String>=sqlx::query_scalar("SELECT CASE WHEN json_extract(value,'$.share')=1 THEN json_extract(value,'$.profile.role') ELSE '' END FROM background_library WHERE key='profile'").fetch_optional(&self.pool).await.map_err(|e|e.to_string())?;
        Ok(role)
    }
}
