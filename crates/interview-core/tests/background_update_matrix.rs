//! Sequential source updates across retrieval, settings and local preparation.
//! All records and databases are synthetic; no provider or user files are used.
use interview_core::{background::*, background_settings::{Change,Field}, source_policy::SourceExclusion, *};
use serde_json::{json,Value};
use sqlx::Row;

fn bundle(id:char)->Bundle {
    serde_json::from_value(json!({"format":"meetily-background-v2","documents":[{"id":id.to_string().repeat(64),"hash":"f".repeat(64),"name":format!("source-{id}.md"),"records":[{"kind":"personal","location":"行1","quote":format!("marker{id} 仅做本地模拟，生产实践尚未完成。"),"note":"虚构参与范围"}]}],"profile":{"overview":format!("marker{id} 仅为模拟"),"role":format!("role{id}"),"aliases":format!("alias{id},marker{id}")},"settings":{"role":format!("role{id}"),"context":format!("context{id}"),"terminology":format!("term{id}")}})).unwrap()
}
async fn apply(index:&Index,b:&Bundle) {
    index.apply_background(b,&index.setting("version").await.unwrap(),true,Value::Null,true).await.unwrap();
}
async fn document(index:&Index,b:&Bundle)->DocumentRecord {
    let path=format!(".背景/{}/{}",b.documents[0].id,b.documents[0].name);
    index.documents().await.unwrap().into_iter().find(|d|d.path==path).unwrap()
}
fn exclusion(d:&DocumentRecord,excluded:bool)->SourceExclusion {
    SourceExclusion{path:d.path.clone(),expected_root:d.root.clone(),expected_hash:d.hash.clone(),excluded}
}
async fn select(index:&Index,field:Field,value:&str) {
    let p=index.background_settings_preview().await.unwrap();let before=index.preferences().await.unwrap();
    let expected=match field{Field::Role=>before.role,Field::Context=>before.context,_=>unreachable!()};
    index.apply_background_settings(p["version"].as_str().unwrap(),p["profile_hash"].as_str().unwrap(),&[Change{field,expected,value:value.into()}]).await.unwrap();
}

#[tokio::test]
async fn renamed_withdrawn_source_stays_blocked_across_retrieval_settings_and_preparation() {
    let dir=tempfile::tempdir().unwrap().keep();let db=dir.join("matrix.sqlite");let index=Index::open(&db).await.unwrap();
    let mut a=bundle('a');let b=bundle('b');apply(&index,&a).await;select(&index,Field::Role,"rolea").await;
    apply(&index,&b).await;select(&index,Field::Context,"contextb").await;
    let old=document(&index,&a).await;index.exclude_source(exclusion(&old,true)).await.unwrap();
    a.documents[0].name="改名/资料.md".into();apply(&index,&a).await;
    let renamed=document(&index,&a).await;
    assert!(renamed.excluded,"renaming the same source ID must not reauthorize it");
    assert!(index.search("markera","").await.unwrap().is_empty());
    assert!(!index.background_inputs(false).await.unwrap().iter().any(|v|v.to_string().contains("markera")));
    assert!(!index.preparation_export().await.unwrap().to_string().contains("markera"));
    assert!(index.background_role().await.unwrap().is_none());
    assert!(!index.request_aliases().await.unwrap().contains("aliasa"));
    assert!(index.request_aliases().await.unwrap().contains("aliasb"));
    let preferences=index.request_preferences().await.unwrap();assert!(preferences.role.is_empty());assert_eq!(preferences.context,"contextb");
    assert!(!index.search("markerb","").await.unwrap().is_empty());
    let history=[Turn{question:"追问".into(),confirmed_answer:Some("我说的是未完成".into()),suggested_answer:Some("旧AI建议".into())}];
    let filtered=index.request_history(&history).await.unwrap();assert_eq!(filtered[0].confirmed_answer,history[0].confirmed_answer);assert!(filtered[0].suggested_answer.is_none());
    assert!(index.exclude_source(exclusion(&old,false)).await.is_err(),"old path cannot restore permission");
    let reopened=Index::open(&db).await.unwrap();assert!(document(&reopened,&a).await.excluded);
    reopened.exclude_source(exclusion(&renamed,false)).await.unwrap();
    assert!(!document(&reopened,&a).await.excluded);assert!(!reopened.search("markera","").await.unwrap().is_empty());
    assert_eq!(reopened.fact_card().await.unwrap().version,0);
    assert_eq!(reopened.background_versions(&a.documents[0].id,&renamed.hash,0).await.unwrap()["rows"].as_array().unwrap().len(),2);
}

