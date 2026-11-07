//! Synthetic local settings-resolution timing; no application DB or API access.
use interview_core::{background::{Bundle,Doc,Profile,Record},background_settings::{Suggestions,Field,Change},Index};
use serde_json::{json,Value};
use std::time::Instant;
fn stats(mut samples:Vec<f64>)->Value {
    samples.sort_by(f64::total_cmp);
    json!({"samples":samples.len(),"p50_ms":samples[samples.len()/2],"p95_ms":samples[(samples.len()*95).div_ceil(100)-1]})
}
#[tokio::main]
async fn main()->Result<(),String> {
    if std::env::args().len()!=1{return Err("synthetic-only; no path/API arguments".into());}
    let root=tempfile::Builder::new().prefix("tingxian-preference-timing-").tempdir().map_err(|e|e.to_string())?.keep();
    let mut results=vec![];
    for count in [10,100,1000,2000] {
        eprintln!("Preparing {count} synthetic sources with four overlapping dependency snapshots");
        let index=Index::open(&root.join(format!("{count}.sqlite"))).await?;
        for (offset,field) in [Field::Role,Field::Organization,Field::Context,Field::Terminology].into_iter().enumerate() {
            let bundle=Bundle {format:"meetily-background-v2".into(),documents:(0..count-offset).map(|i|Doc {id:format!("{:064x}",i+1),hash:"f".repeat(64),name:format!("fictional-{i}.txt"),records:vec![Record {kind:"personal".into(),location:"行1".into(),quote:format!("虚构资料 {i}：只做模拟测试，未用于生产。"),note:"合成计时".into()}]}).collect(),profile:Profile{overview:"虚构背景".into(),role:"虚构岗位".into(),aliases:String::new()},settings:Some(Suggestions{role:"虚构岗位".into(),organization:"虚构组织".into(),context:"虚构背景".into(),terminology:"虚构术语".into(),call_names:String::new()})};
            index.apply_background(&bundle,&index.setting("version").await?,true,Value::Null,false).await?;
            let preview=index.background_settings_preview().await?;
            index.apply_background_settings(preview["version"].as_str().unwrap(),preview["profile_hash"].as_str().unwrap(),&[Change{field,expected:String::new(),value:format!("synthetic-value-{offset}")}]).await?;
        }
        let expected=index.preferences().await?;let mut stored=vec![];let mut resolved=vec![];
        for i in 0..35 {
            let start=Instant::now();let baseline=index.preferences().await?;let saved_ms=start.elapsed().as_secs_f64()*1000.0;
            let start=Instant::now();let effective=index.request_preferences().await?;let resolved_ms=start.elapsed().as_secs_f64()*1000.0;
            if baseline!=expected||effective!=expected{return Err("source resolution changed valid synthetic fields".into());}
            if i>=5{stored.push(saved_ms);resolved.push(resolved_ms);}
        }
        let metadata=index.setting("interview_preference_sources").await?;
        let snapshots=serde_json::from_str::<Value>(&metadata).map_err(|e|e.to_string())?["snapshots"].as_object().unwrap().len();
        results.push(json!({"library_source_documents":count,"dependency_snapshots":snapshots,"dependency_references":(0..4).map(|i|count-i).sum::<usize>(),"stored_metadata_bytes":metadata.len(),"saved_preferences":stats(stored),"resolved_preferences":stats(resolved)}));
    }
    println!("{}",json!({"schema":"preference-source-timing-v1","fixture_root":root,"scope":"debug build, warm local SQLite; 5 warmups then 30 paired reads; four overlapping valid snapshots; excludes model, ASR, network and process startup","new_model_prompt_fields":0,"real_api_requests":0,"results":results}));
    Ok(())
}
