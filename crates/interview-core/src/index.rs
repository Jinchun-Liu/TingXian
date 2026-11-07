use crate::{Result, ImportControl, ImportProgress};
use jieba_rs::Jieba;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{sqlite::{SqliteConnectOptions, SqlitePoolOptions}, Row, SqlitePool};
use std::{collections::HashSet, path::{Component, Path, PathBuf}, sync::Arc, time::{Duration, Instant}};
use tokio::sync::Mutex;

const MAX_FILE: u64 = 50 * 1024 * 1024;
const MAX_TEXT: usize = 500_000;
pub const DEFAULT_ALIASES: &str = "";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate { pub path: String, pub bytes: u64, pub eligible: bool, pub reason: String }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Selection { pub path: String, pub kind: String, pub allow_external: bool, #[serde(default)] pub structured:Option<crate::StructuredSelection> }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment { pub location: String, pub text: String }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub source_id: String, pub path: String, pub location: String, pub text: String,
    pub kind: String, pub allow_external: bool, pub score: f64,
    #[serde(default)] pub hash: String,
    #[serde(default)] pub root: String,
    #[serde(default)] pub metadata: crate::DocumentStatus,
    #[serde(default)] pub user_assertion: Option<crate::assertions::UserAssertion>,
}
impl Evidence {
    pub fn current_text(&self)->&str {self.user_assertion.as_ref().map(|a|a.claim.as_str()).unwrap_or(&self.text)}
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImportReport {
    pub root: String, pub discovered: usize, pub included: usize, pub success: usize,
    pub unchanged: usize, pub excluded: usize, pub chunks: i64, pub version: i64,
    pub elapsed_ms: u128, pub failures: Vec<(String, String)>,
}
#[derive(Clone)]
pub struct Index { pub pool: SqlitePool, jieba: Arc<Jieba>, pub gate: Arc<Mutex<()>> }

pub fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }

fn excluded(path: &Path) -> bool {
    path.components().any(|p| {
        let s = p.as_os_str().to_string_lossy().to_lowercase();
        s.starts_with('.') || ["node_modules", "target", "dist", "build", "__pycache__"].contains(&s.as_str())
            || ["私密", "凭证", "证件", "本轮申请信息", "密钥", "secret", "credential"].iter().any(|word| s.contains(word))
    })
}
fn supported(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref(), Some("md"|"txt"|"pdf"|"docx"|"json"|"yaml"|"yml"))
}
pub fn safe_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let p = Path::new(relative);
    if p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) || excluded(p) {
        return Err("文件不在允许范围，或属于默认排除项".into());
    }
    let mut current = root.to_path_buf();
    for part in p.components() {
        current.push(part);
        if std::fs::symlink_metadata(&current).map_err(|e| e.to_string())?.file_type().is_symlink() {
            return Err("首版不导入符号链接".into());
        }
    }
    let path = current.canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(root) || !path.is_file() { return Err("路径越出资料根目录或不是文件".into()); }
    Ok(path)
}
pub fn scan(root: &str) -> Result<Vec<Candidate>> { scan_controlled(root, &ImportControl::default()) }
fn scan_controlled(root: &str, control: &ImportControl) -> Result<Vec<Candidate>> {
    control.check()?;
    let root = Path::new(root).canonicalize().map_err(|e| e.to_string())?;
    if !root.is_dir() { return Err("请选择可读文件夹".into()); }
    let mut out = vec![];
    for entry in walkdir::WalkDir::new(&root).follow_links(false).into_iter().filter_entry(|e| {
        e.path() == root || !excluded(e.path().strip_prefix(&root).unwrap_or(e.path()))
    }) {
        control.check()?;
        let e = entry.map_err(|e| e.to_string())?;
        if e.file_type().is_dir() { continue; }
        if out.len() >= 20_000 { return Err("扫描超过 2 万个文件，请选择更小目录".into()); }
        let path = e.path().strip_prefix(&root).map_err(|e| e.to_string())?.to_string_lossy().into_owned();
        let bytes = e.metadata().map_err(|e| e.to_string())?.len();
        let reason = if e.file_type().is_symlink() { "符号链接，未纳入" }
            else if bytes > MAX_FILE { "文件超过 50 MB" }
            else if !supported(e.path()) { "格式暂不支持" }
            else if crate::structured_format(e.path()).is_some() { "需预览并选择字段或整份文件" }
            else { "待选择纳入" };
        out.push(Candidate { path, bytes, eligible: ["待选择纳入","需预览并选择字段或整份文件"].contains(&reason), reason: reason.into() });
    }
    out.sort_by(|a,b| a.path.cmp(&b.path));
    Ok(out)
}