#[tokio::test]
async fn rename_keeps_review_dates_and_cannot_restore_an_older_path_review() {
    let dir=tempfile::tempdir().unwrap().keep();let index=Index::open(&dir.join("review.sqlite")).await.unwrap();let mut a=bundle('a');apply(&index,&a).await;
    let old=document(&index,&a).await;
    index.update_document(DocumentUpdate{path:old.path.clone(),expected_root:old.root,expected_hash:old.hash,confirmed:true,valid_from:None,valid_until:Some("2000-01-01".into())}).await.unwrap();
    a.documents[0].name="新资料.md".into();apply(&index,&a).await;
    let renamed=document(&index,&a).await;
    assert_eq!(renamed.metadata.valid_until.as_deref(),Some("2000-01-01"),"rename discarded explicit expiry");
    assert_eq!(renamed.metadata.confirmation,Confirmation::ContentChanged);
    assert!(index.search("markera","").await.unwrap().is_empty());
    assert!(index.background_inputs(false).await.unwrap().is_empty());
    index.update_document(DocumentUpdate{path:renamed.path,expected_root:renamed.root,expected_hash:renamed.hash,confirmed:true,valid_from:Some("2090-01-01".into()),valid_until:None}).await.unwrap();
    a.documents[0].name="source-a.md".into();
    let before=index.setting("version").await.unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_import BEFORE UPDATE ON settings WHEN NEW.key='version' BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").execute(&index.pool).await.unwrap();
    assert!(index.apply_background(&a,&before,true,Value::Null,true).await.is_err());
    assert_eq!(index.setting("version").await.unwrap(),before);
    let old_expiry:Option<String>=sqlx::query_scalar("SELECT valid_until FROM document_metadata WHERE path=? AND root='背景资料库'").bind(&old.path).fetch_one(&index.pool).await.unwrap();assert_eq!(old_expiry.as_deref(),Some("2000-01-01"),"failed import partially rewrote historical review");
    assert!(index.documents().await.unwrap().iter().any(|d|d.path.ends_with("新资料.md")));
    sqlx::raw_sql("DROP TRIGGER fail_import;").execute(&index.pool).await.unwrap();apply(&index,&a).await;
    let back=document(&index,&a).await;
    assert_eq!(back.metadata.valid_from.as_deref(),Some("2090-01-01"));assert!(back.metadata.valid_until.is_none(),"old filename must not revive old review");
    assert_eq!(back.metadata.confirmation,Confirmation::ContentChanged);assert!(index.search("markera","").await.unwrap().is_empty());
    index.update_document(DocumentUpdate{path:back.path,expected_root:back.root,expected_hash:back.hash,confirmed:true,valid_from:None,valid_until:None}).await.unwrap();
    assert!(!index.search("markera","").await.unwrap().is_empty());
}

