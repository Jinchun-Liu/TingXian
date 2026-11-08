//! One corrected-frame slot between the reader and MTA output owner. No device
//! I/O/wait under the stream lease or this slot mutex. Revocation is a request;
//! the owner separately resets audio, clears video and proves thread retirement.
use crate::{
    audio,
    av_timeline::{CorrectedFrame, Time},
    eye_qpc::{CorrectedEyeFrame, NativeCorrections},
    stream::{RevokeOutput, Shared},
    timed_video::Cancellation,
    PreviewPhase, Reason,
};
use std::{
    os::windows::io::BorrowedHandle,
    ptr::null,
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    System::Threading::{CreateEventW, ResetEvent, SetEvent},
};

const FIRST_FRAME: u64 = 20_000_000;
const MAX_AGE: u64 = 2_500_000;
fn now() -> Result<Time, Reason> {
    audio::native_now().map_err(|_| Reason::Protocol)
}
struct Event(HANDLE);
// Kernel events permit concurrent Set/Reset/Wait. Arc<Gate> owns their lifetime.
unsafe impl Send for Event {}
unsafe impl Sync for Event {}
impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EyeStatus {
    pub stream_generation: u64,
    pub received: u64,
    pub taken: u64,
    pub replaced: u64,
    pub discarded: u64,
    pub queued: bool,
    pub revoked: bool,
    pub reason: Option<Reason>,
    pub owner_in_flight: usize,
    pub capture_qpc: u64,
    pub completed_qpc: u64,
}
struct State {
    status: EyeStatus,
    frame: Option<CorrectedFrame>,
    last_id: u64,
}
struct Gate {
    state: Mutex<State>,
    event: Event,
    cancel: Cancellation,
    deadline: u64,
}
impl Gate {
    fn new(generation: u64, cancel: Cancellation) -> Result<Arc<Self>, Reason> {
        let at = now()?.qpc_100ns();
        let event = unsafe { CreateEventW(null(), 1, 0, null()) };
        if event.is_null() {
            return Err(Reason::Launch);
        }
        Ok(Arc::new(Self {
            state: Mutex::new(State {
                frame: None,
                last_id: 0,
                status: EyeStatus {
                    stream_generation: generation,
                    received: 0,
                    taken: 0,
                    replaced: 0,
                    discarded: 0,
                    queued: false,
                    revoked: false,
                    reason: None,
                    owner_in_flight: 0,
                    capture_qpc: 0,
                    completed_qpc: 0,
                },
            }),
            event: Event(event),
            cancel,
            deadline: at.saturating_add(FIRST_FRAME),
        }))
    }
    fn valid(&self, s: &State, at: Time) -> Result<bool, Reason> {
        if s.status.revoked {
            return Err(s.status.reason.unwrap_or(Reason::Abandoned));
        }
        let at = at.qpc_100ns();
        if s.status.received == 0 {
            return if at >= self.deadline {
                Err(Reason::Timeout)
            } else {
                Ok(false)
            };
        }
        if at < s.status.completed_qpc {
            return Err(Reason::Protocol);
        }
        if at - s.status.capture_qpc > MAX_AGE {
            return Err(Reason::EyeInput);
        }
        Ok(true)
    }
    fn check(&self) -> Result<bool, Reason> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.valid(&s, now()?)
    }
    fn publish(&self, f: &CorrectedEyeFrame) -> Result<(), Reason> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let at = now()?.qpc_100ns();
        if s.status.revoked {
            return Err(s.status.reason.unwrap_or(Reason::Abandoned));
        }
        if s.status.received == 0 && at >= self.deadline {
            return Err(Reason::Timeout);
        }
        if f.generation != s.status.stream_generation
            || f.id <= s.last_id
            || f.id == 0
            || f.nv12.len() != crate::av_timeline::NV12_BYTES
            || f.capture.qpc_100ns() == 0
            || f.capture.qpc_100ns() < s.status.capture_qpc
            || f.completed < f.capture
            || f.completed.qpc_100ns() > at
            || at - f.capture.qpc_100ns() > MAX_AGE
        {
            return Err(Reason::Protocol);
        }
        let frame = CorrectedFrame {
            id: f.id,
            capture: f.capture,
            nv12: Arc::from(f.nv12.as_slice()),
        };
        if s.frame.replace(frame).is_some() {
            s.status.replaced += 1;
        }
        s.last_id = f.id;
        s.status.received += 1;
        s.status.queued = true;
        s.status.capture_qpc = f.capture.qpc_100ns();
        s.status.completed_qpc = f.completed.qpc_100ns();
        unsafe {
            SetEvent(self.event.0);
        }
        Ok(())
    }
    fn revoke(&self, reason: Option<Reason>) {
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if !s.status.revoked || (s.status.reason == Some(Reason::Abandoned) && reason.is_some())
            {
                s.status.reason = reason;
            }
            s.status.revoked = true;
            if s.frame.take().is_some() {
                s.status.discarded += 1;
            }
            s.status.queued = false;
            unsafe {
                SetEvent(self.event.0);
            }
        }
        // Lifetime-safe concurrent cancel, no join or device wait. The audio
        // owner is woken separately and remains the only WASAPI caller.
        self.cancel.cancel();
    }
    fn snapshot(&self) -> EyeStatus {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).status
    }
    fn admit(self: &Arc<Self>) -> Result<Option<Permit>, Reason> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !self.valid(&s, now()?)? {
            return Ok(None);
        }
        if s.status.owner_in_flight != 0 {
            return Err(Reason::Protocol);
        }
        s.status.owner_in_flight = 1;
        Ok(Some(Permit(self.clone())))
    }
    fn take(&self) -> Result<Option<CorrectedFrame>, Reason> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !self.valid(&s, now()?)? {
            return Ok(None);
        }
        let frame = s.frame.take();
        if frame.is_some() {
            s.status.taken += 1;
        }
        s.status.queued = false;
        unsafe {
            ResetEvent(self.event.0);
        }
        Ok(frame)
    }
}
impl RevokeOutput for Gate {
    fn revoke(&self) {
        Gate::revoke(self, Some(Reason::Abandoned));
    }
}
pub(super) struct Permit(Arc<Gate>);
impl Permit {
    pub(super) fn check(&self) -> Result<(), Reason> {
        self.0
            .check()
            .and_then(|ready| if ready { Ok(()) } else { Err(Reason::EyeInput) })
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status
            .owner_in_flight = 0;
    }
}
pub(super) struct EyeRoute {
    gate: Arc<Gate>,
    lease: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}
