//! Synthetic inputs only. Fixtures are retained; no cloud request is made.
use interview_core::*;
fn fake()->String {format!("sk-{}","synthetic01234567".repeat(2))}
fn selection(path:&str)->Selection {Selection{path:path.into(),kind:"technical".into(),allow_external:true,structured:None}}
fn fixture()->std::path::PathBuf {tempfile::Builder::new().prefix("interview-key-boundaries-").tempdir().unwrap().keep().canonicalize().unwrap()}
fn python()->String {std::env::var("INTERVIEW_PYTHON").unwrap_or_else(|_|"python3".into())}

#[tokio::test]
async fn new_updated_and_legacy_cached_sources_cannot_index_credential_text() {
    let root=fixture();let file=root.join("notes.md");let index=Index::open(&root.join("index.sqlite")).await.unwrap();
    let unsafe_text=format!("笔记中的测试凭证为{}，null不等于0。",fake());
    std::fs::write(&file,&unsafe_text).unwrap();
    let rejected=index.import(root.to_str().unwrap(),vec![selection("notes.md")],&python()).await.unwrap();
    assert_eq!(rejected.success,0);assert_eq!(rejected.chunks,0);assert!(rejected.failures[0].1.contains("凭证格式"));
    assert!(!serde_json::to_string(&rejected).unwrap().contains(&fake()));assert_eq!(std::fs::read_to_string(&file).unwrap(),unsafe_text);
    std::fs::write(&file,"DemoProject 的契约测试必须区分 null 与 0。").unwrap();
    assert_eq!(index.import(root.to_str().unwrap(),vec![selection("notes.md")],&python()).await.unwrap().success,1);
    // Simulate a pre-filter cache while the source revision remains unchanged.
    sqlx::query("UPDATE chunks SET text=?").bind(&unsafe_text).execute(&index.pool).await.unwrap();
    let cached=index.import(root.to_str().unwrap(),vec![selection("notes.md")],&python()).await.unwrap();
    assert_eq!(cached.unchanged,0);assert_eq!(cached.success,0);assert_eq!(cached.chunks,0);
    assert_eq!(index.import(root.to_str().unwrap(),vec![selection("notes.md")],&python()).await.unwrap().success,1);
    std::fs::write(&file,&unsafe_text).unwrap();
    let changed=index.import(root.to_str().unwrap(),vec![selection("notes.md")],&python()).await.unwrap();
    assert_eq!(changed.chunks,0);assert!(index.documents().await.unwrap().is_empty());
}

#[tokio::test]
async fn only_explicit_structured_fields_are_checked_and_fact_rejection_is_atomic() {
    let root=fixture();let input=serde_json::json!({"topic":"JSON Schema 中 null 不等于零，也不同于缺失字段","unused":fake()}).to_string();
    std::fs::write(root.join("notes.json"),&input).unwrap();let index=Index::open(&root.join("index.sqlite")).await.unwrap();
    let preview=preview_structured(root.to_str().unwrap(),"notes.json",&python()).await.unwrap();
    let mut chosen=selection("notes.json");chosen.structured=Some(StructuredSelection{root:preview.root,file_hash:preview.file_hash,pointers:vec!["/topic".into()]});
    assert_eq!(index.import(root.to_str().unwrap(),vec![chosen.clone()],&python()).await.unwrap().success,1);
    chosen.structured.as_mut().unwrap().pointers=vec!["/unused".into()];
    assert_eq!(index.import(root.to_str().unwrap(),vec![chosen],&python()).await.unwrap().success,0);
    assert_eq!(std::fs::read_to_string(root.join("notes.json")).unwrap(),input);
    let before=serde_json::to_value(index.fact_card().await.unwrap()).unwrap();
    for field in ["text","source_note","reason"] {
        let mut update=FactCorrection{expected_version:0,text:"合成事实".into(),source_note:"合成来源".into(),sources:vec![],reason:"合成理由".into(),confirmed:true};
        let target=match field {"text"=>&mut update.text,"source_note"=>&mut update.source_note,_=>&mut update.reason};*target=format!("凭证为{}",fake());
        assert!(index.correct_facts(update).await.unwrap_err().contains("凭证格式"));
        assert_eq!(serde_json::to_value(index.fact_card().await.unwrap()).unwrap(),before);
    }
}
