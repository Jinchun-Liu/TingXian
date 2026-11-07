use interview_core::{memory_store as store,session_memory::*,digest};
use sqlx::{sqlite::SqliteConnectOptions,SqlitePool};
use std::collections::BTreeMap;
fn deps()->Dependencies {Dependencies{source_policy:digest(b"sources"),facts:digest(b"facts"),model:digest(b"model"),policy:digest(b"policy")}}
fn event(n:u64,epoch:u64)->Event {Event{id:format!("event-{n}"),session_id:"local-session".into(),epoch,sequence:n,revision:1,kind:EventKind::SelfSpeech,origin:Origin::Microphone,capture_session:Some("capture-1".into()),utterance_id:Some(format!("utterance-{n}")),replaces_revision:None,text:format!("实际发言{n}：我没有做过生产事故处置，只模拟了 HTTP 504，null 不是 0。"),closed:true,terminal:None,question:None,source_dependencies:BTreeMap::new(),asset_sha256:None}}

#[tokio::test]async fn more_than_ring_capacity_reopens_with_all_revisions_and_exact_ack(){
    let dir=tempfile::tempdir().unwrap();let options=SqliteConnectOptions::new().filename(dir.path().join("existing-session-db.sqlite")).create_if_missing(true);let pool=SqlitePool::connect_with(options.clone()).await.unwrap();
    let s=store::open(&pool,"local-session",deps(),false).await.unwrap();let mut l=Ledger::new(&s.id,s.epoch,s.dependencies,false).unwrap();
    for i in 1..=150 {let record=l.append(event(i,s.epoch)).unwrap();let ack=store::append(&pool,&record).await.unwrap();assert_eq!(ack.record_sequence,i);assert_eq!(store::append(&pool,&record).await.unwrap(),ack);}
    let mut revised=event(2,s.epoch);revised.revision=2;revised.replaces_revision=Some(1);revised.text="最终修正：契约测试，不能混用 null、0 与缺失字段。".into();let record=l.append(revised.clone()).unwrap();store::append(&pool,&record).await.unwrap();
    pool.close().await;let pool=SqlitePool::connect_with(options).await.unwrap();let s=store::open(&pool,"local-session",deps(),false).await.unwrap();assert_eq!(s.last_record,151);assert_eq!(s.last_event,150);assert!(!s.missing_older_history);
    let mut replay=Ledger::new(&s.id,s.epoch,s.dependencies,false).unwrap();let mut after=0;loop{let rows=store::page(&pool,&s.id,s.epoch,after,64).await.unwrap();if rows.is_empty(){break;}for r in rows {after=r.record_sequence;replay.append(r.event).unwrap();}}
    assert_eq!(replay.journal(),l.journal());assert_eq!(replay.replay_view().uncovered[1],revised);assert_eq!(replay.journal()[1].event.revision,1);
}
#[tokio::test]async fn failed_transaction_no_ack_no_gap_and_clear_rollback_preserves_raw_data(){
    let pool=SqlitePool::connect("sqlite::memory:").await.unwrap();let s=store::open(&pool,"local-session",deps(),true).await.unwrap();assert!(s.missing_older_history);
    let first=JournalRecord{record_sequence:1,event:event(1,s.epoch)};store::append(&pool,&first).await.unwrap();
    sqlx::query("CREATE TRIGGER simulated_disk_full BEFORE INSERT ON tingxian_memory_latest BEGIN SELECT RAISE(ABORT,'synthetic disk full'); END").execute(&pool).await.unwrap();
    let second=JournalRecord{record_sequence:2,event:event(2,s.epoch)};assert!(store::append(&pool,&second).await.is_err());assert_eq!(store::page(&pool,&s.id,s.epoch,0,64).await.unwrap().len(),1);
    sqlx::query("DROP TRIGGER simulated_disk_full").execute(&pool).await.unwrap();store::append(&pool,&second).await.unwrap();
    let gap=JournalRecord{record_sequence:4,event:event(3,s.epoch)};assert!(store::append(&pool,&gap).await.is_err());
    let mut tx=pool.begin().await.unwrap();store::clear_archived(&mut tx).await.unwrap();interview_core::recovery::clear_archived(&mut tx).await.unwrap();tx.rollback().await.unwrap();assert_eq!(store::page(&pool,&s.id,s.epoch,0,64).await.unwrap().len(),2);
    // Test-only transaction stands in for the host's verified recoverable archive.
    let mut tx=pool.begin().await.unwrap();store::clear_archived(&mut tx).await.unwrap();interview_core::recovery::clear_archived(&mut tx).await.unwrap();tx.commit().await.unwrap();assert!(store::append(&pool,&first).await.is_err());assert!(store::page(&pool,&s.id,s.epoch,0,64).await.is_err());
    let fresh=store::open(&pool,"local-session",deps(),true).await.unwrap();assert_eq!(fresh.epoch,s.epoch+1);assert!(store::append(&pool,&first).await.is_err());
}
#[tokio::test]async fn corruption_identity_and_explicit_size_limit_fail_without_overwriting(){
    let pool=SqlitePool::connect("sqlite::memory:").await.unwrap();let s=store::open(&pool,"local-session",deps(),false).await.unwrap();let first=JournalRecord{record_sequence:1,event:event(1,s.epoch)};store::append(&pool,&first).await.unwrap();
    let mut replace=first.clone();replace.event.text="试图覆盖旧顺序".into();assert!(store::append(&pool,&replace).await.is_err());
    let mut wrong=event(1,s.epoch);wrong.revision=2;wrong.replaces_revision=Some(1);wrong.capture_session=Some("other-device".into());assert!(store::append(&pool,&JournalRecord{record_sequence:2,event:wrong}).await.is_err());
    sqlx::query("UPDATE tingxian_memory_sessions SET bytes=?").bind(32_i64*1024*1024).execute(&pool).await.unwrap();assert!(store::append(&pool,&JournalRecord{record_sequence:2,event:event(2,s.epoch)}).await.is_err());
    assert_eq!(store::page(&pool,&s.id,s.epoch,0,64).await.unwrap(),vec![first]);
    sqlx::query("UPDATE tingxian_memory_events SET payload='{}'").execute(&pool).await.unwrap();assert!(store::page(&pool,&s.id,s.epoch,0,64).await.is_err());
    sqlx::query("DELETE FROM tingxian_memory_events").execute(&pool).await.unwrap();assert!(store::page(&pool,&s.id,s.epoch,0,64).await.is_err(),"a missing final row must not look like a complete empty page");
}
