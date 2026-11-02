use crate::Result;
use jieba_rs::Jieba;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{sqlite::{SqliteConnectOptions, SqlitePoolOptions}, Row, SqlitePool};
use std::{collections::HashSet, path::{Component, Path, PathBuf}, sync::Arc, time::{Duration, Instant}};
use tokio::sync::Mutex;

const MAX_FILE: u64 = 50 * 1024 * 1024;
const MAX_TEXT: usize = 500_000;
pub const DEFAULT_ALIASES: &str = "示例项目,DemoProject,demo project\n扩缩容,autoscaling\n契约测试,contract test\n空值,null\n零值,zero\n示例大学,Example University\n云数据库,cloud database,Doris\n结构校验,schema,JSON Schema";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate { pub path: String, pub bytes: u64, pub eligible: bool, pub reason: String }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Selection { pub path: String, pub kind: String, pub allow_external: bool }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment { pub location: String, pub text: String }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub source_id: String, pub path: String, pub location: String, pub text: String,
    pub kind: String, pub allow_external: bool, pub score: f64,
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
    matches!(path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref(), Some("md"|"txt"|"pdf"|"docx"))
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
pub fn scan(root: &str) -> Result<Vec<Candidate>> {
    let root = Path::new(root).canonicalize().map_err(|e| e.to_string())?;
    if !root.is_dir() { return Err("请选择可读文件夹".into()); }
    let mut out = vec![];
    for entry in walkdir::WalkDir::new(&root).follow_links(false).into_iter().filter_entry(|e| {
        e.path() == root || !excluded(e.path().strip_prefix(&root).unwrap_or(e.path()))
    }) {
        let e = entry.map_err(|e| e.to_string())?;
        if e.file_type().is_dir() { continue; }
        if out.len() >= 20_000 { return Err("扫描超过 2 万个文件，请选择更小目录".into()); }
        let path = e.path().strip_prefix(&root).map_err(|e| e.to_string())?.to_string_lossy().into_owned();
        let bytes = e.metadata().map_err(|e| e.to_string())?.len();
        let reason = if e.file_type().is_symlink() { "符号链接，未纳入" }
            else if bytes > MAX_FILE { "文件超过 50 MB" }
            else if !supported(e.path()) { "格式不支持；JSON/YAML 需另行选择字段后转文本" }
            else { "待选择纳入" };
        out.push(Candidate { path, bytes, eligible: reason == "待选择纳入", reason: reason.into() });
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
            CREATE VIRTUAL TABLE IF NOT EXISTS search USING fts5(id UNINDEXED, tokens);
            CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT OR IGNORE INTO settings VALUES('version','0');").execute(&pool).await.map_err(|e| e.to_string())?;
        Ok(Self { pool, jieba: Arc::new(Jieba::new()), gate: Arc::new(Mutex::new(())) })
    }
    pub async fn setting(&self, key: &str) -> Result<String> {
        Ok(sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key=?").bind(key).fetch_optional(&self.pool).await.map_err(|e|e.to_string())?.unwrap_or_default())
    }
    pub async fn set(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query("INSERT OR REPLACE INTO settings(key,value) VALUES(?,?)").bind(key).bind(value).execute(&self.pool).await.map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn tokens(&self, text: &str) -> Vec<String> {
        self.jieba.cut_for_search(text, false).into_iter().map(str::to_lowercase)
            .filter(|s| s.chars().any(char::is_alphanumeric)).collect()
    }
    pub async fn import(&self, root: &str, selected: Vec<Selection>, python: &str) -> Result<ImportReport> {
        let _guard = self.gate.lock().await;
        let start = Instant::now();
        let root = Path::new(root).canonicalize().map_err(|e|e.to_string())?;
        let root_text = root.to_string_lossy().into_owned();
        let all = scan(&root_text)?;
        let mut report = ImportReport { root: root_text.clone(), discovered: all.len(), included: selected.len(), excluded: all.len().saturating_sub(selected.len()), ..Default::default() };
        let mut prepared = vec![];
        let old_root = self.setting("root").await?;
        let mut seen = HashSet::new();
        for item in selected {
            if !seen.insert(item.path.clone()) { continue; }
            let parsed = async {
                if !["personal", "evidence", "practice", "job", "technical"].contains(&item.kind.as_str()) { return Err("资料类型无效".into()); }
                let path = safe_path(&root, &item.path)?;
                if !supported(&path) { return Err("不支持该格式".into()); }
                let meta = tokio::fs::metadata(&path).await.map_err(|e|e.to_string())?;
                if meta.len() > MAX_FILE { return Err("文件超过 50 MB，未导入".into()); }
                let bytes = tokio::fs::read(&path).await.map_err(|e|e.to_string())?;
                let hash = digest(&bytes);
                let old: Option<String> = sqlx::query_scalar("SELECT hash FROM documents WHERE path=?").bind(&item.path).fetch_optional(&self.pool).await.map_err(|e|e.to_string())?;
                if old_root == root_text && old.as_deref() == Some(&hash) { return Ok((hash, None)); }
                let segments = if matches!(path.extension().and_then(|s|s.to_str()).map(str::to_lowercase).as_deref(), Some("md"|"txt")) {
                    let text = String::from_utf8(bytes).map_err(|_|"文本不是 UTF-8")?;
                    text.lines().enumerate().filter(|(_,t)|!t.trim().is_empty()).map(|(i,t)|Segment{location:format!("行 {}",i+1),text:t.into()}).collect::<Vec<_>>()
                } else {
                    let output = tokio::time::timeout(Duration::from_secs(30), tokio::process::Command::new(python)
                        .arg("-c").arg(include_str!("extract.py")).arg(&path).kill_on_drop(true).output()).await
                        .map_err(|_|"解析超时（30 秒）")?.map_err(|e| format!("解析工具不可用：{e}"))?;
                    if !output.status.success() { return Err(format!("抽取失败：{}",String::from_utf8_lossy(&output.stderr).chars().take(300).collect::<String>())); }
                    String::from_utf8(output.stdout).map_err(|e|e.to_string())?.lines().map(|line| serde_json::from_str::<Segment>(line).map_err(|e|e.to_string())).collect::<Result<Vec<_>>>()?
                };
                if segments.iter().map(|s|s.text.chars().count()).sum::<usize>() > MAX_TEXT { return Err("文本超过 50 万字符，未截断导入".into()); }
                if segments.is_empty() { return Err("文档没有文本".into()); }
                Ok((hash, Some(chunk(segments))))
            }.await;
            match parsed {
                Ok((hash, chunks)) => { if chunks.is_none() {report.unchanged+=1;} report.success+=1; prepared.push((item,hash,chunks)); }
                Err(e) => report.failures.push((item.path,e)),
            }
        }
        // Atomic publication: failed/removed/excluded documents cannot leave stale evidence active.
        let mut tx = self.pool.begin().await.map_err(|e|e.to_string())?;
        let active: HashSet<_> = prepared.iter().map(|(s,_,_)|s.path.as_str()).collect();
        let old: Vec<String> = sqlx::query_scalar("SELECT path FROM documents").fetch_all(&mut *tx).await.map_err(|e|e.to_string())?;
        for path in old {
            if old_root != root_text || !active.contains(path.as_str()) { remove_document(&mut tx, &path).await?; }
        }
        for (item, hash, chunks) in prepared {
            if let Some(chunks) = chunks {
                remove_document(&mut tx, &item.path).await?;
                for (i,segment) in chunks.into_iter().enumerate() {
                    let id = format!("S{}", &digest(format!("{}:{hash}:{i}",item.path).as_bytes())[..16]);
                    sqlx::query("INSERT INTO chunks VALUES(?,?,?,?)").bind(&id).bind(&item.path).bind(&segment.location).bind(&segment.text).execute(&mut *tx).await.map_err(|e|e.to_string())?;
                    sqlx::query("INSERT INTO search(id,tokens) VALUES(?,?)").bind(&id).bind(self.tokens(&segment.text).join(" ")).execute(&mut *tx).await.map_err(|e|e.to_string())?;
                }
            }
            sqlx::query("INSERT OR REPLACE INTO documents VALUES(?,?,?,?)").bind(&item.path).bind(hash).bind(&item.kind).bind(item.allow_external).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        }
        sqlx::query("INSERT OR REPLACE INTO settings VALUES('root',?)").bind(&root_text).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        sqlx::query("UPDATE settings SET value=CAST(value AS INTEGER)+1 WHERE key='version'").execute(&mut *tx).await.map_err(|e|e.to_string())?;
        tx.commit().await.map_err(|e|e.to_string())?;
        report.version = self.setting("version").await?.parse().unwrap_or(0);
        report.chunks = sqlx::query_scalar("SELECT count(*) FROM chunks").fetch_one(&self.pool).await.map_err(|e|e.to_string())?;
        report.elapsed_ms=start.elapsed().as_millis();
        Ok(report)
    }
    pub async fn search(&self, question: &str, aliases: &str) -> Result<Vec<Evidence>> {
        let mut expanded = question.to_string();
        let lower = question.to_lowercase();
        for line in aliases.lines().take(100) {
            let words: Vec<_> = line.split(',').map(str::trim).filter(|w| !w.is_empty()).collect();
            if words.iter().any(|w| lower.contains(&w.to_lowercase())) { expanded.push(' ');expanded.push_str(&words.join(" ")); }
        }
        let mut seen = HashSet::new();
        let tokens: Vec<_> = self.tokens(&expanded).into_iter().filter(|s| {
            !["的","是","了","你","我","在","吗","如何","为什么","一下","什么","怎么","是否","请","介绍"].contains(&s.as_str()) && seen.insert(s.clone())
        }).take(80).collect();
        if tokens.is_empty() {return Ok(vec![]);}
        let query = tokens.iter().map(|t|format!("\"{}\"",t.replace('"',"\"\""))).collect::<Vec<_>>().join(" OR ");
        let rows=sqlx::query("SELECT c.id,c.path,c.location,c.text,d.kind,d.allow_external,bm25(search) AS score FROM search JOIN chunks c ON c.id=search.id JOIN documents d ON d.path=c.path WHERE search MATCH ? ORDER BY score LIMIT 40")
            .bind(query).fetch_all(&self.pool).await.map_err(|e|e.to_string())?;
        let mut texts=HashSet::new();let mut result=vec![];
        for r in rows {
            let text:String=r.get("text");
            if !texts.insert(digest(text.as_bytes())) {continue;}
            result.push(Evidence{source_id:r.get("id"),path:r.get("path"),location:r.get("location"),text,kind:r.get("kind"),allow_external:r.get("allow_external"),score:r.get("score")});
            if result.len()==6 {break;}
        }
        Ok(result)
    }
}
async fn remove_document(tx: &mut sqlx::Transaction<'_,sqlx::Sqlite>,path:&str)->Result<()> {
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
        let dir=tempfile::tempdir().unwrap(); let source=dir.path().join("中文经历.md");
        std::fs::write(&source,"# 个人经历\n我主要借助 AI 编码，能够阅读、修改、验证代码。\n已获示例大学有条件录取，尚未入读。\n离线扩缩容研究不能等同生产经验。").unwrap();
        let index=Index::open(&dir.path().join("test.sqlite")).await.unwrap();
        let selected=vec![Selection{path:"中文经历.md".into(),kind:"personal".into(),allow_external:true}];
        let r=index.import(dir.path().to_str().unwrap(),selected.clone(),"python3").await.unwrap();assert_eq!(r.success,1);
        let hits=index.search("示例大学在读吗",DEFAULT_ALIASES).await.unwrap();assert!(hits[0].text.contains("尚未入读"));
        assert!(!index.search("autoscaling",DEFAULT_ALIASES).await.unwrap().is_empty());
        assert!(index.search("\" OR * --",DEFAULT_ALIASES).await.is_ok());
        let r=index.import(dir.path().to_str().unwrap(),selected,"python3").await.unwrap();assert_eq!(r.unchanged,1);
        index.import(dir.path().to_str().unwrap(),vec![],"python3").await.unwrap();assert!(index.search("示例大学",DEFAULT_ALIASES).await.unwrap().is_empty());
    }
    #[test]
    fn reject_escapes_and_credentials() {
        let root=Path::new("/tmp");
        for path in ["../etc/passwd","/etc/passwd",".env","私密/key.txt","本轮申请信息.json"] {assert!(safe_path(root,path).is_err());}
    }
    #[tokio::test]
    async fn changed_invalid_and_missing_files_never_leave_stale_evidence() {
        let dir=tempfile::tempdir().unwrap();let file=dir.path().join("事实.txt");
        let index=Index::open(&dir.path().join("test.sqlite")).await.unwrap();
        let selected=vec![Selection{path:"事实.txt".into(),kind:"personal".into(),allow_external:true}];
        std::fs::write(&file,"旧词棕熊").unwrap();
        index.import(dir.path().to_str().unwrap(),selected.clone(),"python3").await.unwrap();
        std::fs::write(&file,"新词海豚").unwrap();
        let report=index.import(dir.path().to_str().unwrap(),selected.clone(),"python3").await.unwrap();
        assert_eq!(report.unchanged,0);assert!(index.search("棕熊","").await.unwrap().is_empty());
        assert!(!index.search("海豚","").await.unwrap().is_empty());
        std::fs::write(&file,[0xff,0xfe]).unwrap();
        let report=index.import(dir.path().to_str().unwrap(),selected.clone(),"python3").await.unwrap();
        assert_eq!(report.failures.len(),1);assert_eq!(report.chunks,0);
        std::fs::rename(&file,dir.path().join("moved-away.txt")).unwrap();
        let report=index.import(dir.path().to_str().unwrap(),selected,"python3").await.unwrap();
        assert_eq!(report.failures.len(),1);assert_eq!(report.chunks,0);
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_are_visible_but_not_importable() {
        let dir=tempfile::tempdir().unwrap();let outside=tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("fact.txt"),"private").unwrap();
        std::os::unix::fs::symlink(outside.path().join("fact.txt"),dir.path().join("link.txt")).unwrap();
        let found=scan(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(found.len(),1);assert!(!found[0].eligible);assert!(safe_path(dir.path(),"link.txt").is_err());
    }
}
