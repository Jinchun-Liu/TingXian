//! Native-only QPC worker input and bounded corrected-frame reception.
//! No capture, SDK selection, MF activation, device output or UI authority.
use crate::{
    av_timeline::{Time, NV12_BYTES},
    preview::{connect_native, transfer, Handle},
    stream::{PendingFrame, Protocol, Shared},
    NativeOutputEndpoint, PreviewPhase, Reason, StreamFeed, StreamInputStatus,
};
use serde::Serialize;
use std::{
    ptr::null_mut,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::System::Pipes::PeekNamedPipe;

const IO_BOUND: Duration = Duration::from_millis(500);
const MAX_AGE: u64 = 2_500_000;
fn now() -> Result<Time, Reason> {
    crate::audio::native_now().map_err(|_| Reason::Protocol)
}
fn fresh(capture: Time, at: Time) -> bool {
    capture.qpc_100ns() != 0 && capture <= at && at.qpc_100ns() - capture.qpc_100ns() <= MAX_AGE
}

pub struct QpcStreamFrame {
    pub id: u64,
    pub capture: Time,
    pub bgr: Vec<u8>,
}
impl Drop for QpcStreamFrame {
    fn drop(&mut self) {
        self.bgr.fill(0);
    }
}
// Separate type: no legacy submit/open_preview/take_endpoint escape hatch.
pub struct QpcStreamFeed {
    inner: StreamFeed,
}
impl QpcStreamFeed {
    pub(crate) fn new(inner: StreamFeed) -> Self {
        Self { inner }
    }
    pub fn submit(&self, mut frame: QpcStreamFrame) -> Result<bool, Reason> {
        let valid = now().is_ok_and(|at| fresh(frame.capture, at));
        if self.inner.shared.protocol != Protocol::Qpc || !valid {
            self.inner.shared.reject_input();
            return Err(Reason::EyeInput);
        }
        self.inner.submit_pending(PendingFrame {
            id: frame.id,
            time: frame.capture.qpc_100ns(),
            bgr: std::mem::take(&mut frame.bgr),
        })
    }
    pub fn finish(&self) -> bool {
        self.inner.finish()
    }
    pub fn status(&self) -> StreamInputStatus {
        self.inner.status()
    }
    pub fn open_corrections(&self, timeout: Duration) -> Result<NativeCorrections, Reason> {
        let endpoint = self.inner.take_endpoint(timeout)?;
        let result = NativeCorrections::connect(endpoint, self.inner.shared.clone());
        if result.is_err() {
            self.inner.shared.close();
        }
        result
    }
}

// Pixels/transport handles are never serialized or logged. Borrow under the
// live lease; a handoff must not leave an independently replayable old frame.
pub struct CorrectedEyeFrame {
    pub generation: u64,
    pub worker_generation: u64,
    pub id: u64,
    pub capture: Time,
    pub completed: Time,
    pub nv12: Vec<u8>,
}
impl Drop for CorrectedEyeFrame {
    fn drop(&mut self) {
        self.nv12.fill(0);
    }
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct CorrectionStatus {
    pub phase: PreviewPhase,
    pub reason: Option<Reason>,
    pub received: u64,
    pub rejected: u64,
    pub expired: u64,
    pub replaced: u64,
    pub discarded: u64,
    pub unverifiable: u64,
}
impl Default for CorrectionStatus {
    fn default() -> Self {
        Self {
            phase: PreviewPhase::Waiting,
            reason: None,
            received: 0,
            rejected: 0,
            expired: 0,
            replaced: 0,
            discarded: 0,
            unverifiable: 0,
        }
    }
}
struct Header {
    kind: u16,
    bytes: u32,
    generation: u64,
    id: u64,
}
impl Header {
    fn parse(b: &[u8; 40]) -> Result<Self, Reason> {
        if b[..4] != 0x31565854u32.to_le_bytes()
            || b[4..6] != 3u16.to_le_bytes()
            || b[12..16] != [0; 4]
            || b[32..40] != [0; 8]
        {
            return Err(Reason::Protocol);
        }
        Ok(Self {
            kind: u16::from_le_bytes(b[6..8].try_into().unwrap()),
            bytes: u32::from_le_bytes(b[8..12].try_into().unwrap()),
            generation: u64::from_le_bytes(b[16..24].try_into().unwrap()),
            id: u64::from_le_bytes(b[24..32].try_into().unwrap()),
        })
    }
    fn control(&self, kind: u16, generation: u64) -> bool {
        self.kind == kind && self.generation == generation && self.id == 0 && self.bytes == 0
    }
}
pub struct NativeCorrections {
    pipe: Option<Handle>,
    lease: Arc<Shared>,
    generation: u64,
    worker_generation: Option<u64>,
    last_id: u64,
    last_capture: Time,
    last_completed: Time,
    frame: Option<CorrectedEyeFrame>,
    status: CorrectionStatus,
}
impl Drop for NativeCorrections {
    fn drop(&mut self) {
        self.lease.close();
    }
}
impl NativeCorrections {
    fn connect(endpoint: NativeOutputEndpoint, lease: Arc<Shared>) -> Result<Self, Reason> {
        if lease.protocol != Protocol::Qpc {
            return Err(Reason::Protocol);
        }
        let generation = lease
            .while_open(|open, g| open.then_some(g))
            .ok_or(Reason::Abandoned)?;
        let pipe = connect_native(&endpoint, &lease, 3)?;
        Ok(Self {
            pipe: Some(pipe),
            lease,
            generation,
            worker_generation: None,
            last_id: 0,
            last_capture: Time::from_qpc_100ns(0),
            last_completed: Time::from_qpc_100ns(0),
            frame: None,
            status: CorrectionStatus::default(),
        })
    }
    fn clear(&mut self, replaced: bool) {
        if self.frame.take().is_some() {
            if replaced {
                self.status.replaced += 1;
            } else {
                self.status.discarded += 1;
            }
        }
    }
    fn end_local(&mut self, reason: Option<Reason>) {
        self.clear(false);
        self.pipe = None;
        self.status.reason = reason;
        self.status.phase = if reason.is_some() {
            PreviewPhase::Fault
        } else {
            PreviewPhase::Stopped
        };
    }
    fn refresh(&mut self) {
        if self.lease.closed() {
            self.end_local(self.status.reason);
        } else if self
            .frame
            .as_ref()
            .is_some_and(|f| !now().is_ok_and(|at| fresh(f.capture, at)))
        {
            self.status.expired += 1;
            self.clear(false);
        }
    }
    pub fn status(&mut self) -> CorrectionStatus {
        self.refresh();
        self.status
    }
    /// At most one bounded packet. No waiting, receiving or GPU work on the
    /// joint device-owner lane; service this reader on a separate native lane.
    pub fn poll(&mut self, wait: Duration) -> Result<bool, Reason> {
        self.refresh();
        if self.pipe.is_none() {
            return self.status.reason.map_or(Ok(false), Err);
        }
        let result = self.receive(wait.min(IO_BOUND));
        if let Err(reason) = result {
            if self.lease.closed() {
                self.end_local(self.status.reason);
                return Ok(false);
            }
            self.status.rejected += 1;
            self.end_local(Some(reason));
            self.lease.close();
        }
        result
    }
    fn receive(&mut self, wait: Duration) -> Result<bool, Reason> {
        let pipe = self.pipe.as_ref().ok_or(Reason::EyeOutput)?;
        let idle = Instant::now() + wait;
        loop {
            if self.lease.closed() {
                return Err(Reason::Abandoned);
            }
            let mut available = 0;
            if unsafe {
                PeekNamedPipe(
                    pipe.0,
                    null_mut(),
                    0,
                    null_mut(),
                    &mut available,
                    null_mut(),
                )
            } == 0
            {
                return Err(Reason::EyeOutput);
            }
            if available != 0 {
                break;
            }
            if Instant::now() >= idle {
                return Ok(false);
            }
            thread::sleep(Duration::from_millis(2));
        }
        let deadline = Instant::now() + IO_BOUND;
        let mut raw = [0; 40];
        transfer(pipe, false, &mut raw, &self.lease, deadline)?;
        let h = Header::parse(&raw)?;
        if h.kind == 3 {
            if self.worker_generation.is_some() || h.generation == 0 || !h.control(3, h.generation)
            {
                return Err(Reason::Protocol);
            }
            self.worker_generation = Some(h.generation);
            self.status.phase = PreviewPhase::Receiving;
            return Ok(false);
        }
        if h.kind == 5 {
            if self.worker_generation != Some(h.generation) || !h.control(5, h.generation) {
                return Err(Reason::Protocol);
            }
            self.end_local(None);
            self.lease.close();
            return Ok(false);
        }
        if self.worker_generation != Some(h.generation)
            || h.kind != 8
            || h.bytes as usize != NV12_BYTES + 16
            || h.id == 0
            || h.id <= self.last_id
        {
            return Err(Reason::Protocol);
        }
        let mut timing = [0; 16];
        transfer(pipe, false, &mut timing, &self.lease, deadline)?;
        let capture = Time::from_qpc_100ns(u64::from_le_bytes(timing[..8].try_into().unwrap()));
        let completed = Time::from_qpc_100ns(u64::from_le_bytes(timing[8..].try_into().unwrap()));
        if capture.qpc_100ns() == 0
            || capture < self.last_capture
            || completed < self.last_completed
            || completed < capture
            || completed > now()?
        {
            return Err(Reason::Protocol);
        }
        let verified = self.lease.verify_capture(h.id, capture.qpc_100ns())?;
        let mut frame = CorrectedEyeFrame {
            generation: self.generation,
            worker_generation: h.generation,
            id: h.id,
            capture,
            completed,
            nv12: vec![0; NV12_BYTES],
        };
        transfer(pipe, false, &mut frame.nv12, &self.lease, deadline)?;
        self.last_id = h.id;
        self.last_capture = capture;
        self.last_completed = completed;
        self.status.received += 1;
        // Whole-payload arrival and handoff are separate freshness boundaries.
        if !fresh(capture, now()?) {
            self.status.expired += 1;
            self.clear(false);
            return Ok(false);
        }
        if !verified {
            self.status.unverifiable += 1;
            self.clear(false);
            return Ok(false);
        }
        self.clear(true);
        self.frame = Some(frame);
        Ok(true)
    }
    /// Short synchronous native callback under the same stop lease. No pipe I/O,
    /// device commit/wait, Controller/Feed/reader reentry, or deferred replay.
    /// Copy into a bounded timeline only with a separate output lifecycle gate;
    /// this reader does not grant authority to activate joint/meeting output.
    pub fn with_current<T>(&mut self, handoff: impl FnOnce(Option<&CorrectedEyeFrame>) -> T) -> T {
        self.refresh();
        let lease = self.lease.clone();
        lease.while_open(|open, _| {
            if !open {
                self.end_local(self.status.reason);
            }
            if self
                .frame
                .as_ref()
                .is_some_and(|f| !now().is_ok_and(|at| fresh(f.capture, at)))
            {
                self.status.expired += 1;
                self.clear(false);
            }
            handoff(self.frame.as_ref())
        })
    }
}

#[cfg(test)]
#[path = "eye_qpc_tests.rs"]
mod tests;
