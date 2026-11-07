//! Real JSON/YAML parser and SQLite, synthetic files retained for inspection.
use interview_core::*;
use std::path::Path;
fn python()->String {std::env::var("INTERVIEW_PYTHON").unwrap_or_else(|_|"python3".into())}
async fn select(root:&Path,path:&str,pointers:&[&str])->Selection {
    let preview=preview_structured(root.to_str().unwrap(),path,&python()).await.unwrap();
    Selection {path:path.into(),kind:"personal".into(),allow_external:false,structured:Some(StructuredSelection {root:preview.root,file_hash:preview.file_hash,pointers:pointers.iter().map(|s|(*s).into()).collect()})}
}
fn fixture()->std::path::PathBuf {tempfile::Builder::new().prefix("interview-structured-").tempdir().unwrap().keep().canonicalize().unwrap()}

#[tokio::test]
async fn explicit_fields_preserve_types_paths_arrays_and_exclude_unselected_values() {
    let root=fixture();let input=r#"{"profile":{"name":"合成候选人","null_value":null,"zero":0,"empty":"","false_value":false,"precise":123456789.12345678901234567890,"tiny":1e-999,"signed_zero":-0},"projects":[{"name":"OrbitDemo"}],"a/b":{"~key":"斜杠字段"},"omitted":"未选择的机密花栗鼠"}"#;
    std::fs::write(root.join("资料.json"),input).unwrap();
    let preview=preview_structured(root.to_str().unwrap(),"资料.json",&python()).await.unwrap();
    assert!(preview.fields.iter().any(|f|f.pointer=="/a~1b/~0key"));assert!(preview.fields.iter().any(|f|f.pointer=="/projects/0/name"));
    for (pointer,kind,literal) in [("/profile/null_value","null","null"),("/profile/zero","number","0"),("/profile/empty","string","\"\""),("/profile/false_value","boolean","false")] {
        let f=preview.fields.iter().find(|f|f.pointer==pointer).unwrap();assert_eq!(f.kind,kind);assert_eq!(f.preview,literal);
    }
    let index=Index::open(&root.join("index.sqlite")).await.unwrap();assert_eq!(index.documents().await.unwrap().len(),0,"preview is read-only");
    let selection=select(&root,"资料.json",&["/profile","/projects/0/name","/a~1b/~0key"]).await;
    let report=index.import(root.to_str().unwrap(),vec![selection],&python()).await.unwrap();assert_eq!(report.success,1);assert!(report.failures.is_empty());
    let doc=index.documents().await.unwrap().remove(0);let parts=index.document_segments(&doc.path,&doc.root,&doc.hash).await.unwrap();let all=parts.iter().map(|s|s.text.as_str()).collect::<Vec<_>>().join("\n");
    assert!(all.contains("类型：null 空值\n值：null"));assert!(all.contains("类型：数字\n值：0"));assert!(all.contains("类型：字符串\n值：\"\""));
    assert!(all.contains("值：123456789.12345678901234567890"));assert!(all.contains("值：1e-999"));assert!(all.contains("值：-0"));
    assert!(all.contains("OrbitDemo"));assert!(parts.iter().any(|p|p.location.contains("/projects/0/name")));assert!(!all.contains("花栗鼠"));assert!(!all.contains("/missing"));
    assert!(index.search("花栗鼠","").await.unwrap().is_empty());assert_eq!(std::fs::read_to_string(root.join("资料.json")).unwrap(),input);
}

