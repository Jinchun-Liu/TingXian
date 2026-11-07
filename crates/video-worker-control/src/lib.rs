//! Main-controller authority for inspection and explicit native offline eye jobs.
//! No webview/camera start command is added. Restoration constructs an idle controller.
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::{Arc, Condvar, Mutex}, time::Duration};
mod eye;
pub use eye::{EyeJob, EyeOutcome, EyeProvider, EyeReport};
mod stream;
pub use stream::{NativeOutputEndpoint, StreamFeed, StreamFrame, StreamInputStatus, StreamReport, STREAM_BGR_BYTES};
#[cfg(windows)]
mod preview;
#[cfg(windows)]
pub use preview::{NativePreview, PreviewFrame, PreviewPhase, PreviewStatus};
#[cfg(windows)]
mod output;
#[cfg(windows)]
pub use output::{PinnedOutputLibrary, RelayPhase, RelayStatus, OutputSnapshot};
#[cfg(windows)]
mod windows;
#[cfg(all(test, windows))]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase { Off, Preparing, Ready, Unavailable, Stopping, Fault, Completed }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason { NotBundled, UnsupportedPlatform, RuntimeNotPinned, Integrity, Launch, Protocol, WorkerExit, Timeout, ForcedStop, Cleanup, Abandoned, EyeInput, EyeOutput, EyeProcessing, ProviderCleanup }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inspection {
    pub protocol: u32,
    pub api_exports_present: bool,
    pub machine_registration_present: bool,
    pub user_registration_conflict: bool,
    pub api_error: u32,
    pub registration_error: u32,
    pub runtime: String,
    pub media_started: bool,
}
impl Inspection {
    fn parse(bytes: &[u8]) -> Result<Self, Reason> {
        if bytes.len() > 4096 { return Err(Reason::Protocol); }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| Reason::Protocol)?;
        // Only the currently implemented, no-media inspection protocol is valid.
        // Future processing requires a separate explicit capability/intent path.
        if value.protocol != 1 || value.media_started || value.runtime != "not_pinned" {
            return Err(Reason::Protocol);
        }
        Ok(value)
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot { pub generation: u64, pub phase: Phase, pub reason: Option<Reason>, pub inspection: Option<Inspection>, pub eye: Option<EyeReport>, pub stream: Option<StreamReport> }
impl Default for Snapshot {
    fn default() -> Self { Self { generation: 0, phase: Phase::Off, reason: None, inspection: None, eye: None, stream: None } }
}
// Native-only input. Never deserialize a webview-supplied path/hash/command.
pub struct BundledWorker { pub path: PathBuf, pub sha256: String }
enum Work { Inspect, Eye(EyeJob), Stream(stream::StreamJob) }
enum Report { Inspection(Inspection), Eye(EyeReport), Stream(StreamReport) }
#[cfg(windows)]
type Signal = windows::Signal;
#[cfg(not(windows))]
struct Signal;
#[cfg(not(windows))]
impl Signal {
    fn new() -> Result<Self, Reason> { Err(Reason::UnsupportedPlatform) }
    fn stop(&self) {}
    fn stopped(&self) -> bool { true }
}
struct Active { id: u64, signal: Arc<Signal>, stream: Option<Arc<stream::Shared>> }
#[derive(Default)]
struct Inner { snapshot: Snapshot, suspended: bool, cleanup_uncertain: bool, active: Option<Active> }
#[derive(Default)]
pub struct Controller { inner: Mutex<Inner>, idle: Condvar }
impl Controller {
    pub fn snapshot(&self) -> Snapshot { self.inner.lock().unwrap_or_else(|e|e.into_inner()).snapshot.clone() }
    pub fn begin(self: &Arc<Self>, worker: Option<BundledWorker>) -> Result<Option<Run>, &'static str> {
        self.begin_inner(worker,None,Work::Inspect)
    }
    pub fn begin_if_current(self:&Arc<Self>,worker:Option<BundledWorker>,generation:u64)->Result<Option<Run>,&'static str> {
        self.begin_inner(worker,Some(generation),Work::Inspect)
    }
    // Native caller must have explicit offline-input permission. Never expose
    // this method by accepting a webview-supplied executable/hash/path/intent.
    pub fn begin_eye_if_current(self:&Arc<Self>,worker:Option<BundledWorker>,job:EyeJob,generation:u64)->Result<Option<Run>,&'static str> {
        self.begin_inner(worker,Some(generation),Work::Eye(job))
    }
    /// Native-only explicit intent. This grants no camera or virtual-device
    /// permission. Ready means the worker is prepared, not that anyone sees it.
    pub fn begin_stream_if_current(self:&Arc<Self>,worker:Option<BundledWorker>,provider:EyeProvider,generation:u64)->Result<Option<(Run,StreamFeed)>,&'static str> {
        let (job,feed)=stream::pair(provider);
        Ok(self.begin_inner(worker,Some(generation),Work::Stream(job))?.map(|run|(run,feed)))
    }
    fn begin_inner(self:&Arc<Self>,worker:Option<BundledWorker>,generation:Option<u64>,work:Work)->Result<Option<Run>,&'static str> {
        let mut inner = self.inner.lock().unwrap_or_else(|e|e.into_inner());
        if generation.is_some_and(|value|value!=inner.snapshot.generation) {return Err("视频状态已变化，请重新检查");}
        if inner.suspended { return Err("正在退出，视频检查已暂停"); }
        if inner.cleanup_uncertain { return Err("视频进程清理未确认，请保留诊断并重启应用后再检查"); }
        if inner.active.is_some() { return Err("视频工作进程仍在运行，请先停止"); }
        inner.snapshot.generation = inner.snapshot.generation.checked_add(1).ok_or("视频代次已耗尽，请重启应用")?;
        inner.snapshot.inspection = None;
        inner.snapshot.eye = None;
        inner.snapshot.stream = None;
        let Some(worker) = worker else {
            inner.snapshot.phase = Phase::Unavailable; inner.snapshot.reason = Some(Reason::NotBundled); return Ok(None);
        };
        let signal = match Signal::new() {
            Ok(signal) => Arc::new(signal),
            Err(reason) => { inner.snapshot.phase = Phase::Unavailable; inner.snapshot.reason = Some(reason); return Ok(None); }
        };
        let id = inner.snapshot.generation;
        inner.snapshot.phase = Phase::Preparing; inner.snapshot.reason = None;
        let stream=if let Work::Stream(job)=&work { job.shared.admit(id);Some(job.shared.clone()) } else { None };
        inner.active = Some(Active { id, signal: signal.clone(), stream });
        Ok(Some(Run { controller: self.clone(), signal, worker, work, id, finished: false }))
    }
    fn stop_locked(inner: &mut Inner) {
        if inner.cleanup_uncertain { return; }
        inner.snapshot.generation = inner.snapshot.generation.saturating_add(1);
        inner.snapshot.inspection = None;
        inner.snapshot.eye = None;
        inner.snapshot.stream = None;
        if let Some(active) = &inner.active {
            if let Some(stream)=&active.stream { stream.close(); }
            active.signal.stop(); inner.snapshot.phase = Phase::Stopping;
        } else { inner.snapshot.phase = Phase::Off; inner.snapshot.reason = None; }
    }
    pub fn request_stop(&self) { Self::stop_locked(&mut self.inner.lock().unwrap_or_else(|e|e.into_inner())); }
    pub fn suspend(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e|e.into_inner());
        inner.suspended = true; Self::stop_locked(&mut inner);
    }
    pub fn resume(&self) { self.inner.lock().unwrap_or_else(|e|e.into_inner()).suspended = false; }
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let inner = self.inner.lock().unwrap_or_else(|e|e.into_inner());
        let (inner, _) = self.idle.wait_timeout_while(inner, timeout, |s|s.active.is_some()).unwrap_or_else(|e|e.into_inner());
        inner.active.is_none() && !inner.cleanup_uncertain
    }
    #[cfg(windows)]
    fn publish_stream_endpoint(&self,id:u64,endpoint:NativeOutputEndpoint)->bool {
        let mut inner=self.inner.lock().unwrap_or_else(|e|e.into_inner());
        let Some(active)=&inner.active else { return false; };
        if active.id!=id||inner.snapshot.generation!=id||active.signal.stopped()||inner.snapshot.phase!=Phase::Preparing { return false; }
        if !active.stream.as_ref().is_some_and(|s|s.publish(endpoint)) { return false; }
        inner.snapshot.phase=Phase::Ready;true
    }
    fn complete(&self, id: u64, cancelled: bool, result: Result<Report, Reason>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e|e.into_inner());
        if inner.active.as_ref().map(|a|a.id) != Some(id) { return; }
        // Recheck under the admission lock. The caller's atomic snapshot can
        // precede a stop that wins this lock before completion is published.
        // Drop also signals its owned event for cleanup; that is not user intent.
        // Only a controller stop changes this locked generation/phase pair.
        let cancelled = cancelled || inner.snapshot.generation != id
            || inner.snapshot.phase == Phase::Stopping;
        // Cancellation wins over late reports, but never conceals forced cleanup.
        let cleanup_failure = matches!(&result, Err(Reason::ForcedStop | Reason::Cleanup | Reason::Timeout)
            | Ok(Report::Eye(EyeReport { cleanup_uncertain: true, .. }))
            | Ok(Report::Stream(StreamReport { cleanup_uncertain: true, .. })));
        if matches!(result, Err(Reason::Cleanup)) { inner.cleanup_uncertain = true; }
        if let Some(stream)=inner.active.as_ref().and_then(|a|a.stream.as_ref()) { stream.close(); }
        inner.active = None;
        inner.snapshot.stream = None;
        if cancelled && !cleanup_failure {
            inner.snapshot.phase = Phase::Off; inner.snapshot.reason = None; inner.snapshot.inspection = None; inner.snapshot.eye = None;
        } else {
            match result {
                Ok(Report::Inspection(report)) => { inner.snapshot.phase = Phase::Unavailable; inner.snapshot.reason = Some(Reason::RuntimeNotPinned); inner.snapshot.inspection = Some(report); }
                Ok(Report::Eye(report)) => {
                    inner.snapshot.phase = if report.outcome == EyeOutcome::Completed { Phase::Completed } else { Phase::Fault };
                    inner.snapshot.reason = match report.outcome {
                        EyeOutcome::Completed => None,
                        _ if report.cleanup_uncertain => Some(Reason::ProviderCleanup),
                        _ => Some(Reason::EyeProcessing),
                    };
                    inner.snapshot.eye = Some(report);
                }
                Ok(Report::Stream(report)) => {
                    inner.snapshot.phase=if report.outcome==EyeOutcome::Completed {Phase::Completed} else {Phase::Fault};
                    inner.snapshot.reason=match report.outcome { EyeOutcome::Completed=>None,
                        _ if report.cleanup_uncertain=>Some(Reason::ProviderCleanup),_=>Some(Reason::EyeProcessing) };
                    inner.snapshot.stream=Some(report);
                }
                Err(reason) => { inner.snapshot.phase = Phase::Fault; inner.snapshot.reason = Some(reason); inner.snapshot.inspection = None; inner.snapshot.eye = None; }
            }
        }
        self.idle.notify_all();
    }
}
// Reserved before async dispatch. Dropping a queued task settles its slot and
// invalidates its authority instead of leaving the app permanently busy.
pub struct Run { controller: Arc<Controller>, signal: Arc<Signal>, worker: BundledWorker, work: Work, id: u64, finished: bool }
impl Run {
    pub fn execute(self) -> Snapshot {
        let timeout = match &self.work { Work::Eye(_)=>120, Work::Stream(_)=>7200, Work::Inspect=>5 };
        self.execute_with_timeout(Duration::from_secs(timeout))
    }
    fn execute_with_timeout(mut self, timeout: Duration) -> Snapshot {
        #[cfg(windows)]
        let result = windows::execute(&self.worker, &self.signal, &self.work, self.id, timeout, &self.controller);
        #[cfg(not(windows))]
        let result = { let _ = timeout; Err(Reason::UnsupportedPlatform) };
        self.controller.complete(self.id, self.signal.stopped(), result);
        self.finished = true;
        self.controller.snapshot()
    }
}
impl Drop for Run {
    fn drop(&mut self) {
        if !self.finished {
            let already_cancelled=self.signal.stopped();
            self.signal.stop();self.controller.complete(self.id, already_cancelled, Err(Reason::Abandoned));
        }
    }
}

