//! All files are synthetic and retained. No network or user profile access.
use interview_core::*;
use std::path::PathBuf;
fn folder()->PathBuf {
    let root=tempfile::Builder::new().prefix("interview-facts-").tempdir().unwrap().keep();
    std::fs::create_dir_all(root.join("source")).unwrap();root
}
fn correction(version:i64,text:&str)->FactCorrection {FactCorrection {expected_version:version,text:text.into(),source_note:"Synthetic personal evidence, paragraph 1".into(),sources:vec![],reason:"User checked synthetic correction".into(),confirmed:true}}
fn selection(path:&str,kind:&str)->Selection {Selection {path:path.into(),kind:kind.into(),allow_external:false,structured:None}}
fn reference(e:&Evidence)->FactSourceRef {FactSourceRef {source_id:e.source_id.clone(),path:e.path.clone(),hash:e.hash.clone(),root:e.root.clone()}}

#[tokio::test]
async fn legacy_migration_preserves_old_value_without_inventing_confirmation_or_sending_history() {
    let root=folder();let database=root.join("profile.sqlite");
    let pool=sqlx::sqlite::SqlitePoolOptions::new().connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&database).create_if_missing(true)).await.unwrap();
    sqlx::raw_sql("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);INSERT INTO settings VALUES('facts','legacy_old_value source paragraph 4');INSERT INTO settings VALUES('version','7');INSERT INTO settings VALUES('aliases','keep alias');").execute(&pool).await.unwrap();pool.close().await;
    let index=Index::open(&database).await.unwrap();let baseline=index.fact_card().await.unwrap();
    assert_eq!(baseline.version,0);assert!(baseline.confirmed_at.is_none());assert_eq!(baseline.text,"legacy_old_value source paragraph 4");
    assert_eq!(index.setting("version").await.unwrap(),"7");assert_eq!(index.setting("aliases").await.unwrap(),"keep alias");
    let legacy=messages_with_fact_card("synthetic question",&baseline,&[],&[],&[]).unwrap();
    let legacy:serde_json::Value=serde_json::from_str(legacy[1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(legacy["user_confirmed_facts"],"");assert!(legacy["legacy_fact_card"].as_str().unwrap().contains("legacy_old_value"));
    assert!(index.set("facts","silent replacement").await.is_err());
    let mut update=correction(0,"current_corrected_value");update.source_note="/synthetic/private/locally-retained-source paragraph 8".into();
    let next=index.correct_facts(update).await.unwrap();assert_eq!(next.version,1);assert_eq!(next.previous_version,Some(0));assert!(next.confirmed_at.is_some());
    let history=index.fact_history(None).await.unwrap();assert_eq!(history.versions.len(),2);assert_eq!(history.versions[1].text,baseline.text);assert!(history.versions[1].confirmed_at.is_none());
    let payload=messages_with_fact_card("synthetic question",&next,&[],&[],&[]).unwrap();let content=payload[1]["content"].as_str().unwrap();
    assert!(content.contains("current_corrected_value"));assert!(!content.contains("legacy_old_value"));assert!(!content.contains("locally-retained-source"));
    assert_eq!(index.setting("version").await.unwrap(),"7","fact version is separate from retrieval index version");
    drop(index);let reopened=Index::open(&database).await.unwrap();assert_eq!(reopened.fact_card().await.unwrap().version,1);assert_eq!(reopened.fact_history(None).await.unwrap().versions.len(),2);
}
#[tokio::test]
async fn evidence_snapshots_survive_updates_and_reject_practice_or_stale_identity() {
    let root=folder();std::fs::write(root.join("source/evidence.md"),"personalneedle original evidence").unwrap();std::fs::write(root.join("source/practice.md"),"practiceneedle I built everything").unwrap();
    let index=Index::open(&root.join("profile.sqlite")).await.unwrap();let selected=vec![selection("evidence.md","evidence"),selection("practice.md","practice")];
    index.import(root.join("source").to_str().unwrap(),selected.clone(),"unused").await.unwrap();
    let evidence=index.search("personalneedle","").await.unwrap().remove(0);let practice=index.search("practiceneedle","").await.unwrap().remove(0);
    let mut bad=correction(0,"not accepted");bad.sources=vec![reference(&practice)];assert!(index.correct_facts(bad).await.unwrap_err().contains("练习"));
    let mut good=correction(0,"synthetic checked contribution");good.sources=vec![reference(&evidence)];let saved=index.correct_facts(good).await.unwrap();
    assert_eq!(saved.sources[0].text,evidence.text);assert_eq!(saved.sources[0].reference.hash,evidence.hash);
    std::fs::write(root.join("source/evidence.md"),"personalneedle new evidence").unwrap();index.import(root.join("source").to_str().unwrap(),selected,"unused").await.unwrap();
    let mut stale=correction(1,"stale source cannot confirm");stale.sources=vec![reference(&evidence)];assert!(index.correct_facts(stale).await.is_err());
    let current=index.search("personalneedle","").await.unwrap().remove(0);let mut wrong_root=correction(1,"wrong root");let mut r=reference(&current);r.root="/wrong/root".into();wrong_root.sources=vec![r];assert!(index.correct_facts(wrong_root).await.is_err());
    let mut duplicates=correction(1,"duplicates");duplicates.sources=vec![reference(&current),reference(&current)];assert!(index.correct_facts(duplicates).await.is_err());
    assert_eq!(index.fact_card().await.unwrap().sources[0].text,evidence.text);assert_eq!(index.fact_history(None).await.unwrap().versions.len(),2);
    let payload=messages_with_fact_card("synthetic question",&saved,&[],&[],&[]).unwrap().to_string();assert!(!payload.contains("original evidence"));assert!(!payload.contains(&evidence.hash));assert!(!payload.contains(&evidence.root));
}
#[tokio::test]
async fn concurrent_edits_do_not_lose_a_version_and_storage_failure_rolls_back_both_tables() {
    let root=folder();let index=Index::open(&root.join("profile.sqlite")).await.unwrap();
    let (a,b)=tokio::join!(index.correct_facts(correction(0,"edit A")),index.correct_facts(correction(0,"edit B")));
    assert_ne!(a.is_ok(),b.is_ok());let before=index.fact_card().await.unwrap();assert_eq!(before.version,1);assert_eq!(index.fact_history(None).await.unwrap().versions.len(),2);
    sqlx::raw_sql("CREATE TRIGGER refuse_fact_projection BEFORE INSERT ON settings WHEN NEW.key='facts' BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").execute(&index.pool).await.unwrap();
    assert!(index.correct_facts(correction(1,"must roll back")).await.is_err());
    assert_eq!(index.fact_card().await.unwrap().text,before.text);assert_eq!(index.fact_history(None).await.unwrap().versions.len(),2);
}
#[tokio::test]
async fn confirmation_limits_pagination_and_projection_drift_are_enforced() {
    let root=folder();let index=Index::open(&root.join("profile.sqlite")).await.unwrap();
    for mode in 0..5 {
        let mut bad=correction(0,"not accepted");match mode {0=>bad.confirmed=false,1=>bad.reason=" ".into(),2=>bad.source_note=" ".into(),3=>bad.text="中".repeat(2201),_=>bad.source_note="中".repeat(1201)};
        assert!(index.correct_facts(bad).await.is_err());
    }
    assert_eq!(index.fact_card().await.unwrap().version,0);
    for version in 0..23 {index.correct_facts(correction(version,&format!("version {version}"))).await.unwrap();}
    let first=index.fact_history(None).await.unwrap();assert_eq!(first.versions.len(),20);assert_eq!(first.versions[0].version,23);assert_eq!(first.next_before,Some(4));
    let rest=index.fact_history(first.next_before).await.unwrap();assert_eq!(rest.versions.len(),4);assert_eq!(rest.versions.last().unwrap().version,0);assert!(rest.next_before.is_none());
    let empty=index.correct_facts(correction(23,"")).await.unwrap();assert_eq!(empty.version,24);assert!(index.fact_card().await.unwrap().text.is_empty());assert_eq!(index.fact_history(Some(24)).await.unwrap().versions[0].text,"version 22");
    sqlx::query("UPDATE settings SET value='outside unversioned change' WHERE key='facts'").execute(&index.pool).await.unwrap();
    assert!(index.fact_card().await.is_err());assert!(index.correct_facts(correction(24,"do not overwrite drift")).await.is_err());
    assert_eq!(index.fact_history(None).await.unwrap().versions[0].version,24);
}
