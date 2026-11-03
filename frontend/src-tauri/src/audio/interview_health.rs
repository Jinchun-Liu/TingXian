//! Per-recording observations only: silence is not proof of permission denial.
//! No audio/text is retained here and old workers only update their own session.
use serde::Serialize;
use std::sync::{Arc,Mutex,atomic::{AtomicBool,AtomicU64,Ordering}};
use std::time::Instant;
use tauri::{Emitter,Runtime};

static NEXT_SESSION:AtomicU64=AtomicU64::new(1);
#[derive(Clone,Debug,Serialize)]
pub struct AudioFault {pub session_id:u64,pub sequence:u64,pub code:String,pub at_ms:u64}
#[derive(Clone,Debug,Serialize)]
pub struct AudioHealthSnapshot {
    pub session_id:u64,pub recording:bool,pub active:bool,pub paused:bool,
    pub system_started:bool,pub microphone_enabled:bool,pub active_for_ms:u64,
    pub system_frame_age_ms:Option<u64>,pub microphone_frame_age_ms:Option<u64>,
    pub system_signal_age_ms:Option<u64>,pub asr_pending:u64,pub asr_no_progress_ms:Option<u64>,
    pub asr_pending_bytes:u64,pub asr_peak_bytes:u64,
    pub fault:Option<AudioFault>,pub restart_required:bool,pub restart_reason:Option<String>,pub elapsed_ms:u64,
}
#[derive(Default)]
struct Work {pending:u64,last_progress_ms:u64,bytes:u64,peak_bytes:u64}
pub struct AudioHealth {
    pub session_id:u64,origin:Instant,active_since:AtomicU64,
    system_frame:AtomicU64,microphone_frame:AtomicU64,system_signal:AtomicU64,
    work:Mutex<Work>,fault:Mutex<Option<AudioFault>>,restart_required:AtomicBool,restart_reason:Mutex<Option<String>>,
}
pub struct WorkTicket {health:Arc<AudioHealth>,bytes:u64}
impl Drop for WorkTicket {fn drop(&mut self){self.health.completed(self.bytes);}}
impl AudioHealth {
    pub fn new()->Arc<Self> {Arc::new(Self {session_id:NEXT_SESSION.fetch_add(1,Ordering::Relaxed),origin:Instant::now(),active_since:AtomicU64::new(0),system_frame:AtomicU64::new(0),microphone_frame:AtomicU64::new(0),system_signal:AtomicU64::new(0),work:Mutex::new(Work::default()),fault:Mutex::new(None),restart_required:AtomicBool::new(false),restart_reason:Mutex::new(None)})}
    fn now(&self)->u64 {self.origin.elapsed().as_millis() as u64+1}
    pub fn resumed(&self) {self.active_since.store(self.now(),Ordering::Relaxed);}
    pub fn observe(&self,system:bool,data:&[f32]) {
        if data.is_empty(){return;}let now=self.now();
        if system {self.system_frame.store(now,Ordering::Relaxed);
            if data.iter().any(|v|v.is_finite() && v.abs()>0.003) {self.system_signal.store(now,Ordering::Relaxed);}
        }else{self.microphone_frame.store(now,Ordering::Relaxed);}
    }
    pub fn begin_work(self:&Arc<Self>)->WorkTicket {self.begin_audio_work(0)}
    pub fn has_pending_work(&self)->bool {self.work.lock().unwrap().pending>0}
    pub fn begin_audio_work(self:&Arc<Self>,bytes:u64)->WorkTicket {
        let mut work=self.work.lock().unwrap();if work.pending==0{work.last_progress_ms=self.now();}
        work.pending+=1;work.bytes+=bytes;work.peak_bytes=work.peak_bytes.max(work.bytes);
        WorkTicket {health:self.clone(),bytes}
    }
    fn completed(&self,bytes:u64) {let mut work=self.work.lock().unwrap();work.pending=work.pending.saturating_sub(1);work.bytes=work.bytes.saturating_sub(bytes);work.last_progress_ms=self.now();}
    pub fn record_fault(&self,code:&str)->AudioFault {
        if matches!(code,"asr_init_failed"|"asr_worker_failed") {
            self.restart_reason.lock().unwrap().get_or_insert_with(||code.to_string());
            self.restart_required.store(true,Ordering::Relaxed);
        }
        let mut current=self.fault.lock().unwrap();let event=AudioFault {session_id:self.session_id,sequence:current.as_ref().map_or(1,|f|f.sequence+1),code:code.into(),at_ms:self.now()};*current=Some(event.clone());event
    }
    pub fn snapshot(&self,recording:bool,active:bool,paused:bool,system_started:bool,microphone_enabled:bool)->AudioHealthSnapshot {
        let now=self.now();let age=|counter:&AtomicU64|{let value=counter.load(Ordering::Relaxed);(value!=0).then(||now.saturating_sub(value))};
        let work=self.work.lock().unwrap();
        AudioHealthSnapshot {session_id:self.session_id,recording,active,paused,system_started,microphone_enabled,active_for_ms:now.saturating_sub(self.active_since.load(Ordering::Relaxed)),system_frame_age_ms:age(&self.system_frame),microphone_frame_age_ms:age(&self.microphone_frame),system_signal_age_ms:age(&self.system_signal),asr_pending:work.pending,asr_no_progress_ms:(work.pending>0).then(||now.saturating_sub(work.last_progress_ms)),asr_pending_bytes:work.bytes,asr_peak_bytes:work.peak_bytes,fault:self.fault.lock().unwrap().clone(),restart_required:self.restart_required.load(Ordering::Relaxed),restart_reason:self.restart_reason.lock().unwrap().clone(),elapsed_ms:now}
    }
}
pub fn report<R:Runtime>(app:&tauri::AppHandle<R>,health:&AudioHealth,code:&str) {
    let _=app.emit("interview-audio-fault",health.record_fault(code));
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn silent_frames_are_distinct_from_missing_frames_and_recordings_are_isolated() {
        let first=AudioHealth::new();let second=AudioHealth::new();assert!(second.session_id>first.session_id);
        first.observe(true,&[0.0;1024]);first.observe(false,&[0.1]);let ticket=first.begin_work();first.record_fault("asr_failed");
        let a=first.snapshot(true,true,false,true,true);let b=second.snapshot(true,true,false,true,true);
        assert!(a.system_frame_age_ms.is_some());assert!(a.system_signal_age_ms.is_none());assert!(a.microphone_frame_age_ms.is_some());assert_eq!(a.asr_pending,1);assert!(a.asr_no_progress_ms.is_some());assert_eq!(a.fault.unwrap().sequence,1);
        assert!(b.system_frame_age_ms.is_none());assert_eq!(b.asr_pending,0);assert!(b.fault.is_none());
        drop(ticket);let a=first.snapshot(true,true,false,true,true);assert_eq!(a.asr_pending,0);assert!(a.asr_no_progress_ms.is_none());
        assert_eq!(first.record_fault("capture_error").sequence,2);
        first.record_fault("asr_init_failed");first.record_fault("capture_error");
        assert!(first.snapshot(true,true,false,true,true).restart_required);
        assert!(!second.snapshot(true,true,false,true,true).restart_required);
        second.record_fault("asr_worker_failed");second.record_fault("capture_error");
        assert_eq!(second.snapshot(true,true,false,true,true).restart_reason.as_deref(),Some("asr_worker_failed"));
    }
    #[tokio::test]
    async fn pending_work_is_released_on_send_failure_and_worker_abort() {
        let health=AudioHealth::new();
        let (tx,rx)=tokio::sync::mpsc::unbounded_channel();drop(rx);
        assert!(tx.send(health.begin_work()).is_err());
        assert_eq!(health.snapshot(true,true,false,true,false).asr_pending,0);
        let ticket=health.begin_work();
        let task=tokio::spawn(async move {let _ticket=ticket;std::future::pending::<()>().await;});
        assert_eq!(health.snapshot(true,true,false,true,false).asr_pending,1);
        task.abort();let _=task.await;
        let snapshot=health.snapshot(true,true,false,true,false);
        assert_eq!(snapshot.asr_pending,0);assert!(snapshot.asr_no_progress_ms.is_none());
    }
}
