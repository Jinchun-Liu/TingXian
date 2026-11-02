//! Real Silero pipeline regression with deterministic PCM; no live audio/API.
use anyhow::{ensure, Result};
use app_lib::audio::{pipeline::AudioPipeline, microphone_gate::{MicrophoneGate, allowed},
    recording_state::{AudioChunk, DeviceType, RecordingState}, device_detection::InputDeviceKind};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    let path = std::env::args().nth(1).expect("PCM fixture argument required");
    let pcm: Vec<f32> = std::fs::read(path)?.chunks_exact(4).map(|v| f32::from_le_bytes(v.try_into().unwrap())).collect();
    let dense: Vec<f32> = pcm.chunks(480).filter(|frame|frame.iter().any(|v|v.abs()>0.02)).flatten().copied().collect();
    ensure!(!dense.is_empty(), "silent fixture");
    let (input_tx,input_rx)=mpsc::unbounded_channel();
    let (output_tx,mut output_rx)=mpsc::unbounded_channel();
    let state=RecordingState::new();
    let old=state.microphone_gate();
    let pipeline=AudioPipeline::new(input_rx,output_tx,state.clone(),0,48000,"test mic".into(),InputDeviceKind::Unknown,"test system".into(),InputDeviceKind::Unknown)?;
    let task=tokio::spawn(pipeline.run());
    let speech:Vec<f32>=dense.iter().cycle().take(16000*12).flat_map(|v|[*v,*v,*v]).collect();
    let send = |data:&[f32], start:f64, gate:std::sync::Arc<MicrophoneGate>| -> Result<()> {
        for (id,frame) in data.chunks(1440).enumerate() {
            for source in [DeviceType::System,DeviceType::Microphone] {
                let microphone_gate=if source==DeviceType::Microphone {Some(gate.clone())}else{None};
                input_tx.send(AudioChunk{data:frame.to_vec(),sample_rate:48000,timestamp:start+(id+1) as f64*0.03,chunk_id:id as u64,device_type:source,speech_boundary:None,microphone_gate})?;
            }
        }
        Ok(())
    };
    send(&speech,0.0,old.clone())?;
    let mut queued=vec![];
    // Wait for an old microphone ASR prefix, proving mute happens after VAD,
    // with both emitted-prefix and buffered-tail cases in play.
    loop {
        let chunk=tokio::time::timeout(std::time::Duration::from_secs(20),output_rx.recv()).await?.unwrap();
        let mic=chunk.device_type==DeviceType::Microphone;
        queued.push(chunk);
        if mic {break;}
    }
    state.disable_microphone();
    ensure!(!allowed(&queued.last().unwrap().microphone_gate),"queued prefix survived mute");
    send(&speech,12.0,old.clone())?; // Simulate callbacks queued during teardown.
    state.renew_microphone();
    let fresh=state.microphone_gate();
    ensure!(fresh.epoch!=old.epoch && !old.enabled(),"reopen revived revoked audio");
    send(&speech,24.0,fresh.clone())?;
    drop(input_tx);task.await??;
    while let Some(chunk)=output_rx.recv().await {queued.push(chunk);}
    let system:Vec<_>=queued.iter().filter(|c|c.device_type==DeviceType::System).collect();
    let mic:Vec<_>=queued.iter().filter(|c|c.device_type==DeviceType::Microphone && allowed(&c.microphone_gate)).collect();
    ensure!(system.iter().map(|c|c.data.len()).sum::<usize>()>30*16000,"mic toggle disrupted system stream");
    ensure!(!mic.is_empty(),"reopened microphone missing");
    ensure!(mic.iter().all(|c|c.timestamp>=24.0 && c.microphone_gate.as_ref().unwrap().epoch==fresh.epoch),"old buffered speech joined new microphone epoch");
    ensure!(mic.iter().filter(|c|c.speech_boundary.unwrap().is_final).count()==1,"new microphone speech boundary missing");
    println!("PASS: queued old ASR prefix revoked; callbacks after mute discarded; reopening resets VAD/timestamps; system audio continues across mute; new mic ends once");
    Ok(())
}
