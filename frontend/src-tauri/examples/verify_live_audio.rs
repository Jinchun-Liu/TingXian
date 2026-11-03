//! Local integration regression: real Silero VAD, no API or device capture.
use anyhow::{ensure, Result};
use app_lib::audio::{vad::ContinuousVadProcessor, pipeline::AudioPipeline,
    recording_state::{AudioChunk, DeviceType, RecordingState}, device_detection::InputDeviceKind};
use tokio::sync::mpsc;

fn read_audio(path: &str) -> Result<Vec<f32>> {
    Ok(std::fs::read(path)?.chunks_exact(4).map(|v| f32::from_le_bytes(v.try_into().unwrap())).collect())
}
#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(args.len() == 2, "usage: verify_live_audio SPEECH_16K.f32");
    let audio = read_audio(&args[1])?;
    let dense: Vec<f32> = audio.chunks(480).filter(|frame| frame.iter().any(|v| v.abs() > 0.02)).flatten().copied().collect();
    let continuous = dense.repeat((125 * 16000 / dense.len()) + 1);
    let mut batch = ContinuousVadProcessor::new(16000, 1500)?;
    let mut live = ContinuousVadProcessor::new_live(16000, 1500)?;
    let mut expected = vec![];
    let mut actual = vec![];
    for chunk in continuous.chunks(1234) {
        expected.extend(batch.process_audio(chunk)?);
        actual.extend(live.process_audio(chunk)?);
    }
    expected.extend(batch.flush()?);
    actual.extend(live.flush()?);
    ensure!(actual.len() > expected.len(), "fixture did not exercise continuous-speech splitting");
    let expected_pcm: Vec<f32> = expected.iter().flat_map(|s|s.samples.clone()).collect();
    let actual_pcm: Vec<f32> = actual.iter().flat_map(|s|s.samples.clone()).collect();
    ensure!(expected_pcm == actual_pcm, "live split duplicated or lost samples");
    ensure!(actual.iter().all(|s|s.samples.len() <= 10 * 16000), "unbounded live segment");
    ensure!(actual.windows(2).all(|s|s[1].start_timestamp_ms >= s[0].end_timestamp_ms - 0.1), "timestamps overlap/reverse");
    for segment in &expected {
        let parts: Vec<_> = actual.iter().filter(|p| p.boundary.utterance_id == segment.boundary.utterance_id).collect();
        ensure!(!parts.is_empty() && parts.last().unwrap().boundary.is_final, "missing utterance end");
        ensure!(parts.iter().filter(|p| p.boundary.is_final).count() == 1, "prefix incorrectly ended utterance");
    }
    println!("PASS: {:.1}s continuous speech, {} live segments; identical PCM and monotonic timestamps", continuous.len() as f64/16000.0,actual.len());

    for (silence_samples, expected_turns) in [(11200, 1), (40000, 2)] {
        let mut vad = ContinuousVadProcessor::new_live(16000, 1500)?;
        let fixture = [dense.clone(), vec![0.0; silence_samples], dense.clone(), vec![0.0; 40000]].concat();
        let mut segments = vec![];
        for frame in fixture.chunks(480) { segments.extend(vad.process_audio(frame)?); }
        segments.extend(vad.flush()?);
        ensure!(segments.iter().filter(|s|s.boundary.is_final).count() == expected_turns,
            "pause {}ms produced wrong number of speech ends", silence_samples / 16);
    }
    println!("PASS: 700ms thinking pause stays in one utterance; 2500ms silence closes it; prefixes never finalize");

    let (input_tx,input_rx) = mpsc::unbounded_channel();
    let state = RecordingState::new();
    let (output_tx,mut output_rx) = app_lib::audio::asr_queue::channel(&state);
    let pipeline = AudioPipeline::new(input_rx,output_tx,state,0,48000,
        "test microphone".into(),InputDeviceKind::Unknown,"test system".into(),InputDeviceKind::Unknown)?;
    let task = tokio::spawn(pipeline.run());
    let stereo_fixture: Vec<f32> = dense.repeat(3).iter().flat_map(|v|[*v,*v,*v]).collect();
    for (id,chunk) in stereo_fixture.chunks(1440).enumerate() {
        for source in [DeviceType::System, DeviceType::Microphone] {
            input_tx.send(AudioChunk {data:chunk.to_vec(),sample_rate:48000,timestamp:(id+1) as f64 * 0.03,chunk_id:id as u64,device_type:source,speech_boundary:None,microphone_gate:None,replaces_utterance:false})?;
        }
    }
    drop(input_tx);
    task.await??;
    let mut system = vec![]; let mut microphone = vec![];
    let mut system_ends = 0; let mut microphone_ends = 0; let mut prefix_count = 0;
    while let Some(accepted)=output_rx.recv().await {
        let chunk=accepted.chunk;
        let boundary = chunk.speech_boundary.expect("pipeline dropped utterance boundary");
        if !boundary.is_final { prefix_count += 1; }
        match chunk.device_type {
            DeviceType::System => { system.extend(chunk.data); if boundary.is_final { system_ends += 1; } },
            DeviceType::Microphone => { microphone.extend(chunk.data); if boundary.is_final { microphone_ends += 1; } }
        }
    }
    ensure!(!system.is_empty() && system==microphone,"capture channels missing or mixed/distorted");
    ensure!(prefix_count >= 2 && system_ends == 1 && microphone_ends == 1, "wrong channel turn boundaries");
    println!("PASS: both capture channels retain their source and independent identical PCM; {} samples each",system.len());
    Ok(())
}
