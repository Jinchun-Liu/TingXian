//! Independently clear imported retrieval data, with a verified Trash backup.
//! Original documents, confirmed facts, settings and conversation data stay put.
use interview_core::Index;
use serde::Serialize;
use serde_json::{json, Map};
use std::{fs, io::Write, path::{Path,PathBuf}};

#[derive(Debug, Serialize)]
pub struct IndexClearReport {
    pub archive_path: Option<String>,
    pub documents: usize,
    pub chunks: usize,
    pub metadata_records: usize,
    pub version: i64,
}

pub async fn clear_index_with<F>(index:&Index, archive_root:&Path, trash:F)->Result<IndexClearReport,String>
where F:FnOnce(&Path)->Result<PathBuf,String> + Send + 'static {
    let _gate=index.gate.lock().await;
    let mut tx=index.pool.begin().await.map_err(|e|e.to_string())?;
    let old_version:String=sqlx::query_scalar("SELECT value FROM settings WHERE key='version'").fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    let old_version=old_version.parse::<i64>().map_err(|_|"索引版本无效，已停止清理")?;
    let root:Option<String>=sqlx::query_scalar("SELECT value FROM settings WHERE key='root'").fetch_optional(&mut *tx).await.map_err(|e|e.to_string())?;
    let mut rows=Map::new();
    for table in ["documents","chunks","search","document_metadata","background_library","record_assertions","assertion_search"] {
        let values=crate::interview_history::rows_to_json(sqlx::query(&format!("SELECT * FROM {table}")).fetch_all(&mut *tx).await.map_err(|e|e.to_string())?)?;
        rows.insert(table.into(),json!(values));
    }
    let documents=rows["documents"].as_array().unwrap().len();
    let chunks=rows["chunks"].as_array().unwrap().len();
    let metadata_records=rows["document_metadata"].as_array().unwrap().len();
    if rows["record_assertions"].as_array().unwrap().is_empty() && rows["assertion_search"].as_array().unwrap().is_empty() && rows["background_library"].as_array().unwrap().is_empty() && metadata_records==0 && documents==0 && chunks==0 && rows["search"].as_array().unwrap().is_empty() {
        tx.rollback().await.map_err(|e|e.to_string())?;
        return Ok(IndexClearReport {archive_path:None,documents,chunks,metadata_records,version:old_version});
    }
    let version=old_version.checked_add(1).ok_or("索引版本已达上限，未清理")?;
    let root_path=archive_root.to_path_buf();
    let trashed=tokio::task::spawn_blocking(move || {
        let meta=fs::symlink_metadata(&root_path).map_err(|e|e.to_string())?;
        if !meta.is_dir() || meta.file_type().is_symlink() {return Err("索引归档位置不是普通目录，已停止清理".into());}
        let archive=json!({"kind":"meetily-profile-index-backup","format_version":3,
            "created_at":chrono::Utc::now().to_rfc3339(),"index_version":old_version,"profile_root":root,
            "scope":"Imported documents, chunks, FTS, document reviews and manual assertion revisions; fact cards/settings/original files/conversations are preserved",
            "database_rows":rows});
        let bytes=serde_json::to_vec_pretty(&archive).map_err(|e|e.to_string())?;
        let hash=interview_core::digest(&bytes);
        let staging=root_path.join(format!("Meetily-索引清理-{}-{}",chrono::Local::now().format("%Y%m%d-%H%M%S"),uuid::Uuid::new_v4()));
        fs::create_dir(&staging).map_err(|e|e.to_string())?;
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&staging,fs::Permissions::from_mode(0o700)).map_err(|e|e.to_string())?;
        }
        let path=staging.join("index.json");
        let mut options=fs::OpenOptions::new();options.write(true).create_new(true);
        #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
        let mut file=options.open(&path).map_err(|e|e.to_string())?;
        file.write_all(&bytes).map_err(|e|e.to_string())?;file.sync_all().map_err(|e|e.to_string())?;drop(file);
        let destination=trash(&staging).map_err(|e|format!("{e}；未清除索引，备份保留在 {}",staging.display()))?;
        let verify=||->Result<(),String> {
            let dir_meta=fs::symlink_metadata(&destination).map_err(|e|e.to_string())?;
            let file_path=destination.join("index.json");let file_meta=fs::symlink_metadata(&file_path).map_err(|e|e.to_string())?;
            if staging.exists() || !dir_meta.is_dir() || dir_meta.file_type().is_symlink() || !file_meta.is_file() || file_meta.file_type().is_symlink() {
                return Err("废纸篓目标未通过普通文件与目录核对".into());
            }
            if interview_core::digest(&fs::read(file_path).map_err(|e|e.to_string())?)!=hash {return Err("废纸篓备份校验不一致".into());}
            Ok(())
        };
        verify().map_err(|e|format!("{e}；未清除索引，请检查备份 {}",destination.display()))?;
        Ok::<PathBuf,String>(destination)
    }).await.map_err(|e|format!("索引备份任务失败，未清理：{e}"))??;
    let result=async {
        for table in ["assertion_search","record_assertions","search","chunks","documents","document_metadata","background_library"] {sqlx::query(&format!("DELETE FROM {table}")).execute(&mut *tx).await.map_err(|e|e.to_string())?;}
        sqlx::query("UPDATE settings SET value=? WHERE key='version'").bind(version.to_string()).execute(&mut *tx).await.map_err(|e|e.to_string())?;
        Ok::<(),String>(())
    }.await;
    if let Err(error)=result {
        let rollback=tx.rollback().await;
        return Err(format!("索引清理失败：{error}；回滚结果：{rollback:?}；完整备份在 {}",trashed.display()));
    }
    tx.commit().await.map_err(|e|format!("索引提交失败：{e}；完整备份在 {}，请重新检查当前状态",trashed.display()))?;
    Ok(IndexClearReport {archive_path:Some(trashed.to_string_lossy().into_owned()),documents,chunks,metadata_records,version})
}

