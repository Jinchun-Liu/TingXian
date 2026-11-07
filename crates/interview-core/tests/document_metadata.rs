//! Synthetic fixtures are retained; no user documents or network requests.
use interview_core::*;
use chrono::{Local,Duration};
use std::path::PathBuf;
fn folder()->PathBuf {
    let path=tempfile::Builder::new().prefix("interview-doc-meta-").tempdir().unwrap().keep();
    std::fs::create_dir_all(path.join("a")).unwrap();std::fs::create_dir(path.join("b")).unwrap();path
}
fn selection(path:&str)->Selection {Selection{path:path.into(),kind:"technical".into(),allow_external:true,structured:None}}
fn update(record:&DocumentRecord,confirmed:bool,from:Option<String>,until:Option<String>)->DocumentUpdate {
    DocumentUpdate {path:record.path.clone(),expected_root:record.root.clone(),expected_hash:record.hash.clone(),confirmed,valid_from:from,valid_until:until}
}
#[tokio::test]
async fn confirmation_is_bound_to_content_and_root_not_mtime() {
    let path=folder();let file=path.join("a/note.md");std::fs::write(&file,"syntheticneedle original").unwrap();
    let index=Index::open(&path.join("index.sqlite")).await.unwrap();
    index.import(path.join("a").to_str().unwrap(),vec![selection("note.md")],"unused").await.unwrap();
    index.correct_facts(interview_core::FactCorrection {expected_version:0,text:"confirmed facts remain separate".into(),source_note:"Synthetic fixture, not real personal data".into(),sources:vec![],reason:"Synthetic fixture setup".into(),confirmed:true}).await.unwrap();
    let first=index.documents().await.unwrap().remove(0);
    assert_eq!(first.metadata.confirmation,Confirmation::Unreviewed);assert_eq!(first.metadata.validity,Validity::Unknown);
    index.update_document(update(&first,true,None,None)).await.unwrap();
    let reviewed=index.documents().await.unwrap().remove(0);assert_eq!(reviewed.metadata.confirmation,Confirmation::Confirmed);
    assert!(reviewed.metadata.confirmed_at.is_some());assert_eq!(reviewed.metadata.validity,Validity::Unknown);
    std::fs::OpenOptions::new().write(true).open(&file).unwrap().set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::now()+std::time::Duration::from_secs(60))).unwrap();
    let same=index.import(path.join("a").to_str().unwrap(),vec![selection("note.md")],"unused").await.unwrap();
    assert_eq!(same.unchanged,1);assert_eq!(index.documents().await.unwrap()[0].metadata.confirmation,Confirmation::Confirmed);
    assert!(index.documents().await.unwrap()[0].metadata.valid_from.is_none(),"mtime must not become a fact date");
    std::fs::write(&file,"syntheticneedle replacement").unwrap();
    index.import(path.join("a").to_str().unwrap(),vec![selection("note.md")],"unused").await.unwrap();
    let changed=index.documents().await.unwrap().remove(0);
    assert_eq!(changed.metadata.confirmation,Confirmation::ContentChanged);assert_eq!(changed.metadata.validity,Validity::ContentChanged);
    let version=index.setting("version").await.unwrap();
    assert!(index.update_document(update(&first,true,None,None)).await.is_err());assert_eq!(index.setting("version").await.unwrap(),version);
    assert!(index.document_segments(&first.path,&first.root,&first.hash).await.is_err());
    assert!(index.document_segments(&changed.path,&changed.root,&changed.hash).await.unwrap()[0].text.contains("replacement"));
    index.update_document(update(&changed,true,None,None)).await.unwrap();
    std::fs::write(path.join("b/note.md"),"syntheticneedle replacement").unwrap();
    index.import(path.join("b").to_str().unwrap(),vec![selection("note.md")],"unused").await.unwrap();
    let moved=index.documents().await.unwrap().remove(0);assert_eq!(moved.hash,changed.hash);
    assert_eq!(moved.metadata.confirmation,Confirmation::Unreviewed,"same bytes in another root must not inherit review");
    assert!(index.update_document(update(&changed,true,None,None)).await.is_err());
    assert!(index.document_segments(&changed.path,&changed.root,&changed.hash).await.is_err());
    assert_eq!(index.setting("facts").await.unwrap(),"confirmed facts remain separate");
}

