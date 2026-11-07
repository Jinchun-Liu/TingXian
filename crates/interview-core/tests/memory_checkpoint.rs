use interview_core::{compaction::*,memory_checkpoint as cp,memory_store as store,session_memory::*,digest};
use sqlx::SqlitePool;
use std::collections::BTreeMap;
fn deps()->Dependencies {Dependencies{source_policy:digest(b"sources"),facts:digest(b"facts"),model:digest(b"model"),policy:digest(b"policy")}}
fn event(n:u64)->Event {Event{id:format!("event-{n}"),session_id:"session".into(),epoch:1,sequence:n,revision:1,kind:EventKind::SelfSpeech,origin:Origin::Microphone,capture_session:Some("capture-1".into()),utterance_id:Some(format!("utterance-{n}")),replaces_revision:None,text:format!("第{n}条：我只模拟过 HTTP 504；状态未知，不证明回滚。"),closed:true,terminal:None,question:None,source_dependencies:BTreeMap::new(),asset_sha256:None}}
async fn fixture()->(SqlitePool,Ledger) {let pool=SqlitePool::connect("sqlite::memory:").await.unwrap();let s=store::open(&pool,"session",deps(),false).await.unwrap();let mut l=Ledger::new(&s.id,s.epoch,s.dependencies,false).unwrap();for n in 1..=40{let r=l.append(event(n)).unwrap();store::append(&pool,&r).await.unwrap();}(pool,l)}
fn outcome(s:&Snapshot)->Outcome {Outcome{candidate:Some(Candidate{schema_version:1,snapshot_id:s.id().unwrap(),finish_reason:"stop".into(),items:s.events.iter().map(|e|MemoryItem{kind:e.kind.clone(),origin:e.origin.clone(),refs:vec![e.reference().unwrap()],quote:e.text.clone()}).collect(),omitted:vec![]}),usage:Some(Usage{input_tokens:10,output_tokens:5,reasoning_tokens:None,cache_hit_tokens:None,cache_miss_tokens:None}),error:None,cancelled:false,transport_started:true}}

#[tokio::test]async fn restart_retains_reserved_attempt_and_never_restarts_network(){
    let(pool,l)=fixture().await;let c=Config{enabled:true,..Default::default()};let mut s=Scheduler::default();let a=s.begin(&l,&c,true,false,28,100_000).unwrap().unwrap();s.mark_sent(&a.id).unwrap();
    let reference=cp::save(&pool,&cp::Checkpoint::capture(&l,&s),0).await.unwrap();let(restored,mut scheduler)=cp::load(&pool,&reference).await.unwrap();assert_eq!(restored.journal(),l.journal());assert_eq!(scheduler.attempts()[0].status,Status::Interrupted);assert!(scheduler.attempts()[0].usage.is_none());assert_eq!(scheduler.charged_units(),a.reserved_units);assert!(!scheduler.active());assert!(scheduler.begin(&restored,&c,true,false,28,500_000).unwrap().is_none());
    let mut bad=cp::Checkpoint::capture(&l,&s);bad.attempts[0].reserved_units=0;assert!(cp::save(&pool,&bad,1).await.is_err());assert_eq!(cp::latest(&pool,"session").await.unwrap(),Some(reference));
}
#[tokio::test]async fn completion_with_concurrent_append_pins_exact_old_and_new_versions(){
    let(pool,mut l)=fixture().await;let c=Config{enabled:true,..Default::default()};let mut s=Scheduler::default();let a=s.begin(&l,&c,true,false,28,100_000).unwrap().unwrap();s.mark_sent(&a.id).unwrap();let old=cp::save(&pool,&cp::Checkpoint::capture(&l,&s),0).await.unwrap();
    for n in 41..=48{let r=l.append(event(n)).unwrap();store::append(&pool,&r).await.unwrap();}
    s.finish(&a.id,&mut l,&c,outcome(&a.snapshot)).unwrap();assert_eq!(s.attempts()[0].status,Status::Committed);
    let saved=cp::Checkpoint::capture(&l,&s);let new=cp::save(&pool,&saved,1).await.unwrap();assert!(cp::save(&pool,&saved,1).await.is_err());
    let(old_l,old_s)=cp::load(&pool,&old).await.unwrap();assert_eq!(old_l.journal().len(),40);assert_eq!(old_l.version(),0);assert_eq!(old_s.attempts()[0].status,Status::Interrupted);
    let(new_l,new_s)=cp::load(&pool,&new).await.unwrap();assert_eq!(new_l.journal().len(),48);assert_eq!(new_l.version(),1);assert_eq!(new_s.attempts()[0].status,Status::Committed);assert_eq!(new_l.replay_view().uncovered.len(),20);
    let r=l.append(event(49)).unwrap();store::append(&pool,&r).await.unwrap();assert!(cp::save(&pool,&saved,2).await.is_err(),"stale full checkpoint must not discard appended originals");assert_eq!(cp::load(&pool,&new).await.unwrap().0.journal().len(),48);
}
#[tokio::test]async fn invalidation_restore_and_failure_preserve_prior_checkpoint(){
    let(pool,mut l)=fixture().await;let c=Config{enabled:true,..Default::default()};let mut s=Scheduler::default();let a=s.begin(&l,&c,true,false,28,100_000).unwrap().unwrap();s.mark_sent(&a.id).unwrap();s.finish(&a.id,&mut l,&c,outcome(&a.snapshot)).unwrap();let first=cp::save(&pool,&cp::Checkpoint::capture(&l,&s),0).await.unwrap();
    let mut e=event(2);e.revision=2;e.replaces_revision=Some(1);e.text="更正：未做过真实生产事故；只使用模拟数据。".into();let r=l.append(e).unwrap();store::append(&pool,&r).await.unwrap();
    assert!(l.replay_view().chunks.is_empty());let second=cp::save(&pool,&cp::Checkpoint::capture(&l,&s),1).await.unwrap();let(restored,_)=cp::load(&pool,&second).await.unwrap();assert!(restored.replay_view().chunks.is_empty());assert_eq!(restored.chunks().len(),1);
    let mut fabricated=cp::Checkpoint::capture(&l,&s);fabricated.chunks[0].valid=true;assert!(cp::save(&pool,&fabricated,2).await.is_err());
    sqlx::query("CREATE TRIGGER full_disk BEFORE INSERT ON tingxian_memory_checkpoints BEGIN SELECT RAISE(ABORT,'synthetic disk full'); END").execute(&pool).await.unwrap();assert!(cp::save(&pool,&cp::Checkpoint::capture(&l,&s),2).await.is_err());assert_eq!(cp::latest(&pool,"session").await.unwrap(),Some(second));assert_eq!(cp::load(&pool,&first).await.unwrap().0.replay_view().chunks.len(),1);
}
#[tokio::test]async fn forged_quotes_cross_session_missing_original_and_clear_are_rejected(){
    let(pool,l)=fixture().await;let s=Scheduler::default();let mut c=cp::Checkpoint::capture(&l,&s);let saved=cp::save(&pool,&c,0).await.unwrap();
    c.session_id="other".into();assert!(cp::save(&pool,&c,1).await.is_err());let mut wrong=saved.clone();wrong.last_record=39;assert!(cp::load(&pool,&wrong).await.is_err());
    sqlx::query("UPDATE tingxian_memory_events SET payload='{}' WHERE record_sequence=1").execute(&pool).await.unwrap();assert!(cp::load(&pool,&saved).await.is_err());
    let mut tx=pool.begin().await.unwrap();store::clear_archived(&mut tx).await.unwrap();interview_core::recovery::clear_archived(&mut tx).await.unwrap();tx.rollback().await.unwrap();assert!(cp::latest(&pool,"session").await.unwrap().is_some());
    let mut tx=pool.begin().await.unwrap();store::clear_archived(&mut tx).await.unwrap();interview_core::recovery::clear_archived(&mut tx).await.unwrap();tx.commit().await.unwrap();assert!(cp::latest(&pool,"session").await.unwrap().is_none());assert!(cp::load(&pool,&saved).await.is_err());
}

