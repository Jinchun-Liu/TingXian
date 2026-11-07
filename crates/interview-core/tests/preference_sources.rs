use interview_core::{*,background::*,background_settings::{Change,Field},preference_sources::ManualSetting};
use serde_json::{json,Value};

async fn fixture()->(std::path::PathBuf,Index) {
    let root=tempfile::tempdir().unwrap().keep();let index=Index::open(&root.join("synthetic.sqlite")).await.unwrap();(root,index)
}
fn bundle(id:char)->Bundle {
    serde_json::from_value(json!({"format":"meetily-background-v2","documents":[{"id":id.to_string().repeat(64),"hash":"f".repeat(64),"name":format!("fictional-{id}.txt"),"records":[{"kind":"personal","location":"行1","quote":format!("虚构来源 {id} 的测试计划"),"note":"仅模拟"}]}],"profile":{"overview":format!("综合 {id}"),"role":"虚构岗位","aliases":""},"settings":{"role":"建议岗位","context":"建议背景","terminology":"建议术语"}})).unwrap()
}
async fn apply(index:&Index,b:&Bundle) {index.apply_background(b,&index.setting("version").await.unwrap(),true,Value::Null,false).await.unwrap();}
async fn choose(index:&Index,fields:&[(Field,&str)]) {
    let preview=index.background_settings_preview().await.unwrap();
    let saved=index.preferences().await.unwrap();
    let changes:Vec<_>=fields.iter().map(|(field,text)|Change {field:*field,expected:match field {Field::Role=>saved.role.clone(),Field::Organization=>saved.organization.clone(),Field::Context=>saved.context.clone(),Field::Terminology=>saved.terminology.clone()},value:text.to_string()}).collect();
    index.apply_background_settings(preview["version"].as_str().unwrap(),preview["profile_hash"].as_str().unwrap(),&changes).await.unwrap();
}
async fn withdraw(index:&Index,b:&Bundle,value:bool) {
    let path=format!(".背景/{}/{}",b.documents[0].id,b.documents[0].name);
    let source=index.documents().await.unwrap().into_iter().find(|d|d.path==path).unwrap();
    index.exclude_source(source_policy::SourceExclusion{path,expected_root:source.root,expected_hash:source.hash,excluded:value}).await.unwrap();
}

