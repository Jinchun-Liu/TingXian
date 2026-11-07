use interview_core::{*,background::*};
use serde_json::{json,Value};
fn bundle(letter:char,quote:&str)->Bundle {Bundle {format:"meetily-background-v2".into(),documents:vec![Doc{id:letter.to_string().repeat(64),name:format!("fictional-{letter}.txt"),hash:"f".repeat(64),records:vec![Record{kind:"personal".into(),location:"行1".into(),quote:quote.into(),note:"虚构测试".into()}]}],profile:Profile{overview:format!("综合概述：{quote}"),role:format!("fictional-role-{letter}"),aliases:String::new()},settings:Some(background_settings::Suggestions{role:format!("fictional-role-{letter}"),..Default::default()})}}
async fn fixture()->(std::path::PathBuf,Index){let dir=tempfile::tempdir().unwrap().keep();let index=Index::open(&dir.join("test.sqlite")).await.unwrap();(dir,index)}
async fn apply(index:&Index,b:&Bundle){index.apply_background(b,&index.setting("version").await.unwrap(),true,Value::Null,false).await.unwrap();}
async fn evidence(index:&Index,b:&Bundle)->Evidence {index.search(&b.documents[0].records[0].quote,"").await.unwrap().into_iter().find(|e|e.path.ends_with(&b.documents[0].name)).unwrap()}
fn source(e:&Evidence)->FactSourceRef{FactSourceRef{source_id:e.source_id.clone(),root:e.root.clone(),path:e.path.clone(),hash:e.hash.clone()}}
async fn exclude(index:&Index,e:&Evidence,value:bool){index.exclude_source(source_policy::SourceExclusion{path:e.path.clone(),expected_root:e.root.clone(),expected_hash:e.hash.clone(),excluded:value}).await.unwrap();}
fn edit(e:&Evidence,revision:i64,claim:&str)->assertions::AssertionUpdate{assertions::AssertionUpdate{source:source(e),expected_revision:revision,claim:claim.into(),activity:assertions::Activity::Planned,reason:"本人核对，仍为计划".into(),confirmed:true}}
#[tokio::test]
async fn unrelated_withdrawal_preserves_new_profile_but_its_actual_source_revokes_it(){
    let (_,index)=fixture().await;let a=bundle('a',"北斗模拟项目");let b=bundle('b',"天琴测试计划");apply(&index,&a).await;let ea=evidence(&index,&a).await;apply(&index,&b).await;let eb=evidence(&index,&b).await;
    exclude(&index,&ea,true).await;assert!(index.profile_available().await.unwrap());assert_eq!(index.background_role().await.unwrap(),Some(b.profile.role.clone()));
    assert!(!index.search("天琴","").await.unwrap().is_empty());
    exclude(&index,&eb,true).await;assert!(!index.profile_available().await.unwrap());assert!(index.background_role().await.unwrap().is_none());assert!(index.search("天琴","").await.unwrap().is_empty());assert!(index.background_settings_preview().await.is_err());
    exclude(&index,&eb,false).await;assert!(index.profile_available().await.unwrap());
    // Old installs have no observed manifest and retain conservative fallback.
    sqlx::query("DELETE FROM background_library WHERE key='profile_dependencies'").execute(&index.pool).await.unwrap();assert!(!index.profile_available().await.unwrap());
}
#[tokio::test]
async fn manual_revision_invalidates_profile_and_regeneration_binds_exact_observed_claims(){
    let (dir,index)=fixture().await;let mut b=bundle('a',"虚构项目已完成");apply(&index,&b).await;let e=evidence(&index,&b).await;
    index.revise_record(edit(&e,0,"虚构项目尚未完成，仅为计划")).await.unwrap();
    assert!(index.background_role().await.unwrap().is_none());assert!(!index.profile_available().await.unwrap());
    let (inputs,deps)=index.background_inputs_observed().await.unwrap();assert_eq!(deps.len(),1);assert_eq!(inputs[0]["dependency_id"],deps[0].id());assert_eq!(inputs[0]["origin"],"user_asserted");
    assert!(!inputs[0].to_string().contains(dir.to_str().unwrap()),"local root must not enter background prompt");
    assert!(index.background_inputs(true).await.unwrap().iter().all(|row|row.get("dependency_id").is_none()),"controller-only trace must not enter token-measurement prompts");
    b.profile.overview="虚构项目尚未完成，仅为计划".into();
    index.apply_background_observed(&b,&index.setting("version").await.unwrap(),true,json!({}),false,&deps).await.unwrap();assert!(index.profile_available().await.unwrap());
    index.revise_record(edit(&e,1,"虚构项目计划延期，未实施")).await.unwrap();assert!(!index.profile_available().await.unwrap());
    index.edit_background(b.profile.clone(),&index.setting("version").await.unwrap(),true,json!({}),false).await.unwrap();assert!(!index.profile_available().await.unwrap(),"editing summary wording must not rebind stale sources");
    let fresh=index.background_dependencies().await.unwrap();b.profile.overview="虚构项目计划延期，未实施".into();
    index.apply_background_observed(&b,&index.setting("version").await.unwrap(),true,json!({}),false,&fresh).await.unwrap();assert!(index.profile_available().await.unwrap());
    let reopened=Index::open(&dir.join("test.sqlite")).await.unwrap();assert!(reopened.profile_available().await.unwrap());
}
#[tokio::test]
async fn expiry_and_updated_source_block_summary_role_and_settings_until_reprocessed(){
    let (_,index)=fixture().await;let mut b=bundle('a',"猎户资料当前版本");apply(&index,&b).await;let e=evidence(&index,&b).await;
    let preview=index.background_settings_preview().await.unwrap();
    index.update_document(DocumentUpdate{path:e.path.clone(),expected_root:e.root.clone(),expected_hash:e.hash.clone(),confirmed:true,valid_from:None,valid_until:Some("2000-01-01".into())}).await.unwrap();
    assert!(index.background_role().await.unwrap().is_none());assert!(index.search("猎户","").await.unwrap().is_empty());
    assert!(index.apply_background_settings(&index.setting("version").await.unwrap(),preview["profile_hash"].as_str().unwrap(),&[]).await.is_err());
    assert!(!index.document_segments(&e.path,&e.root,&e.hash).await.unwrap().is_empty());
    b.documents[0].hash="b".repeat(64);b.documents[0].records[0].quote="猎户资料新版本".into();apply(&index,&b).await;assert!(!index.profile_available().await.unwrap());
    let doc=index.documents().await.unwrap().into_iter().find(|d|d.path==e.path).unwrap();
    index.update_document(DocumentUpdate{path:doc.path,expected_root:doc.root,expected_hash:doc.hash,confirmed:true,valid_from:None,valid_until:None}).await.unwrap();assert!(!index.profile_available().await.unwrap());apply(&index,&b).await;assert!(index.profile_available().await.unwrap());
    // Re-serialization by a host with different JSON map ordering is not a source change.
    sqlx::query("UPDATE background_library SET value=json_set(value,'$.sources[0].review',json_object('reviewed_hash',json_extract(value,'$.sources[0].review.reviewed_hash'),'valid_until',json_extract(value,'$.sources[0].review.valid_until'),'confirmed',json_extract(value,'$.sources[0].review.confirmed'),'valid_from',json_extract(value,'$.sources[0].review.valid_from'),'confirmed_at',json_extract(value,'$.sources[0].review.confirmed_at'))) WHERE key='profile_dependencies'").execute(&index.pool).await.unwrap();
    assert!(index.profile_available().await.unwrap(),"equivalent reordered review remains current");
}
#[tokio::test]
async fn imported_raw_bundle_cannot_claim_to_have_incorporated_existing_manual_revision(){
    let (_,index)=fixture().await;let b=bundle('a',"虚构原始项目描述");apply(&index,&b).await;let e=evidence(&index,&b).await;
    index.revise_record(edit(&e,0,"本人声明：仅准备项目")).await.unwrap();apply(&index,&b).await;
    assert!(!index.profile_available().await.unwrap());assert!(index.background_role().await.unwrap().is_none());
    let mut changed=b.clone();changed.documents[0].records[0].note="重新解释同一来源".into();
    let observed=index.background_dependencies().await.unwrap();
    index.apply_background_observed(&changed,&index.setting("version").await.unwrap(),true,json!({}),false,&observed).await.unwrap();
    assert!(!index.profile_available().await.unwrap(),"changed extraction cannot silently rebind an old manual assertion");
}