#[tokio::test]
async fn changed_scope_is_reindexed_and_invalidates_review_even_when_file_unchanged() {
    let root=fixture();std::fs::write(root.join("facts.json"),r#"{"old":"旧字段棕熊","new":"新字段海豚"}"#).unwrap();
    let index=Index::open(&root.join("index.sqlite")).await.unwrap();
    let original=select(&root,"facts.json",&["/old"]).await;
    index.import(root.to_str().unwrap(),vec![original.clone()],&python()).await.unwrap();
    let first=index.documents().await.unwrap().remove(0);
    index.update_document(DocumentUpdate {path:first.path.clone(),expected_root:first.root.clone(),expected_hash:first.hash.clone(),confirmed:true,valid_from:None,valid_until:None}).await.unwrap();
    assert_eq!(index.import(root.to_str().unwrap(),vec![original],&python()).await.unwrap().unchanged,1);
    let changed=select(&root,"facts.json",&["/new","/new"]).await;
    assert_eq!(index.import(root.to_str().unwrap(),vec![changed],&python()).await.unwrap().unchanged,0);
    let second=index.documents().await.unwrap().remove(0);assert_ne!(first.hash,second.hash);assert_eq!(second.metadata.confirmation,Confirmation::ContentChanged);
    assert!(index.document_segments(&first.path,&first.root,&first.hash).await.is_err());assert!(index.search("棕熊","").await.unwrap().is_empty());assert!(index.search("海豚","").await.unwrap().is_empty(),"changed selection requires re-review");
    index.update_document(DocumentUpdate {path:second.path.clone(),expected_root:second.root.clone(),expected_hash:second.hash.clone(),confirmed:true,valid_from:None,valid_until:None}).await.unwrap();
    assert!(!index.search("海豚","").await.unwrap().is_empty());
    // Parent selection absorbs duplicate/descendant scopes, preserving a
    // stable revision for equivalent explicit choices.
    let all=select(&root,"facts.json",&["","/new"]).await;
    index.import(root.to_str().unwrap(),vec![all],&python()).await.unwrap();
    let all=select(&root,"facts.json",&[""]).await;
    assert_eq!(index.import(root.to_str().unwrap(),vec![all],&python()).await.unwrap().unchanged,1);
}

#[tokio::test]
async fn preview_identity_missing_fields_and_no_implicit_selection_are_enforced() {
    let root=fixture();std::fs::write(root.join("data.json"),r#"{"topic":"原数据"}"#).unwrap();
    let index=Index::open(&root.join("index.sqlite")).await.unwrap();
    let none=Selection {path:"data.json".into(),kind:"technical".into(),allow_external:false,structured:None};
    let r=index.import(root.to_str().unwrap(),vec![none],&python()).await.unwrap();assert_eq!(r.success,0);assert!(r.failures[0].1.contains("明确选择"));
    let stale=select(&root,"data.json",&["/topic"]).await;std::fs::write(root.join("data.json"),r#"{"topic":"变化后的内容"}"#).unwrap();
    let r=index.import(root.to_str().unwrap(),vec![stale],&python()).await.unwrap();assert_eq!(r.chunks,0);assert!(r.failures[0].1.contains("重新预览"));
    let mut wrong_root=select(&root,"data.json",&["/topic"]).await;wrong_root.structured.as_mut().unwrap().root="/different/root".into();
    let r=index.import(root.to_str().unwrap(),vec![wrong_root],&python()).await.unwrap();assert_eq!(r.success,0);
    let nonexistent=select(&root,"data.json",&["/missing"]).await;
    let r=index.import(root.to_str().unwrap(),vec![nonexistent],&python()).await.unwrap();assert_eq!(r.success,0);assert!(r.failures[0].1.contains("不存在"));
    let visible=scan(root.to_str().unwrap()).unwrap();assert!(visible.iter().find(|f|f.path=="data.json").unwrap().eligible);
    assert!(preview_structured(root.to_str().unwrap(),"../other.json",&python()).await.is_err());
}

#[tokio::test]
async fn yaml_nested_scalars_anchors_merges_dates_and_empty_values_are_explicit() {
    let root=fixture();let text="defaults: &defaults\n  attempts: 2\n  label: 原值\nprofile:\n  <<: *defaults\n  label: 覆盖值\n  zero: 0\n  nothing: null\n  empty: ''\n  when: 2027-07-01\n  enabled: yes\n  precise: 123456789.12345678901234567890\n  tiny: 1.0e-999\n  base60: 1:30.5\nlist: [null, 0, '']\nempty_map: {}\nempty_list: []\n";
    std::fs::write(root.join("资料.YAML"),text).unwrap();
    let p=preview_structured(root.to_str().unwrap(),"资料.YAML",&python()).await.unwrap();
    assert_eq!(p.fields.iter().find(|f|f.pointer=="/profile/label").unwrap().preview,"\"覆盖值\"");
    assert_eq!(p.fields.iter().find(|f|f.pointer=="/profile/attempts").unwrap().preview,"2");
    assert_eq!(p.fields.iter().find(|f|f.pointer=="/profile/when").unwrap().preview,"\"2027-07-01\"");
    assert_eq!(p.fields.iter().find(|f|f.pointer=="/profile/enabled").unwrap().kind,"boolean");
    assert_eq!(p.fields.iter().find(|f|f.pointer=="/profile/precise").unwrap().preview,"123456789.12345678901234567890");
    assert_eq!(p.fields.iter().find(|f|f.pointer=="/profile/tiny").unwrap().preview,"1.0E-999");
    assert_eq!(p.fields.iter().find(|f|f.pointer=="/profile/base60").unwrap().preview,"90.5");
    let index=Index::open(&root.join("index.sqlite")).await.unwrap();let chosen=select(&root,"资料.YAML",&[""]).await;
    let r=index.import(root.to_str().unwrap(),vec![chosen],&python()).await.unwrap();assert_eq!(r.success,1);assert!(r.failures.is_empty());
    let doc=index.documents().await.unwrap().remove(0);let parts=index.document_segments(&doc.path,&doc.root,&doc.hash).await.unwrap();let all=parts.iter().map(|s|s.text.as_str()).collect::<Vec<_>>().join("\n");
    assert!(all.contains("值：{}"));assert!(all.contains("值：[]"));assert!(all.contains("字段 /list/1"));
    for (literal,kind) in [("null","null"),("0","number"),("\"\"","string"),("[]","array"),("{}","object")] {
        std::fs::write(root.join("scalar.json"),literal).unwrap();let p=preview_structured(root.to_str().unwrap(),"scalar.json",&python()).await.unwrap();
        assert_eq!(p.fields.len(),1);assert_eq!(p.fields[0].kind,kind);assert_eq!(p.fields[0].preview,literal);
        let chosen=select(&root,"scalar.json",&[""]).await;let r=index.import(root.to_str().unwrap(),vec![chosen],&python()).await.unwrap();assert_eq!(r.success,1);assert_eq!(r.chunks,1);
    }
}

#[tokio::test]
async fn ambiguous_malformed_recursive_and_oversized_data_fail_without_partial_index() {
    let root=fixture();let index=Index::open(&root.join("index.sqlite")).await.unwrap();
    let cases=[("duplicate.json","{\"x\":1,\"x\":2}"), ("bad.json","{broken"),("nan.json","{\"x\":NaN}"),
        ("duplicate.yaml","x: 1\nx: 2"),("merge-duplicate.yaml","x:\n  <<: {a: 1, a: 2}"),
        ("cycle.yaml","x: &x [*x]"),("cycle-merge.yaml","x: &x {<<: *x}"),("multi.yaml","---\nx: 1\n---\nx: 2"),
        ("tag.yaml","x: !!python/object/apply:builtins.print ['must not run']"),("key.yaml","1: value"),("infinity.yaml","x: .inf")];
    for (name,contents) in cases {
        std::fs::write(root.join(name),contents).unwrap();assert!(preview_structured(root.to_str().unwrap(),name,&python()).await.is_err(),"unexpected acceptance: {name}");
        let selected=Selection {path:name.into(),kind:"technical".into(),allow_external:false,structured:Some(StructuredSelection {root:root.to_string_lossy().into_owned(),file_hash:digest(contents.as_bytes()),pointers:vec!["".into()]})};
        let r=index.import(root.to_str().unwrap(),vec![selected],&python()).await.unwrap();assert_eq!(r.success,0,"{name}");assert_eq!(r.failures.len(),1);assert_eq!(r.chunks,0);
    }
    for (name,contents) in [("deep.json",format!("{}0{}","[".repeat(70),"]".repeat(70))),("large.json",format!("\"{}\"","字".repeat(500_001)))] {
        std::fs::write(root.join(&name),contents).unwrap();assert!(preview_structured(root.to_str().unwrap(),&name,&python()).await.is_err());
    }
    assert!(index.documents().await.unwrap().is_empty());
}
