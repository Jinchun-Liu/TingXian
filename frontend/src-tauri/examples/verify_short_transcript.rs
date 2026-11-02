//! Opt-in integration regression using an installed base model and a short
//! 16 kHz mono f32le speech fixture. Does not download models or contact an API.
use anyhow::{ensure, Context, Result};
use app_lib::whisper_engine::WhisperEngine;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    ensure!((3..=5).contains(&args.len()), "usage: verify_short_transcript MODELS_DIR SPEECH.f32 [MODEL] [LANGUAGE]");
    let bytes = std::fs::read(&args[2])?;
    ensure!(bytes.len() % 4 == 0, "fixture must contain f32le samples");
    let samples: Vec<f32> = bytes.chunks_exact(4)
        .map(|value| f32::from_le_bytes(value.try_into().unwrap())).collect();
    let seconds = samples.len() as f64 / 16000.0;
    ensure!(seconds >= 1.0 && seconds < 15.0, "fixture must be 1–15 seconds");
    let engine = WhisperEngine::new_with_models_dir(Some((&args[1]).into()))?;
    engine.discover_models().await?;
    let model = args.get(3).and_then(|s|s.to_str()).unwrap_or("base");
    let language = args.get(4).and_then(|s|s.to_str()).unwrap_or("en");
    engine.load_model(model).await.context("load installed multilingual model")?;
    let start = std::time::Instant::now();
    let (text, _, is_partial) = engine
        .transcribe_audio_with_confidence(samples, Some(language.into())).await?;
    ensure!(!text.trim().is_empty(), "spoken fixture produced no transcript");
    ensure!(!is_partial, "completed {:.2}s utterance incorrectly marked partial: {}", seconds, text);
    println!("PASS: {:.2}s completed utterance is final; inference={}ms; text={}", seconds, start.elapsed().as_millis(), text);
    engine.unload_model().await;
    Ok(())
}