#[tokio::test]async fn saved_prefix_fork_is_atomic_and_does_not_absorb_newer_history(){
    let(pool,mut l)=fixture().await;let c=Config{enabled:true,..Default::default()};let mut scheduler=Scheduler::default();
    let a=scheduler.begin(&l,&c,true,false,28,100_000).unwrap().unwrap();scheduler.mark_sent(&a.id).unwrap();scheduler.finish(&a.id,&mut l,&c,outcome(&a.snapshot)).unwrap();
    let reference=cp::save(&pool,&cp::Checkpoint::capture(&l,&scheduler),0).await.unwrap();
    for n in 41..=44{store::append(&pool,&l.append(event(n)).unwrap()).await.unwrap();}
    let boundary=cp::Boundary::capture(&l,Some(reference.clone())).unwrap();
    store::append(&pool,&l.append(event(45)).unwrap()).await.unwrap();
    let(fork,book,saved)=cp::fork_boundary(&pool,&boundary,"saved-branch",l.dependencies().clone()).await.unwrap();
    assert_eq!(fork.journal().len(),44);assert_eq!(fork.replay_view().uncovered.len(),16);assert_eq!(book.charged_units(),15);assert!(!book.active());
    assert!(fork.journal().iter().all(|r|r.event.session_id=="saved-branch"));assert_eq!(cp::load(&pool,&saved).await.unwrap().0.version(),1);
    assert_eq!(cp::load(&pool,&reference).await.unwrap().0.journal().len(),40);assert_eq!(store::page(&pool,"session",1,44,4).await.unwrap().len(),1);
    let mut changed=l.dependencies().clone();changed.source_policy=digest(b"withdrawn");let(fork,_,_)=cp::fork_boundary(&pool,&boundary,"withdrawn-branch",changed).await.unwrap();assert!(fork.replay_view().chunks.is_empty());
    sqlx::query("CREATE TRIGGER full_fork BEFORE INSERT ON tingxian_memory_events WHEN NEW.session_id='failed-branch' BEGIN SELECT RAISE(ABORT,'synthetic full'); END").execute(&pool).await.unwrap();
    assert!(cp::fork_boundary(&pool,&boundary,"failed-branch",l.dependencies().clone()).await.is_err());
    let n:i64=sqlx::query_scalar("SELECT count(*) FROM tingxian_memory_sessions WHERE id='failed-branch'").fetch_one(&pool).await.unwrap();assert_eq!(n,0);
    let mut bad=boundary;bad.last_record+=1;assert!(cp::load_boundary(&pool,&bad).await.is_err());
}
