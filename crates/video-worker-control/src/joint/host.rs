//! Main-process native owner lane. No capture, registration, UI start command,
//! or serializable endpoint authority. Inputs are already authorized native PCM.
use super::{Config, JointOutput, Snapshot as JointSnapshot};
use crate::{
    av_timeline::{PcmPacket, Source, MAX_PACKET_FRAMES, RATE},
    stream::Shared as StreamLease,
    timed_video::TimedEndpoint,
    Controller, NativeCorrections,
};
use std::{
    collections::VecDeque,
    os::windows::io::BorrowedHandle,
    ptr::null,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::Threading::{CreateEventW, SetEvent},
};
const MAX_QUEUED_FRAMES: usize = 2400;
const MAX_PACKETS: usize = 16;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Permission,
    Stale,
    Suspended,
    Busy,
    Source,
    Closed,
    Backpressure,
    Thread,
    Output(super::Error),
    SourceRetirement,
    CleanupUncertain,
}
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub generation: u64,
    pub busy: bool,
    pub suspended: bool,
    pub cleanup_uncertain: bool,
    pub queued_frames: usize,
    pub peak_frames: usize,
    pub admitted_packets: u64,
    pub cleared_frames: u64,
    pub failure: Option<Error>,
    pub output: Option<JointSnapshot>,
}
struct StopEvent(HANDLE);
// Kernel event operations are thread-safe; Shared owns its lifetime.
unsafe impl Send for StopEvent {}
unsafe impl Sync for StopEvent {}
impl StopEvent {
    fn new() -> Result<Self, Error> {
        let handle = unsafe { CreateEventW(null(), 1, 0, null()) };
        if handle.is_null() {
            Err(Error::Thread)
        } else {
            Ok(Self(handle))
        }
    }
    fn signal(&self) {
        unsafe {
            SetEvent(self.0);
        }
    }
    fn handle(&self) -> BorrowedHandle<'_> {
        unsafe { BorrowedHandle::borrow_raw(self.0) }
    }
}
impl Drop for StopEvent {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct State {
    closed: bool,
    finished: bool,
    clean: bool,
    failure: Option<Error>,
    queue: VecDeque<PcmPacket>,
    frames: usize,
    peak: usize,
    admitted: u64,
    cleared: u64,
    endpoint: Option<TimedEndpoint>,
    endpoint_taken: bool,
    output: Option<JointSnapshot>,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    stop: StopEvent,
    generation: u64,
    microphone: u64,
    source: Arc<Controller>,
    source_id: u64,
    source_lease: Arc<StreamLease>,
}
impl Shared {
    fn close(&self, failure: Option<Error>) {
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.closed = true;
            if s.failure.is_none() {
                s.failure = failure;
            }
            s.cleared += s.frames as u64;
            s.frames = 0;
            s.queue.clear();
            s.endpoint = None;
            self.stop.signal();
            self.changed.notify_all();
        }
        // Never invoke the controller or wait for I/O under the ingress lock.
        // Identity+lease comparison prevents an old handle stopping a later run.
        self.source.stop_stream(self.source_id, &self.source_lease);
    }
    fn closed(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).closed
    }
    fn pop(&self) -> Option<PcmPacket> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.closed {
            return None;
        }
        let packet = s.queue.pop_front()?;
        s.frames -= packet.samples.len();
        Some(packet)
    }
    fn publish(&self, endpoint: TimedEndpoint) -> Result<(), Error> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.closed {
            return Err(Error::Closed);
        }
        s.endpoint = Some(endpoint);
        self.changed.notify_all();
        Ok(())
    }
    fn output(&self, output: JointSnapshot) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).output = Some(output);
    }
}
/// One native input authority. Drop means stop, not implicit background output.
/// It cannot supply raw video, select DLLs, change the source or start capture.
pub struct Input {
    shared: Arc<Shared>,
}
impl Input {
    pub fn generation(&self) -> u64 {
        self.shared.generation
    }
    pub fn submit(&self, generation: u64, packet: PcmPacket) -> Result<(), Error> {
        if generation != self.shared.generation {
            return Err(Error::Stale);
        }
        let result = self.shared.source_lease.while_open(|open, source_id| {
            if !open || source_id != self.shared.source_id {
                return Err(Error::Closed);
            }
            let mut s = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if s.closed {
                return Err(Error::Closed);
            }
            if packet.source != Source::SelfMicrophone
                || packet.device != self.shared.microphone
                || packet.rate != RATE
                || packet.channels != 1
                || packet.samples.is_empty()
                || packet.samples.len() > MAX_PACKET_FRAMES
            {
                Err(Error::Source)
            } else if s.queue.len() >= MAX_PACKETS
                || s.frames + packet.samples.len() > MAX_QUEUED_FRAMES
            {
                Err(Error::Backpressure)
            } else {
                s.frames += packet.samples.len();
                s.peak = s.peak.max(s.frames);
                s.admitted += 1;
                s.queue.push_back(packet);
                Ok(())
            }
        });
        if let Err(e) = result {
            self.shared.close(Some(e));
        }
        result
    }
    /// Native device owner consumes this capability once. Never serialize it
    /// into status, logs, a display window, settings, or an untrusted caller.
    pub fn take_endpoint(&self, timeout: Duration) -> Result<TimedEndpoint, Error> {
        if self.shared.source_lease.closed() {
            self.shared.close(None);
            return Err(Error::Closed);
        }
        let s = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let (s, _) = self
            .shared
            .changed
            .wait_timeout_while(s, timeout, |s| {
                !s.closed && s.endpoint.is_none() && !s.endpoint_taken
            })
            .unwrap_or_else(|e| e.into_inner());
        drop(s); // No reverse ingress -> source lock order and no wait under the source lease.
        let result = self.shared.source_lease.while_open(|open, source_id| {
            if !open || source_id != self.shared.source_id {
                return Err(Error::Closed);
            }
            let mut s = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if s.closed {
                return Err(s.failure.unwrap_or(Error::Closed));
            }
            if s.endpoint_taken {
                return Err(Error::Source);
            }
            let endpoint = s.endpoint.take().ok_or(Error::Busy)?;
            s.endpoint_taken = true;
            Ok(endpoint)
        });
        if matches!(result, Err(Error::Closed)) {
            self.shared.close(None);
        }
        result
    }
    pub fn request_stop(&self) {
        self.shared.close(None);
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        self.shared.close(None);
    }
}
struct Active {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}
#[derive(Default)]
struct Inner {
    generation: u64,
    suspended: bool,
    uncertain: bool,
    active: Option<Active>,
    last: Option<Snapshot>,
}
/// Default is inert. Thread creation requires a native, explicitly authorized
/// config and a reader belonging to the supplied source controller.
#[derive(Default)]
pub struct Host {
    inner: Mutex<Inner>,
}
impl Host {
    pub fn snapshot(&self) -> Snapshot {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = inner.last.unwrap_or(Snapshot {
            generation: 0,
            busy: false,
            suspended: false,
            cleanup_uncertain: false,
            queued_frames: 0,
            peak_frames: 0,
            admitted_packets: 0,
            cleared_frames: 0,
            failure: None,
            output: None,
        });
        out.generation = inner.generation;
        out.busy = inner.active.is_some();
        out.suspended = inner.suspended;
        out.cleanup_uncertain = inner.uncertain;
        if let Some(active) = &inner.active {
            let s = active
                .shared
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            out.queued_frames = s.frames;
            out.peak_frames = s.peak;
            out.admitted_packets = s.admitted;
            out.cleared_frames = s.cleared;
            out.failure = s.failure;
            out.output = s.output;
        }
        out
    }
    pub fn start(
        &self,
        config: Config,
        source: Arc<Controller>,
        reader: NativeCorrections,
        expected_generation: u64,
        authorized: bool,
    ) -> Result<Input, Error> {
        self.start_inner(
            config,
            source,
            reader,
            expected_generation,
            authorized,
            |_| {},
        )
    }
    #[cfg(test)]
    pub(super) fn start_tapped<F>(
        &self,
        config: Config,
        source: Arc<Controller>,
        reader: NativeCorrections,
        expected_generation: u64,
        authorized: bool,
        tap: F,
    ) -> Result<Input, Error>
    where
        F: FnMut(&mut JointOutput) + Send + 'static,
    {
        self.start_inner(config, source, reader, expected_generation, authorized, tap)
    }
    fn start_inner<F>(
        &self,
        config: Config,
        source: Arc<Controller>,
        reader: NativeCorrections,
        expected_generation: u64,
        authorized: bool,
        tap: F,
    ) -> Result<Input, Error>
    where
        F: FnMut(&mut JointOutput) + Send + 'static,
    {
        if !authorized {
            return Err(Error::Permission);
        }
        let source_id = reader.generation();
        let source_lease = reader.lease();
        if !source.owns_stream(source_id, &source_lease) {
            return Err(Error::Source);
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if expected_generation != inner.generation {
            return Err(Error::Stale);
        }
        if inner.suspended {
            return Err(Error::Suspended);
        }
        if inner.uncertain {
            return Err(Error::CleanupUncertain);
        }
        if inner.active.is_some() {
            return Err(Error::Busy);
        }
        let generation = inner
            .generation
            .checked_add(1)
            .ok_or(Error::CleanupUncertain)?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                closed: false,
                finished: false,
                clean: false,
                failure: None,
                queue: VecDeque::new(),
                frames: 0,
                peak: 0,
                admitted: 0,
                cleared: 0,
                endpoint: None,
                endpoint_taken: false,
                output: None,
            }),
            changed: Condvar::new(),
            stop: StopEvent::new()?,
            generation,
            microphone: config.selected_microphone,
            source,
            source_id,
            source_lease,
        });
        let running = shared.clone();
        let thread = thread::Builder::new()
            .name("tingxian-media-output".into())
            .spawn(move || {
                let clean = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    drive(config, reader, &running, tap)
                })) {
                    Ok(clean) => clean,
                    Err(_) => {
                        running.close(Some(Error::Thread));
                        false
                    }
                };
                running.close(None);
                let source_clean = running.source.wait_stream_idle(
                    running.source_id,
                    &running.source_lease,
                    Duration::from_secs(7),
                );
                if !source_clean {
                    running.close(Some(Error::SourceRetirement));
                }
                let mut s = running.state.lock().unwrap_or_else(|e| e.into_inner());
                s.clean = clean && source_clean;
                s.finished = true;
                running.changed.notify_all();
            })
            .map_err(|_| Error::Thread)?;
        inner.generation = generation;
        inner.active = Some(Active {
            shared: shared.clone(),
            thread: Some(thread),
        });
        Ok(Input { shared })
    }
    fn stop(&self, suspend: bool) {
        let shared = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if suspend {
                inner.suspended = true;
            }
            inner.generation = inner.generation.saturating_add(1);
            inner.active.as_ref().map(|a| a.shared.clone())
        };
        if let Some(shared) = shared {
            shared.close(None);
        }
    }
    pub fn request_stop(&self) {
        self.stop(false);
    }
    pub fn suspend(&self) {
        self.stop(true);
    }
    pub fn resume(&self) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .suspended = false;
    }
    /// True means actual local thread/source retirement and verified local
    /// cleanup. It never clears the persistent external-drain restart barrier.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let until = Instant::now() + timeout;
        loop {
            let retire = {
                let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                match &mut inner.active {
                    None => return !inner.uncertain,
                    Some(active) if active.thread.as_ref().is_some_and(|t| t.is_finished()) => {
                        Some((active.shared.clone(), active.thread.take().unwrap()))
                    }
                    _ => None,
                }
            };
            if let Some((shared, thread)) = retire {
                let joined = thread.join().is_ok();
                let (finished, clean, mut snapshot) = {
                    let s = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                    (
                        s.finished,
                        s.clean,
                        Snapshot {
                            generation: 0,
                            busy: false,
                            suspended: false,
                            cleanup_uncertain: false,
                            queued_frames: s.frames,
                            peak_frames: s.peak,
                            admitted_packets: s.admitted,
                            cleared_frames: s.cleared,
                            failure: s.failure,
                            output: s.output,
                        },
                    )
                };
                let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                inner.uncertain |= !joined || !finished || !clean;
                snapshot.generation = inner.generation;
                snapshot.suspended = inner.suspended;
                snapshot.cleanup_uncertain = inner.uncertain;
                inner.last = Some(snapshot);
                inner.active = None;
                return !inner.uncertain;
            }
            if Instant::now() >= until {
                return false;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        // Normal application quit explicitly waits. Drop requests stop without
        // blocking the UI indefinitely; the detached lane owns cleanup lifetime.
        self.request_stop();
    }
}
fn drive<F>(config: Config, reader: NativeCorrections, shared: &Arc<Shared>, mut tap: F) -> bool
where
    F: FnMut(&mut JointOutput),
{
    let mut clean = true;
    let result = (|| {
        if shared.closed() {
            return Ok(());
        }
        let candidate = super::Candidate::inspect(config).map_err(Error::Output)?;
        if shared.closed() {
            return Ok(());
        }
        let (mut owner, endpoint) = candidate.activate(true).map_err(|failure| {
            clean = failure.cleanup.is_none_or(|c| c.failure().is_none());
            Error::Output(failure.cause)
        })?;
        let result = (|| {
            owner.bind_corrections(reader).map_err(Error::Output)?;
            shared.publish(endpoint)?;
            while !shared.closed() {
                tap(&mut owner);
                for _ in 0..MAX_PACKETS {
                    let Some(packet) = shared.pop() else {
                        break;
                    };
                    owner
                        .push_audio(owner.generation, packet)
                        .map_err(Error::Output)?;
                }
                if shared.closed() {
                    break;
                }
                let progress = owner.wait(shared.stop.handle()).map_err(Error::Output)?;
                shared.output(owner.snapshot());
                if matches!(progress, super::Progress::Stopped) {
                    break;
                }
            }
            Ok(())
        })();
        let cleanup = owner.stop();
        clean = cleanup.failure().is_none();
        if let Some(e) = cleanup.failure() {
            shared.close(Some(Error::Output(e)));
        }
        shared.output(owner.snapshot());
        drop(owner); // Release all native instances/DLLs on this owning lane.
        result
    })();
    if let Err(e) = result {
        shared.close(Some(e));
    }
    clean
}
