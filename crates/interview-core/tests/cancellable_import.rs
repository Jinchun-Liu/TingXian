//! Retains all synthetic fixtures; no automatic temp-directory deletion.
use interview_core::*;
use sqlx::Row;
use std::{path::{Path,PathBuf}, time::{Duration,SystemTime,UNIX_EPOCH}, sync::atomic::{AtomicU64,Ordering}};

fn fixture() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path=std::env::temp_dir().join(format!("interview-import-{}-{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(),NEXT.fetch_add(1,Ordering::SeqCst)));
    std::fs::create_dir_all(&path).unwrap(); path
}
fn select(path:&str)->Vec<Selection> {vec![Selection {path:path.into(),kind:"technical".into(),allow_external:false,structured:None}]}
async fn snapshot(index:&Index)->Vec<String> {
    let mut result=vec![];
    for query in ["SELECT path||':'||hash||':'||kind||':'||allow_external FROM documents ORDER BY path", "SELECT id||':'||path||':'||location||':'||text FROM chunks ORDER BY id", "SELECT id||':'||tokens FROM search ORDER BY id", "SELECT key||':'||value FROM settings ORDER BY key"] {
        result.extend(sqlx::query(query).fetch_all(&index.pool).await.unwrap().iter().map(|r|r.get::<String,_>(0)));
    }
    result
}
async fn seed(path:&Path)->Index {
    std::fs::write(path.join("old.md"),"originalmark 契约测试的旧证据").unwrap();
    let index=Index::open(&path.join("index.sqlite")).await.unwrap();
    index.import(path.to_str().unwrap(),select("old.md"),"unused").await.unwrap();
    index.correct_facts(interview_core::FactCorrection {expected_version:0,text:"synthetic fact retained".into(),source_note:"Synthetic fixture, not real personal data".into(),sources:vec![],reason:"Synthetic fixture setup".into(),confirmed:true}).await.unwrap(); index
}

#[tokio::test]
async fn cancellation_after_database_mutation_rolls_back_and_restart_succeeds() {
    let path=fixture();let index=seed(&path).await;
    let before=snapshot(&index).await;
    std::fs::write(path.join("new.md"),"replacementmark 新知识").unwrap();
    let control=ImportControl::default();let mut reached=false;
    let result=index.import_controlled(path.to_str().unwrap(),select("new.md"),"unused",&control,|p| {
        if p.phase=="indexing" { reached=true; assert_eq!(control.cancel(),ImportCancelStatus::Requested); }
    }).await;
    assert!(reached);assert_eq!(result.unwrap_err(),IMPORT_CANCELLED);
    assert_eq!(snapshot(&index).await,before,"cancelled job altered documents, chunks, FTS, root/version or facts");
    assert_eq!(std::fs::read_to_string(path.join("old.md")).unwrap(),"originalmark 契约测试的旧证据");
    let report=index.import(path.to_str().unwrap(),select("new.md"),"unused").await.unwrap();
    assert_eq!(report.version,2);assert_eq!(report.success,1);
    assert!(index.search("originalmark","").await.unwrap().is_empty());
    assert_eq!(index.search("replacementmark","").await.unwrap().len(),1);
    assert!(path.join("old.md").exists(),"updating a cache must not delete the original document");
}

#[tokio::test]
async fn cancellation_does_not_wait_for_import_gate_and_pre_cancel_never_scans() {
    let path=fixture();let index=seed(&path).await;let before=snapshot(&index).await;
    let control=ImportControl::default();control.cancel();
    assert_eq!(index.import_controlled("/nonexistent/synthetic",vec![],"unused",&control,|_|panic!("pre-cancelled job started")).await.unwrap_err(),IMPORT_CANCELLED);
    let guard=index.gate.lock().await;
    let control=ImportControl::default();let copy=control.clone();let root=path.to_string_lossy().into_owned();let other=index.clone();
    let task=tokio::spawn(async move {other.import_controlled(&root,select("old.md"),"unused",&copy,|_|{}).await});
    control.cancel();
    assert_eq!(tokio::time::timeout(Duration::from_secs(1),task).await.unwrap().unwrap().unwrap_err(),IMPORT_CANCELLED);
    drop(guard);assert_eq!(snapshot(&index).await,before);
}

#[tokio::test]
async fn cancellation_at_commit_boundary_is_reported_too_late_not_falsely_accepted() {
    let path=fixture();let index=seed(&path).await;
    let control=ImportControl::default();let mut committing=false;
    let report=index.import_controlled(path.to_str().unwrap(),select("old.md"),"unused",&control,|p| {
        if p.phase=="committing" {committing=true;assert_eq!(control.cancel(),ImportCancelStatus::Committing);}
    }).await.unwrap();
    assert!(committing);assert_eq!(report.version,2);assert_eq!(report.unchanged,1);
    assert_eq!(control.cancel(),ImportCancelStatus::Finished);
}

#[cfg(unix)]
#[tokio::test]
async fn cancellation_terminates_live_parser_and_preserves_old_index() {
    use std::os::unix::fs::PermissionsExt;
    for structured in [false,true] {
    let path=fixture();let index=seed(&path).await;let before=snapshot(&index).await;
    let filename=if structured {"slow.json"}else{"slow.docx"};
    let content="{\"topic\":\"synthetic parser cancellation\"}";
    std::fs::write(path.join(filename),content).unwrap();
    let selected=vec![Selection {path:filename.into(),kind:"technical".into(),allow_external:false,structured:structured.then(||StructuredSelection {root:path.canonicalize().unwrap().to_string_lossy().into_owned(),file_hash:digest(content.as_bytes()),pointers:vec!["".into()]})}];
    let python=std::env::var("INTERVIEW_PYTHON").unwrap_or("/usr/bin/python3".into());
    let parser=path.join("slow-parser");
    std::fs::write(&parser,format!("#!{python}\nimport os,time\nfrom pathlib import Path\nPath(__file__).with_suffix('.pid').write_text(str(os.getpid()))\ntime.sleep(60)\n")).unwrap();
    std::fs::set_permissions(&parser,std::fs::Permissions::from_mode(0o700)).unwrap();
    let control=ImportControl::default();let other=index.clone();let copy=control.clone();let root=path.to_string_lossy().into_owned();let program=parser.to_string_lossy().into_owned();
    let task=tokio::spawn(async move {other.import_controlled(&root,selected,&program,&copy,|_|{}).await});
    let pid=tokio::time::timeout(Duration::from_secs(5),async {
        loop {if let Ok(pid)=tokio::fs::read_to_string(parser.with_extension("pid")).await {if !pid.trim().is_empty(){break pid;}} tokio::time::sleep(Duration::from_millis(20)).await;}
    }).await.unwrap();
    assert_eq!(control.cancel(),ImportCancelStatus::Requested);
    assert_eq!(tokio::time::timeout(Duration::from_secs(2),task).await.unwrap().unwrap().unwrap_err(),IMPORT_CANCELLED);
    tokio::time::timeout(Duration::from_secs(2),async {
        loop {
            let result=tokio::process::Command::new("/bin/kill").arg("-0").arg(pid.trim()).output().await.unwrap();
            if !result.status.success() {
                let message=String::from_utf8_lossy(&result.stderr).to_lowercase();
                assert!(message.contains("no such process"),"process probe failed, not proof of termination: {message}");break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("cancelled parser is still alive");
    assert_eq!(snapshot(&index).await,before);
    }
}