#[tokio::test]
async fn dates_are_validated_explicit_and_dedup_prefers_current_reviewed_source() {
    let path=folder();for file in ["one.md","two.md"] {std::fs::write(path.join("a").join(file),"syntheticneedle identical statement").unwrap();}
    let index=Index::open(&path.join("index.sqlite")).await.unwrap();
    index.import(path.join("a").to_str().unwrap(),vec![selection("one.md"),selection("two.md")],"unused").await.unwrap();
    let docs=index.documents().await.unwrap();let today=Local::now().date_naive();
    index.update_document(update(&docs[0],true,None,Some((today-Duration::days(1)).to_string()))).await.unwrap();
    index.update_document(update(&docs[1],true,Some(today.to_string()),Some(today.to_string()))).await.unwrap();
    let status=index.documents().await.unwrap();assert_eq!(status[0].metadata.validity,Validity::Expired);assert_eq!(status[1].metadata.validity,Validity::Current);
    let hits=index.search("syntheticneedle","").await.unwrap();assert_eq!(hits.len(),1);assert_eq!(hits[0].path,"two.md");assert_eq!(hits[0].hash,docs[1].hash);
    let version=index.setting("version").await.unwrap();
    for (from,until) in [(Some("2026-02-30".into()),None),(Some("2026-11-04".into()),Some("2026-11-03".into())),(Some("2026-1-2".into()),None)] {
        assert!(index.update_document(update(&docs[1],true,from,until)).await.is_err());assert_eq!(index.setting("version").await.unwrap(),version);
    }
    index.update_document(update(&docs[1],false,Some((today+Duration::days(1)).to_string()),None)).await.unwrap();
    assert_eq!(index.documents().await.unwrap()[1].metadata.validity,Validity::NotYetValid);
    assert_eq!(index.documents().await.unwrap()[1].metadata.confirmation,Confirmation::Unreviewed);
    assert!(index.search("syntheticneedle","").await.unwrap().is_empty(),"expired and future sources must not enter default retrieval");
    index.update_document(update(&docs[0],true,None,None)).await.unwrap();
    let evidence=index.search("syntheticneedle","").await.unwrap();
    let messages=messages("synthetic technical question","",&[],&evidence).unwrap();
    let payload:serde_json::Value=serde_json::from_str(messages[1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(payload["evidence"][0]["source_status"]["validity"],"unknown");
    assert!(!messages.to_string().contains(&docs[0].root));assert!(!messages.to_string().contains(&docs[0].hash));
    let mut private=evidence.clone();private[0].allow_external=false;
    let private_messages=interview_core::messages("synthetic question","",&[],&private).unwrap();
    let private_payload:serde_json::Value=serde_json::from_str(private_messages[1]["content"].as_str().unwrap()).unwrap();assert!(private_payload["evidence"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn cancelled_import_keeps_review_attached_to_old_active_document() {
    let path=folder();std::fs::write(path.join("a/note.md"),"syntheticneedle original").unwrap();
    let index=Index::open(&path.join("index.sqlite")).await.unwrap();index.import(path.join("a").to_str().unwrap(),vec![selection("note.md")],"unused").await.unwrap();
    let record=index.documents().await.unwrap().remove(0);index.update_document(update(&record,true,None,None)).await.unwrap();
    let before=serde_json::to_value(index.documents().await.unwrap()).unwrap();let version=index.setting("version").await.unwrap();
    std::fs::write(path.join("a/note.md"),"syntheticneedle changed").unwrap();
    let control=ImportControl::default();let result=index.import_controlled(path.join("a").to_str().unwrap(),vec![selection("note.md")],"unused",&control,|progress| {if progress.phase=="indexing"{control.cancel();}}).await;
    assert_eq!(result.unwrap_err(),IMPORT_CANCELLED);assert_eq!(serde_json::to_value(index.documents().await.unwrap()).unwrap(),before);assert_eq!(index.setting("version").await.unwrap(),version);
}

#[tokio::test]
async fn legacy_database_migration_preserves_rows_version_and_marks_status_unknown() {
    let path=folder();let database=path.join("legacy.sqlite");
    let pool=sqlx::sqlite::SqlitePoolOptions::new().connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&database).create_if_missing(true)).await.unwrap();
    sqlx::raw_sql("CREATE TABLE documents(path TEXT PRIMARY KEY,hash TEXT NOT NULL,kind TEXT NOT NULL,allow_external INTEGER NOT NULL);CREATE TABLE chunks(id TEXT PRIMARY KEY,path TEXT NOT NULL,location TEXT NOT NULL,text TEXT NOT NULL);CREATE VIRTUAL TABLE search USING fts5(id UNINDEXED,tokens);CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);INSERT INTO settings VALUES('version','7');INSERT INTO documents VALUES('legacy.md','legacyhash','technical',1);INSERT INTO chunks VALUES('S1','legacy.md','line 1','legacyneedle');INSERT INTO search VALUES('S1','legacyneedle');").execute(&pool).await.unwrap();pool.close().await;
    let index=Index::open(&database).await.unwrap();assert_eq!(index.setting("version").await.unwrap(),"7");
    let hits=index.search("legacyneedle","").await.unwrap();assert_eq!(hits.len(),1);assert_eq!(hits[0].source_id,"S1");assert_eq!(hits[0].metadata.confirmation,Confirmation::Unreviewed);assert_eq!(hits[0].metadata.validity,Validity::Unknown);
}
