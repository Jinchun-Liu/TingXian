//! Offline C5 request planner. There is intentionally no network/run mode and
//! no credential lookup. Model output is an untrusted optional input, never a
//! synthetic stand-in for a successful AI2 call.
use interview_core::{compaction, digest, request_budget::*, session_memory::*, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::Read};

const CASES:&str=include_str!("../../../validation/context-compaction/cases.json");
#[derive(Deserialize)]
struct Case {id:String,split:String,kind:String,question:String,terms:Vec<String>,old_statement:String,image:bool}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {case_id:String,image:Option<String>,image_sha256:Option<String>,candidate:Option<Candidate>}
fn event(n:u64,text:String)->Event {Event{id:format!("event-{n}"),session_id:"context-trial".into(),epoch:1,sequence:n,revision:1,kind:if n==1{EventKind::SelfSpeech}else{EventKind::RemoteSpeech},origin:if n==1{Origin::Microphone}else{Origin::SystemAudio},capture_session:Some("synthetic-capture".into()),utterance_id:Some(format!("utterance-{n}")),replaces_revision:None,text,closed:true,terminal:None,question:None,source_dependencies:BTreeMap::new(),asset_sha256:None}}
fn ledger(case:&Case)->Result<Ledger> {
    let deps=Dependencies{source_policy:digest(b"c5-fictional-sources-v1"),facts:digest(b"no-personal-fact-card"),model:digest(b"deepseek-flash"),policy:digest(b"legacy-extractive-v1")};
    let mut l=Ledger::new("context-trial",1,deps,false)?;
    l.append(event(1,case.old_statement.clone()))?;
    for n in 2..=if case.kind=="short"{4}else{40}{l.append(event(n,format!("这是不相关的议程片段 {n}。")))?;}
    Ok(l)
}
fn prepare_answer(mut messages:Value,png:Option<&[u8]>)->Result<Value> {
    let rules=messages[0]["content"].as_str().ok_or("missing system")?;
    messages[0]["content"]=json!(format!("{rules}\n{}",interview_core::ANSWER_FIRST_RULES));
    if let Some(png)=png{messages=interview_core::attach_screenshot(messages,png)?;}
    let config=interview_core::RequestConfig::default();
    interview_core::redact_json_credentials(&mut messages,None)?;
    interview_core::context_budget::fit_text(&mut messages,&config.budget,interview_core::MAX_ANSWER_TOKENS.into())?;
    let mut payload=json!({"model":config.model,"thinking":{"type":"disabled"},"stream":true,"stream_options":{"include_usage":true},"max_tokens":config.budget.output(interview_core::MAX_ANSWER_TOKENS.into()),"messages":messages});
    // Match the real answer transport, which reports unknown image accounting.
    let measurement=fit(&mut payload,&config.budget,WindowBasis::ConfiguredUnknown,None)?;
    Ok(json!({"payload":payload,"measurement":measurement,"max_attempts":2,"output_cap":4096,"total_timeout_secs":config.total_timeout_secs}))
}
fn prepare(input:Input)->Result<Value> {
    let cases:Vec<Case>=serde_json::from_str(CASES).map_err(|e|e.to_string())?;
    let case=cases.into_iter().find(|c|c.id==input.case_id).ok_or("unknown frozen case")?;
    let png=match (case.image,input.image,input.image_sha256) {
        (true,Some(path),Some(hash))=>{
            let meta=std::fs::symlink_metadata(&path).map_err(|_|"cannot inspect PNG")?;
            if !meta.is_file()||meta.len()>interview_core::MAX_SCREENSHOT_BYTES as u64{return Err("expected a regular PNG up to 10 MiB".into());}
            let bytes=std::fs::read(path).map_err(|_|"cannot read PNG")?;
            if digest(&bytes)!=hash{return Err("PNG hash changed".into());}
            // attach_screenshot decodes and validates it, not just the header.
            Some(bytes)
        },
        (false,None,None)=>None,
        _=>return Err("vision cases require a real hashed PNG; text cases must not carry images".into()),
    };
    let mut l=ledger(&case)?;
    let work=l.workset(&case.terms);
    let spoken:Vec<_>=work.recent_events.iter().map(|e|interview_core::SpokenTurn{source:if e.origin==Origin::Microphone{"Microphone"}else{"System"}.into(),text:e.text.clone()}).collect();
    let a=interview_core::messages_with_dialogue(&case.question,"",&[],&[],&spoken)?;
    let mut b=a.clone();append_workset(&mut b,&work)?;
    let a=prepare_answer(a,png.as_deref())?;let b=prepare_answer(b,png.as_deref())?;
    let (ai2,c)=if case.kind=="short" {
        if input.candidate.is_some(){return Err("short cases must not create an AI2 task".into());}
        (Value::Null,json!({"state":"no_compaction_needed","request":b}))
    }else{
        let snapshot=l.snapshot(28,8)?;
        let profile=compaction::Config::default().profile;
        let (payload,measurement)=compaction::prepare(&snapshot,&profile)?;
        let ai2=json!({"snapshot":snapshot,"payload":payload,"measurement":measurement,"max_attempts":1,"output_cap":8192,"total_timeout_secs":profile.request.total_timeout_secs});
        let c=if let Some(candidate)=input.candidate {
            let candidate_hash=digest(&serde_json::to_vec(&candidate).map_err(|e|e.to_string())?);
            l.promote(snapshot,candidate)?;
            let mut messages=interview_core::messages_with_dialogue(&case.question,"",&[],&[],&spoken)?;
            append_workset(&mut messages,&l.workset(&case.terms))?;
            json!({"state":"validated_candidate_not_evidence_of_model_call","candidate_sha256":candidate_hash,"request":prepare_answer(messages,png.as_deref())?})
        }else{json!({"state":"awaiting_real_ai2","request":null,"fallback_request":b,"max_attempts":2,"output_cap":4096,"input_reservation_tokens":262144-4096,"reservation_basis":"whole configured input window including safety; not expected usage"})};
        (ai2,c)
    };
    Ok(json!({"version":1,"case_id":case.id,"split":case.split,"kind":case.kind,"case_corpus_sha256":digest(CASES.as_bytes()),"source_version":"c5-fictional-sources-v1","model":"deepseek-flash","endpoint":interview_core::DEEPSEEK_ENDPOINT,"protocol":"deepseek","strategy":"legacy","harness2_enabled":false,"app_window":262144,"image_sha256":png.as_ref().map(|p|digest(p)),"A":a,"B":b,"C":c,"AI2":ai2,"external_calls":0,"quality":null}))
}
fn main()->std::result::Result<(),Box<dyn std::error::Error>> {
    if std::env::args_os().len()!=1{return Err("offline only: JSON input on stdin; no execution flags".into());}
    let mut raw=String::new();std::io::stdin().take(128*1024+1).read_to_string(&mut raw)?;
    if raw.len()>128*1024{return Err("input exceeds 128 KiB".into());}
    println!("{}",serde_json::to_string(&prepare(serde_json::from_str(&raw)?)?)?);Ok(())
}
#[cfg(test)]mod tests {
    use super::*;
    fn input(id:&str)->Input{Input{case_id:id.into(),image:None,image_sha256:None,candidate:None}}
    #[test]fn short_has_no_ai2_and_identical_arms(){let p=prepare(input("D01")).unwrap();assert!(p["AI2"].is_null());assert_eq!(p["A"],p["B"]);assert_eq!(p["B"],p["C"]["request"]);}
    #[test]fn long_defers_c_and_requires_current_exact_sources(){
        let p=prepare(input("D02")).unwrap();assert!(p["C"]["request"].is_null());
        assert!(!p["A"]["payload"].to_string().contains("没有处理过真实生产事故"));
        assert!(p["B"]["payload"].to_string().contains("没有处理过真实生产事故"));
        let snapshot:Snapshot=serde_json::from_value(p["AI2"]["snapshot"].clone()).unwrap();
        let candidate=Candidate{schema_version:1,snapshot_id:snapshot.id().unwrap(),finish_reason:"stop".into(),items:snapshot.events.iter().map(|e|MemoryItem{kind:e.kind.clone(),origin:e.origin.clone(),refs:vec![e.reference().unwrap()],quote:e.text.clone()}).collect(),omitted:vec![]};
        let mut i=input("D02");i.candidate=Some(candidate.clone());let c=prepare(i).unwrap();assert!(c["C"]["request"]["payload"].to_string().contains("没有处理过真实生产事故"));
        let mut bad=candidate.clone();bad.items[0].quote="处理过真实生产事故".into();let mut i=input("D02");i.candidate=Some(bad);assert!(prepare(i).is_err());
        let mut i=input("H02");i.candidate=Some(candidate);assert!(prepare(i).is_err());
    }
    #[test]fn rejects_missing_or_placeholder_images_and_undeclared_case(){assert!(prepare(input("D03")).is_err());assert!(prepare(input("not-frozen")).is_err());}
    #[tokio::test]async fn planned_payload_matches_real_transport(){
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap();
        let server=tokio::spawn(async move{let(mut stream,_)=listener.accept().await.unwrap();let mut data=vec![];let mut buf=[0;8192];let (offset,length)=loop{let n=stream.read(&mut buf).await.unwrap();assert!(n>0);data.extend_from_slice(&buf[..n]);if let Some(i)=data.windows(4).position(|b|b==b"\r\n\r\n"){let head=String::from_utf8_lossy(&data[..i]).to_lowercase();let n:usize=head.lines().find_map(|l|l.strip_prefix("content-length: ")).unwrap().parse().unwrap();break(i+4,n);}};while data.len()<offset+length{let n=stream.read(&mut buf).await.unwrap();assert!(n>0);data.extend_from_slice(&buf[..n]);}let body:Value=serde_json::from_slice(&data[offset..offset+length]).unwrap();let answer="data: {\"choices\":[{\"delta\":{\"content\":\"先核对条件。\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",answer.len(),answer).as_bytes()).await.unwrap();body});
        let p=prepare(input("D02")).unwrap();let request=&p["B"]["payload"];
        let mut profile=interview_core::ProviderConfig::initial(Default::default()).answers.remove(0);profile.endpoint=format!("http://{addr}/chat/completions");
        interview_core::stream_answer_with_provider("",request["messages"].clone(),tokio_util::sync::CancellationToken::new(),&profile,|_|{}).await.unwrap();
        assert_eq!(server.await.unwrap(),*request);
    }
}