impl Index {
    pub async fn open(path: &Path) -> Result<Self> {
        let pool = SqlitePoolOptions::new().max_connections(1).connect_with(
            SqliteConnectOptions::new().filename(path).create_if_missing(true)
        ).await.map_err(|e| e.to_string())?;
        sqlx::raw_sql("CREATE TABLE IF NOT EXISTS documents(path TEXT PRIMARY KEY, hash TEXT NOT NULL, kind TEXT NOT NULL, allow_external INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS chunks(id TEXT PRIMARY KEY, path TEXT NOT NULL, location TEXT NOT NULL, text TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS document_metadata(path TEXT NOT NULL, root TEXT NOT NULL, reviewed_hash TEXT NOT NULL, confirmed INTEGER NOT NULL, confirmed_at TEXT, valid_from TEXT, valid_until TEXT, updated_at TEXT NOT NULL, PRIMARY KEY(path,root));
            CREATE VIRTUAL TABLE IF NOT EXISTS search USING fts5(id UNINDEXED, tokens);
            CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS background_library(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS fact_revisions(version INTEGER PRIMARY KEY, previous_version INTEGER, text TEXT NOT NULL, source_note TEXT NOT NULL, sources_json TEXT NOT NULL, reason TEXT NOT NULL, confirmed_at TEXT, recorded_at TEXT NOT NULL);
            INSERT OR IGNORE INTO fact_revisions(version,previous_version,text,source_note,sources_json,reason,confirmed_at,recorded_at)
                VALUES(0,NULL,COALESCE((SELECT value FROM settings WHERE key='facts'),''),'旧版事实卡；来源仍以原正文为准，未重新确认','[]','迁移基线，不代表新增用户确认',NULL,strftime('%Y-%m-%dT%H:%M:%fZ','now'));
            INSERT OR IGNORE INTO settings VALUES('version','0');").execute(&pool).await.map_err(|e| e.to_string())?;
        sqlx::raw_sql(crate::assertions::SCHEMA).execute(&pool).await.map_err(|e|e.to_string())?;
        Ok(Self { pool, jieba: Arc::new(Jieba::new()), gate: Arc::new(Mutex::new(())) })
    }
    pub async fn setting(&self, key: &str) -> Result<String> {
        Ok(sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key=?").bind(key).fetch_optional(&self.pool).await.map_err(|e|e.to_string())?.unwrap_or_default())
    }
    pub async fn set(&self, key: &str, value: &str) -> Result<()> {
        if key=="facts" {return Err("事实卡须通过明确确认的更正入口保存，不能直接覆盖".into());}
        sqlx::query("INSERT OR REPLACE INTO settings(key,value) VALUES(?,?)").bind(key).bind(value).execute(&self.pool).await.map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn tokens(&self, text: &str) -> Vec<String> {
        self.jieba.cut_for_search(text, false).into_iter().map(str::to_lowercase)
            .filter(|s| s.chars().any(char::is_alphanumeric)).collect()
    }
    pub async fn import(&self, root: &str, selected: Vec<Selection>, python: &str) -> Result<ImportReport> {
        self.import_controlled(root, selected, python, &ImportControl::default(), |_| {}).await
    }
    pub async fn import_controlled<F>(&self, root: &str, selected: Vec<Selection>, python: &str, control: &ImportControl, mut notify: F) -> Result<ImportReport>
    where F: FnMut(ImportProgress) + Send {
        let result = self.import_inner(root, selected, python, control, &mut notify).await;
        control.finish();
        result
    }
    async fn import_inner<F>(&self, root: &str, selected: Vec<Selection>, python: &str, control: &ImportControl, notify: &mut F) -> Result<ImportReport>
    where F: FnMut(ImportProgress) + Send {
        let _guard = control.run(async { Ok(self.gate.lock().await) }).await?;
        let start = Instant::now();
        let root = control.run(async { tokio::fs::canonicalize(root).await.map_err(|e|e.to_string()) }).await?;
        let root_text = root.to_string_lossy().into_owned();
        let total = selected.len();
        notify(ImportProgress {phase:"scanning".into(),completed:0,total,path:None});
        let scan_root = root_text.clone(); let scan_control = control.clone();
        let all = control.run(async { tokio::task::spawn_blocking(move || scan_controlled(&scan_root, &scan_control)).await.map_err(|e| e.to_string())? }).await?;
        let mut report = ImportReport { root: root_text.clone(), discovered: all.len(), included: total, excluded: all.len().saturating_sub(total), ..Default::default() };
        let mut prepared = vec![];
        let old_root = control.run(self.setting("root")).await?;
        let mut seen = HashSet::new();
        for (position, item) in selected.into_iter().enumerate() {
            control.check()?;
            if !seen.insert(item.path.clone()) { continue; }
            notify(ImportProgress {phase:"parsing".into(),completed:position,total,path:Some(item.path.clone())});
            let parsed = control.run(async {
                if !["personal", "evidence", "practice", "job", "technical"].contains(&item.kind.as_str()) { return Err("资料类型无效".into()); }
                let path = safe_path(&root, &item.path)?;
                if !supported(&path) { return Err("不支持该格式".into()); }
                let meta = tokio::fs::metadata(&path).await.map_err(|e|e.to_string())?;
                if meta.len() > MAX_FILE { return Err("文件超过 50 MB，未导入".into()); }
                let bytes = tokio::fs::read(&path).await.map_err(|e|e.to_string())?;
                let hash_control = control.clone();
                let (bytes, hash) = tokio::task::spawn_blocking(move || {
                    hash_control.check()?; let hash = digest(&bytes); hash_control.check()?; Ok::<_,String>((bytes, hash))
                }).await.map_err(|e|e.to_string())??;
                let format=crate::structured_format(&path);
                let pointers=if format.is_some() {Some(crate::structured::selected_pointers(item.structured.as_ref().ok_or("JSON/YAML 须先预览并明确选择字段或整份文件")?,&root_text,&hash)?)}
                    else {if item.structured.is_some(){return Err("字段选择仅适用于 JSON/YAML".into());}None};
                // A different field scope is a different imported revision,
                // even when the original file bytes have not changed.
                let hash=if let Some(pointers)=&pointers {crate::structured::selection_hash(&hash,pointers)}else{hash};
                let old: Option<String> = sqlx::query_scalar("SELECT hash FROM documents WHERE path=?").bind(&item.path).fetch_optional(&self.pool).await.map_err(|e|e.to_string())?;
                if old_root == root_text && old.as_deref() == Some(&hash) {
                    // Revalidate older cached text too; unchanged file hashes
                    // cannot bypass a newly introduced import boundary.
                    let cached:Vec<String>=sqlx::query_scalar("SELECT text FROM chunks WHERE path=?").bind(&item.path).fetch_all(&self.pool).await.map_err(|e|e.to_string())?;
                    if cached.iter().any(|text|crate::contains_credential(text)){return Err("已缓存内容含API凭证格式，未继续纳入；请选择不含凭证的资料或字段".into());}
                    return Ok((hash, None));
                }
                let plain = matches!(path.extension().and_then(|s|s.to_str()).map(str::to_lowercase).as_deref(), Some("md"|"txt"));
                let bytes = if let (Some(format),Some(pointers))=(format,&pointers) {crate::structured::structured_segments(bytes,format,python,pointers).await?} else if plain { bytes } else {
                    let output = tokio::time::timeout(Duration::from_secs(30), tokio::process::Command::new(python).args(["-X","utf8","-B"])
                        .arg("-c").arg(include_str!("extract.py")).arg(&path).kill_on_drop(true).output()).await
                        .map_err(|_|"解析超时（30 秒）")?.map_err(|e| format!("解析工具不可用：{e}"))?;
                    if !output.status.success() { return Err(format!("抽取失败：{}",String::from_utf8_lossy(&output.stderr).chars().take(300).collect::<String>())); }
                    output.stdout
                };
                let parse_control = control.clone(); let tokenizer = self.jieba.clone();
                let chunks = tokio::task::spawn_blocking(move || {
                    parse_control.check()?;
                    let text = String::from_utf8(bytes).map_err(|_|"文本不是 UTF-8")?;
                    let mut segments = vec![]; let mut characters = 0;
                    for (i, line) in text.lines().enumerate() {
                        parse_control.check()?;
                        let segment: Segment = if plain {
                            if line.trim().is_empty() { continue; }
                            Segment {location:format!("行 {}",i+1),text:line.into()}
                        } else { serde_json::from_str(line).map_err(|e|e.to_string())? };
                        characters += segment.text.chars().count();
                        if crate::contains_credential(&segment.text){return Err("所选内容含API凭证格式，未导入；请选择不含凭证的资料或字段".into());}
                        if characters > MAX_TEXT { return Err("文本超过 50 万字符，未截断导入".into()); }
                        segments.push(segment);
                    }
                    if segments.is_empty() { return Err("文档没有文本".into()); }
                    let mut prepared = vec![];
                    for segment in chunk(segments) {
                        parse_control.check()?;
                        let tokens = tokenizer.cut_for_search(&segment.text, false).into_iter().map(str::to_lowercase)
                            .filter(|s|s.chars().any(char::is_alphanumeric)).collect::<Vec<_>>().join(" ");
                        prepared.push((segment,tokens));
                    }
                    parse_control.check()?; Ok::<_,String>(prepared)
                }).await.map_err(|e|e.to_string())??;
                Ok((hash, Some(chunks)))
            }).await;
            // Cancellation is a whole-job rollback, never a failed individual
            // document that could accidentally publish an empty active index.
            control.check()?;
            match parsed {
                Ok((hash, chunks)) => { if chunks.is_none() {report.unchanged+=1;} report.success+=1; prepared.push((item,hash,chunks)); }
                Err(e) => report.failures.push((item.path,e)),
            }
            notify(ImportProgress {phase:"parsed".into(),completed:position+1,total,path:None});
        }
        control.check()?;
        let mut tx = control.run(async { self.pool.begin().await.map_err(|e|e.to_string()) }).await?;
        let active: HashSet<_> = prepared.iter().map(|(s,_,_)|s.path.as_str()).collect();
        let old: Vec<String> = sqlx::query_scalar("SELECT path FROM documents").fetch_all(&mut *tx).await.map_err(|e|e.to_string())?;
        for path in old {
            control.check()?;
            if !path.starts_with(crate::background::PREFIX) && (old_root != root_text || !active.contains(path.as_str())) { remove_document(&mut tx, &path).await?; }
        }
        for (position, (item, hash, chunks)) in prepared.into_iter().enumerate() {
            control.check()?;
            if let Some(chunks) = chunks {
                remove_document(&mut tx, &item.path).await?;
                for (i,(segment,tokens)) in chunks.into_iter().enumerate() {
                    control.check()?;
                    let id = format!("S{}", &digest(format!("{}:{hash}:{i}",item.path).as_bytes())[..16]);
                    sqlx::query("INSERT INTO chunks VALUES(?,?,?,?)").bind(&id).bind(&item.path).bind(&segment.location).bind(&segment.text).execute(&mut *tx).await.map_err(|e|e.to_string())?;
                    sqlx::query("INSERT INTO search(id,tokens) VALUES(?,?)").bind(&id).bind(tokens).execute(&mut *tx).await.map_err(|e|e.to_string())?;
                }
            }
            sqlx::query("INSERT OR REPLACE INTO documents VALUES(?,?,?,?)").bind(&item.path).bind(hash).bind(&item.kind).bind(item.allow_external).execute(&mut *tx).await.map_err(|e|e.to_string())?;
            notify(ImportProgress {phase:"indexing".into(),completed:position+1,total,path:Some(item.path)});
        }
        control.check()?;
        sqlx::query("INSERT OR REPLACE INTO settings VALUES('root',?)").bind(&root_text).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        sqlx::query("UPDATE settings SET value=CAST(value AS INTEGER)+1 WHERE key='version'").execute(&mut *tx).await.map_err(|e|e.to_string())?;
        report.version = sqlx::query_scalar::<_,String>("SELECT value FROM settings WHERE key='version'").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?.parse().unwrap_or(0);
        report.chunks = sqlx::query_scalar("SELECT count(*) FROM chunks").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
        control.begin_commit()?;
        notify(ImportProgress {phase:"committing".into(),completed:total,total,path:None});
        tx.commit().await.map_err(|e|e.to_string())?;
        report.elapsed_ms=start.elapsed().as_millis();
        Ok(report)
    }
    pub async fn search(&self, question: &str, aliases: &str) -> Result<Vec<Evidence>> {
        Ok(self.search_review(question,aliases,None).await?.evidence)
    }
    pub async fn search_review(&self,question:&str,aliases:&str,facts:Option<&crate::FactCard>)->Result<crate::RetrievalReview> {
        Ok(crate::conflicts::choose_evidence(self.search_candidates(question,aliases).await?,question,facts))
    }
    async fn search_candidates(&self, question: &str, aliases: &str) -> Result<Vec<Evidence>> {
        let mut expanded = question.to_string();
        let lower = question.to_lowercase();
        for line in aliases.lines().take(100) {
            let words: Vec<_> = line.split(',').map(str::trim).filter(|w| !w.is_empty()).collect();
            if words.iter().any(|w| lower.contains(&w.to_lowercase())) { expanded.push(' ');expanded.push_str(&words.join(" ")); }
        }
        let make_query=|text:&str| {
            let mut seen=HashSet::new();
            self.tokens(text).into_iter().filter(|s| !["的","是","了","你","我","在","吗","如何","为什么","一下","什么","怎么","是否","请","介绍","做","过","做过"].contains(&s.as_str()) && seen.insert(s.clone()))
                .take(80).map(|t|format!("\"{}\"",t.replace('"',"\"\""))).collect::<Vec<_>>().join(" OR ")
        };
        let original=make_query(question);let expanded=make_query(&expanded);
        if expanded.is_empty(){return Ok(vec![]);}
        let queries=if original.is_empty() || original==expanded {vec![(expanded,1.0)]} else {vec![(original,2.0),(expanded,1.0)]};
        // Broad editable aliases supplement the user's actual wording. Read
        // both ranked sets under the import gate so source revisions cannot mix.
        let _gate=self.gate.lock().await;
        let mut merged=std::collections::HashMap::<String,Evidence>::new();
        let today=chrono::Local::now().date_naive();
        let current=crate::source_policy::CURRENT_SQL;let excluded=crate::source_policy::EXCLUDED_SQL;let summary=crate::source_policy::summary_blocked_sql();
        for (query,weight) in queries {
            // Rank the compact FTS rows first. Joining full excerpts/metadata
            // before LIMIT copies and looks up every matching document. The
            // atomic import/cleanup gate keeps FTS IDs and source rows aligned.
            let assertion_join=crate::assertions::JOIN;
            let rows=sqlx::query(&format!("WITH original_matches AS MATERIALIZED (SELECT c.id,bm25(search) AS score FROM search JOIN chunks c ON c.id=search.id JOIN documents d ON d.path=c.path {assertion_join} WHERE search MATCH ? AND a.record_id IS NULL AND {current} AND NOT {excluded} AND NOT {summary} ORDER BY score LIMIT 40), assertion_matches AS MATERIALIZED (SELECT c.id,bm25(assertion_search) AS score FROM assertion_search JOIN chunks c JOIN documents d ON d.path=c.path {assertion_join} WHERE assertion_search MATCH ? AND assertion_search.id=a.record_id AND {current} AND NOT {excluded} AND NOT {summary} ORDER BY score LIMIT 40), matches AS (SELECT * FROM original_matches UNION ALL SELECT * FROM assertion_matches) SELECT c.id,c.path,c.location,c.text,d.hash,d.kind,d.allow_external,CASE WHEN d.path GLOB '.背景/*' THEN '背景资料库' ELSE COALESCE((SELECT value FROM settings WHERE key='root'),'') END AS current_root,matches.score,a.value AS assertion_json,{} FROM matches JOIN chunks c ON c.id=matches.id JOIN documents d ON d.path=c.path {assertion_join} {} ORDER BY matches.score LIMIT 40",crate::document_meta::META_COLUMNS,crate::document_meta::META_JOIN))
                .bind(&query).bind(&query).fetch_all(&self.pool).await.map_err(|e|e.to_string())?;
            // Query lengths and added alias IDFs have different score scales.
            // Normalize each result set before applying the 2:1 weighting.
            let scale=rows.iter().map(|r|r.get::<f64,_>("score").abs()).fold(0.0,f64::max).max(f64::MIN_POSITIVE);
            for r in rows {
                let id:String=r.get("id");let score=r.get::<f64,_>("score")/scale*weight;
                if let Some(existing)=merged.get_mut(&id){existing.score+=score;continue;}
                let hash:String=r.get("hash");let metadata=crate::document_meta::status(&r,&hash,today);
                let user_assertion=crate::assertions::parse(r.get("assertion_json"))?;
                merged.insert(id.clone(),Evidence {source_id:id,path:r.get("path"),location:r.get("location"),text:r.get("text"),kind:r.get("kind"),allow_external:r.get("allow_external"),score,root:r.get("current_root"),hash,metadata,user_assertion});
            }
        }
        let mut candidates=merged.into_values().collect::<Vec<_>>();
        candidates.sort_by(|a,b| a.score.total_cmp(&b.score)
            .then_with(||crate::document_meta::provenance_rank(&a.kind,&a.metadata).cmp(&crate::document_meta::provenance_rank(&b.kind,&b.metadata)))
            .then_with(||a.source_id.cmp(&b.source_id)));
        let mut texts=HashSet::new();let mut result=vec![];
        for evidence in candidates.into_iter().take(40) {
            if !texts.insert(digest(evidence.current_text().as_bytes())) {continue;}
            result.push(evidence);
        }
        Ok(result)
    }
}
pub(crate) async fn remove_document(tx: &mut sqlx::Transaction<'_,sqlx::Sqlite>,path:&str)->Result<()> {
    sqlx::query("DELETE FROM search WHERE id IN (SELECT id FROM chunks WHERE path=?)").bind(path).execute(&mut **tx).await.map_err(|e|e.to_string())?;
    sqlx::query("DELETE FROM chunks WHERE path=?").bind(path).execute(&mut **tx).await.map_err(|e|e.to_string())?;
    sqlx::query("DELETE FROM documents WHERE path=?").bind(path).execute(&mut **tx).await.map_err(|e|e.to_string())?;
    Ok(())
}

pub fn chunk(segments:Vec<Segment>)->Vec<Segment> {
    let mut out=vec![];let mut text=String::new();let mut locations=vec![];
    for segment in segments {
        let chars:Vec<_>=segment.text.chars().collect();
        for part in chars.chunks(720) {
            if text.chars().count()+part.len()>800 && !text.is_empty() {
                out.push(Segment{location:locations.join(" – "),text:text.clone()});
                let overlap:String=text.chars().rev().take(80).collect::<Vec<_>>().into_iter().rev().collect();
                text=overlap;locations=locations.last().cloned().into_iter().collect();
            }
            text.push('\n');text.extend(part);locations.push(segment.location.clone());
        }
    }
    if !text.trim().is_empty() {out.push(Segment{location:locations.join(" – "),text});}out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn import_query_update_and_remove_are_consistent() {
        let dir=tempfile::tempdir().unwrap().keep(); let source=dir.as_path().join("中文经历.md");
        std::fs::write(&source,"# 个人经历\n我主要借助 AI 编码，能够阅读、修改、验证代码。\n已获示例大学有条件录取，尚未入读。\n离线扩缩容研究不能等同生产经验。").unwrap();
        let index=Index::open(&dir.as_path().join("test.sqlite")).await.unwrap();
        let selected=vec![Selection{path:"中文经历.md".into(),kind:"personal".into(),allow_external:true,structured:None}];
        let r=index.import(dir.as_path().to_str().unwrap(),selected.clone(),"python3").await.unwrap();assert_eq!(r.success,1);
        let hits=index.search("示例大学在读吗",DEFAULT_ALIASES).await.unwrap();assert!(hits[0].text.contains("尚未入读"));
        assert!(!index.search("autoscaling","扩缩容,autoscaling").await.unwrap().is_empty());
        assert!(index.search("\" OR * --",DEFAULT_ALIASES).await.is_ok());
        let r=index.import(dir.as_path().to_str().unwrap(),selected,"python3").await.unwrap();assert_eq!(r.unchanged,1);
        index.import(dir.as_path().to_str().unwrap(),vec![],"python3").await.unwrap();assert!(index.search("示例大学",DEFAULT_ALIASES).await.unwrap().is_empty());
    }
    #[test]
    fn reject_escapes_and_credentials() {
        let root=Path::new("/tmp");
        for path in ["../etc/passwd","/etc/passwd",".env","私密/key.txt","本轮申请信息.json"] {assert!(safe_path(root,path).is_err());}
    }
    #[tokio::test]
    async fn changed_invalid_and_missing_files_never_leave_stale_evidence() {
        let dir=tempfile::tempdir().unwrap().keep();let file=dir.as_path().join("事实.txt");
        let index=Index::open(&dir.as_path().join("test.sqlite")).await.unwrap();
        let selected=vec![Selection{path:"事实.txt".into(),kind:"personal".into(),allow_external:true,structured:None}];
        std::fs::write(&file,"旧词棕熊").unwrap();
        index.import(dir.as_path().to_str().unwrap(),selected.clone(),"python3").await.unwrap();
        std::fs::write(&file,"新词海豚").unwrap();
        let report=index.import(dir.as_path().to_str().unwrap(),selected.clone(),"python3").await.unwrap();
        assert_eq!(report.unchanged,0);assert!(index.search("棕熊","").await.unwrap().is_empty());
        assert!(!index.search("海豚","").await.unwrap().is_empty());
        std::fs::write(&file,[0xff,0xfe]).unwrap();
        let report=index.import(dir.as_path().to_str().unwrap(),selected.clone(),"python3").await.unwrap();
        assert_eq!(report.failures.len(),1);assert_eq!(report.chunks,0);
        std::fs::rename(&file,dir.as_path().join("moved-away.txt")).unwrap();
        let report=index.import(dir.as_path().to_str().unwrap(),selected,"python3").await.unwrap();
        assert_eq!(report.failures.len(),1);assert_eq!(report.chunks,0);
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_are_visible_but_not_importable() {
        let dir=tempfile::tempdir().unwrap().keep();let outside=tempfile::tempdir().unwrap().keep();
        std::fs::write(outside.as_path().join("fact.txt"),"private").unwrap();
        std::os::unix::fs::symlink(outside.as_path().join("fact.txt"),dir.as_path().join("link.txt")).unwrap();
        let found=scan(dir.as_path().to_str().unwrap()).unwrap();
        assert_eq!(found.len(),1);assert!(!found[0].eligible);assert!(safe_path(dir.as_path(),"link.txt").is_err());
    }
}