#[tokio::test]
async fn legacy_rename_exclusion_is_atomic_and_does_not_bleed_into_identical_other_sources() {
    let dir=tempfile::tempdir().unwrap().keep();let index=Index::open(&dir.join("legacy.sqlite")).await.unwrap();let mut a=bundle('a');let mut copy=a.clone();copy.documents[0].id="b".repeat(64);copy.documents[0].name="copy.md".into();
    apply(&index,&a).await;let old=document(&index,&a).await;
    // Exact old permission shape, with two historical filename entries.
    for (key,path) in [("excluded/legacy-one",old.path.clone()),("excluded/legacy-two",format!(".背景/{}/先前名称.md",a.documents[0].id))] {
        sqlx::query("INSERT INTO background_library VALUES(?,?)").bind(key).bind(json!({"root":"背景资料库","path":path,"hash_at_change":old.hash,"excluded":true,"updated_at":"2026-01-01"}).to_string()).execute(&index.pool).await.unwrap();
    }
    a.documents[0].name="renamed.md".into();apply(&index,&a).await;apply(&index,&copy).await;
    let renamed=document(&index,&a).await;let independent=document(&index,&copy).await;
    assert!(renamed.excluded);assert!(!independent.excluded);
    let results=index.search("markera","").await.unwrap();assert!(results.iter().any(|e|e.path==independent.path));assert!(!results.iter().any(|e|e.path==renamed.path));
    let version=index.setting("version").await.unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_version BEFORE UPDATE ON settings WHEN NEW.key='version' BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").execute(&index.pool).await.unwrap();
    assert!(index.exclude_source(exclusion(&renamed,false)).await.is_err());
    assert_eq!(index.setting("version").await.unwrap(),version);assert!(document(&index,&a).await.excluded);
    sqlx::raw_sql("DROP TRIGGER fail_version;").execute(&index.pool).await.unwrap();
    index.exclude_source(exclusion(&renamed,false)).await.unwrap();assert!(!document(&index,&a).await.excluded);
    index.exclude_source(exclusion(&independent,true)).await.unwrap();assert!(!document(&index,&a).await.excluded);assert!(document(&index,&copy).await.excluded);
    assert_eq!(index.fact_card().await.unwrap().version,0,"duplicate text must not confirm personal facts");
}

#[tokio::test]
async fn orphaned_legacy_review_requires_fresh_confirmation_when_name_returns() {
    let dir=tempfile::tempdir().unwrap().keep();let index=Index::open(&dir.join("orphaned.sqlite")).await.unwrap();let mut a=bundle('a');apply(&index,&a).await;
    let old=document(&index,&a).await;
    index.update_document(DocumentUpdate{path:old.path,expected_root:old.root,expected_hash:old.hash,confirmed:true,valid_from:None,valid_until:Some("2090-01-01".into())}).await.unwrap();
    a.documents[0].name="intermediate.md".into();apply(&index,&a).await;
    // Simulate the old build which lost the current-path review on rename.
    sqlx::query("DELETE FROM document_metadata WHERE path=?").bind(document(&index,&a).await.path).execute(&index.pool).await.unwrap();
    a.documents[0].name="source-a.md".into();apply(&index,&a).await;
    let returned=document(&index,&a).await;assert_eq!(returned.metadata.confirmation,Confirmation::ContentChanged);assert_eq!(returned.metadata.valid_until.as_deref(),Some("2090-01-01"));assert!(returned.metadata.confirmed_at.is_none());
    assert!(index.search("markera","").await.unwrap().is_empty());
}

#[tokio::test]
async fn legacy_permission_index_upgrade_preserves_records_and_uses_indexed_lookup() {
    let dir=tempfile::tempdir().unwrap().keep();let db=dir.join("index-upgrade.sqlite");let index=Index::open(&db).await.unwrap();let a=bundle('a');apply(&index,&a).await;
    let doc=document(&index,&a).await;index.exclude_source(exclusion(&doc,true)).await.unwrap();
    sqlx::query("DROP INDEX background_exclusion_identity").execute(&index.pool).await.unwrap();
    let records:Vec<(String,String)>=sqlx::query_as("SELECT key,value FROM background_library ORDER BY key").fetch_all(&index.pool).await.unwrap();let version=index.setting("version").await.unwrap();index.pool.close().await;
    let reopened=Index::open(&db).await.unwrap();assert!(document(&reopened,&a).await.excluded);
    assert_eq!(sqlx::query_as::<_,(String,String)>("SELECT key,value FROM background_library ORDER BY key").fetch_all(&reopened.pool).await.unwrap(),records);assert_eq!(reopened.setting("version").await.unwrap(),version);
    let plan=sqlx::query(&format!("EXPLAIN QUERY PLAN SELECT d.path FROM documents d WHERE NOT {}",source_policy::EXCLUDED_SQL)).fetch_all(&reopened.pool).await.unwrap();
    assert!(plan.iter().any(|r|{let detail:String=r.get("detail");detail.contains("SEARCH x USING INDEX background_exclusion_identity")}),"permission lookup fell back to scanning all rules");
}

