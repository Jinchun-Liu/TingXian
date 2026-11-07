use interview_core::{*,assertions::*};
use serde_json::Value;
async fn fixture()->(std::path::PathBuf,Index,Evidence) {
    let dir=tempfile::tempdir().unwrap().keep();std::fs::create_dir(dir.join("sources")).unwrap();
    std::fs::write(dir.join("sources/fact.md"),"虚构猎户项目已经完成独立部署").unwrap();
    let index=Index::open(&dir.join("index.sqlite")).await.unwrap();
    index.import(dir.join("sources").to_str().unwrap(),selection(),"unused").await.unwrap();
    let evidence=index.search("猎户","").await.unwrap().remove(0);(dir,index,evidence)
}
fn selection()->Vec<Selection>{vec![Selection{path:"fact.md".into(),kind:"personal".into(),allow_external:true,structured:None}]}
fn reference(e:&Evidence)->FactSourceRef{FactSourceRef {source_id:e.source_id.clone(),root:e.root.clone(),path:e.path.clone(),hash:e.hash.clone()}}
fn update(e:&Evidence,revision:i64,claim:&str)->AssertionUpdate{AssertionUpdate{source:reference(e),expected_revision:revision,claim:claim.into(),activity:Activity::Planned,reason:"本人核对，原材料将计划写成完成".into(),confirmed:true}}
#[tokio::test]
async fn latest_assertion_is_retrievable_original_is_immutable_and_history_survives_reopen(){
    let (dir,index,e)=fixture().await;
    let first=index.revise_record(update(&e,0,"虚构天琴项目计划进行容器验证，尚未完成")).await.unwrap();
    assert_eq!(first.assertion.origin,"user_asserted");assert_eq!(first.source.text,e.text);assert_eq!(first.source.reference.hash,e.hash);
    assert!(index.search("猎户","").await.unwrap().is_empty(),"superseded original terms must not rank current facts");
    let result=index.search("天琴","").await.unwrap().remove(0);
    assert_eq!(result.text,e.text);assert_eq!(result.current_text(),first.assertion.claim);
    let messages=messages("项目完成了吗","",&[],&[result]).unwrap();
    let body:Value=serde_json::from_str(messages[1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(body["evidence"][0]["text"],first.assertion.claim);
    assert_eq!(body["evidence"][0]["user_assertion"]["activity"],"planned");
    assert!(!messages.to_string().contains("已经完成独立部署"));
    assert!(messages[0]["content"].as_str().unwrap().contains(ASSERTION_RULES));
    assert!(!interview_core::messages("普通问题","",&[],&[]).unwrap()[0]["content"].as_str().unwrap().contains(ASSERTION_RULES));
    assert!(index.revise_record(update(&e,0,"陈旧编辑不得覆盖")).await.is_err());
    index.revise_record(update(&e,1,"虚构天鹅项目仍在计划阶段")).await.unwrap();
    assert!(index.search("天琴","").await.unwrap().is_empty());
    assert_eq!(std::fs::read_to_string(dir.join("sources/fact.md")).unwrap(),"虚构猎户项目已经完成独立部署");
    let raw: String=sqlx::query_scalar("SELECT text FROM chunks WHERE id=?").bind(&e.source_id).fetch_one(&index.pool).await.unwrap();assert_eq!(raw,e.text);
    index.pool.close().await;let index=Index::open(&dir.join("index.sqlite")).await.unwrap();
    let history=index.assertion_history(reference(&e),None).await.unwrap();assert_eq!(history.versions.len(),2);assert_eq!(history.versions[1].source.text,e.text);
    assert_eq!(index.search("天鹅","").await.unwrap()[0].user_assertion.as_ref().unwrap().revision,2);
    index.import(dir.join("sources").to_str().unwrap(),selection(),"unused").await.unwrap();
    assert!(!index.search("天鹅","").await.unwrap().is_empty(),"unchanged reimport retains current assertion");
    sqlx::query("DELETE FROM assertion_search").execute(&index.pool).await.unwrap();
    assert!(index.search("天鹅","").await.unwrap().is_empty());
    index.rebuild_assertion_search().await.unwrap();
    assert_eq!(index.search("天鹅","").await.unwrap()[0].user_assertion.as_ref().unwrap().revision,2);
    assert_eq!(index.assertion_history(reference(&e),None).await.unwrap().versions.len(),2);
}
#[tokio::test]
async fn source_update_library_scope_and_withdrawal_invalidate_without_erasing_manual_history(){
    let (dir,index,e)=fixture().await;index.revise_record(update(&e,0,"修订白鲸材料")).await.unwrap();
    index.exclude_source(source_policy::SourceExclusion{path:e.path.clone(),expected_root:e.root.clone(),expected_hash:e.hash.clone(),excluded:true}).await.unwrap();
    assert!(index.search("白鲸","").await.unwrap().is_empty());assert!(index.background_inputs(false).await.unwrap().is_empty());
    assert!(index.revise_record(update(&e,1,"不能修改已撤回材料")).await.is_err());
    std::fs::create_dir(dir.join("another")).unwrap();std::fs::copy(dir.join("sources/fact.md"),dir.join("another/fact.md")).unwrap();
    index.import(dir.join("another").to_str().unwrap(),selection(),"unused").await.unwrap();
    assert!(index.search("白鲸","").await.unwrap().is_empty());assert!(!index.search("猎户","").await.unwrap().is_empty());
    assert!(index.revise_record(update(&e,1,"不能跨资料库修改")).await.is_err());
    std::fs::write(dir.join("sources/fact.md"),"全新来源北斗内容").unwrap();
    index.import(dir.join("sources").to_str().unwrap(),selection(),"unused").await.unwrap();
    assert!(index.search("白鲸","").await.unwrap().is_empty());assert!(index.revise_record(update(&e,1,"陈旧来源")).await.is_err());
    assert_eq!(index.assertion_history(reference(&e),None).await.unwrap().versions.len(),1);
    let history=index.document_assertion_history(&e.path,&e.root,0).await.unwrap();assert_eq!(history.versions.len(),1);assert_eq!(history.versions[0].source.reference.hash,e.hash);
    assert!(index.document_assertion_history(&e.path,"/unrelated-library",0).await.unwrap().versions.is_empty());
}
#[tokio::test]
async fn copied_fact_card_requires_reconfirmation_and_worker_keeps_assertion_origin(){
    let (_,index,e)=fixture().await;
    let correction=|version,text:&str|FactCorrection{expected_version:version,text:text.into(),source_note:"本人复核".into(),sources:vec![reference(&e)],reason:"本人核对".into(),confirmed:true};
    index.correct_facts(correction(0,&e.text)).await.unwrap();assert!(index.request_fact_card().await.is_ok());
    index.revise_record(update(&e,0,"天琴项目尚未完成")).await.unwrap();
    assert!(index.request_fact_card().await.is_err());
    index.correct_facts(correction(1,"天琴项目尚未完成")).await.unwrap();assert!(index.request_fact_card().await.is_ok());
    let view=index.background_view().await.unwrap();
    assert!(view["context_estimates"]["manual_current_text_and_framing_bytes"].as_i64().unwrap()>0);
    assert_eq!(view["context_estimates"]["deduplicated_current_user_claim_bytes"].as_i64().unwrap(),"天琴项目尚未完成".len() as i64);
    assert!(view["context_estimates"]["actual_usage"].is_null());
    let inputs=index.background_inputs(false).await.unwrap();assert_eq!(inputs[0]["origin"],"user_asserted");
    assert_eq!(inputs[0]["quote"],"天琴项目尚未完成");assert_ne!(inputs[0]["source_hash"],e.hash);assert_eq!(inputs[0]["original_source"]["source_hash"],e.hash);
    let history=index.request_history(&[Turn{question:"之前的建议".into(),confirmed_answer:Some("本人实际发言".into()),suggested_answer:Some("旧声明的AI转述".into())}]).await.unwrap();
    assert!(history[0].suggested_answer.is_none());assert_eq!(history[0].confirmed_answer.as_deref(),Some("本人实际发言"));
}
#[tokio::test]
async fn failed_or_unconfirmed_edit_leaves_no_partial_revision(){
    let (_,index,e)=fixture().await;let version=index.setting("version").await.unwrap();
    let mut change=update(&e,0,"修订内容");change.confirmed=false;assert!(index.revise_record(change).await.is_err());
    let mut change=update(&e,0,"修订内容");change.source.hash="stale".into();assert!(index.revise_record(change).await.is_err());
    sqlx::raw_sql("CREATE TRIGGER reject_assertion_index BEFORE INSERT ON assertion_search_content BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").execute(&index.pool).await.unwrap();
    assert!(index.revise_record(update(&e,0,"失败声明")).await.is_err());
    assert_eq!(index.setting("version").await.unwrap(),version);assert!(index.assertion_history(reference(&e),None).await.unwrap().versions.is_empty());
    assert!(!index.search("猎户","").await.unwrap().is_empty());
}
#[tokio::test]
async fn expiry_blocks_current_retrieval_and_synthesis_but_keeps_explicit_history(){
    let (_,index,e)=fixture().await;index.revise_record(update(&e,0,"虚构天琴项目计划")).await.unwrap();
    let set=|from:Option<&str>,until:Option<&str>|DocumentUpdate{path:e.path.clone(),expected_root:e.root.clone(),expected_hash:e.hash.clone(),confirmed:true,valid_from:from.map(str::to_string),valid_until:until.map(str::to_string)};
    index.update_document(set(None,Some("2000-01-01"))).await.unwrap();
    assert!(index.search("天琴","").await.unwrap().is_empty());assert!(index.background_inputs(false).await.unwrap().is_empty());
    assert_eq!(index.document_records(&e.path,&e.root,&e.hash).await.unwrap()[0].user_assertion.as_ref().unwrap().revision,1);
    index.update_document(set(Some("2099-01-01"),None)).await.unwrap();assert!(index.search("天琴","").await.unwrap().is_empty());
    index.update_document(set(None,None)).await.unwrap();assert!(!index.search("天琴","").await.unwrap().is_empty());
}
