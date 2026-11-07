//! Native-only live input. No capture/device authority or deserializable handles.
use crate::{EyeOutcome, EyeProvider, Reason};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub const STREAM_BGR_BYTES: usize = 1280 * 720 * 3;
pub struct StreamFrame { pub id: u64, pub captured_ms: u64, pub bgr: Vec<u8> }
impl Drop for StreamFrame { fn drop(&mut self) { self.bgr.fill(0); } }
// Deliberately no Debug/Serialize/Deserialize. Only the native preview/output
// owner may use these bytes as MF activation configuration, never the webview.
pub struct NativeOutputEndpoint { bytes: [u8;236] }
impl NativeOutputEndpoint {
    pub fn activation_blob(&self) -> &[u8;236] { &self.bytes }
    pub(crate) fn parse(bytes: &[u8], pid: u32) -> Result<Self,Reason> {
        if bytes.len()!=236 || u32::from_le_bytes(bytes[0..4].try_into().unwrap())!=1
            || u32::from_le_bytes(bytes[196..200].try_into().unwrap())!=pid || pid==0
            || bytes[204..236].iter().all(|b|*b==0) { return Err(Reason::Protocol); }
        let name: Vec<u16> = bytes[4..196].chunks_exact(2).map(|c|u16::from_le_bytes([c[0],c[1]])).collect();
        let end=name.iter().position(|c|*c==0).ok_or(Reason::Protocol)?;
        let name=String::from_utf16(&name[..end]).map_err(|_|Reason::Protocol)?;
        let suffix=name.strip_prefix(r"\\.\pipe\TingXian.Video.").ok_or(Reason::Protocol)?;
        if suffix.len()!=32 || !suffix.bytes().all(|c|c.is_ascii_digit()||(b'a'..=b'f').contains(&c)) { return Err(Reason::Protocol); }
        Ok(Self{bytes:bytes.try_into().unwrap()})
    }
}
impl Drop for NativeOutputEndpoint {
    fn drop(&mut self) { for b in &mut self.bytes { unsafe { std::ptr::write_volatile(b,0); } } }
}
#[derive(Clone,Copy,Debug,Default,PartialEq,Eq)]
pub struct StreamInputStatus {
    pub accepted:u64, pub replaced:u64, pub rejected:u64, pub discarded:u64,
    /// Fully written to the input pipe; not a provider/output receipt.
    pub written:u64, pub queued:usize, pub in_flight:usize, pub closed:bool,
}
#[derive(Default)]
struct State {
    output:Option<std::sync::Weak<dyn RevokeOutput>>,
    generation: u64,
    admitted: bool, ready: bool, closed: bool, ending: bool,
    pending: Option<StreamFrame>, endpoint: Option<NativeOutputEndpoint>,
    last_id:u64,last_time:u64,stats:StreamInputStatus,
}
#[derive(Default)]
pub(crate) struct Shared { state:Mutex<State>,changed:Condvar }
pub(crate) trait RevokeOutput:Send+Sync {fn revoke(&self);}
pub(crate) enum Next { Frame(StreamFrame), Finish, Closed }
impl Shared {
    pub(crate) fn admit(&self,generation:u64) {
        let mut s=self.state.lock().unwrap_or_else(|e|e.into_inner());s.admitted=true;s.generation=generation;
    }
    // Hold the same lease lock through native frame handoff. Stop cannot finish
    // closing this stream and then allow a delayed preview callback to present.
    pub(crate) fn while_open<T>(&self,f:impl FnOnce(bool,u64)->T)->T {
        let s=self.state.lock().unwrap_or_else(|e|e.into_inner());
        f(s.admitted&&s.ready&&!s.closed,s.generation)
    }
    #[cfg(windows)]
    pub(crate) fn attach_output(&self,output:std::sync::Weak<dyn RevokeOutput>)->bool {
        let mut s=self.state.lock().unwrap_or_else(|e|e.into_inner());
        if !s.admitted||!s.ready||s.closed{return false;}
        if let Some(previous)=s.output.take().and_then(|v|v.upgrade()){previous.revoke();}
        s.output=Some(output);true
    }
    pub(crate) fn publish(&self,endpoint:NativeOutputEndpoint)->bool {
        let mut s=self.state.lock().unwrap_or_else(|e|e.into_inner());
        if !s.admitted || s.closed || s.ready { return false; }
        s.ready=true;s.endpoint=Some(endpoint);self.changed.notify_all();true
    }
    pub(crate) fn close(&self) {
        let mut s=self.state.lock().unwrap_or_else(|e|e.into_inner());s.closed=true;
        if let Some(output)=s.output.take().and_then(|v|v.upgrade()){output.revoke();}
        if s.pending.take().is_some(){s.stats.discarded+=1;}s.endpoint=None;self.changed.notify_all();
    }
    pub(crate) fn settled(&self,written:bool) {
        let mut s=self.state.lock().unwrap_or_else(|e|e.into_inner());
        s.stats.in_flight=0;if written{s.stats.written+=1;}else{s.stats.discarded+=1;}
    }
    pub(crate) fn closed(&self)->bool { self.state.lock().unwrap_or_else(|e|e.into_inner()).closed }
    pub(crate) fn next(&self)->Next {
        let mut s=self.state.lock().unwrap_or_else(|e|e.into_inner());
        loop {
            if s.closed { return Next::Closed; }
            if let Some(frame)=s.pending.take() {s.stats.in_flight=1;return Next::Frame(frame);}
            if s.ending { return Next::Finish; }
            s=self.changed.wait(s).unwrap_or_else(|e|e.into_inner());
        }
    }
}
pub(crate) struct StreamJob { pub provider:EyeProvider,pub shared:Arc<Shared> }
impl Drop for StreamJob { fn drop(&mut self) { self.shared.close(); } }
pub struct StreamFeed { pub(crate) shared:Arc<Shared> }
impl Drop for StreamFeed { fn drop(&mut self) { self.shared.close(); } }
pub(crate) fn pair(provider:EyeProvider)->(StreamJob,StreamFeed) {
    let shared=Arc::new(Shared::default());(StreamJob{provider,shared:shared.clone()},StreamFeed{shared})
}
impl StreamFeed {
    /// One latest pending frame plus the writer's current frame. Call from the
    /// native capture lane with its ORIGINAL GetTickCount64 timestamp.
    pub fn submit(&self,frame:StreamFrame)->Result<bool,Reason> {
        let mut s=self.shared.state.lock().unwrap_or_else(|e|e.into_inner());
        if !s.admitted||!s.ready||s.closed||s.ending||frame.bgr.len()!=STREAM_BGR_BYTES||frame.id==0||frame.id<=s.last_id||frame.captured_ms<s.last_time {
            s.stats.rejected+=1;return Err(Reason::EyeInput);
        }
        let replaced=s.pending.is_some();if replaced{s.stats.replaced+=1;}
        s.last_id=frame.id;s.last_time=frame.captured_ms;s.pending=Some(frame);s.stats.accepted+=1;
        self.shared.changed.notify_all();Ok(replaced)
    }
    pub fn finish(&self)->bool {
        let mut s=self.shared.state.lock().unwrap_or_else(|e|e.into_inner());
        if s.closed||!s.ready||s.ending{return false;}s.ending=true;self.shared.changed.notify_all();true
    }
    pub fn status(&self)->StreamInputStatus {
        let s=self.shared.state.lock().unwrap_or_else(|e|e.into_inner());StreamInputStatus{queued:usize::from(s.pending.is_some()),closed:s.closed,..s.stats}
    }
    pub fn take_endpoint(&self,timeout:Duration)->Result<NativeOutputEndpoint,Reason> {
        let deadline=Instant::now()+timeout;let mut s=self.shared.state.lock().unwrap_or_else(|e|e.into_inner());
        loop {
            if s.closed{return Err(Reason::Abandoned);}if let Some(e)=s.endpoint.take(){return Ok(e);}
            if s.ready{return Err(Reason::EyeOutput);}let remaining=deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero(){return Err(Reason::Timeout);}
            s=self.shared.changed.wait_timeout(s,remaining).unwrap_or_else(|e|e.into_inner()).0;
        }
    }
}
#[derive(Clone,Debug,PartialEq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamReport {
    pub protocol:u32,pub generation:u64,pub provider:EyeProvider,pub width:u32,pub height:u32,
    pub received:u64,pub rejected:u64,pub provider_dropped:u64,pub corrected:u64,pub output_dropped:u64,pub submitted:u64,
    pub outcome:EyeOutcome,pub error:u32,pub cleanup_uncertain:bool,
    pub camera_started:bool,pub audio_started:bool,pub virtual_device_started:bool,
}
impl StreamReport {
    pub(crate) fn parse(bytes:&[u8],provider:EyeProvider,generation:u64)->Result<Self,Reason> {
        if bytes.len()>4096{return Err(Reason::Protocol);}
        let r:Self=serde_json::from_slice(bytes).map_err(|_|Reason::Protocol)?;
        let processed=r.rejected.checked_add(r.provider_dropped).and_then(|n|n.checked_add(r.corrected)).ok_or(Reason::Protocol)?;
        if r.protocol!=3||r.generation!=generation||r.provider!=provider||r.width!=1280||r.height!=720
            ||processed>r.received||r.submitted.checked_add(r.output_dropped)!=Some(r.corrected)||r.error>10
            ||r.camera_started||r.audio_started||r.virtual_device_started||(r.cleanup_uncertain&&r.error!=8)
            ||(r.outcome==EyeOutcome::Completed&&(r.error!=0||r.cleanup_uncertain||processed!=r.received))
            ||(r.outcome==EyeOutcome::Fault&&r.error==0)||(r.outcome==EyeOutcome::Cancelled&&r.cleanup_uncertain) {return Err(Reason::Protocol);}
        Ok(r)
    }
}
