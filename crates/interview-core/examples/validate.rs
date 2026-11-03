//! Development-only smoke test of the exact library linked by Tauri.
use interview_core::*;
use serde_json::{json,Value};
use std::{path::Path,time::Instant};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main()->std::result::Result<(),Box<dyn std::error::Error>> {
    let args:Vec<_>=std::env::args().collect();
    if args.len()<3 {return Err("Usage: validate PROFILE_ROOT OUTPUT_DIRECTORY [--live]".into());}
    let output=Path::new(&args[2]);std::fs::create_dir_all(output)?;
    let index=Index::open(&output.join("profile.sqlite")).await?;
    let selection=serde_json::from_str(include_str!("../../../interview/selection.example.json"))?;
    let python=std::env::var("INTERVIEW_PYTHON").unwrap_or("python3".into());
    let imported=index.import(&args[1],selection,&python).await?;
    println!("Import: {} successful / {} included, {} chunks, {} failures",imported.success,imported.included,imported.chunks,imported.failures.len());
    let questions:Vec<Value>=serde_json::from_str(include_str!("../../../interview/questions.json"))?;
    let mut results=vec![];
    let key=std::env::var("DEEPSEEK_API_KEY").unwrap_or_default();
    for q in questions {
        let question=q["question"].as_str().unwrap();let start=Instant::now();
        let review=index.search_review(question,DEFAULT_ALIASES,None).await?;
        let evidence=review.evidence;
        let ms=start.elapsed().as_millis();
        let matched=q["expected"].as_str().map(|needle|evidence.iter().any(|e|e.text.contains(needle)));
        let mut item=json!({"id":q["id"],"question":question,"expected":q["expected"],"source_ids":evidence.iter().map(|e|&e.source_id).collect::<Vec<_>>(),"local_difference_hint_count":review.conflicts.len(),"keyword_hit_not_human_acceptance":matched,"retrieval_ms":ms});
        item["original_query_terms"]=json!(index.tokens(question));
        let live_id=std::env::var("INTERVIEW_LIVE_ID").ok().and_then(|s|s.parse::<u64>().ok());
        if args.iter().any(|a|a=="--live") && q["id"].as_u64().unwrap()<=10 && live_id.map_or(true,|id|Some(id)==q["id"].as_u64()) {
            let mut text=String::new();let mut timings=Value::Null;
            let result=stream_answer(&key,messages(question,"",&[],&evidence)?,CancellationToken::new(),|e|match e {
                StreamEvent::Delta{text:delta}=>text.push_str(&delta),
                StreamEvent::Done{usage,elapsed_ms,first_text_ms}=>timings=json!({"usage":usage,"total_ms":elapsed_ms,"first_text_ms":first_text_ms}),
                _=>{},
            }).await;
            item["suggestion"]=json!(text);item["metrics"]=timings;item["error"]=json!(result.err());
        }
        println!("Question {}: retrieval {} ms, keyword hit {:?}, live {}",q["id"],ms,matched,item.get("metrics").is_some());
        results.push(item);
    }
    std::fs::write(output.join("validation.json"),serde_json::to_string_pretty(&json!({"import":imported,"results":results,"scope":"Rust library / real selected files; no desktop or Feishu acceptance implied"}))?)?;
    Ok(())
}
