use interview_core::{compaction::*,session_memory::*,digest};
use serde_json::{json,Value};
use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt,AsyncWriteExt};
fn ledger()->Ledger {
    let d=Dependencies{source_policy:digest(b"sources"),facts:digest(b"facts"),model:digest(b"model"),policy:digest(b"policy")};let mut l=Ledger::new("synthetic",1,d,false).unwrap();
    for n in 1..=40 {l.append(Event{id:format!("event-{n}"),session_id:"synthetic".into(),epoch:1,sequence:n,revision:1,kind:EventKind::RemoteSpeech,origin:Origin::SystemAudio,capture_session:Some("capture-1".into()),utterance_id:Some(format!("utterance-{n}")),replaces_revision:None,text:format!("第{n}题：null 不能当 0；HTTP 504 后状态未知；仅做过模拟，未处理过生产事故。"),closed:true,terminal:None,question:None,source_dependencies:BTreeMap::new(),asset_sha256:None}).unwrap();}l
}
fn candidate(s:&Snapshot)->Candidate {Candidate{schema_version:1,snapshot_id:s.id().unwrap(),finish_reason:"stop".into(),items:s.events.iter().map(|e|MemoryItem{kind:e.kind.clone(),origin:e.origin.clone(),refs:vec![e.reference().unwrap()],quote:e.text.clone()}).collect(),omitted:vec![]}}
fn success(s:&Snapshot)->Outcome {Outcome{candidate:Some(candidate(s)),usage:Some(Usage{input_tokens:20,output_tokens:10,reasoning_tokens:Some(2),cache_hit_tokens:Some(5),cache_miss_tokens:Some(15)}),error:None,cancelled:false,transport_started:true}}
#[test]fn disabled_consent_foreground_singleflight_cooldown_and_quota_are_independent(){
    let mut l=ledger();let mut s=Scheduler::default();let mut c=Config::default();
    assert!(!c.enabled);assert!(s.begin(&l,&c,true,false,28,100_000).unwrap().is_none());c.enabled=true;
    assert!(s.begin(&l,&c,false,false,28,100_000).unwrap().is_none());assert!(s.begin(&l,&c,true,true,28,100_000).unwrap().is_none());
    let a=s.begin(&l,&c,true,false,28,100_000).unwrap().unwrap();assert!(s.active());assert!(s.begin(&l,&c,true,false,28,200_000).unwrap().is_none());assert_eq!(s.charged_units(),0);
    s.mark_sent(&a.id).unwrap();assert!(s.mark_sent(&a.id).is_err());assert_eq!(s.charged_units(),a.reserved_units);
    s.finish(&a.id,&mut l,&c,success(&a.snapshot)).unwrap();assert_eq!(s.attempts()[0].status,Status::Committed);assert_eq!(s.charged_units(),30);assert_eq!(l.version(),1);
    assert!(s.begin(&l,&c,true,false,28,101_000).unwrap().is_none());assert!(s.finish(&a.id,&mut l,&c,success(&a.snapshot)).is_err());
    let mut s=Scheduler::default();let mut limited=c.clone();limited.session_units=4096;assert!(s.begin(&ledger(),&limited,true,false,28,100_000).unwrap().is_none());
}
#[test]fn failure_unknown_usage_dedup_restart_and_configuration_changes_do_not_promote(){
    let mut l=ledger();let c=Config{enabled:true,..Default::default()};let mut s=Scheduler::default();let a=s.begin(&l,&c,true,false,28,100_000).unwrap().unwrap();s.mark_sent(&a.id).unwrap();
    let restored=Scheduler::restore(s.attempts().to_vec()).unwrap();assert_eq!(restored.attempts()[0].status,Status::Interrupted);assert_eq!(restored.charged_units(),a.reserved_units);assert!(!restored.active());
    s.finish(&a.id,&mut l,&c,Outcome{candidate:None,usage:None,error:Some("429".into()),cancelled:false,transport_started:true}).unwrap();assert_eq!(s.attempts()[0].status,Status::Failed);assert!(s.attempts()[0].usage.is_none());assert_eq!(s.charged_units(),a.reserved_units);assert!(s.begin(&l,&c,true,false,28,500_000).unwrap().is_none());assert_eq!(l.version(),0);
    let mut s=Scheduler::default();let a=s.begin(&l,&c,true,false,28,100_000).unwrap().unwrap();s.mark_sent(&a.id).unwrap();let mut changed=c.clone();changed.profile.request.model="other-model".into();s.finish(&a.id,&mut l,&changed,success(&a.snapshot)).unwrap();assert_eq!(s.attempts()[0].status,Status::Stale);assert_eq!(l.version(),0);
}
#[test]fn malformed_candidate_or_low_gain_keeps_originals_and_old_memory(){
    for bad in ["negation","length","growth"] {let mut l=ledger();let c=Config{enabled:true,..Default::default()};let mut s=Scheduler::default();let a=s.begin(&l,&c,true,false,28,100_000).unwrap().unwrap();s.mark_sent(&a.id).unwrap();let mut out=success(&a.snapshot);let v=out.candidate.as_mut().unwrap();match bad {"negation"=>v.items[0].quote="已经完成生产事故处置".into(),"length"=>v.finish_reason="length".into(),_=>v.items[0].quote="x".repeat(30000)};s.finish(&a.id,&mut l,&c,out).unwrap();assert_ne!(s.attempts()[0].status,Status::Committed);assert_eq!(l.journal().len(),40);assert_eq!(l.version(),0);assert_eq!(s.charged_units(),30);}
}
#[tokio::test]async fn separate_config_migrates_disabled_without_consuming_answer_lane(){
    let dir=tempfile::tempdir().unwrap();let index=interview_core::Index::open(&dir.path().join("profile.sqlite")).await.unwrap();let answers=index.provider_config().await.unwrap();let config=index.compaction_config().await.unwrap();assert!(!config.enabled);assert_eq!(answers.answers.len(),3);assert_eq!(config.profile.id,"compaction");index.set_compaction_config(&config).await.unwrap();assert_eq!(index.compaction_config().await.unwrap(),config);assert_eq!(index.provider_config().await.unwrap(),answers);
    let mut bad=config.clone();bad.profile.id="answer-2".into();assert!(index.set_compaction_config(&bad).await.is_err());assert_eq!(index.compaction_config().await.unwrap(),config);
}
#[test]fn usage_unknown_is_not_zero_and_reasoning_cache_are_not_double_counted(){
    assert!(Usage::parse(&json!({"total_tokens":40})).is_none());assert!(Usage::parse(&json!({"prompt_tokens":10,"completion_tokens":10,"completion_tokens_details":{"reasoning_tokens":11}})).is_none());
    let u=Usage::parse(&json!({"prompt_tokens":100,"completion_tokens":30,"completion_tokens_details":{"reasoning_tokens":20},"prompt_cache_hit_tokens":80,"prompt_cache_miss_tokens":20})).unwrap();assert_eq!(u.total(),130);
    let l=ledger();let snap=l.snapshot(28,16).unwrap();let (p,m)=prepare(&snap,&Config::default().profile).unwrap();assert_eq!(p["thinking"]["type"],"disabled");assert_eq!(p["max_tokens"],8192);assert_eq!(m.output_tokens,8192);assert!(!p.to_string().contains("implementation_prompt"));
}
#[tokio::test]async fn cancellation_before_transport_has_no_charge_and_no_connection(){
    let mut l=ledger();let c=Config{enabled:true,..Default::default()};let mut scheduler=Scheduler::default();let a=scheduler.begin(&l,&c,true,false,28,100_000).unwrap().unwrap();scheduler.mark_sent(&a.id).unwrap();
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let mut p=c.profile.clone();p.endpoint=format!("http://{}/v1/chat/completions",listener.local_addr().unwrap());
    let cancel=tokio_util::sync::CancellationToken::new();cancel.cancel();let out=run(&p,"synthetic-only",&a.snapshot,cancel).await;
    assert!(out.cancelled);assert!(!out.transport_started);scheduler.finish(&a.id,&mut l,&c,out).unwrap();assert_eq!(scheduler.charged_units(),0);assert!(!scheduler.attempts()[0].request_sent);
    assert!(tokio::time::timeout(std::time::Duration::from_millis(50),listener.accept()).await.is_err());
}

