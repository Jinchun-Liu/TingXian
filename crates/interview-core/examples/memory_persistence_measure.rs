//! Local disk/selection measurement with synthetic data, not live ASR or AI.
//! Usage: memory_persistence_measure /new/path.sqlite (never overwrites).
use interview_core::{compaction::Scheduler, digest, memory_checkpoint as cp, memory_store as store, session_memory::*};
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::{collections::BTreeMap, path::PathBuf, time::Instant};

fn percentiles(mut samples: Vec<u128>) -> serde_json::Value {
    samples.sort_unstable();
    json!({"samples":samples.len(),"p50_us":samples[(samples.len()-1)/2],"p95_us":samples[(samples.len()*95).div_ceil(100)-1],"max_us":samples.last()})
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path=PathBuf::from(std::env::args_os().nth(1).ok_or("specify a new synthetic SQLite path")?);
    std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
    let pool=SqlitePoolOptions::new().max_connections(1).connect_with(SqliteConnectOptions::new().filename(&path)).await?;
    let deps=Dependencies{source_policy:digest(b"synthetic"),facts:digest(b"synthetic"),model:digest(b"synthetic"),policy:digest(b"synthetic")};
    let session=store::open(&pool,"measurement",deps,false).await?;
    let mut ledger=Ledger::new(&session.id,session.epoch,session.dependencies,false)?;
    let mut writes=vec![];let mut results=vec![];let mut previous=0;
    for n in 1..=1000 {
        let event=Event{id:format!("event-{n}"),session_id:session.id.clone(),epoch:session.epoch,sequence:n,revision:1,kind:EventKind::SelfSpeech,origin:Origin::Microphone,capture_session:Some("synthetic-capture".into()),utterance_id:Some(format!("utterance-{n}")),replaces_revision:None,text:format!("第{n}条：我只做过模拟；HTTP 504 后状态未知；null 不能当作 0；金额单位为分。"),closed:true,terminal:None,question:None,source_dependencies:BTreeMap::new(),asset_sha256:None};
        let started=Instant::now();let record=ledger.plan_append(&event)?;store::append(&pool,&record).await?;ledger.append(event)?;writes.push(started.elapsed().as_micros());
        if n==150||n==1000 {
            let started=Instant::now();let reference=cp::save(&pool,&cp::Checkpoint::capture(&ledger,&Scheduler::default()),previous).await?;previous=reference.revision;let save_us=started.elapsed().as_micros();
            let mut reads=vec![];for _ in 0..10 {let started=Instant::now();let (restored,_)=cp::load(&pool,&reference).await?;assert_eq!(restored.journal().len(),n as usize);reads.push(started.elapsed().as_micros());}
            let mut selects=vec![];for _ in 0..100 {let started=Instant::now();let _=ledger.workset(&["HTTP".into(),"null".into()]);selects.push(started.elapsed().as_micros());}
            results.push(json!({"raw_events":n,"append_commit_and_ram":percentiles(writes.clone()),"checkpoint_save_us":save_us,"checkpoint_load":percentiles(reads),"local_selection":percentiles(selects)}));
        }
    }
    pool.close().await;
    println!("{}",serde_json::to_string_pretty(&json!({"evidence":"single local synthetic SQLite run; debug build; no network, capture or real meeting; not maximum capacity validation","api_requests":0,"database_bytes":std::fs::metadata(path)?.len(),"results":results}))?);
    Ok(())
}
