use interview_core::{*,background::*};
use serde_json::{json,Value};
async fn fixture()->(std::path::PathBuf,Index) {
    let root=tempfile::tempdir().unwrap().keep();let index=Index::open(&root.join("synthetic.sqlite")).await.unwrap();(root,index)
}
fn bundle(id:char,aliases:&str)->Bundle {
    serde_json::from_value(json!({"format":"meetily-background-v1","documents":[{"id":id.to_string().repeat(64),"hash":"f".repeat(64),"name":format!("fictional-{id}.txt"),"records":[{"kind":"personal","location":"行1","quote":format!("虚构来源 {id} 的测试计划"),"note":"仅模拟"}]}],"profile":{"overview":format!("综合 {id}"),"role":"虚构岗位","aliases":aliases}})).unwrap()
}
async fn apply(index:&Index,b:&Bundle) {index.apply_background(b,&index.setting("version").await.unwrap(),true,Value::Null,true).await.unwrap();}
async fn withdraw(index:&Index,b:&Bundle,excluded:bool) {
    let path=format!(".背景/{}/{}",b.documents[0].id,b.documents[0].name);
    let source=index.documents().await.unwrap().into_iter().find(|d|d.path==path).unwrap();
    index.exclude_source(source_policy::SourceExclusion{path,expected_root:source.root,expected_hash:source.hash,excluded}).await.unwrap();
}
#[tokio::test]
async fn separate_imports_preserve_manual_lines_and_withdraw_only_dependent_aliases() {
    let (root,index)=fixture().await;
    index.set("aliases","  手动,manual \n").await.unwrap();
    let a=bundle('a',"手动,manual\n企业测试,契约测试\n空,nothing");let b=bundle('b',"接口,API\n企业测试,契约测试");
    apply(&index,&a).await;apply(&index,&b).await;
    assert!(index.alias_state().await.unwrap().blocked_lines.is_empty());
    withdraw(&index,&a,true).await;
    let state=index.alias_state().await.unwrap();assert_eq!(state.blocked_lines,vec!["企业测试,契约测试","空,nothing"]);
    assert!(state.aliases.contains("企业测试"));assert_eq!(state.effective(),"  手动,manual \n接口,API");
    let reopened=Index::open(&root.join("synthetic.sqlite")).await.unwrap();assert_eq!(reopened.request_aliases().await.unwrap(),state.effective());
    withdraw(&index,&a,false).await;assert!(index.alias_state().await.unwrap().blocked_lines.is_empty());
    withdraw(&index,&b,true).await;assert_eq!(index.alias_state().await.unwrap().blocked_lines,vec!["接口,API"]);
}
#[tokio::test]
async fn unchanged_reordered_lines_stay_blocked_and_explicit_manual_review_is_atomic() {
    let (_,index)=fixture().await;let b=bundle('a',"企业,契约\n闹,null");apply(&index,&b).await;withdraw(&index,&b,true).await;
    let before=index.setting("aliases").await.unwrap();
    let reordered=" 闹,null \n\n企业,契约\n本人,manual";
    index.set_aliases(reordered,Some(&before)).await.unwrap();
    assert_eq!(index.request_aliases().await.unwrap(),"\n本人,manual");
    assert!(index.confirm_manual_aliases(&["企业,契约".into(),"missing".into()],reordered).await.is_err());
    assert_eq!(index.alias_state().await.unwrap().blocked_lines.len(),2);
    assert!(index.confirm_manual_aliases(&["企业,契约".into()],&before).await.is_err());
    index.confirm_manual_aliases(&["企业,契约".into()],reordered).await.unwrap();
    assert_eq!(index.alias_state().await.unwrap().blocked_lines,vec!["闹,null"]);
    index.set_aliases("本人改写,null\n企业,契约\n本人,manual",Some(reordered)).await.unwrap();
    assert!(index.alias_state().await.unwrap().blocked_lines.is_empty());
    let meta:Value=serde_json::from_str(&index.setting("alias_sources").await.unwrap()).unwrap();assert_eq!(meta["snapshots"],json!({}));
}
#[tokio::test]
async fn stale_save_new_profile_and_index_clear_cannot_recertify_old_lines() {
    let (_,index)=fixture().await;let a=bundle('a',"企业,契约");apply(&index,&a).await;
    let before=index.setting("aliases").await.unwrap();apply(&index,&bundle('b',"幂等,idempotency")).await;
    assert!(index.set_aliases("old editor",Some(&before)).await.is_err());
    withdraw(&index,&a,true).await;
    // Repeated wording from an unrelated new profile cannot rebind old lines.
    apply(&index,&bundle('c',"企业,契约" )).await;
    assert_eq!(index.alias_state().await.unwrap().blocked_lines,vec!["企业,契约"]);
    let mut profile=a.profile.clone();profile.aliases="新词,new".into();
    index.edit_background(profile,&index.setting("version").await.unwrap(),true,Value::Null,true).await.unwrap();
    sqlx::raw_sql("DELETE FROM chunks; DELETE FROM documents; DELETE FROM background_library;").execute(&index.pool).await.unwrap();
    assert!(index.request_aliases().await.unwrap().is_empty());
    assert!(index.setting("aliases").await.unwrap().contains("企业"));
}
#[tokio::test]
async fn expired_review_revised_record_and_changed_extraction_invalidate_old_aliases() {
    let (_,index)=fixture().await;let mut b=bundle('a',"企业,契约");apply(&index,&b).await;
    let doc=index.documents().await.unwrap().into_iter().find(|d|d.path.ends_with("fictional-a.txt")).unwrap();
    index.update_document(DocumentUpdate{path:doc.path.clone(),expected_root:doc.root.clone(),expected_hash:doc.hash.clone(),confirmed:true,valid_from:None,valid_until:Some("2000-01-01".into())}).await.unwrap();
    assert!(index.request_aliases().await.unwrap().is_empty());
    index.update_document(DocumentUpdate{path:doc.path,expected_root:doc.root,expected_hash:doc.hash,confirmed:true,valid_from:None,valid_until:None}).await.unwrap();
    assert!(index.request_aliases().await.unwrap().is_empty());
    b.profile.aliases="原子,atomicity".into();apply(&index,&b).await;
    assert_eq!(index.request_aliases().await.unwrap(),"原子,atomicity");
    let e=index.search("测试计划","").await.unwrap().into_iter().find(|e|e.path.ends_with("fictional-a.txt")).unwrap();
    index.revise_record(assertions::AssertionUpdate{source:FactSourceRef{source_id:e.source_id,root:e.root,path:e.path,hash:e.hash},expected_revision:0,claim:"仅为计划，尚未实施".into(),activity:assertions::Activity::Planned,reason:"本人核对".into(),confirmed:true}).await.unwrap();
    assert!(index.request_aliases().await.unwrap().is_empty());
    b.documents[0].records[0].quote="新版本未完成".into();b.profile.aliases="零,zero".into();apply(&index,&b).await;
    assert!(!index.request_aliases().await.unwrap().contains("atomicity"));
}
#[tokio::test]
async fn alias_failure_rolls_back_background_import_and_metadata_together() {
    let (_,index)=fixture().await;let a=bundle('a',"企业,契约");apply(&index,&a).await;
    let version=index.setting("version").await.unwrap();let meta=index.setting("alias_sources").await.unwrap();let docs=index.documents().await.unwrap().len();
    sqlx::raw_sql("CREATE TRIGGER reject_aliases BEFORE INSERT ON settings WHEN NEW.key='aliases' BEGIN SELECT RAISE(ABORT,'synthetic rejection'); END;").execute(&index.pool).await.unwrap();
    assert!(index.apply_background(&bundle('b',"新词,new"),&version,true,Value::Null,true).await.is_err());
    assert_eq!(index.setting("version").await.unwrap(),version);assert_eq!(index.setting("alias_sources").await.unwrap(),meta);assert_eq!(index.documents().await.unwrap().len(),docs);
}
#[tokio::test]
async fn legacy_aliases_remain_compatible_and_invalid_metadata_fails_closed() {
    let (_,index)=fixture().await;index.set("aliases","旧版,legacy").await.unwrap();assert_eq!(index.request_aliases().await.unwrap(),"旧版,legacy");
    let b=bundle('a',"企业,契约");apply(&index,&b).await;
    let mut meta:Value=serde_json::from_str(&index.setting("alias_sources").await.unwrap()).unwrap();meta["snapshots"]=json!({});
    index.set("alias_sources",&meta.to_string()).await.unwrap();assert_eq!(index.request_aliases().await.unwrap(),"旧版,legacy");
    meta["format"]=json!("future");index.set("alias_sources",&meta.to_string()).await.unwrap();assert!(index.request_aliases().await.is_err());
    index.set("alias_sources",&"x".repeat(8*1024*1024+1)).await.unwrap();assert!(index.request_aliases().await.is_err());
}
#[tokio::test]
async fn withdrawn_alias_stops_expanding_queries_into_an_unrelated_valid_document() {
    let (_,index)=fixture().await;let a=bundle('a',"nicknamefoo,needleuniquex");apply(&index,&a).await;
    let mut b=bundle('b',"");b.documents[0].records[0].quote="needleuniquex".into();apply(&index,&b).await;
    let aliases=index.request_aliases().await.unwrap();
    assert!(index.search("nicknamefoo",&aliases).await.unwrap().iter().any(|e|e.path.ends_with("fictional-b.txt")));
    withdraw(&index,&a,true).await;
    assert!(index.search("nicknamefoo",&index.request_aliases().await.unwrap()).await.unwrap().is_empty());
    assert!(!index.search("needleuniquex","").await.unwrap().is_empty(),"unrelated valid document must remain searchable");
}
#[tokio::test]
async fn snapshot_budget_rejects_new_merge_without_half_publishing_the_import() {
    let (_,index)=fixture().await;let b=bundle('a',"企业,契约");apply(&index,&b).await;
    let mut meta:Value=serde_json::from_str(&index.setting("alias_sources").await.unwrap()).unwrap();
    let key=meta["snapshots"].as_object().unwrap().keys().next().unwrap().clone();
    meta["snapshots"][&key]["synthetic_padding"]=json!("x".repeat(8*1024*1024-meta.to_string().len()-150));
    let raw=meta.to_string();assert!(raw.len()<8*1024*1024);index.set("alias_sources",&raw).await.unwrap();
    let version=index.setting("version").await.unwrap();
    let result=index.apply_background(&bundle('b',"新词,new"),&version,true,Value::Null,true).await;
    assert!(result.unwrap_err().contains("8 MB"));assert_eq!(index.setting("version").await.unwrap(),version);assert_eq!(index.setting("alias_sources").await.unwrap(),raw);
    // Dropping obsolete aliases releases only derived metadata, not source files.
    index.set_aliases("",Some("企业,契约")).await.unwrap();assert!(index.setting("alias_sources").await.unwrap().len()<200);
    assert!(index.documents().await.unwrap().iter().any(|d|d.path.ends_with("fictional-a.txt")));
}
