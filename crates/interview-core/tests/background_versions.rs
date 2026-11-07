use interview_core::{background::Bundle,Index,DocumentUpdate,Confirmation};
use serde_json::{json,Value};
fn bundle()->Bundle{serde_json::from_value(json!({"format":"meetily-background-v1","profile":{"overview":"虚构资料，完成状态须核对","role":"","aliases":""},"documents":[{"id":"a".repeat(64),"hash":"b".repeat(64),"name":"旧资料.md","records":[{"kind":"personal","location":"行 1","quote":"Orchid 生产经验：无","note":"仅限本地模拟"}]}]})).unwrap()}
async fn apply(index:&Index,b:&Bundle,share:bool){index.apply_background(b,&index.setting("version").await.unwrap(),share,Value::Null,false).await.unwrap();}
#[tokio::test]
async fn repeat_changes_versions_review_and_model_scope_survive_reopen(){
 let dir=tempfile::tempdir().unwrap().keep();let db=dir.join("index.sqlite");let index=Index::open(&db).await.unwrap();let mut b=bundle();
 let version=index.setting("version").await.unwrap();assert_eq!(index.background_changes(&b,&version).await.unwrap()[0]["state"],"new");
 apply(&index,&b,true).await;
 let original=index.documents().await.unwrap().into_iter().find(|d|d.path.ends_with("旧资料.md")).unwrap();
 index.update_document(DocumentUpdate{path:original.path.clone(),expected_root:original.root.clone(),expected_hash:original.hash.clone(),confirmed:true,valid_from:Some("2020-01-01".into()),valid_until:Some("2020-12-31".into())}).await.unwrap();
 assert!(index.background_inputs(false).await.unwrap().is_empty(),"expired sources are historical-only");
 assert_eq!(index.background_changes(&b,&index.setting("version").await.unwrap()).await.unwrap()[0]["state"],"unchanged");
 apply(&index,&b,true).await;
 assert_eq!(index.documents().await.unwrap().iter().find(|d|d.path==original.path).unwrap().metadata.confirmation,Confirmation::Confirmed);
 let history=index.background_versions(&b.documents[0].id,&original.hash,0).await.unwrap();assert_eq!(history["rows"].as_array().unwrap().len(),1,"retry created an extra revision");
 let mut copy=b.clone();copy.documents[0].id="c".repeat(64);copy.documents[0].name="副本.md".into();
 let changes=index.background_changes(&copy,&index.setting("version").await.unwrap()).await.unwrap();assert_eq!(changes[0]["identical_count"],1);assert_eq!(changes[0]["state"],"new");
 apply(&index,&copy,false).await;
 b.documents[0].hash="d".repeat(64);b.documents[0].records[0].quote="Orchid 生产经验：有".into();
 let revision=index.setting("version").await.unwrap();assert_eq!(index.background_changes(&b,&revision).await.unwrap()[0]["state"],"source_changed");
 assert!(index.background_previous(&b.documents[0].id,&revision,0).await.unwrap()["rows"][0]["quote"].as_str().unwrap().contains('无'));
 apply(&index,&b,true).await;
 assert!(index.background_changes(&b,&revision).await.is_err());
 assert!(index.background_versions(&b.documents[0].id,&original.hash,0).await.is_err());
 let updated=index.documents().await.unwrap().into_iter().find(|d|d.path==original.path).unwrap();assert_eq!(updated.metadata.confirmation,Confirmation::ContentChanged);
 let reopened=Index::open(&db).await.unwrap();let history=reopened.background_versions(&b.documents[0].id,&updated.hash,0).await.unwrap();assert_eq!(history["rows"].as_array().unwrap().len(),2);
 let old=history["rows"].as_array().unwrap().iter().find(|r|r["current"]==false).unwrap()["revision"].as_str().unwrap();
 let records=reopened.background_version_records(&b.documents[0].id,&updated.hash,old,0).await.unwrap();assert!(records["rows"][0]["quote"].as_str().unwrap().contains('无'));assert_eq!(records["current"],false);
 assert!(reopened.background_version_records(&copy.documents[0].id,&updated.hash,old,0).await.is_err());
 assert!(reopened.background_inputs(false).await.unwrap().is_empty(),"changed content requires new review");
 reopened.update_document(DocumentUpdate{path:updated.path.clone(),expected_root:updated.root.clone(),expected_hash:updated.hash.clone(),confirmed:true,valid_from:None,valid_until:None}).await.unwrap();
 let input=reopened.background_inputs(false).await.unwrap();assert_eq!(input.len(),1,"private duplicate or archived version escaped into model context");assert!(input[0]["quote"].as_str().unwrap().contains('有'));assert_eq!(input[0]["source_status"]["confirmation"],"confirmed");
 let mut edited=b.clone();edited.documents[0].records[0].note="本次重新解释，仍需本人核实".into();assert_eq!(reopened.background_changes(&edited,&reopened.setting("version").await.unwrap()).await.unwrap()[0]["state"],"extraction_changed");
 assert_eq!(reopened.fact_card().await.unwrap().version,0,"archive became confirmed personal facts");
}

#[tokio::test]
async fn legacy_import_retains_unknown_time_and_archive_limit_rolls_back(){
 let dir=tempfile::tempdir().unwrap().keep();let index=Index::open(&dir.join("index.sqlite")).await.unwrap();let mut b=bundle();apply(&index,&b,true).await;
 // Emulate an index created before provenance versions existed.
 sqlx::query("DELETE FROM background_library WHERE key GLOB 'revision/*'").execute(&index.pool).await.unwrap();
 b.documents[0].hash="c".repeat(64);b.documents[0].records[0].quote="新版本原文".into();apply(&index,&b,true).await;
 let doc=index.documents().await.unwrap().into_iter().find(|d|d.path.ends_with("旧资料.md")).unwrap();
 let versions=index.background_versions(&b.documents[0].id,&doc.hash,0).await.unwrap();assert!(versions["rows"].as_array().unwrap().iter().any(|r|r["current"]==false&&r["recorded_at"].is_null()));
 let revision=index.setting("version").await.unwrap();
 // SQLite allocates the synthetic bound without a large application buffer.
 sqlx::query("INSERT INTO background_library VALUES('revision/limit-test',zeroblob(67108864))").execute(&index.pool).await.unwrap();
 b.documents[0].records[0].quote="不应写入的新数据".into();
 assert!(index.apply_background(&b,&revision,false,Value::Null,false).await.unwrap_err().contains("64 MiB"));
 assert_eq!(index.setting("version").await.unwrap(),revision);assert_eq!(index.documents().await.unwrap().into_iter().find(|d|d.path==doc.path).unwrap().hash,doc.hash);
}
