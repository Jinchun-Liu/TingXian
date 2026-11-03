//! Real dual-channel Silero pipeline: final correction must contain exactly the
//! same PCM as the original turn, with one final marker and unchanged prefixes.
use anyhow::{ensure,Result};
use app_lib::audio::{pipeline::AudioPipeline,recording_state::{AudioChunk,DeviceType,RecordingState},device_detection::InputDeviceKind};
use tokio::sync::mpsc;
async fn run(pcm:&[f32],refine:bool)->Result<Vec<AudioChunk>> {
    let (input_tx,input_rx)=mpsc::unbounded_channel();
    let state=RecordingState::new();state.set_utterance_refinement(refine);
    let (output_tx,mut output_rx)=app_lib::audio::asr_queue::channel(&state);
    let pipeline=AudioPipeline::new(input_rx,output_tx,state,0,48000,"mic".into(),InputDeviceKind::Unknown,"sys".into(),InputDeviceKind::Unknown)?;
    let task=tokio::spawn(pipeline.run());
    for (id,part) in pcm.chunks(1440).enumerate() {
        for source in [DeviceType::System,DeviceType::Microphone] {
            input_tx.send(AudioChunk {data:part.to_vec(),sample_rate:48000,timestamp:(id+1) as f64*0.03,chunk_id:id as u64,device_type:source,speech_boundary:None,microphone_gate:None,replaces_utterance:false})?;
        }
    }
    drop(input_tx);task.await??;let mut results=vec![];while let Some(part)=output_rx.recv().await {results.push(part.chunk);}Ok(results)
}
#[tokio::main]
async fn main()->Result<()> {
    let path=std::env::args().nth(1).expect("16k f32 PCM fixture required");
    let pcm:Vec<f32>=std::fs::read(path)?.chunks_exact(4).map(|v|f32::from_le_bytes(v.try_into().unwrap())).collect();
    let dense:Vec<f32>=pcm.chunks(480).filter(|p|p.iter().any(|v|v.abs()>0.02)).flatten().copied().collect();
    ensure!(!dense.is_empty(), "silent fixture");
    let input:Vec<f32>=dense.iter().cycle().take(22*16000).flat_map(|v|[*v,*v,*v]).collect();
    let original=run(&input,false).await?;let revised=run(&input,true).await?;
    for source in [DeviceType::System,DeviceType::Microphone] {
        let a:Vec<_>=original.iter().filter(|c|c.device_type==source).collect();let b:Vec<_>=revised.iter().filter(|c|c.device_type==source).collect();
        ensure!(a.len()>1 && a.len()==b.len(),"prefix/ending count changed: original={}, refined={}",a.len(),b.len());
        let expected:Vec<f32>=a.iter().flat_map(|c|c.data.clone()).collect();
        for (old,new) in a.iter().zip(&b).take(a.len()-1) {ensure!(old.data==new.data && !new.replaces_utterance && !new.speech_boundary.unwrap().is_final,"prefix changed/finalized");}
        let final_=b.last().unwrap();ensure!(final_.replaces_utterance && final_.speech_boundary.unwrap().is_final,"missing correction final");
        ensure!(final_.speech_boundary.unwrap().last_voice_ms.is_none(),"forced flush must not invent a voiced endpoint");
        ensure!(final_.data==expected && final_.timestamp==a[0].timestamp,"whole-turn PCM lost/duplicated or timestamp changed");
    }
    let mut natural=input.clone();natural.extend(vec![0.0;3*48000]);
    let natural=run(&natural,true).await?;
    for source in [DeviceType::System,DeviceType::Microphone] {
        let finals:Vec<_>=natural.iter().filter(|c|c.device_type==source && c.speech_boundary.unwrap().is_final).collect();
        ensure!(finals.len()==1,"natural end duplicated");
        let final_=finals[0];let boundary=final_.speech_boundary.unwrap();
        let voice_end=boundary.last_voice_ms.ok_or_else(||anyhow::anyhow!("natural end missing VAD endpoint"))?;
        let padded_end=(final_.timestamp+final_.data.len() as f64/16000.0)*1000.0;
        ensure!((padded_end-voice_end-400.0).abs()<1.0,"last voice estimate still includes post padding");
        ensure!(boundary.timing.is_some() && boundary.timing.unwrap().audio_to_queue_ms.is_none(),"offline fixture must not manufacture live timing");
    }
    println!("PASS: real dual-channel VAD preserves live prefixes; one final replacement per channel contains exact full-turn PCM and start timestamp");Ok(())
}