impl EyeRoute {
    pub(super) fn start(reader: NativeCorrections, cancel: Cancellation) -> Result<Self, Reason> {
        let lease = reader.lease();
        let gate = Gate::new(reader.generation(), cancel)?;
        let observer: Arc<dyn RevokeOutput> = gate.clone();
        if !lease.attach_output(Arc::downgrade(&observer)) {
            return Err(Reason::Abandoned);
        }
        let run_gate = gate.clone();
        let thread = thread::Builder::new()
            .name("eye-joint-input".into())
            .spawn(move || {
                let gate = run_gate.clone();
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(reader, &gate)))
                    .is_err()
                {
                    run_gate.revoke(Some(Reason::Cleanup));
                }
            })
            .map_err(|_| Reason::Launch)?;
        Ok(Self {
            gate,
            lease,
            thread: Some(thread),
        })
    }
    pub(super) fn snapshot(&self) -> EyeStatus {
        self.gate.snapshot()
    }
    pub(super) fn admit(&self) -> Result<Option<Permit>, Reason> {
        self.gate.admit()
    }
    pub(super) fn take(&self) -> Result<Option<CorrectedFrame>, Reason> {
        self.gate.take()
    }
    pub(super) fn check(&self) -> Result<(), Reason> {
        self.gate
            .check()
            .and_then(|ready| if ready { Ok(()) } else { Err(Reason::EyeInput) })
    }
    pub(super) fn event(&self) -> BorrowedHandle<'_> {
        unsafe { BorrowedHandle::borrow_raw(self.gate.event.0) }
    }
    pub(super) fn request_stop(&self) {
        self.gate.revoke(None);
        self.lease.close();
    }
    pub(super) fn retire(&mut self) -> Result<(), Reason> {
        self.request_stop();
        let deadline = Instant::now() + Duration::from_secs(1);
        if let Some(thread) = &self.thread {
            while !thread.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            if !thread.is_finished() {
                return Err(Reason::Cleanup);
            }
            self.thread
                .take()
                .unwrap()
                .join()
                .map_err(|_| Reason::Cleanup)?;
        }
        Ok(())
    }
}
impl Drop for EyeRoute {
    fn drop(&mut self) {
        self.request_stop();
        // Unconfirmed retirement is already reported by owner.stop. Do not
        // invent success or wait forever here; a detached reader owns its pipe
        // and revoked gate, with an inert cancel pointer after video teardown.
        if self.thread.as_ref().is_some_and(|t| t.is_finished()) {
            let _ = self.thread.take().unwrap().join();
        }
    }
}
fn run(mut reader: NativeCorrections, gate: &Arc<Gate>) {
    while !gate.snapshot().revoked {
        let result = match reader.poll(Duration::from_millis(10)) {
            Ok(true) => {
                reader.with_current(|f| f.ok_or(Reason::EyeInput).and_then(|f| gate.publish(f)))
            }
            Ok(false) => {
                let status = reader.status();
                if matches!(status.phase, PreviewPhase::Stopped | PreviewPhase::Fault) {
                    Err(status.reason.unwrap_or(Reason::Abandoned))
                } else {
                    gate.check().map(|_| ())
                }
            }
            Err(reason) => Err(reason),
        };
        if let Err(reason) = result {
            gate.revoke(Some(reason));
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn eye_admitted_operation_is_distinct_from_revocation_and_no_new_admission_can_follow() {
        let mut video =
            crate::timed_video::NativeTimedVideo::load(crate::timed_video::tests::pin()).unwrap();
        let _endpoint = video.open(7, true).unwrap();
        let gate = Gate::new(41, video.cancellation()).unwrap();
        assert!(gate.admit().unwrap().is_none());
        let capture = now().unwrap();
        let frame = CorrectedEyeFrame {
            generation: 41,
            worker_generation: 1,
            id: 1,
            capture,
            completed: capture,
            nv12: vec![188; crate::av_timeline::NV12_BYTES],
        };
        gate.publish(&frame).unwrap();
        let permit = gate.admit().unwrap().unwrap();
        gate.revoke(Some(Reason::Protocol));
        assert_eq!(
            gate.snapshot().owner_in_flight,
            1,
            "request does not invent operation retirement"
        );
        assert!(matches!(gate.admit(), Err(Reason::Protocol)));
        assert_eq!(permit.check(), Err(Reason::Protocol));
        assert_eq!(gate.publish(&frame), Err(Reason::Protocol));
        assert!(!gate.snapshot().queued);
        drop(permit);
        assert_eq!(gate.snapshot().owner_in_flight, 0);
        video.stop().unwrap();
    }
}