#[test]
fn pressure_does_not_bypass_minimum_new_speech_or_retry_each_sentence() {
    let c=Config{enabled:true,..Default::default()};let mut l=ledger();let mut s=Scheduler::default();
    let a=s.begin_for(&l,&c,true,false,28,100_000,Trigger::Capacity).unwrap().unwrap();
    assert_eq!(a.trigger,Trigger::Capacity);s.mark_sent(&a.id).unwrap();
    s.finish(&a.id,&mut l,&c,Outcome{candidate:None,usage:None,error:Some("429".into()),cancelled:false,transport_started:true}).unwrap();
    for n in 41..=48 {
        let mut e=l.event("event-40").unwrap().clone();e.id=format!("event-{n}");e.sequence=n;e.utterance_id=Some(format!("utterance-{n}"));l.append(e).unwrap();
        if n<48 {assert!(s.begin_for(&l,&c,true,false,n-12,500_000,Trigger::Capacity).unwrap().is_none());}
    }
    let next=s.begin_for(&l,&c,true,false,36,500_000,Trigger::Capacity).unwrap().unwrap();
    assert_eq!(next.snapshot.cutoff_sequence,36);assert_ne!(a.id,next.id);

    // Many generated/status events cannot manufacture the required eight old
    // actual utterances. Recent twelve actual utterances remain protected.
    let original=ledger();let mut short=Ledger::new("synthetic",1,original.dependencies().clone(),false).unwrap();
    for n in 1..=19 {short.append(original.event(&format!("event-{n}")).unwrap().clone()).unwrap();}
    for n in 20..=70 {
        let mut e=original.event("event-20").unwrap().clone();e.id=format!("status-{n}");e.sequence=n;e.kind=EventKind::RequestTerminal;e.origin=Origin::Runtime;e.capture_session=None;e.utterance_id=None;e.text="cancelled".into();e.terminal=Some(Terminal::Cancelled);e.question=Some(QuestionVersion{question_id:format!("question-{n}"),revision:1,context_version:0});short.append(e).unwrap();
    }
    assert!(Scheduler::default().begin_for(&short,&c,true,false,70,100_000,Trigger::Capacity).unwrap().is_none());
}

