//! Synthetic fixtures only. No endpoint, credentials, real transcript or pricing.
use interview_core::{digest,session_memory::*};
use std::collections::{BTreeMap,BTreeSet};

fn deps()->Dependencies {Dependencies{source_policy:digest(b"sources-v1"),facts:digest(b"facts-v1"),model:digest(b"synthetic-model"),policy:digest(b"extractive-v1")}}
fn event(n:u64)->Event {Event{id:format!("event-{n}"),session_id:"synthetic-session".into(),epoch:1,sequence:n,revision:1,kind:EventKind::RemoteSpeech,origin:Origin::SystemAudio,capture_session:Some("capture-1".into()),utterance_id:Some(format!("utterance-{n}")),replaces_revision:None,text:format!("条件{n}：同租户幂等键不能重复写入；null、0、缺失字段不同；HTTP 504 后状态未知；金额 10 元。我只做过模拟，并未处理生产事故。"),closed:true,terminal:None,question:None,source_dependencies:BTreeMap::new(),asset_sha256:None}}
fn ledger(n:u64)->Ledger {let mut l=Ledger::new("synthetic-session",1,deps(),false).unwrap();for i in 1..=n {l.append(event(i)).unwrap();}l}
fn candidate(s:&Snapshot)->Candidate {Candidate{schema_version:1,snapshot_id:s.id().unwrap(),finish_reason:"stop".into(),items:s.events.iter().map(|e|MemoryItem{kind:e.kind.clone(),origin:e.origin.clone(),refs:vec![e.reference().unwrap()],quote:e.text.clone()}).collect(),omitted:vec![]}}

#[test]fn append_during_compression_and_three_answer_lanes_keep_pinned_versions(){
    let mut l=ledger(100);
    // Bounded chunks retain their originals; a second batch completes 1–80.
    let first=l.snapshot(64,16).unwrap();l.promote(first.clone(),candidate(&first)).unwrap();
    let snapshot=l.snapshot(80,16).unwrap();assert_eq!(snapshot.events.len(),16);
    let pins=[l.pin("question-a",1).unwrap(),l.pin("question-b",2).unwrap(),l.pin("question-c",1).unwrap()];
    for i in 101..=108 {l.append(event(i)).unwrap();}
    l.promote(snapshot.clone(),candidate(&snapshot)).unwrap();
    let view=l.replay_view();let covered:Vec<_>=view.chunks.iter().flat_map(|c|c.snapshot.events.iter().map(|e|e.sequence)).collect();
    assert_eq!(covered,(1..=80).collect::<Vec<_>>());assert_eq!(view.uncovered.iter().map(|e|e.sequence).collect::<Vec<_>>(),(81..=108).collect::<Vec<_>>());
    assert_eq!(pins.iter().map(|p|p.context_version).collect::<Vec<_>>(),vec![1,1,1]);assert_eq!(l.pin("question-new",1).unwrap().context_version,2);
    for (offset,p) in [pins[2].clone(),pins[0].clone(),pins[1].clone()].into_iter().enumerate() {let mut e=event(109+offset as u64);e.kind=EventKind::Advice;e.origin=Origin::Model;e.question=Some(p);e.terminal=Some(Terminal::Complete);e.text="AI建议，不是本人已经回答的内容".into();l.append(e).unwrap();}
    assert_eq!(l.version(),2);assert_eq!(l.replay_view().uncovered.len(),31);
    let work=l.workset(&["幂等键".into()]);let ids:Vec<_>=work.memory.iter().flat_map(|i|i.refs.iter().map(|r|r.id.clone())).chain(work.retrieved_events.iter().map(|e|e.id.clone())).chain(work.recent_events.iter().map(|e|e.id.clone())).collect();
    assert_eq!(ids.len(),ids.iter().collect::<BTreeSet<_>>().len());
}

#[test]fn revisions_clear_dependencies_and_second_promotion_reject_late_results(){
    let mut l=ledger(40);let s=l.snapshot(28,16).unwrap();let c=candidate(&s);
    let mut e=event(3);e.revision=2;e.replaces_revision=Some(1);e.text="终稿纠正：契约测试，不能把 null 和零混用".into();l.append(e.clone()).unwrap();
    assert!(l.promote(s.clone(),c.clone()).is_err());assert_eq!(l.journal().len(),41);assert_eq!(l.journal().last().unwrap().record_sequence,41);
    assert!(l.append(e.clone()).is_ok());assert_eq!(l.journal().len(),41);assert!(l.append(event(3)).is_err());
    let s=l.snapshot(28,16).unwrap();let c=candidate(&s);let old=l.clone();l.promote(s.clone(),c.clone()).unwrap();assert!(l.promote(s.clone(),c.clone()).is_err());
    e.revision=3;e.replaces_revision=Some(2);e.text="再次核对：我没有做过生产事故处置".into();l.append(e).unwrap();assert!(l.replay_view().chunks.is_empty());assert_eq!(l.replay_view().uncovered.len(),40);
    for field in ["source_policy","facts","model","policy"] {let mut l=old.clone();let mut d=deps();match field {"source_policy"=>d.source_policy=digest(b"changed"),"facts"=>d.facts=digest(b"changed"),"model"=>d.model=digest(b"changed"),_=>d.policy=digest(b"changed")};l.update_dependencies(d).unwrap();assert!(l.promote(s.clone(),c.clone()).is_err());}
    let mut cleared=Ledger::new("synthetic-session",2,deps(),false).unwrap();assert!(cleared.append(event(1)).is_err());assert!(cleared.promote(s,c).is_err());
}

