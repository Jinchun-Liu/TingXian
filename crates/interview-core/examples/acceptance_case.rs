//! One synthetic acceptance case through production context and stream code.
//! Default is prepare-only: no credential read and no network access.
//! The Python coordinator owns scope, consent, durable budget and review files.
use interview_core::{ProviderConfig,RequestConfig,SpokenTurn,StreamEvent,Turn};
use serde::Deserialize;
use serde_json::{json,Value};
use sha2::{Digest,Sha256};
use std::io::{Read,Write};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    question:String,
    facts:String,
    #[serde(default)] dialogue:Vec<SpokenTurn>,
    #[serde(default)] history:Vec<Turn>,
    image:Option<String>,
    image_sha256:Option<String>,
}

fn line(value:Value){
    let mut out=std::io::stdout().lock();
    // No unreported request may proceed after the event channel fails.
    if writeln!(out,"{value}").and_then(|_|out.flush()).is_err(){std::process::exit(2);}
}

fn prepare(input:&Input)->interview_core::Result<(Value,Value)> {
    let mut messages=interview_core::messages_with_dialogue(&input.question,&input.facts,&input.history,&[],&input.dialogue)?;
    let system=messages[0]["content"].as_str().ok_or("missing system rules")?;
    messages[0]["content"]=json!(format!("{system}\n{}",interview_core::ANSWER_FIRST_RULES));
    let image_sha=if let Some(path)=&input.image {
        let meta=std::fs::symlink_metadata(path).map_err(|_|"cannot inspect fixture image")?;
        if !meta.is_file() || meta.len()>interview_core::MAX_SCREENSHOT_BYTES as u64 {return Err("fixture image must be a regular PNG within 10 MiB".into());}
        let png=std::fs::read(path).map_err(|_|"cannot read fixture image")?;
        let hash=format!("{:x}",Sha256::digest(&png));
        if input.image_sha256.as_deref()!=Some(hash.as_str()){return Err("fixture image hash mismatch".into());}
        messages=interview_core::attach_screenshot(messages,&png)?;
        Some(hash)
    }else{
        if input.image_sha256.is_some(){return Err("image hash supplied without image".into());}
        None
    };
    let config=RequestConfig::default();
    let has_image=interview_core::context_budget::fit_text(&mut messages,&config.budget,interview_core::MAX_ANSWER_TOKENS.into())?;
    // Count actual transmitted text, not base64 bytes. Use an extra 1024-token
    // text framing reserve. DeepSeek's dated image bound is separately declared.
    let text_bytes:u64=messages.as_array().unwrap().iter().map(|m|match &m["content"] {
        Value::String(s)=>s.len() as u64,
        Value::Array(parts)=>parts.iter().filter_map(|p|p["text"].as_str()).map(|s|s.len() as u64).sum(),
        _=>0,
    }).sum();
    let input_upper=text_bytes+1024+if has_image {1024}else{0};
    let output=config.budget.output(interview_core::MAX_ANSWER_TOKENS.into());
    if input_upper+output+config.budget.safety_tokens>config.budget.window(){return Err("fixture plus image reserve exceeds application window".into());}
    let receipt=json!({"model":config.model,"protocol":"deepseek","thinking":"disabled","endpoint":interview_core::DEEPSEEK_ENDPOINT,
        "text_bytes":text_bytes,"input_upper_tokens":input_upper,"output_cap":output,"image_count":usize::from(has_image),"image_sha256":image_sha,
        "max_attempts":2,"single_attempt_reserve_micro_cny":input_upper*2+output*8,"total_timeout_secs":config.total_timeout_secs,
        "image_bound_reference":"https://api-docs.deepseek.com/guides/vision/","price_reference":"https://api-docs.deepseek.com/zh-cn/quick_start/pricing/","checked_on":"2026-11-07"});
    Ok((messages,receipt))
}

#[tokio::main]
async fn main()->Result<(),Box<dyn std::error::Error>>{
    let args:Vec<_>=std::env::args().skip(1).collect();
    if args.len()>1 || args.first().is_some_and(|a|a!="--run") {return Err("usage: acceptance_case [--run], JSON input on stdin".into());}
    let mut raw=String::new();std::io::stdin().take(1024*1024+1).read_to_string(&mut raw)?;
    if raw.len()>1024*1024 {return Err("fixture input too large".into());}
    let input:Input=serde_json::from_str(&raw)?;
    let (messages,receipt)=prepare(&input)?;
    if args.is_empty(){line(json!({"type":"prepared","receipt":receipt,"messages":messages}));return Ok(());}
    if std::env::var("TINGXIAN_ACCEPTANCE_AUTHORIZED").as_deref()!=Ok("1") {return Err("use the budgeted coordinator and an explicit authorization file".into());}
    let key=std::env::var("TINGXIAN_VALIDATION_API_KEY").map_err(|_|"missing validation credential")?;
    if key.trim().is_empty(){return Err("empty validation credential".into());}
    let mut profile=ProviderConfig::initial(RequestConfig::default()).answers.remove(0);
    if input.image.is_some(){profile.id="vision".into();}
    let cancel=CancellationToken::new();let trigger=cancel.clone();
    let interrupt=tokio::spawn(async move{if tokio::signal::ctrl_c().await.is_ok(){trigger.cancel();}});
    let mut attempts=0;
    let result=interview_core::stream_answer_with_provider(&key,messages,cancel,&profile,|event|{
        if let StreamEvent::RequestSent{attempt}=&event{attempts=*attempt;}
        line(json!({"type":"event","event":event}));
    }).await;
    interrupt.abort();
    line(json!({"type":"terminal","complete":result.is_ok(),"attempts":attempts,
        "error":result.as_ref().err().map(|s|interview_core::redact_credentials(s,Some(&key)))}));
    if result.is_err(){std::process::exit(1);}
    Ok(())
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn prepared_case_uses_production_context_and_separates_suggestion(){
        let i=Input{question:"刚才的方法怎么测试？".into(),facts:"仅本地练习".into(),dialogue:vec![SpokenTurn{source:"Microphone".into(),text:"使用幂等键".into()}],history:vec![Turn{question:"方案".into(),confirmed_answer:None,suggested_answer:Some("使用全局锁".into())}],image:None,image_sha256:None};
        let (m,r)=prepare(&i).unwrap();let text=m.to_string();
        assert!(text.contains("<<<QUESTION>>>"));
        let context:Value=serde_json::from_str(m[1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(context["recent_spoken_dialogue"][0]["transcript"],"使用幂等键");
        assert!(context["recent_turns"][0]["confirmed_actual_answer"].is_null());
        assert_eq!(context["recent_turns"][0]["assistant_suggestion"],"使用全局锁");
        assert!(context["dialogue_source_note"].as_str().unwrap().contains("不是任何参与者发言或已确认事实"));
        assert_eq!(r["model"],"deepseek-flash");assert_eq!(r["max_attempts"],2);assert_eq!(r["image_count"],0);
        assert_eq!(r["single_attempt_reserve_micro_cny"].as_u64().unwrap(),r["input_upper_tokens"].as_u64().unwrap()*2+4096*8);
    }
}