async fn serve(status:u16,value:Value,delay_ms:u64)->(String,tokio::task::JoinHandle<Value>) {
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let url=format!("http://{}/chat/completions",listener.local_addr().unwrap());
    let task=tokio::spawn(async move{let(mut stream,_)=listener.accept().await.unwrap();let mut bytes=vec![];let mut buf=[0;4096];let end=loop{let n=stream.read(&mut buf).await.unwrap();assert!(n>0);bytes.extend_from_slice(&buf[..n]);if let Some(i)=bytes.windows(4).position(|p|p==b"\r\n\r\n"){break i+4;}};let header=String::from_utf8_lossy(&bytes[..end]);let len:usize=header.lines().find_map(|l|l.to_lowercase().strip_prefix("content-length:").map(|v|v.trim().parse().unwrap())).unwrap();while bytes.len()<end+len{let n=stream.read(&mut buf).await.unwrap();assert!(n>0);bytes.extend_from_slice(&buf[..n]);}let request=serde_json::from_slice(&bytes[end..end+len]).unwrap();tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;let body=value.to_string();let _=stream.write_all(format!("HTTP/1.1 {status} Synthetic\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await;request});(url,task)
}
#[tokio::test]async fn loopback_terminal_usage_error_and_cancel_never_become_success(){
    let snapshot=ledger().snapshot(28,16).unwrap();
    for reason in ["stop","length","aborted","insufficient_system_resource"] {
        let response=json!({"choices":[{"finish_reason":reason,"message":{"content":serde_json::to_string(&candidate(&snapshot)).unwrap(),"reasoning_content":"private reasoning must not be kept"}}],"usage":{"prompt_tokens":30,"completion_tokens":10}});
        let(url,task)=serve(200,response,0).await;let mut p=Config::default().profile;p.endpoint=url;
        let out=run(&p,"synthetic-credential",&snapshot,tokio_util::sync::CancellationToken::new()).await;let sent=task.await.unwrap();assert!(!sent.to_string().contains("synthetic-credential"));assert_eq!(out.usage.unwrap().total(),40);assert_eq!(out.candidate.is_some(),reason=="stop");
    }
    for status in [429,500] {let(url,task)=serve(status,json!({}),0).await;let mut p=Config::default().profile;p.endpoint=url;let out=run(&p,"synthetic-credential",&snapshot,tokio_util::sync::CancellationToken::new()).await;task.await.unwrap();assert!(out.error.unwrap().contains(&status.to_string()));assert!(out.usage.is_none());}
    let(url,task)=serve(429,json!({"usage":{"prompt_tokens":25,"completion_tokens":0}}),0).await;let mut p=Config::default().profile;p.endpoint=url;let out=run(&p,"synthetic-credential",&snapshot,tokio_util::sync::CancellationToken::new()).await;task.await.unwrap();assert!(out.error.unwrap().contains("429"));assert_eq!(out.usage.unwrap().total(),25,"error responses retain provider-reported usage without retrying");
    let(url,task)=serve(200,json!({}),1000).await;let mut p=Config::default().profile;p.endpoint=url;let cancel=tokio_util::sync::CancellationToken::new();let trigger=cancel.clone();tokio::spawn(async move{tokio::time::sleep(std::time::Duration::from_millis(100)).await;trigger.cancel();});let out=run(&p,"synthetic-credential",&snapshot,cancel).await;assert!(out.cancelled);assert!(out.candidate.is_none());assert!(out.usage.is_none());task.abort();
}
