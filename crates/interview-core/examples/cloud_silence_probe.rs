//! Manual provider smoke test. Launch in a terminal with input echo disabled.
//! Credential is read once from stdin, never from process arguments or a file.
use interview_core::cloud_asr::{AsrMode, CloudAsrConfig, PreparedTask};
use std::{io,time::Duration};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() {
    let _=rustls::crypto::ring::default_provider().install_default();
    let workspace_id=std::env::args().nth(1).expect("Workspace ID argument required");
    eprintln!("Enter API Key on stdin (disable terminal echo first):");
    let mut key=String::new();io::stdin().read_line(&mut key).unwrap();
    let config=CloudAsrConfig{mode:AsrMode::Aliyun,workspace_id,silence_ms:1300};
    let cancel=CancellationToken::new();
    let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let id=format!("{stamp:032x}");let id=format!("{}-{}-{}-{}-{}",&id[..8],&id[8..12],&id[12..16],&id[16..20],&id[20..]);
    let task=PreparedTask::connect(&config,key.trim(),id,48000,"auto","",&cancel).await;
    key.clear();
    let task=match task {Ok(task)=>task,Err(error)=>{eprintln!("START ERROR: {error}");std::process::exit(1)}};
    let (tx,rx)=mpsc::channel(4);
    let feed=tokio::spawn(async move {
        let mut tick=tokio::time::interval(Duration::from_millis(20));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        for _ in 0..6000 {tick.tick().await;if tx.send(vec![0u8;1920]).await.is_err(){break;}}
    });
    let start=std::time::Instant::now();
    let result=task.stream(rx,cancel,|event|eprintln!("ASR event at {:.1}s: {event:?}",start.elapsed().as_secs_f64())).await;
    feed.abort();
    eprintln!("RESULT after {:.1}s: {result:?}",start.elapsed().as_secs_f64());
    if result.is_err(){std::process::exit(1)}
}
