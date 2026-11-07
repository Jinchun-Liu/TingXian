use interview_core::{background::Bundle,preparation::hash,Index,source_policy::SourceExclusion,DocumentUpdate,FactSourceRef,assertions::{AssertionUpdate,Activity}};
use serde_json::{json,Value};
use std::path::{Path,PathBuf};

fn bundle()->Bundle {serde_json::from_value(json!({"format":"meetily-background-v1","profile":{"overview":"虚构背景","role":"","aliases":""},"documents":[{"id":"a".repeat(64),"hash":"b".repeat(64),"name":"合成项目.md","records":[{"kind":"personal","location":"行1","quote":"我只在本地模拟契约测试，没有生产经验。","note":"个人参与范围"},{"kind":"practice","location":"行2","quote":"练习题：分布式事务怎么测试？","note":"不是本人经历"}]}]})).unwrap()}
async fn apply(index:&Index,b:&Bundle) {index.apply_background(b,&index.setting("version").await.unwrap(),false,Value::Null,false).await.unwrap();}
fn repo()->PathBuf {Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")}
fn candidate(scope:&Value,dir:&Path)->Value {
    let input=dir.join("scope.json");std::fs::write(&input,scope.to_string()).unwrap();
    let output=std::process::Command::new(if cfg!(windows){"python"}else{"python3"}).args(["-B","scripts/preparation-roundtrip-fixture.py"]).arg(&input).current_dir(repo()).output().unwrap();
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));serde_json::from_slice(&output.stdout).unwrap()
}
fn rehash(v:&mut Value){v.as_object_mut().unwrap().remove("candidate_hash");v["candidate_hash"]=json!(hash(v));}
async fn preview(index:&Index,c:&Value)->Value {index.preparation_preview(c).await.unwrap()}
async fn save(index:&Index,c:&Value,p:&Value) {index.preparation_save(c,p["index_version"].as_str().unwrap(),p["previous_hash"].as_str()).await.unwrap();}

#[tokio::test]
async fn portable_cli_roundtrip_is_separate_persistent_and_cas_protected(){
    let dir=tempfile::tempdir().unwrap().keep();let path=dir.join("index.sqlite");let index=Index::open(&path).await.unwrap();apply(&index,&bundle()).await;
    let scope=index.preparation_export().await.unwrap();let c=candidate(&scope,&dir);
    assert_eq!(c["execution_mode"],"fixture");let p=preview(&index,&c).await;
    let version=index.setting("version").await.unwrap();let facts=index.fact_card().await.unwrap();let docs=index.documents().await.unwrap().len();
    save(&index,&c,&p).await;
    assert!(index.preparation_save(&c,&version,None).await.is_err(),"stale simultaneous writer replaced the saved view");
    assert_eq!(index.setting("version").await.unwrap(),version);assert_eq!(index.fact_card().await.unwrap().text,facts.text);assert_eq!(index.documents().await.unwrap().len(),docs);
    assert!(index.background_inputs(false).await.unwrap().is_empty(),"local preparation became external model input");
    let reopened=Index::open(&path).await.unwrap();let view=reopened.preparation_view().await.unwrap();assert_eq!(view["candidate"],c);assert_eq!(view["source_binding_valid"],true);assert_eq!(view["personal_facts_confirmed"],false);
    let foreign=Index::open(&dir.join("other.sqlite")).await.unwrap();apply(&foreign,&bundle()).await;foreign.preparation_export().await.unwrap();
    assert!(foreign.preparation_preview(&c).await.is_err(),"identical files bypassed library isolation");
}

