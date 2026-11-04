use interview_core::{Index,Selection,background::*};
use serde_json::json;
fn fixture()->Bundle{
    serde_json::from_value(json!({"format":"meetily-background-v1","documents":[{"id":"a".repeat(64),"hash":"b".repeat(64),"name":"模拟面试.md",
        "records":[{"kind":"practice","location":"行 2","quote":"Orchid 只是练习方案","note":"幂等键 retryfixture"}]}],
        "profile":{"overview":"Orchid 的练习不代表实际生产经验","role":"接口测试","aliases":"幂等,重复写入"}})).unwrap()
}
#[tokio::test]
async fn append_review_retrieve_reopen_and_prevent_stale_writes(){
    let dir=tempfile::tempdir().unwrap().keep();let db=dir.join("index.sqlite");let index=Index::open(&db).await.unwrap();
    std::fs::write(dir.join("legacy.md"),"SecretLocalMarker 局部资料").unwrap();
    let source=||vec![Selection{path:"legacy.md".into(),kind:"personal".into(),allow_external:false,structured:None}];
    index.import(dir.to_str().unwrap(),source(),"unused").await.unwrap();
    let b=fixture();let rev=index.setting("version").await.unwrap();
    index.apply_background(&b,&rev,true,json!(null),true).await.unwrap();
    assert!(index.apply_background(&b,&rev,true,json!(null),true).await.is_err());
    let hit=index.search("retryfixture","").await.unwrap().remove(0);
    assert_eq!(hit.text,"Orchid 只是练习方案");assert_eq!(hit.kind,"practice");
    assert_eq!(hit.root,"背景资料库");assert!(index.setting("aliases").await.unwrap().contains("幂等,重复写入"));
    assert!(!index.background_inputs(false).await.unwrap().iter().any(|v|v.to_string().contains("SecretLocalMarker")));
    assert_eq!(index.background_role().await.unwrap().as_deref(),Some("接口测试"));
    let other=dir.join("second");std::fs::create_dir(&other).unwrap();
    index.import(other.to_str().unwrap(),vec![],"unused").await.unwrap();
    let after=index.search("retryfixture","").await.unwrap().remove(0);
    assert_eq!(after.root,hit.root);assert_eq!(index.document_segments(&after.path,&after.root,&after.hash).await.unwrap()[0].text,hit.text);
    let rev=index.setting("version").await.unwrap();let mut changed=b.clone();changed.documents[0].records[0].quote="新版本 testrevision".into();
    index.apply_background(&changed,&rev,false,json!({"calls":1}),false).await.unwrap();
    assert_eq!(index.documents().await.unwrap().len(),2);
    assert!(index.background_inputs(true).await.unwrap().is_empty());assert_eq!(index.background_role().await.unwrap().as_deref(),Some(""));
    assert_eq!(index.fact_card().await.unwrap().version,0);
    let reopened=Index::open(&db).await.unwrap();let view=reopened.background_view().await.unwrap();
    assert_eq!(view["baseline"],262144);assert!(view["rag_budget"].as_u64().unwrap()<65536);assert_eq!(view["state"]["profile"]["overview"],changed.profile.overview);
    let rev=reopened.setting("version").await.unwrap();let mut invalid=changed.clone();invalid.documents[0].name="../outside".into();
    assert!(reopened.apply_background(&invalid,&rev,true,json!(null),true).await.is_err());assert_eq!(reopened.setting("version").await.unwrap(),rev);
}
#[test]
fn pre_background_provider_config_migrates_to_an_independent_disabled_slot(){
    let mut legacy=serde_json::to_value(interview_core::ProviderConfig::initial(Default::default())).unwrap();
    legacy.as_object_mut().unwrap().remove("background");
    let migrated:interview_core::ProviderConfig=serde_json::from_value(legacy).unwrap();
    migrated.validate().unwrap();assert!(!migrated.background.enabled);assert_eq!(migrated.background.id,"background");
}