fn candidate(scope:&Value,dir:&std::path::Path)->Value {
    let input=dir.join("scope.json");std::fs::write(&input,scope.to_string()).unwrap();
    let output=std::process::Command::new(if cfg!(windows){"python"}else{"python3"})
        .args(["-B","scripts/preparation-roundtrip-fixture.py"]).arg(input)
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).output().unwrap();
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));serde_json::from_slice(&output.stdout).unwrap()
}
async fn save_candidate(index:&Index,candidate:&Value)->Value {
    let p=index.preparation_preview(candidate).await.unwrap();
    index.preparation_save(candidate,p["index_version"].as_str().unwrap(),p["previous_hash"].as_str()).await.unwrap();
    index.preparation_view().await.unwrap()
}
#[tokio::test]
async fn multi_source_revision_withdrawal_and_rename_keep_preparation_history_read_only() {
    let dir=tempfile::tempdir().unwrap().keep();let db=dir.join("prepared.sqlite");let index=Index::open(&db).await.unwrap();let a=bundle('a');let mut b=bundle('b');apply(&index,&a).await;apply(&index,&b).await;
    let initial=candidate(&index.preparation_export().await.unwrap(),&dir);let first=save_candidate(&index,&initial).await;
    let d=document(&index,&a).await;let e=index.document_records(&d.path,&d.root,&d.hash).await.unwrap().remove(0);
    let reference=FactSourceRef{source_id:e.source_id,root:d.root,path:d.path,hash:d.hash};
    index.revise_record(assertions::AssertionUpdate{source:reference.clone(),expected_revision:0,claim:"revisedmarker 只提供建议，测试仍在计划中。".into(),activity:assertions::Activity::Planned,reason:"本人修订贡献范围".into(),confirmed:true}).await.unwrap();
    assert_eq!(index.preparation_view().await.unwrap()["source_binding_valid"],false);
    let revised=candidate(&index.preparation_export().await.unwrap(),&dir);assert!(revised["source_records"].as_array().unwrap().iter().any(|r|r["claim_status"]=="user_asserted"&&r["activity_status"]=="planned"));
    let second=save_candidate(&index,&revised).await;
    let old_b=document(&index,&b).await;index.exclude_source(exclusion(&old_b,true)).await.unwrap();b.documents[0].name="renamed-b.md".into();apply(&index,&b).await;
    let fresh=candidate(&index.preparation_export().await.unwrap(),&dir);
    assert!(fresh.to_string().contains("revisedmarker"));assert!(!fresh.to_string().contains("markerb"));
    let third=save_candidate(&index,&fresh).await;let version=index.setting("version").await.unwrap();
    for saved in [&first,&second] {
        let id=saved["id"].as_str().unwrap();let old=index.preparation_version_view(id).await.unwrap();assert_eq!(old["source_binding_valid"],false);
        assert!(index.preparation_restore(id,&version,third["id"].as_str()).await.is_err());
        assert_eq!(index.preparation_version_export(id).await.unwrap(),saved["candidate"]);
    }
    let reopened=Index::open(&db).await.unwrap();assert_eq!(reopened.preparation_view().await.unwrap()["candidate"],fresh);assert_eq!(reopened.preparation_history(None).await.unwrap()["rows"].as_array().unwrap().len(),3);
    assert_eq!(reopened.assertion_history(reference,None).await.unwrap().versions.len(),1);assert_eq!(reopened.fact_card().await.unwrap().version,0);
    assert!(reopened.search("markerb","").await.unwrap().is_empty());assert!(!reopened.search("revisedmarker","").await.unwrap().is_empty());
}
