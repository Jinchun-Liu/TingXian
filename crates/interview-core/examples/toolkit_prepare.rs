//! Prepare paired policy inputs only. This executable has no run/API option.
use serde::Deserialize;
use serde_json::json;
use std::io::{Read,Write};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    question:String,
    #[serde(default)] facts:String,
    #[serde(default)] dialogue:Vec<interview_core::SpokenTurn>,
    #[serde(default)] preferences:interview_core::InterviewPreferences,
    #[serde(default)] method_text:String,
}
fn main()->Result<(),Box<dyn std::error::Error>>{
    if std::env::args().len()!=1 {return Err("prepare-only; no API/run flags accepted".into());}
    let mut raw=String::new();std::io::stdin().take(128*1024+1).read_to_string(&mut raw)?;
    if raw.len()>128*1024{return Err("input too large".into());}
    let input:Input=serde_json::from_str(&raw)?;
    let facts=interview_core::FactCard{fact_id:"synthetic-fixture".into(),version:0,previous_version:None,text:input.facts,source_note:"synthetic fixture only".into(),sources:vec![],reason:"paired offline preparation".into(),confirmed_at:None,recorded_at:String::new()};
    let (mut messages,report)=interview_core::toolkit_policy::preview_messages(&input.question,&facts,&[],&[],&input.dialogue,&input.preferences,&input.method_text)?;
    let system=messages[0]["content"].as_str().unwrap();
    messages[0]["content"]=json!(format!("{system}\n{}",interview_core::ANSWER_FIRST_RULES));
    let value=json!({"messages":messages,"report":report});
    writeln!(std::io::stdout(),"{value}")?;
    Ok(())
}
