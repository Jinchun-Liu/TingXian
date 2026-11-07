//! Deterministic session contracts. This module performs no network calls and
//! does not promote conversation material into the personal fact library.
use crate::{digest, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_RECORDS:usize=20_000;
const MAX_JOURNAL_BYTES:usize=32*1024*1024;
const MAX_EVENT_BYTES:usize=64*1024;
const MAX_SNAPSHOT_BYTES:usize=64*1024;
const MAX_MEMORY_BYTES:usize=32*1024;
const RECENT:usize=12;

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum EventKind {RemoteSpeech,SelfSpeech,ConfirmedSelf,NormalizedQuestion,Advice,Screenshot,RequestTerminal}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum Origin {SystemAudio,Microphone,UserText,Model,Screen,Runtime}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum Terminal {Complete,Failed,Cancelled,Interrupted}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionVersion {pub question_id:String,pub revision:u64,pub context_version:u64}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub id:String, pub session_id:String, pub epoch:u64,
    /// Stable logical position; revisions retain this position. Journal records
    /// have a separate monotonic order so an ASR correction is not an append.
    pub sequence:u64, pub revision:u64, pub kind:EventKind, pub origin:Origin,
    pub capture_session:Option<String>, pub utterance_id:Option<String>,
    pub replaces_revision:Option<u64>, pub text:String, pub closed:bool,
    pub terminal:Option<Terminal>, pub question:Option<QuestionVersion>,
    pub source_dependencies:BTreeMap<String,String>, pub asset_sha256:Option<String>,
}
fn identifier(s:&str)->bool { !s.is_empty() && s.len()<=160 && s.bytes().all(|b|b.is_ascii_alphanumeric() || b"-_:./".contains(&b)) }
fn hash(s:&str)->bool {s.len()==64 && s.bytes().all(|b|b.is_ascii_hexdigit())}
fn encoded<T:Serialize>(v:&T)->Result<Vec<u8>> {serde_json::to_vec(v).map_err(|e|e.to_string())}
impl Event {
    pub(crate) fn validate(&self)->Result<()> {
        if !identifier(&self.id)||!identifier(&self.session_id)||self.epoch==0||self.sequence==0||self.revision==0||self.text.len()>MAX_EVENT_BYTES {
            return Err("会话事件标识、修订或大小无效".into());
        }
        let source_ok=matches!((&self.kind,&self.origin),
            (EventKind::RemoteSpeech,Origin::SystemAudio)|(EventKind::SelfSpeech,Origin::Microphone)|
            (EventKind::ConfirmedSelf,Origin::UserText)|(EventKind::NormalizedQuestion,Origin::Model)|
            (EventKind::Advice,Origin::Model)|(EventKind::Screenshot,Origin::Screen)|
            (EventKind::RequestTerminal,Origin::Runtime));
        if !source_ok {return Err("事件来源不能升级为本人发言".into());}
        if matches!(self.kind,EventKind::RemoteSpeech|EventKind::SelfSpeech) && (self.capture_session.as_deref().is_none_or(|v|!identifier(v))||self.utterance_id.as_deref().is_none_or(|v|!identifier(v))) {
            return Err("转写事件缺少采集/发言标识".into());
        }
        if matches!(self.kind,EventKind::Advice|EventKind::RequestTerminal) && (self.question.is_none() || (self.closed && self.terminal.is_none())) {
            return Err("答题事件缺少问题版本或完整终态".into());
        }
        if self.question.as_ref().is_some_and(|q|!identifier(&q.question_id)||q.revision==0) {return Err("问题版本无效".into());}
        if self.kind==EventKind::Screenshot && self.asset_sha256.as_deref().is_none_or(|v|!hash(v)) {return Err("截图事件缺少资产校验".into());}
        if self.source_dependencies.len()>64 || self.source_dependencies.iter().any(|(id,v)|!identifier(id)||!hash(v)) {return Err("事件来源依赖无效".into());}
        if encoded(self)?.len()>MAX_EVENT_BYTES+16*1024 {return Err("会话事件元数据过大".into());}
        Ok(())
    }
    pub fn reference(&self)->Result<EventRef> {Ok(EventRef{id:self.id.clone(),sequence:self.sequence,revision:self.revision,sha256:digest(&encoded(self)?)})}
    fn mandatory(&self)->bool {matches!(self.kind,EventKind::RemoteSpeech|EventKind::SelfSpeech|EventKind::ConfirmedSelf|EventKind::Screenshot)}
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventRef {pub id:String,pub sequence:u64,pub revision:u64,pub sha256:String}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependencies {pub source_policy:String,pub facts:String,pub model:String,pub policy:String}
impl Dependencies {fn validate(&self)->Result<()> {if [&self.source_policy,&self.facts,&self.model,&self.policy].iter().any(|s|!hash(s)){return Err("会话依赖须为版本摘要".into());}Ok(())}}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalRecord {pub record_sequence:u64,pub event:Event}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema_version:u32,pub session_id:String,pub epoch:u64,pub base_context_version:u64,
    pub dependencies:Dependencies,pub from_sequence:u64,pub cutoff_sequence:u64,
    pub events:Vec<Event>,
}
impl Snapshot {pub fn id(&self)->Result<String>{Ok(digest(&encoded(self)?))}}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryItem {
    pub kind:EventKind,pub origin:Origin,pub refs:Vec<EventRef>,
    /// Entire source text, not a model paraphrase. Unverifiable semantic
    /// assertions cannot be promoted by a successful JSON parse.
    pub quote:String,
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum OmissionReason {DerivedAdvice,NormalizedDuplicate,RequestStatus}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Omission {pub reference:EventRef,pub reason:OmissionReason}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {pub schema_version:u32,pub snapshot_id:String,pub finish_reason:String,pub items:Vec<MemoryItem>,pub omitted:Vec<Omission>}

#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct MemoryChunk {pub id:String,pub context_version:u64,pub snapshot:Snapshot,pub candidate:Candidate,pub valid:bool,pub validation_status:String}
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct Workset {
    pub context_version:u64,pub missing_older_history:bool,
    pub memory:Vec<MemoryItem>,pub retrieved_events:Vec<Event>,pub recent_events:Vec<Event>,
}
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct ReplayView {pub context_version:u64,pub chunks:Vec<MemoryChunk>,pub uncovered:Vec<Event>,pub missing_older_history:bool}

/// In-memory replay state. Persistence must acknowledge each JournalRecord
/// before a UI ring may evict it. No claim of durability is made here.
#[derive(Clone,Debug)]
pub struct Ledger {
    session_id:String,epoch:u64,context_version:u64,dependencies:Dependencies,
    journal:Vec<JournalRecord>,latest:BTreeMap<u64,Event>,ids:BTreeMap<String,u64>,
    chunks:Vec<MemoryChunk>,journal_bytes:usize,missing_older_history:bool,
}
impl Ledger {
    pub fn new(session_id:&str,epoch:u64,dependencies:Dependencies,missing_older_history:bool)->Result<Self> {
        if !identifier(session_id)||epoch==0 {return Err("会话标识或 epoch 无效".into());}dependencies.validate()?;
        Ok(Self{session_id:session_id.into(),epoch,context_version:0,dependencies,journal:vec![],latest:BTreeMap::new(),ids:BTreeMap::new(),chunks:vec![],journal_bytes:0,missing_older_history})
    }
    pub fn version(&self)->u64 {self.context_version}
    pub fn journal(&self)->&[JournalRecord] {&self.journal}
    pub fn chunks(&self)->&[MemoryChunk] {&self.chunks}
    /// Lossless logical view before relevance selection. Never sent wholesale.
    pub fn replay_view(&self)->ReplayView {
        let chunks:Vec<_>=self.chunks.iter().filter(|c|c.valid).cloned().collect();
        let end=chunks.iter().map(|c|c.snapshot.cutoff_sequence).max().unwrap_or(0);
        ReplayView{context_version:self.context_version,chunks,uncovered:self.latest.range(end+1..).map(|(_,e)|e.clone()).collect(),missing_older_history:self.missing_older_history}
    }
    pub fn append(&mut self,event:Event)->Result<JournalRecord> {
        event.validate()?;
        if event.session_id!=self.session_id||event.epoch!=self.epoch {return Err("旧会话事件已拦截".into());}
        if let Some(old)=self.ids.get(&event.id).and_then(|n|self.latest.get(n)) {
            if event==*old {return self.journal.iter().rev().find(|r|r.event==event).cloned().ok_or("重复事件缺少原记录".into());}
            if event.sequence!=old.sequence||event.revision!=old.revision+1||event.replaces_revision!=Some(old.revision)||event.kind!=old.kind||event.origin!=old.origin||event.capture_session!=old.capture_session||event.utterance_id!=old.utterance_id||event.question!=old.question {
                return Err("事件修订或来源关系无效".into());
            }
        } else if event.sequence!=self.latest.last_key_value().map_or(1,|(seq,_)|seq+1)||event.revision!=1||event.replaces_revision.is_some() {
            return Err("新增事件必须按顺序追加，不能覆盖旧位置".into());
        }
        let record=JournalRecord{record_sequence:self.journal.len() as u64+1,event:event.clone()};let bytes=encoded(&record)?.len();
        if self.journal.len()>=MAX_RECORDS||self.journal_bytes.saturating_add(bytes)>MAX_JOURNAL_BYTES {return Err("会话事件达到本地保留上限；未写入，请导出并新建会话".into());}
        if self.chunks.iter().any(|c|c.valid && c.snapshot.events.iter().any(|e|e.id==event.id)) {self.invalidate();}
        self.ids.insert(event.id.clone(),event.sequence);self.latest.insert(event.sequence,event);
        self.journal.push(record.clone());self.journal_bytes+=bytes;Ok(record)
    }
    fn invalidate(&mut self) {for c in &mut self.chunks {c.valid=false;}self.context_version+=1;}
    pub fn update_dependencies(&mut self,dependencies:Dependencies)->Result<()> {
        dependencies.validate()?;if self.dependencies!=dependencies {self.dependencies=dependencies;self.invalidate();}Ok(())
    }
    pub fn pin(&self,question_id:&str,revision:u64)->Result<QuestionVersion> {
        if !identifier(question_id)||revision==0 {return Err("问题版本无效".into());}
        Ok(QuestionVersion{question_id:question_id.into(),revision,context_version:self.context_version})
    }
    /// `closed_through` comes from an observed completed-topic boundary. A last-N
    /// calculation alone must not assert the topic has finished.
    pub fn snapshot(&self,closed_through:u64,min_events:usize)->Result<Snapshot> {
        if min_events==0||min_events>64 {return Err("压缩最小批次无效".into());}
        let start=self.chunks.iter().filter(|c|c.valid).map(|c|c.snapshot.cutoff_sequence).max().unwrap_or(0)+1;
        let tail=self.latest.last_key_value().map_or(0,|(n,_)|*n).saturating_sub(RECENT as u64);
        let end=closed_through.min(tail).min(start.saturating_add(63));
        if start>end {return Err("没有可压缩的已结束旧话题".into());}
        let mut snapshot=Snapshot{schema_version:1,session_id:self.session_id.clone(),epoch:self.epoch,base_context_version:self.context_version,dependencies:self.dependencies.clone(),from_sequence:start,cutoff_sequence:end,events:vec![]};
        for (_,e) in self.latest.range(start..=end) {
            if !e.closed {break;}
            snapshot.events.push(e.clone());snapshot.cutoff_sequence=e.sequence;
            if encoded(&snapshot)?.len()>MAX_SNAPSHOT_BYTES {snapshot.events.pop();snapshot.cutoff_sequence=e.sequence-1;break;}
        }
        if snapshot.events.len()<min_events {return Err("已结束旧发言不足最小批次，未请求压缩".into());}
        snapshot.cutoff_sequence=snapshot.events.last().unwrap().sequence;Ok(snapshot)
    }
    fn current_snapshot(&self,snapshot:&Snapshot)->Result<()> {
        if snapshot.schema_version!=1||snapshot.session_id!=self.session_id||snapshot.epoch!=self.epoch||snapshot.base_context_version!=self.context_version||snapshot.dependencies!=self.dependencies {
            return Err("压缩快照的会话、上下文或依赖已过期".into());
        }
        let from=self.chunks.iter().filter(|c|c.valid).map(|c|c.snapshot.cutoff_sequence).max().unwrap_or(0)+1;
        let tail=self.latest.last_key_value().map_or(0,|(n,_)|*n).saturating_sub(RECENT as u64);
        if snapshot.from_sequence!=from||snapshot.events.is_empty()||snapshot.events.len()>64||snapshot.cutoff_sequence>tail||snapshot.cutoff_sequence!=from+snapshot.events.len() as u64-1||encoded(snapshot)?.len()>MAX_SNAPSHOT_BYTES {return Err("压缩覆盖范围无效或存在空洞".into());}
        for (i,e) in snapshot.events.iter().enumerate() {
            if e.sequence!=from+i as u64||!e.closed||self.latest.get(&e.sequence)!=Some(e) {return Err("压缩原文已修订或未结束".into());}
        }
        Ok(())
    }
    pub fn promote(&mut self,snapshot:Snapshot,candidate:Candidate)->Result<String> {
        self.current_snapshot(&snapshot)?;validate_candidate(&snapshot,&candidate)?;
        let id=digest(&encoded(&candidate)?);
        if self.chunks.len()>=512 {return Err("会话记忆版本达到上限，请新建会话".into());}
        self.context_version+=1;
        self.chunks.push(MemoryChunk{id:id.clone(),context_version:self.context_version,snapshot,candidate,valid:true,validation_status:"extractive_quotes_verified_not_semantic_paraphrase".into()});
        Ok(id)
    }
    /// Query terms are supplied by the existing local tokenizer. Selected quotes
    /// and raw events are deduplicated by logical ID, not wall-clock text.
    pub fn workset(&self,terms:&[String])->Workset {
        self.workset_with_sources(terms,&BTreeMap::new())
    }
    pub fn workset_with_sources(&self,terms:&[String],valid_sources:&BTreeMap<String,String>)->Workset {
        let mut out=Workset{context_version:self.context_version,missing_older_history:self.missing_older_history,memory:vec![],retrieved_events:vec![],recent_events:vec![]};
        let permitted=|e:&Event|e.source_dependencies.iter().all(|(id,version)|valid_sources.get(id)==Some(version));
        let recent_start=self.latest.last_key_value().map_or(0,|(n,_)|*n).saturating_sub(RECENT as u64-1);
        let mut seen=BTreeSet::new();
        for (_,e) in self.latest.range(recent_start..) {if permitted(e) {seen.insert(e.id.clone());out.recent_events.push(e.clone());}}
        let relevant=|text:&str|{let text=text.to_lowercase();terms.iter().take(32).filter(|s|s.chars().count()>=2).any(|s|text.contains(&s.to_lowercase()))};
        let mut used=0;
        for item in self.chunks.iter().filter(|c|c.valid).rev().flat_map(|c|c.candidate.items.iter()) {
            if out.memory.len()>=4 {break;}
            if item.refs.iter().any(|r|seen.contains(&r.id)||self.latest.get(&r.sequence).is_none_or(|e|!permitted(e)))||!relevant(&item.quote)||used+item.quote.len()>8000 {continue;}
            used+=item.quote.len();for r in &item.refs {seen.insert(r.id.clone());}out.memory.push(item.clone());
        }
        for e in self.latest.values().rev() {
            if out.retrieved_events.len()>=5 {break;}
            if !permitted(e)||!e.closed||!e.mandatory()||seen.contains(&e.id)||!relevant(&e.text)||used+e.text.len()>8000 {continue;}
            used+=e.text.len();seen.insert(e.id.clone());out.retrieved_events.push(e.clone());
        }
        out.retrieved_events.sort_by_key(|e|e.sequence);out
    }
}

pub fn validate_candidate(snapshot:&Snapshot,candidate:&Candidate)->Result<()> {
    if candidate.schema_version!=1||candidate.snapshot_id!=snapshot.id()?||candidate.finish_reason!="stop"||encoded(candidate)?.len()>MAX_MEMORY_BYTES {
        return Err("记忆格式、终态、快照或长度无效，未晋级".into());
    }
    let events:BTreeMap<_,_>=snapshot.events.iter().map(|e|(e.id.as_str(),e)).collect();let mut covered=BTreeSet::new();
    for item in &candidate.items {
        if item.refs.is_empty() {return Err("记忆条目没有原文引用".into());}
        for reference in &item.refs {
            let event=events.get(reference.id.as_str()).ok_or("记忆引用越出快照")?;
            if event.reference()?!=*reference||item.kind!=event.kind||item.origin!=event.origin||item.quote!=event.text||!covered.insert(reference.id.clone()) {
                return Err("记忆引用、原文、身份或覆盖重复；未晋级".into());
            }
            if matches!(event.kind,EventKind::Advice|EventKind::RequestTerminal)&&event.terminal!=Some(Terminal::Complete) {return Err("失败或未完成答复不能作为完成结论".into());}
        }
    }
    for omitted in &candidate.omitted {
        let event=events.get(omitted.reference.id.as_str()).ok_or("省略引用越出快照")?;
        let reason_ok=matches!((&event.kind,&omitted.reason),(EventKind::Advice,OmissionReason::DerivedAdvice)|(EventKind::NormalizedQuestion,OmissionReason::NormalizedDuplicate)|(EventKind::RequestTerminal,OmissionReason::RequestStatus));
        if !reason_ok||event.mandatory()||event.reference()?!=omitted.reference||!covered.insert(event.id.clone()) {return Err("原始发言/条件不能省略或重复，未晋级".into());}
    }
    if covered.len()!=snapshot.events.len() {return Err("记忆覆盖存在空洞，未晋级".into());}
    Ok(())
}