#[tokio::test]
async fn withdrawn_applied_values_are_hidden_from_requests_but_retained_for_editing() {
    let (_,index)=fixture().await;let b=bundle('a');
    index.set_preferences(&InterviewPreferences {organization:"手填组织".into(),..Default::default()}).await.unwrap();apply(&index,&b).await;
    choose(&index,&[(Field::Context,"WITHDRAWN_CONTEXT"),(Field::Terminology,"WITHDRAWN_TERM")]).await;
    assert_eq!(index.request_preferences().await.unwrap().context,"WITHDRAWN_CONTEXT");
    withdraw(&index,&b,true).await;
    let state=index.preference_state().await.unwrap();assert_eq!(state.blocked_fields,vec![Field::Context,Field::Terminology]);
    assert_eq!(state.preferences.context,"WITHDRAWN_CONTEXT");
    let effective=state.effective();assert!(effective.context.is_empty()&&effective.terminology.is_empty());assert_eq!(effective.organization,"手填组织");
    let messages=messages_with_preferences("讨论测试方法",&index.fact_card().await.unwrap(),&[],&[],&[],&effective).unwrap();
    assert!(!messages.to_string().contains("WITHDRAWN_"));assert!(messages.to_string().contains("手填组织"));
    withdraw(&index,&b,false).await;assert_eq!(index.request_preferences().await.unwrap().context,"WITHDRAWN_CONTEXT");
}
#[tokio::test]
async fn individual_fields_keep_their_own_snapshot_when_a_new_profile_is_applied() {
    let (_,index)=fixture().await;let a=bundle('a');let b=bundle('b');
    apply(&index,&a).await;choose(&index,&[(Field::Role,"角色来自 A")]).await;
    apply(&index,&b).await;choose(&index,&[(Field::Context,"背景来自 B")]).await;
    assert_eq!(index.request_preferences().await.unwrap().role,"角色来自 A");
    withdraw(&index,&a,true).await;let effective=index.request_preferences().await.unwrap();
    assert!(effective.role.is_empty());assert_eq!(effective.context,"背景来自 B");
    assert!(index.profile_available().await.unwrap(),"unrelated old field must not invalidate new background");
    withdraw(&index,&b,true).await;assert!(index.request_preferences().await.unwrap().context.is_empty());
}
#[tokio::test]
async fn unrelated_saves_do_not_detach_stale_values_and_explicit_manual_actions_are_atomic() {
    let (root,index)=fixture().await;let b=bundle('a');apply(&index,&b).await;
    choose(&index,&[(Field::Context,"旧背景"),(Field::Role,"旧岗位")]).await;withdraw(&index,&b,true).await;
    let mut prefs=index.preferences().await.unwrap();prefs.answer_language="en".into();index.set_preferences(&prefs).await.unwrap();
    assert!(index.request_preferences().await.unwrap().context.is_empty());
    prefs.role="本人改写岗位".into();index.set_preferences(&prefs).await.unwrap();
    let current=index.request_preferences().await.unwrap();assert_eq!(current.role,prefs.role);assert!(current.context.is_empty());
    assert!(index.confirm_manual_settings(&[ManualSetting{field:Field::Context,expected:"旧背景".into()},ManualSetting{field:Field::Role,expected:"过时岗位".into()}]).await.is_err());
    assert!(index.request_preferences().await.unwrap().context.is_empty(),"failed batch detached one field");
    index.confirm_manual_settings(&[ManualSetting{field:Field::Context,expected:"旧背景".into()}]).await.unwrap();
    let reopened=Index::open(&root.join("synthetic.sqlite")).await.unwrap();
    assert_eq!(reopened.request_preferences().await.unwrap().context,"旧背景");
    let meta:Value=serde_json::from_str(&index.setting("interview_preference_sources").await.unwrap()).unwrap();assert_eq!(meta["snapshots"],json!({}));
}
#[tokio::test]
async fn expiry_review_changes_and_index_clear_cannot_revive_applied_suggestions() {
    let (_,index)=fixture().await;let b=bundle('a');apply(&index,&b).await;choose(&index,&[(Field::Context,"旧来源摘要")]).await;
    let doc=index.documents().await.unwrap().into_iter().find(|d|d.path.ends_with("fictional-a.txt")).unwrap();
    index.update_document(DocumentUpdate{path:doc.path.clone(),expected_root:doc.root.clone(),expected_hash:doc.hash.clone(),confirmed:true,valid_from:None,valid_until:Some("2000-01-01".into())}).await.unwrap();
    assert!(index.request_preferences().await.unwrap().context.is_empty());
    index.update_document(DocumentUpdate{path:doc.path,expected_root:doc.root,expected_hash:doc.hash,confirmed:true,valid_from:None,valid_until:None}).await.unwrap();
    assert!(index.request_preferences().await.unwrap().context.is_empty(),"re-review must not certify old generated text");
    apply(&index,&b).await;assert!(index.request_preferences().await.unwrap().context.is_empty(),"new profile must not silently rebind already applied fields");
    choose(&index,&[(Field::Context,"核对后的来源摘要")]).await;assert!(!index.request_preferences().await.unwrap().context.is_empty());
    sqlx::raw_sql("DELETE FROM chunks; DELETE FROM documents; DELETE FROM background_library;").execute(&index.pool).await.unwrap();
    assert_eq!(index.preferences().await.unwrap().context,"核对后的来源摘要");assert!(index.request_preferences().await.unwrap().context.is_empty());
    let mut cleared=index.preferences().await.unwrap();cleared.context.clear();index.set_preferences(&cleared).await.unwrap();
    assert!(index.preference_state().await.unwrap().blocked_fields.is_empty(),"an empty field should not keep a stale source binding");
}
#[tokio::test]
async fn stale_editor_cannot_overwrite_newly_applied_background_fields() {
    let (_,index)=fixture().await;let b=bundle('a');apply(&index,&b).await;
    let before=index.preferences().await.unwrap();choose(&index,&[(Field::Role,"新应用的岗位")]).await;
    let mut stale=before.clone();stale.answer_language="en".into();
    assert!(index.set_preferences_expected(&stale,Some(&before)).await.is_err());
    assert_eq!(index.preferences().await.unwrap().role,"新应用的岗位");
    withdraw(&index,&b,true).await;assert!(index.request_preferences().await.unwrap().role.is_empty(),"stale save detached source ownership");
}
#[tokio::test]
async fn setting_write_failure_rolls_back_provenance_and_preferences_together() {
    let (_,index)=fixture().await;let b=bundle('a');apply(&index,&b).await;
    let preview=index.background_settings_preview().await.unwrap();
    sqlx::raw_sql("CREATE TRIGGER reject_preferences BEFORE INSERT ON settings WHEN NEW.key='interview_preferences' BEGIN SELECT RAISE(ABORT,'synthetic rejection'); END;").execute(&index.pool).await.unwrap();
    let change=Change {field:Field::Context,expected:String::new(),value:"must roll back".into()};
    assert!(index.apply_background_settings(preview["version"].as_str().unwrap(),preview["profile_hash"].as_str().unwrap(),&[change]).await.is_err());
    assert!(index.setting("interview_preference_sources").await.unwrap().is_empty());assert!(index.preferences().await.unwrap().context.is_empty());
}
#[tokio::test]
async fn legacy_settings_stay_compatible_but_new_legacy_bindings_fail_closed_on_library_change() {
    let (_,index)=fixture().await;
    index.set("interview_preferences",r#"{"context":"旧版手填值"}"#).await.unwrap();assert_eq!(index.request_preferences().await.unwrap().context,"旧版手填值");
    let b=bundle('a');apply(&index,&b).await;
    sqlx::query("DELETE FROM background_library WHERE key='profile_dependencies'").execute(&index.pool).await.unwrap();
    choose(&index,&[(Field::Role,"旧版背景建议")]).await;assert_eq!(index.request_preferences().await.unwrap().role,"旧版背景建议");
    index.set("version","500").await.unwrap();let effective=index.request_preferences().await.unwrap();assert!(effective.role.is_empty());assert_eq!(effective.context,"旧版手填值");
    let mut raw:Value=serde_json::from_str(&index.setting("interview_preference_sources").await.unwrap()).unwrap();raw["snapshots"]=json!({});
    index.set("interview_preference_sources",&raw.to_string()).await.unwrap();assert!(index.request_preferences().await.unwrap().role.is_empty());
    raw["format"]=json!("unsupported-future");index.set("interview_preference_sources",&raw.to_string()).await.unwrap();assert!(index.request_preferences().await.is_err());
}