#[cfg(test)]
mod tests {
    use super::*;
    use interview_core::Selection;
    use sqlx::Row;
    async fn fixture()->(PathBuf,Index) {
        let root=std::env::temp_dir().join(format!("interview-index-clear-{}",uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("source")).unwrap();fs::create_dir(root.join("app")).unwrap();fs::create_dir(root.join("synthetic-trash")).unwrap();
        fs::write(root.join("source/fact.md"),"syntheticneedle 契约测试资料").unwrap();
        fs::write(root.join("app/session.json"),"synthetic conversation must remain").unwrap();
        let index=Index::open(&root.join("app/profile.sqlite")).await.unwrap();
        index.import(root.join("source").to_str().unwrap(),selection(),"unused").await.unwrap();
        let doc=index.documents().await.unwrap().remove(0);
        index.update_document(interview_core::DocumentUpdate {path:doc.path,expected_root:doc.root,expected_hash:doc.hash,confirmed:true,valid_from:Some("2026-01-01".into()),valid_until:Some("2026-12-31".into())}).await.unwrap();
        index.correct_facts(interview_core::FactCorrection {expected_version:0,text:"synthetic confirmed fact".into(),source_note:"Synthetic fixture, not real personal data".into(),sources:vec![],reason:"Synthetic fixture setup".into(),confirmed:true}).await.unwrap();index.set("aliases","synthetic,alias").await.unwrap();
        sqlx::query("INSERT INTO background_library VALUES('revision/synthetic-old','{\"quote\":\"synthetic archived source\"}')").execute(&index.pool).await.unwrap();
        (root,index)
    }
    fn selection()->Vec<Selection> {vec![Selection{path:"fact.md".into(),kind:"technical".into(),allow_external:false,structured:None}]}
    async fn snapshot(index:&Index)->Vec<String> {
        let mut result=vec![];
        for query in ["SELECT path||hash||kind||allow_external FROM documents ORDER BY path","SELECT id||path||location||text FROM chunks ORDER BY id","SELECT id||tokens FROM search ORDER BY id","SELECT key||value FROM settings ORDER BY key","SELECT key||value FROM background_library ORDER BY key","SELECT record_id||revision||value FROM record_assertions ORDER BY record_id,revision","SELECT id||tokens FROM assertion_search ORDER BY id"] {
            result.extend(sqlx::query(query).fetch_all(&index.pool).await.unwrap().iter().map(|r|r.get::<String,_>(0)));
        }result.push(serde_json::to_string(&crate::interview_history::rows_to_json(sqlx::query("SELECT * FROM document_metadata ORDER BY root,path").fetch_all(&index.pool).await.unwrap()).unwrap()).unwrap());result
    }
    fn simulated_move(destination:PathBuf)->impl FnOnce(&Path)->Result<PathBuf,String> {
        move |path| {let target=destination.join(path.file_name().unwrap());fs::rename(path,&target).map_err(|e|e.to_string())?;Ok(target)}
    }
    #[tokio::test]
    async fn manual_revisions_are_archived_and_trash_failure_leaves_them_usable(){
        let (root,index)=fixture().await;
        let doc=index.documents().await.unwrap().remove(0);
        let segment=index.document_records(&doc.path,&doc.root,&doc.hash).await.unwrap().remove(0);
        index.revise_record(interview_core::assertions::AssertionUpdate{source:interview_core::FactSourceRef{source_id:segment.source_id,path:doc.path,root:doc.root,hash:doc.hash},expected_revision:0,claim:"synthetic manual plan".into(),reason:"synthetic clarification".into(),activity:interview_core::assertions::Activity::Planned,confirmed:true}).await.unwrap();
        let before=snapshot(&index).await;
        assert!(clear_index_with(&index,&root.join("app"),|_|Err("synthetic Trash unavailable".into())).await.is_err());assert_eq!(snapshot(&index).await,before);
        let report=clear_index_with(&index,&root.join("app"),simulated_move(root.join("synthetic-trash"))).await.unwrap();
        let archive:serde_json::Value=serde_json::from_slice(&fs::read(Path::new(report.archive_path.as_ref().unwrap()).join("index.json")).unwrap()).unwrap();
        assert_eq!(archive["database_rows"]["record_assertions"].as_array().unwrap().len(),1);
        assert_eq!(archive["database_rows"]["assertion_search"].as_array().unwrap().len(),1);
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM record_assertions").fetch_one(&index.pool).await.unwrap(),0);
    }
    #[tokio::test]
    async fn verified_backup_precedes_clear_and_originals_facts_history_survive() {
        let (root,index)=fixture().await;
        let facts_before=serde_json::to_value(index.fact_history(None).await.unwrap()).unwrap();
        let report=clear_index_with(&index,&root.join("app"),simulated_move(root.join("synthetic-trash"))).await.unwrap();
        assert_eq!((report.documents,report.chunks,report.metadata_records,report.version),(1,1,1,3));
        let archive:serde_json::Value=serde_json::from_slice(&fs::read(Path::new(report.archive_path.as_ref().unwrap()).join("index.json")).unwrap()).unwrap();
        assert_eq!(archive["kind"],"meetily-profile-index-backup");assert_eq!(archive["index_version"],2);assert_eq!(archive["format_version"],3);
        assert_eq!(archive["database_rows"]["document_metadata"][0]["confirmed"],1);
        assert_eq!(archive["database_rows"]["document_metadata"][0]["valid_until"],"2026-12-31");
        assert_eq!(archive["database_rows"]["background_library"][0]["value"],"{\"quote\":\"synthetic archived source\"}");
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM background_library").fetch_one(&index.pool).await.unwrap(),0);
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM document_metadata").fetch_one(&index.pool).await.unwrap(),0);
        assert_eq!(archive["database_rows"]["documents"].as_array().unwrap().len(),1);
        assert_eq!(archive["database_rows"]["chunks"][0]["text"],"\nsyntheticneedle 契约测试资料");
        assert_eq!(archive["database_rows"]["search"].as_array().unwrap().len(),1);
        assert!(index.search("syntheticneedle","").await.unwrap().is_empty());
        assert_eq!(index.setting("facts").await.unwrap(),"synthetic confirmed fact");assert_eq!(index.setting("aliases").await.unwrap(),"synthetic,alias");
        assert_eq!(index.setting("root").await.unwrap(),root.join("source").canonicalize().unwrap().to_string_lossy());
        assert_eq!(fs::read_to_string(root.join("source/fact.md")).unwrap(),"syntheticneedle 契约测试资料");
        assert_eq!(fs::read_to_string(root.join("app/session.json")).unwrap(),"synthetic conversation must remain");
        let empty=clear_index_with(&index,&root.join("app"),|_|panic!("empty index must not create a Trash item")).await.unwrap();
        assert!(empty.archive_path.is_none());assert_eq!(empty.version,3);
        let rebuilt=index.import(root.join("source").to_str().unwrap(),selection(),"unused").await.unwrap();
        assert_eq!(rebuilt.version,4);assert_eq!(rebuilt.unchanged,0);assert_eq!(index.search("syntheticneedle","").await.unwrap().len(),1);
        assert_eq!(index.documents().await.unwrap()[0].metadata.confirmation,interview_core::Confirmation::Unreviewed);
        assert_eq!(serde_json::to_value(index.fact_history(None).await.unwrap()).unwrap(),facts_before,"index clearing must preserve fact revisions and provenance");
    }
    #[tokio::test]
    async fn orphaned_reviews_are_backed_up_and_cleared_even_without_documents() {
        let (root,index)=fixture().await;
        index.import(root.join("source").to_str().unwrap(),vec![],"unused").await.unwrap();
        assert!(index.documents().await.unwrap().is_empty());
        let report=clear_index_with(&index,&root.join("app"),simulated_move(root.join("synthetic-trash"))).await.unwrap();
        assert_eq!((report.documents,report.chunks,report.metadata_records),(0,0,1));
        assert!(report.archive_path.is_some());
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM document_metadata").fetch_one(&index.pool).await.unwrap(),0);
    }
    #[tokio::test]
    async fn trash_failure_or_unverified_destination_keeps_entire_index() {
        for mode in 0..3 {
            let (root,index)=fixture().await;let before=snapshot(&index).await;let destination=root.join("synthetic-trash");
            let result=clear_index_with(&index,&root.join("app"),move |path| {
                if mode==0 {return Err("synthetic Trash refusal".into());}
                if mode==1 {return Ok(path.to_path_buf());}
                let target=destination.join(path.file_name().unwrap());fs::rename(path,&target).map_err(|e|e.to_string())?;
                fs::write(target.join("index.json"),"synthetic tampering").map_err(|e|e.to_string())?;Ok(target)
            }).await;
            assert!(result.is_err());assert_eq!(snapshot(&index).await,before,"failure mode {mode} cleared live data");
        }
    }
    #[tokio::test]
    async fn database_failure_after_trash_rolls_back_with_backup_retained() {
        let (root,index)=fixture().await;let before=snapshot(&index).await;
        sqlx::raw_sql("CREATE TRIGGER refuse_clear BEFORE DELETE ON documents BEGIN SELECT RAISE(ABORT,'synthetic refusal'); END;").execute(&index.pool).await.unwrap();
        let error=clear_index_with(&index,&root.join("app"),simulated_move(root.join("synthetic-trash"))).await.unwrap_err();
        assert!(error.contains("完整备份"));assert_eq!(snapshot(&index).await,before);
        let archive=fs::read_dir(root.join("synthetic-trash")).unwrap().next().unwrap().unwrap().path().join("index.json");
        let data:serde_json::Value=serde_json::from_slice(&fs::read(archive).unwrap()).unwrap();assert_eq!(data["database_rows"]["documents"].as_array().unwrap().len(),1);
    }
}