#[tokio::test]
async fn manual_revision_is_rebound_without_creating_or_confirming_assertions(){
    let dir=tempfile::tempdir().unwrap().keep();let index=Index::open(&dir.join("index.sqlite")).await.unwrap();apply(&index,&bundle()).await;
    let old=candidate(&index.preparation_export().await.unwrap(),&dir);let p=preview(&index,&old).await;save(&index,&old,&p).await;
    let d=index.documents().await.unwrap().into_iter().find(|d|d.path.ends_with("合成项目.md")).unwrap();let record=index.document_records(&d.path,&d.root,&d.hash).await.unwrap().remove(0);
    let source=FactSourceRef{source_id:record.source_id,root:d.root,path:d.path,hash:d.hash};
    let update=|revision,claim:&str|AssertionUpdate{source:source.clone(),expected_revision:revision,claim:claim.into(),activity:Activity::Planned,reason:"本轮合成纠正".into(),confirmed:true};
    index.revise_record(update(0,"计划补充幂等键测试，尚未完成。" )).await.unwrap();
    assert!(index.preparation_preview(&old).await.is_err());assert_eq!(index.preparation_view().await.unwrap()["source_binding_valid"],false);
    let c=candidate(&index.preparation_export().await.unwrap(),&dir);assert_eq!(c["source_records"][0]["claim_status"],"user_asserted");
    let p=preview(&index,&c).await;save(&index,&c,&p).await;
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM record_assertions").fetch_one(&index.pool).await.unwrap();assert_eq!(count,1);
    let mut forged=c.clone();forged["source_records"][0]["claim"]="已经在生产环境验证".into();rehash(&mut forged);assert!(index.preparation_preview(&forged).await.is_err());
    index.revise_record(update(1,"计划仍未执行；需要再确认。" )).await.unwrap();
    assert!(index.preparation_save(&c,p["index_version"].as_str().unwrap(),p["previous_hash"].as_str()).await.is_err());
    let revised=candidate(&index.preparation_export().await.unwrap(),&dir);preview(&index,&revised).await;
}

#[tokio::test]
async fn source_change_exclusion_expiry_and_clear_invalidate_saved_preparation(){
    let dir=tempfile::tempdir().unwrap().keep();let index=Index::open(&dir.join("index.sqlite")).await.unwrap();let b=bundle();apply(&index,&b).await;
    let c=candidate(&index.preparation_export().await.unwrap(),&dir);let p=preview(&index,&c).await;save(&index,&c,&p).await;
    let d=index.documents().await.unwrap().into_iter().find(|d|d.path.ends_with("合成项目.md")).unwrap();
    let exclusion=|excluded|SourceExclusion{path:d.path.clone(),expected_root:d.root.clone(),expected_hash:d.hash.clone(),excluded};
    index.exclude_source(exclusion(true)).await.unwrap();assert!(index.preparation_preview(&c).await.is_err());assert_eq!(index.preparation_view().await.unwrap()["source_binding_valid"],false);
    index.exclude_source(exclusion(false)).await.unwrap();preview(&index,&c).await;
    index.update_document(DocumentUpdate{path:d.path.clone(),expected_root:d.root.clone(),expected_hash:d.hash.clone(),confirmed:true,valid_from:None,valid_until:Some("2020-01-01".into())}).await.unwrap();
    assert!(index.preparation_preview(&c).await.is_err());
    index.update_document(DocumentUpdate{path:d.path.clone(),expected_root:d.root.clone(),expected_hash:d.hash.clone(),confirmed:false,valid_from:None,valid_until:None}).await.unwrap();
    let mut edited=b.clone();edited.documents[0].records[0].note="不同的提取解释".into();apply(&index,&edited).await;assert!(index.preparation_preview(&c).await.is_err());
    // Emulate the source removal portion of the already tested recoverable clear.
    sqlx::query("DELETE FROM documents").execute(&index.pool).await.unwrap();assert_eq!(index.preparation_view().await.unwrap()["source_binding_valid"],false);
}

#[tokio::test]
async fn candidate_validation_rejects_fact_pollution_tampering_and_missing_scope(){
    let dir=tempfile::tempdir().unwrap().keep();let index=Index::open(&dir.join("index.sqlite")).await.unwrap();apply(&index,&bundle()).await;
    let c=candidate(&index.preparation_export().await.unwrap(),&dir);
    let mut edits=vec![];
    let mut x=c.clone();x["quality_pass"]=true.into();edits.push(x);
    let mut x=c.clone();x["harness2_enabled"]=true.into();edits.push(x);
    let mut x=c.clone();x["source_records"][0]["source_refs"][0]["record_index"]=json!(99);edits.push(x);
    let mut x=c.clone();x["unselected_record_ids"]=json!([]);edits.push(x);
    let mut x=c.clone();x["stories"][0]["activity"]="completed".into();edits.push(x);
    let mut x=c.clone();x["stories"][0]["sections"]["action"][0]["record_ids"]=json!(["f".repeat(64)]);edits.push(x);
    let mut x=c.clone();x["role_view"][0]["matches"]=json!([]);x["role_view"][0]["gaps"]=json!([]);edits.push(x);
    for mut x in edits {rehash(&mut x);assert!(index.preparation_preview(&x).await.is_err());}
    let mut x=c.clone();x["candidate_hash"]="0".repeat(64).into();assert!(index.preparation_preview(&x).await.is_err());
    assert!(index.preparation_view().await.unwrap().is_null());assert!(!index.has_manual_assertions().await.unwrap());
}
