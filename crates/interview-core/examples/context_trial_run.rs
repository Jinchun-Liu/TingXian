//! One frozen C5 task. Default is local planning; only the budget coordinator
//! sets the manifest grant and supplies a process-only credential for --execute.
#[path="context_trial_prepare.rs"] mod plan;
use interview_core::{compaction,digest,session_memory::Snapshot,StreamEvent};
use serde::Deserialize;
use serde_json::{json,Value};
use std::io::{Read,Write};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Task {job_id:String,role:String,arm:Option<String>,input:Value,fallback_raw:bool,expected_payload_sha256:String,manifest_sha256:String}
fn line(value:Value){let mut out=std::io::stdout().lock();if writeln!(out,"{value}").and_then(|_|out.flush()).is_err(){std::process::exit(2);}}
fn selected(task:&Task)->interview_core::Result<(Value,Value)> {
    if task.manifest_sha256.len()!=64||!task.manifest_sha256.bytes().all(|b|b.is_ascii_hexdigit()){return Err("invalid manifest grant".into());}
    let p=plan::prepare_value(task.input.clone())?;
    let id=p["case_id"].as_str().ok_or("missing case")?;
    let request=match(task.role.as_str(),task.arm.as_deref(),task.fallback_raw){
        ("AI2",None,false) if task.job_id==format!("{id}-AI2")&&!p["AI2"].is_null()=>p["AI2"].clone(),
        ("AI1",Some(arm),fallback) if ["A","B","C"].contains(&arm)&&task.job_id==format!("{id}-{arm}")=>{
            if fallback && (arm!="C"||p["AI2"].is_null()||task.input.get("candidate").is_some()){return Err("invalid fallback".into());}
            if fallback {p["B"].clone()} else if arm=="C"{p["C"]["request"].clone()}else{p[arm].clone()}
        },_=>return Err("task does not match frozen case role".into()),
    };
    if request.is_null()||request["payload_sha256"]!=task.expected_payload_sha256||digest(&serde_json::to_vec(&request["payload"]).map_err(|e|e.to_string())?)!=task.expected_payload_sha256{return Err("task payload changed or AI2 has not been validated".into());}
    Ok((p,request))
}
async fn execute(task:Task,key:&str,cancel:CancellationToken)->interview_core::Result<()> {
    let (_,request)=selected(&task)?;
    let start=std::time::Instant::now();
    if task.role=="AI2" {
        let snapshot:Snapshot=serde_json::from_value(request["snapshot"].clone()).map_err(|e|e.to_string())?;
        if cancel.is_cancelled(){line(json!({"type":"terminal","complete":false,"attempts":0,"error":"cancelled before dispatch","elapsed_ms":0}));return Ok(());}
        // Dispatch is recorded before transport; on interrupted output the
        // coordinator conservatively keeps the entire single-attempt reserve.
        line(json!({"type":"event","at_ms":start.elapsed().as_millis(),"event":{"type":"request_sent","attempt":1}}));
        let outcome=compaction::run(&compaction::Config::default().profile,key,&snapshot,cancel).await;
        if let Some(u)=&outcome.usage{line(json!({"type":"event","at_ms":start.elapsed().as_millis(),"event":{"type":"usage","attempt":1,"usage":{"prompt_tokens":u.input_tokens,"completion_tokens":u.output_tokens,"completion_tokens_details":{"reasoning_tokens":u.reasoning_tokens},"prompt_cache_hit_tokens":u.cache_hit_tokens,"prompt_cache_miss_tokens":u.cache_miss_tokens}}}));}
        let mut candidate=outcome.candidate;let mut error=outcome.error;
        if let Some(c)=candidate.as_ref(){let mut input=task.input.clone();input["candidate"]=json!(c);if let Err(e)=plan::prepare_value(input){error=Some(format!("candidate rejected: {e}"));candidate=None;}}
        let complete=candidate.is_some()&&error.is_none();
        line(json!({"type":"terminal","complete":complete,"attempts":1,"candidate":candidate,"error":error.map(|e|interview_core::redact_credentials(&e,Some(key))),"transport_started":outcome.transport_started,"cancelled":outcome.cancelled,"elapsed_ms":start.elapsed().as_millis(),"first_meaningful_answer_ms":null}));
    }else{
        let mut profile=interview_core::ProviderConfig::initial(Default::default()).answers.remove(0);
        if task.input.get("image").is_some(){profile.id="vision".into();}
        let mut attempts=0;
        let result=interview_core::stream_answer_with_provider(key,request["payload"]["messages"].clone(),cancel,&profile,|event|{
            if let StreamEvent::RequestSent{attempt}=&event{attempts=*attempt;}
            line(json!({"type":"event","at_ms":start.elapsed().as_millis(),"event":event}));
        }).await;
        line(json!({"type":"terminal","complete":result.is_ok(),"attempts":attempts,"error":result.err().map(|e|interview_core::redact_credentials(&e,Some(key))),"elapsed_ms":start.elapsed().as_millis(),"first_meaningful_answer_ms":null,"timing_note":"delta timestamps are observations; a human must identify the first substantive answer, not a filler token"}));
    }Ok(())
}
#[tokio::main]
async fn main()->Result<(),Box<dyn std::error::Error>> {
    let args:Vec<_>=std::env::args().skip(1).collect();if args.len()>1||args.first().is_some_and(|a|a!="--execute"){return Err("context_trial_run [--execute], frozen task JSON on stdin".into());}
    let mut raw=String::new();std::io::stdin().take(128*1024+1).read_to_string(&mut raw)?;if raw.len()>128*1024{return Err("task too large".into());}
    let task:Task=serde_json::from_str(&raw)?;let (_,request)=selected(&task)?;
    if args.is_empty(){line(json!({"type":"prepared","external_calls":0,"request":request}));return Ok(());}
    if std::env::var("TINGXIAN_C5_AUTHORIZED_MANIFEST").ok().as_deref()!=Some(&task.manifest_sha256){return Err("run through the C5 consent and durable-budget coordinator".into());}
    let key=std::env::var("TINGXIAN_C5_API_KEY").map_err(|_|"missing process-only C5 credential")?;if key.trim().is_empty(){return Err("empty credential".into());}
    let cancel=CancellationToken::new();let trigger=cancel.clone();let interrupt=tokio::spawn(async move{if tokio::signal::ctrl_c().await.is_ok(){trigger.cancel();}});
    let result=execute(task,&key,cancel).await;interrupt.abort();result.map_err(Into::into)
}
#[cfg(test)]mod tests {
    use super::*;
    fn task()->Task {let input=json!({"case_id":"D02"});let p=plan::prepare_value(input.clone()).unwrap();Task{job_id:"D02-B".into(),role:"AI1".into(),arm:Some("B".into()),input,fallback_raw:false,expected_payload_sha256:p["B"]["payload_sha256"].as_str().unwrap().into(),manifest_sha256:"a".repeat(64)}}
    #[test]fn frozen_role_payload_and_unvalidated_c_are_enforced(){let mut t=task();selected(&t).unwrap();t.expected_payload_sha256="b".repeat(64);assert!(selected(&t).is_err());let mut t=task();t.arm=Some("C".into());t.job_id="D02-C".into();assert!(selected(&t).is_err());t.fallback_raw=true;selected(&t).unwrap();t.job_id="H02-C".into();assert!(selected(&t).is_err());}
}