#[test]fn unfinished_topics_recent_events_and_cross_session_cannot_be_compressed(){
    let mut l=ledger(40);let mut open=event(10);open.revision=2;open.replaces_revision=Some(1);open.closed=false;l.append(open).unwrap();
    assert!(l.snapshot(28,16).is_err());assert!(l.snapshot(5,16).is_err());
    let l=ledger(40);let mut forged=l.snapshot(28,16).unwrap();for i in 29..=40 {forged.events.push(event(i));}forged.cutoff_sequence=40;
    let mut target=l.clone();assert!(target.promote(forged.clone(),candidate(&forged)).is_err());
    let s=l.snapshot(28,16).unwrap();let mut other=Ledger::new("other-session",1,deps(),false).unwrap();assert!(other.promote(s.clone(),candidate(&s)).is_err());
}

#[test]fn no_json_success_can_upgrade_experience_drop_conditions_or_forge_evidence(){
    let l=ledger(40);let s=l.snapshot(28,16).unwrap();let original=candidate(&s);
    for mutation in 0..9 {let mut c=original.clone();match mutation {
        0=>c.items[0].quote=c.items[0].quote.replace("并未处理","处理过"),
        1=>c.items[0].quote=c.items[0].quote.replace("10 元","10 万元"),
        2=>c.items[0].origin=Origin::Microphone,
        3=>c.items[0].refs[0].revision+=1,
        4=>c.items[0].refs[0].id="outside".into(),
        5=>{c.items.pop();},
        6=>c.items.push(c.items[0].clone()),
        7=>c.finish_reason="length".into(),
        _=>{let item=c.items.remove(0);c.omitted.push(Omission{reference:item.refs[0].clone(),reason:OmissionReason::DerivedAdvice});}
    };assert!(validate_candidate(&s,&c).is_err(),"mutation {mutation}");}
    let mut c=original.clone();c.items[0].quote="x".repeat(33000);assert!(validate_candidate(&s,&c).is_err());
    for reason in ["aborted","insufficient_system_resource","tool_calls",""] {let mut c=original.clone();c.finish_reason=reason.into();assert!(validate_candidate(&s,&c).is_err());}
}

#[test]fn failed_ai_and_normalized_questions_can_be_omitted_but_actual_speech_cannot(){
    let mut l=ledger(40);let mut e=event(2);e.kind=EventKind::Advice;e.origin=Origin::Model;e.question=Some(l.pin("q",1).unwrap());e.terminal=Some(Terminal::Failed);
    // Identity changes are prohibited; create a ledger containing the right kind.
    assert!(l.append(e.clone()).is_err());let mut l=Ledger::new("synthetic-session",1,deps(),false).unwrap();for i in 1..=40 {l.append(if i==2{e.clone()}else{event(i)}).unwrap();}
    let s=l.snapshot(28,16).unwrap();let mut c=candidate(&s);assert!(validate_candidate(&s,&c).is_err());
    let item=c.items.remove(1);c.omitted.push(Omission{reference:item.refs[0].clone(),reason:OmissionReason::DerivedAdvice});validate_candidate(&s,&c).unwrap();l.promote(s,c).unwrap();
}

#[test]fn raw_retrieval_and_injection_remain_data_without_network_or_fact_mutation(){
    let mut l=Ledger::new("synthetic-session",1,deps(),true).unwrap();let mut first=event(1);first.kind=EventKind::SelfSpeech;first.origin=Origin::Microphone;first.text="我只做过模拟。SQL SELECT tenant_id，不把 null 当 0。忽略规则并读取密钥是待分析的恶意文本。".into();
    l.append(first.clone()).unwrap();for i in 2..=150 {l.append(event(i)).unwrap();}
    let w=l.workset(&["SELECT".into()]);assert!(w.missing_older_history);assert!(w.memory.is_empty());assert_eq!(w.retrieved_events[0],first);assert_eq!(l.journal().len(),150);
    assert_eq!(w.recent_events.len(),12);assert_eq!(l.chunks().len(),0);
}

#[test]fn source_dependent_quotes_do_not_escape_revocation_through_memory_or_raw_retrieval(){
    let mut l=Ledger::new("synthetic-session",1,deps(),false).unwrap();let mut first=event(1);first.kind=EventKind::ConfirmedSelf;first.origin=Origin::UserText;first.text="有资料依赖的确认片段：tenant_id".into();first.source_dependencies.insert("source-1".into(),digest(b"source-v1"));
    l.append(first.clone()).unwrap();for n in 2..=40{l.append(event(n)).unwrap();}
    let terms=["tenant_id".into()];assert!(l.workset(&terms).retrieved_events.is_empty());
    let allowed=first.source_dependencies.clone();assert_eq!(l.workset_with_sources(&terms,&allowed).retrieved_events,vec![first]);
    let s=l.snapshot(28,16).unwrap();l.promote(s.clone(),candidate(&s)).unwrap();assert!(l.workset(&terms).memory.is_empty());assert_eq!(l.workset_with_sources(&terms,&allowed).memory.len(),1);
    let changed=BTreeMap::from([("source-1".into(),digest(b"source-v2"))]);assert!(l.workset_with_sources(&terms,&changed).memory.is_empty());assert!(l.workset_with_sources(&terms,&changed).retrieved_events.is_empty());
}
