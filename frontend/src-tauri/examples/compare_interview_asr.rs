//! Serial model/segmentation comparison using the production Whisper path.
//! No microphone, app preferences, remote API or document index is modified.
use anyhow::{ensure,Context,Result};
use app_lib::{whisper_engine::WhisperEngine,audio::vad::ContinuousVadProcessor};
use serde_json::{json,Value};
use std::{path::Path,time::Instant,io::Write};

fn normalized(s:&str)->String {s.chars().filter(|c|c.is_alphanumeric()).flat_map(char::to_lowercase).collect()}
fn edits(reference:&str,actual:&str)->usize {
    let a:Vec<_>=reference.chars().collect();let b:Vec<_>=actual.chars().collect();
    let mut row:Vec<usize>=(0..=b.len()).collect();
    for (i,x) in a.iter().enumerate() {
        let mut previous=row[0];row[0]=i+1;
        for (j,y) in b.iter().enumerate() {let old=row[j+1];row[j+1]=(previous+usize::from(x!=y)).min(row[j]+1).min(old+1);previous=old;}
    }
    row[b.len()]
}
#[tokio::main]
async fn main()->Result<()> {
    let args:Vec<String>=std::env::args().collect();
    ensure!((5..=6).contains(&args.len()),"usage: compare_interview_asr MODELS_DIR MANIFEST MODEL OUTPUT_NDJSON [refined-only]");
    ensure!(args.len()==5 || args[5]=="refined-only", "unknown mode");
    let manifest:Value=serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let cases=manifest["cases"].as_array().context("cases array")?;
    let mut output=std::fs::OpenOptions::new().write(true).create_new(true).open(&args[4])?;
    let engine=WhisperEngine::new_with_models_dir(Some(Path::new(&args[1]).into()))?;
    engine.discover_models().await?;let load=Instant::now();engine.load_model(&args[3]).await?;
    writeln!(output,"{}",json!({"type":"environment","model":args[3],"model_load_ms":load.elapsed().as_millis(),"kind":manifest["kind"],"voice":manifest["voice"],"limitations":manifest["limitations"]}))?;
    for case in cases {
        let bytes=std::fs::read(case["pcm"].as_str().context("pcm path")?)?;
        ensure!(bytes.len()%4==0,"invalid f32 fixture");
        let audio:Vec<f32>=bytes.chunks_exact(4).map(|v|f32::from_le_bytes(v.try_into().unwrap())).collect();
        ensure!(audio.len()>=16000 && audio.iter().all(|v|v.is_finite()) && audio.iter().any(|v|v.abs()>0.01),"empty, invalid or silent speech fixture");
        let reference=case["text"].as_str().context("reference text")?;
        ensure!(audio.len()<=16000*180,"fixture exceeds 180 seconds");
        let mut vad=ContinuousVadProcessor::new_live(16000,1500)?;
        let mut live=vec![];
        for frame in audio.chunks(480) {live.extend(vad.process_audio(frame)?);}
        live.extend(vad.flush()?);
        let modes=if args.len()==6 {vec!["refined"]} else {vec!["whole","live"]};
        for mode in modes {
            let mut refinement=app_lib::audio::utterance_refinement::UtteranceRefinement::new(180*16000);
            let chunks:Vec<(Vec<f32>,f64,bool,bool)>=if mode=="whole" {vec![(audio.clone(),0.0,true,false)]}else{live.iter().map(|s|{
                let (part,replace)=if mode=="refined" {refinement.receive(s.clone())}else{(s.clone(),false)};
                (part.samples,part.start_timestamp_ms/1000.0,part.boundary.is_final,replace)
            }).collect()};
            let started=Instant::now();let mut text=String::new();let mut draft=String::new();let mut details=vec![];
            for (pcm,start,is_final,replace) in chunks {
                if pcm.len()<800 {continue;}
                let seconds=pcm.len() as f64/16000.0;let tick=Instant::now();
                let (part,confidence,partial)=engine.transcribe_audio_with_confidence(pcm,Some("zh".into())).await?;
                if replace {draft=part.clone();} else {if !draft.is_empty(){draft.push(' ');}draft.push_str(&part);}
                if is_final {if !text.is_empty(){text.push(' ');}text.push_str(&draft);draft.clear();}
                details.push(json!({"text":part,"start_s":start,"audio_s":seconds,"inference_ms":tick.elapsed().as_millis(),"confidence":confidence,"is_partial":partial,"is_utterance_end":is_final,"replaces_utterance":replace}));
            }
            if !draft.is_empty() {if !text.is_empty(){text.push(' ');}text.push_str(&draft);}
            let elapsed=started.elapsed().as_millis();let actual=normalized(&text);let expected=normalized(reference);
            let terms=case["terms"].as_array().context("terms array")?;
            let missing:Vec<_>=terms.iter().filter(|t|!actual.contains(&normalized(t.as_str().unwrap()))).collect();
            let result=json!({"type":"case","id":case["id"],"model":args[3],"mode":mode,"reference":reference,"text":text,"audio_s":audio.len() as f64/16000.0,"inference_ms":elapsed,"character_errors":edits(&expected,&actual),"reference_characters":expected.chars().count(),"terms":terms.len(),"missing_terms":missing,"segments":details,"human_semantics_review":"required"});
            writeln!(output,"{}",result)?;output.flush()?;
            println!("model={} case={} mode={} inference_ms={} missing_terms={}",args[3],case["id"],mode,elapsed,missing.len());
        }
    }
    engine.unload_model().await;Ok(())
}
