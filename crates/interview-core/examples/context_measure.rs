//! Offline paired assembly only. C uses a deterministic protocol fixture, NOT
//! model inference; no response quality, actual token, latency or fee claims.
use interview_core::{session_memory::*,request_budget::*,digest,compaction};
use serde::Deserialize;
use serde_json::{json,Value};
use std::collections::BTreeMap;
#[derive(Deserialize)]struct Case{id:String,split:String,kind:String,question:String,terms:Vec<String>,old_statement:String,required:Vec<String>,image:bool}
fn event(n:u64,text:String,own:bool)->Event {Event{id:format!("event-{n}"),session_id:"fixture".into(),epoch:1,sequence:n,revision:1,kind:if own{EventKind::SelfSpeech}else{EventKind::RemoteSpeech},origin:if own{Origin::Microphone}else{Origin::SystemAudio},capture_session:Some("fixture-capture".into()),utterance_id:Some(format!("utterance-{n}")),replaces_revision:None,text,closed:true,terminal:None,question:None,source_dependencies:BTreeMap::new(),asset_sha256:None}}
fn add(messages:&mut Value,name:&str,value:Value){if value.as_array().is_some_and(|a|a.is_empty()){return;}let mut context:Value=serde_json::from_str(messages[1]["content"].as_str().unwrap()).unwrap();context[name]=value;messages[1]["content"]=context.to_string().into();}
fn measure(messages:Value,image:bool)->(Value,RequestMeasurement){let mut payload=json!({"model":"synthetic-same-model","max_tokens":4096,"stream":true,"thinking":{"type":"disabled"},"messages":messages});if image{let text=payload["messages"][1]["content"].as_str().unwrap().to_string();payload["messages"][1]["content"]=json!([{"type":"text","text":text},{"type":"image_url","image_url":{"url":"data:image/png;base64,synthetic-budget-only-not-recognition"}}]);}let m=fit(&mut payload,&Default::default(),WindowBasis::ConfiguredUnknown,None).unwrap();(payload,m)}
fn main(){
    let cases:Vec<Case>=serde_json::from_str(include_str!("../../../validation/context-compaction/cases.json")).unwrap();let mut results=vec![];
    for case in cases {
        let deps=Dependencies{source_policy:digest(b"source-v1"),facts:digest(b"same-facts"),model:digest(b"synthetic-same-model"),policy:digest(b"extractive-v1")};let mut l=Ledger::new("fixture",1,deps,false).unwrap();
        l.append(event(1,case.old_statement.clone(),true)).unwrap();for n in 2..=40{l.append(event(n,format!("这是不相关的议程片段 {n}。"),false)).unwrap();}
        let spoken:Vec<_>=l.workset(&[]).recent_events.iter().map(|e|interview_core::SpokenTurn{source:if e.origin==Origin::Microphone{"Microphone"}else{"System"}.into(),text:e.text.clone()}).collect();
        let a=interview_core::messages_with_dialogue(&case.question,"",&[],&[],&spoken).unwrap();let mut b=a.clone();let w=l.workset(&case.terms);add(&mut b,"retrieved_dialogue",serde_json::to_value(w.retrieved_events).unwrap());
        let snapshot=l.snapshot(28,16).unwrap();let (_,ai2_estimate)=compaction::prepare(&snapshot,&compaction::Config::default().profile).unwrap();let candidate=Candidate{schema_version:1,snapshot_id:snapshot.id().unwrap(),finish_reason:"stop".into(),items:snapshot.events.iter().map(|e|MemoryItem{kind:e.kind.clone(),origin:e.origin.clone(),refs:vec![e.reference().unwrap()],quote:e.text.clone()}).collect(),omitted:vec![]};l.promote(snapshot,candidate).unwrap();
        let mut times=vec![];let mut c=a.clone();for _ in 0..100{let start=std::time::Instant::now();let _=l.workset(&case.terms);times.push(start.elapsed().as_nanos());}times.sort();
        let w=l.workset(&case.terms);add(&mut c,"session_memory",serde_json::to_value(w.memory).unwrap());add(&mut c,"retrieved_dialogue",serde_json::to_value(w.retrieved_events).unwrap());
        let arms:Vec<_>=[("A_current_short",a),("B_raw_retrieval_candidate",b),("C_extract_fixture_not_model",c)].into_iter().map(|(arm,m)|{let(p,estimate)=measure(m,case.image);let text=p.to_string();json!({"arm":arm,"estimate":estimate,"required_verbatim_present":case.required.iter().filter(|r|text.contains(r.as_str())).count(),"required_count":case.required.len(),"answer_quality":null,"actual_usage":null,"actual_cost":null})}).collect();
        results.push(json!({"id":case.id,"split":case.split,"kind":case.kind,"arms":arms,"ai2_candidate_request_estimate":ai2_estimate,"local_workset_p50_ns":times[49],"local_workset_p95_ns":times[94],"iterations":100}));
    }
    println!("{}",serde_json::to_string_pretty(&json!({"version":1,"evidence":"offline assembly with synthetic input; C is not model compression; no network calls","case_sha256":digest(include_bytes!("../../../validation/context-compaction/cases.json")),"policy":"legacy; Harness2 disabled; optional memory connected in source, default off; this example measures only core assembly","api_requests":0,"actual_token_usage":null,"actual_cost":null,"results":results})).unwrap());
}
