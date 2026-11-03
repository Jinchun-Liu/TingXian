//! Opt-in: installed local model + synthetic f32le speech, no mic/API/settings.
use anyhow::{ensure,Context,Result};
use app_lib::whisper_engine::WhisperEngine;
use std::{sync::{Arc,atomic::{AtomicBool,Ordering}},time::{Duration,Instant}};

#[tokio::main]
async fn main()->Result<()> {
    let args:Vec<_>=std::env::args_os().collect();
    ensure!(args.len()==4,"usage: verify_whisper_cancel MODELS_DIR SPEECH.f32 MODEL");
    let bytes=std::fs::read(&args[2])?;ensure!(bytes.len()%4==0,"invalid f32le fixture");
    let speech:Vec<_>=bytes.chunks_exact(4).map(|v|f32::from_le_bytes(v.try_into().unwrap())).collect();
    ensure!(speech.len()>=16000 && speech.iter().all(|v|v.is_finite()),"invalid speech fixture");
    let engine=WhisperEngine::new_with_models_dir(Some((&args[1]).into()))?;
    engine.discover_models().await?;let model=args[3].to_str().context("model name")?;
    engine.load_model(model).await?;
    let flag=Arc::new(AtomicBool::new(false));let trigger=flag.clone();
    let long:Vec<f32>=speech.iter().copied().cycle().take(180*16000).collect();
    let cancel=std::thread::spawn(move||{std::thread::sleep(Duration::from_millis(100));let at=Instant::now();trigger.store(true,Ordering::SeqCst);at});
    let result=engine.transcribe_audio_cancellable(long,Some("zh".into()),Some(flag.clone())).await;
    let finished=Instant::now();let cancelled=cancel.join().unwrap();
    ensure!(result.is_err(),"cancelled decode returned apparently complete text");
    let cancellation_ms=finished.saturating_duration_since(cancelled).as_millis();
    ensure!(cancellation_ms<5000,"native decode did not exit within the cancellation grace");
    ensure!(Arc::strong_count(&flag)==1,"callback retained its cancellation flag");
    ensure!(engine.unload_model().await,"model could not unload after cancellation");
    engine.load_model(model).await?;
    let fresh=Arc::new(AtomicBool::new(false));
    let (text,_,partial)=engine.transcribe_audio_cancellable(speech,Some("zh".into()),Some(fresh.clone())).await?;
    ensure!(!partial && !text.trim().is_empty(),"new session did not produce a complete transcript");
    ensure!(flag.load(Ordering::SeqCst) && !fresh.load(Ordering::SeqCst),"old/new cancellation flags were mixed");
    ensure!(Arc::strong_count(&fresh)==1,"normal decode retained its cancellation flag");
    ensure!(engine.unload_model().await,"final model unload failed");
    println!("{}",serde_json::json!({"model":model,"synthetic":true,"cancelled_decode_seconds":180,"cancel_to_return_ms":cancellation_ms,"callback_flag_released":true,"unload_reload_succeeded":true,"fresh_transcript_chars":text.chars().count(),"fresh_transcript_final":!partial}));
    Ok(())
}