#[cfg(test)]
mod contracts {
    use super::*;
    #[test]
    fn restored_state_never_starts_and_missing_bundle_has_explicit_reason() {
        let controller = Arc::new(Controller::default());
        assert_eq!(controller.snapshot().phase, Phase::Off);
        assert!(controller.begin(None).unwrap().is_none());
        assert_eq!(controller.snapshot().reason, Some(Reason::NotBundled));
        controller.suspend(); assert!(controller.begin(None).is_err());
        controller.resume(); assert_eq!(controller.snapshot().phase, Phase::Off);
        assert!(controller.wait_idle(Duration::ZERO));
    }
    #[test]
    fn untrusted_or_media_capable_report_is_rejected() {
        let good = br#"{"protocol":1,"api_exports_present":true,"machine_registration_present":false,"user_registration_conflict":false,"api_error":0,"registration_error":2147746132,"runtime":"not_pinned","media_started":false}"#;
        assert!(Inspection::parse(good).is_ok());
        for value in [String::from_utf8_lossy(good).replace("not_pinned", "ready"),
                      String::from_utf8_lossy(good).replace("\"media_started\":false", "\"media_started\":true"),
                      String::from_utf8_lossy(good).replace("\"protocol\":1", "\"protocol\":2"),
                      String::from_utf8_lossy(good).replace("\"protocol\":1", "\"protocol\":1,\"path\":\"private\"")] {
            assert_eq!(Inspection::parse(value.as_bytes()), Err(Reason::Protocol));
        }
        assert_eq!(Inspection::parse(&vec![b' ';4097]), Err(Reason::Protocol));
    }
}
