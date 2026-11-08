//! Main-process native owner lane. Capture is optional and explicitly authorized;
//! no registration, UI start command or serializable endpoint authority.
use super::{Config, JointOutput, Snapshot as JointSnapshot};
use crate::{
    av_timeline::{PcmPacket, Source, MAX_PACKET_FRAMES, RATE},
    camera, capture,
    stream::Shared as StreamLease,
    timed_video::TimedEndpoint,
    Controller, NativeCorrections, QpcStreamFeed, QpcStreamFrame,
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
    Capture(capture::Error),
    Camera(camera::Error),
    CameraFeed(crate::Reason),
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
    pub capture: Option<capture::Status>,
    pub capture_cleanup: Option<capture::Cleanup>,
    pub camera: Option<camera::Status>,
    pub camera_cleanup: Option<camera::Cleanup>,
    pub camera_submitted: u64,
    pub camera_gaps: u64,
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
    capture: Option<capture::Status>,
    capture_cleanup: Option<capture::Cleanup>,
    camera: Option<camera::Status>,
    camera_cleanup: Option<camera::Cleanup>,
    camera_submitted: u64,
    camera_gaps: u64,
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
    native_capture: bool,
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
    fn input_closed(&self) -> bool {
        self.closed() || self.source_lease.closed()
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
        // A capture-owned run has exactly one PCM producer. Reject injection
        // without stopping the legitimate owner or permitting source mixing.
        if self.shared.native_capture {
            return Err(Error::Source);
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
            capture: None,
            capture_cleanup: None,
            camera: None,
            camera_cleanup: None,
            camera_submitted: 0,
            camera_gaps: 0,
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
            out.capture = s.capture;
            out.capture_cleanup = s.capture_cleanup;
            out.camera = s.camera;
            out.camera_cleanup = s.camera_cleanup;
            out.camera_submitted = s.camera_submitted;
            out.camera_gaps = s.camera_gaps;
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
            None,
            None,
            |_| {},
        )
    }
    /// Native-only explicit microphone+output authorization. The return endpoint
    /// is derived from the selected output pair, not independently spoofable.
    /// Restored settings must never call this method automatically.
    pub fn start_captured(
        &self,
        config: Config,
        capture: capture::Config,
        source: Arc<Controller>,
        reader: NativeCorrections,
        expected_generation: u64,
        authorized: bool,
    ) -> Result<Input, Error> {
        let launch = capture.launch(config.pair.capture_id.clone());
        self.start_inner(
            config,
            source,
            reader,
            expected_generation,
            authorized,
            Some(launch),
            None,
            |_| {},
        )
    }
    /// Explicit full native input route. The feed must belong to this exact
    /// controller/reader lease, and ownership moves into the media thread.
    pub fn start_media(
        &self,
        config: Config,
        microphone: capture::Config,
        camera: camera::Config,
        feed: QpcStreamFeed,
        source: Arc<Controller>,
        reader: NativeCorrections,
        expected_generation: u64,
        authorized: bool,
    ) -> Result<Input, Error> {
        let microphone = microphone.launch(config.pair.capture_id.clone());
        self.start_inner(
            config,
            source,
            reader,
            expected_generation,
            authorized,
            Some(microphone),
            Some((camera.launch(), feed)),
            |_| {},
        )
    }
    #[cfg(test)]
    pub(super) fn start_media_tapped<F>(
        &self,
        config: Config,
        microphone: capture::Launch,
        camera: camera::Launch,
        feed: QpcStreamFeed,
        source: Arc<Controller>,
        reader: NativeCorrections,
        expected_generation: u64,
        authorized: bool,
        tap: F,
    ) -> Result<Input, Error>
    where
        F: FnMut(&mut JointOutput) + Send + 'static,
    {
        self.start_inner(
            config,
            source,
            reader,
            expected_generation,
            authorized,
            Some(microphone),
            Some((camera, feed)),
            tap,
        )
    }
    #[cfg(test)]
    pub(super) fn start_capture_tapped<F>(
        &self,
        config: Config,
        capture: capture::Launch,
        source: Arc<Controller>,
        reader: NativeCorrections,
        tap: F,
    ) -> Result<Input, Error>
    where
        F: FnMut(&mut JointOutput) + Send + 'static,
    {
        self.start_inner(
            config,
            source,
            reader,
            self.snapshot().generation,
            true,
            Some(capture),
            None,
            tap,
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
        self.start_inner(
            config,
            source,
            reader,
            expected_generation,
            authorized,
            None,
            None,
            tap,
        )
    }
    fn start_inner<F>(
        &self,
        config: Config,
        source: Arc<Controller>,
        reader: NativeCorrections,
        expected_generation: u64,
        authorized: bool,
        capture: Option<capture::Launch>,
        camera: Option<(camera::Launch, QpcStreamFeed)>,
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
        if camera
            .as_ref()
            .is_some_and(|(_, feed)| !feed.belongs_to(&source_lease))
        {
            return Err(Error::Source);
        }
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
                capture: None,
                capture_cleanup: None,
                camera: None,
                camera_cleanup: None,
                camera_submitted: 0,
                camera_gaps: 0,
            }),
            changed: Condvar::new(),
            stop: StopEvent::new()?,
            generation,
            microphone: config.selected_microphone,
            source,
            source_id,
            source_lease,
            native_capture: capture.is_some(),
        });
        let running = shared.clone();
        let thread = thread::Builder::new()
            .name("tingxian-media-output".into())
            .spawn(move || {
                let clean = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    drive(config, reader, &running, capture, camera, tap)
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
                            capture: s.capture,
                            capture_cleanup: s.capture_cleanup,
                            camera: s.camera,
                            camera_cleanup: s.camera_cleanup,
                            camera_submitted: s.camera_submitted,
                            camera_gaps: s.camera_gaps,
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
struct CaptureOwner<'a> {
    device: Box<dyn capture::Device>,
    selection: capture::Selection,
    shared: &'a Arc<Shared>,
    finished: Option<bool>,
}
impl CaptureOwner<'_> {
    fn finish(&mut self) -> bool {
        if let Some(clean) = self.finished {
            return clean;
        }
        let cleanup = self.device.finish();
        {
            let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.capture_cleanup = Some(cleanup);
            if let Ok(status) = cleanup.status {
                state.capture = Some(status);
            }
        }
        if let Some(e) = cleanup.failure() {
            self.shared.close(Some(Error::Capture(e)));
        }
        let clean = cleanup.failure().is_none();
        self.finished = Some(clean);
        clean
    }
    fn status(&self) -> Result<(), Error> {
        let status = self.device.status().map_err(Error::Capture)?;
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .capture = Some(status);
        Ok(())
    }
}
impl Drop for CaptureOwner<'_> {
    fn drop(&mut self) {
        self.finish();
    }
}
struct CameraOwner<'a> {
    device: Box<dyn camera::Device>,
    selected: String,
    feed: QpcStreamFeed,
    shared: &'a Arc<Shared>,
    revoked: Option<Result<(), camera::Error>>,
    finished: Option<bool>,
}
impl CameraOwner<'_> {
    fn revoke(&mut self) {
        if self.revoked.is_none() {
            self.revoked = Some(self.device.revoke());
        }
    }
    fn finish(&mut self) -> bool {
        if let Some(clean) = self.finished {
            return clean;
        }
        self.revoke();
        let mut cleanup = self.device.finish();
        if let Some(Err(e)) = self.revoked {
            cleanup.revoke = Err(e);
        }
        {
            let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.camera_cleanup = Some(cleanup);
            if let Ok(status) = cleanup.status {
                state.camera = Some(status);
            }
        }
        if let Some(e) = cleanup.failure() {
            self.shared.close(Some(Error::Camera(e)));
        }
        let clean = cleanup.failure().is_none();
        self.finished = Some(clean);
        clean
    }
    fn status(&self) -> Result<(), Error> {
        let status = self.device.status().map_err(Error::Camera)?;
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .camera = Some(status);
        Ok(())
    }
    fn pump(&mut self) -> Result<(), Error> {
        if self.shared.closed() || self.shared.source_lease.closed() {
            return Ok(());
        }
        let Some(mut frame) = self.device.read().map_err(Error::Camera)? else {
            return self.status();
        };
        if self.shared.closed() || self.shared.source_lease.closed() {
            return Ok(());
        }
        if frame.generation != self.shared.generation {
            return Err(Error::Camera(camera::Error::Protocol));
        }
        self.feed
            .submit(QpcStreamFrame {
                id: frame.sequence,
                capture: frame.capture,
                bgr: std::mem::take(&mut frame.bgr),
            })
            .map_err(Error::CameraFeed)?;
        {
            let mut s = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            s.camera_submitted += 1;
            s.camera_gaps += u64::from(frame.discontinuity);
        }
        self.status()
    }
}
impl Drop for CameraOwner<'_> {
    fn drop(&mut self) {
        self.finish();
    }
}
// A single guard owns both inputs. On normal exit AND unwinding it revokes the
// stream/video and camera delivery before any potentially blocking cleanup;
// microphone retirement precedes camera Flush/Shutdown waits.
struct InputOwners<'a> {
    shared: &'a Arc<Shared>,
    capture: Option<CaptureOwner<'a>>,
    camera: Option<CameraOwner<'a>>,
    cancellation: Option<crate::timed_video::Cancellation>,
    finished: Option<bool>,
}
impl InputOwners<'_> {
    fn finish(&mut self) -> bool {
        if let Some(clean) = self.finished {
            return clean;
        }
        self.shared.close(None);
        if let Some(c) = &self.cancellation {
            c.cancel();
        }
        if let Some(c) = self.camera.as_mut() {
            c.revoke();
        }
        let mut clean = true;
        if let Some(c) = self.capture.as_mut() {
            clean &= c.finish();
        }
        if let Some(c) = self.camera.as_mut() {
            clean &= c.finish();
        }
        self.finished = Some(clean);
        clean
    }
}
impl Drop for InputOwners<'_> {
    fn drop(&mut self) {
        self.finish();
    }
}
fn drive<F>(
    config: Config,
    reader: NativeCorrections,
    shared: &Arc<Shared>,
    launch: Option<capture::Launch>,
    camera_launch: Option<(camera::Launch, QpcStreamFeed)>,
    mut tap: F,
) -> bool
where
    F: FnMut(&mut JointOutput),
{
    let mut clean = true;
    let mut inputs = Some(InputOwners {
        shared,
        capture: None,
        camera: None,
        cancellation: None,
        finished: None,
    });
    // Factory execution is on the output owner after host authorization and
    // generation admission, never on the UI thread or while holding its lock.
    let result = (|| {
        if shared.input_closed() {
            return Ok(());
        }
        if let Some(launch) = launch {
            inputs.as_mut().unwrap().capture = Some(CaptureOwner {
                device: (launch.factory)().map_err(Error::Capture)?,
                selection: launch.selection,
                shared,
                finished: None,
            });
            let c = inputs.as_ref().unwrap().capture.as_ref().unwrap();
            c.device.inspect(&c.selection).map_err(Error::Capture)?;
        }
        if let Some((launch, feed)) = camera_launch {
            inputs.as_mut().unwrap().camera = Some(CameraOwner {
                device: (launch.factory)().map_err(Error::Camera)?,
                selected: launch.selected,
                feed,
                shared,
                revoked: None,
                finished: None,
            });
        }
        let candidate = super::Candidate::inspect(config).map_err(Error::Output)?;
        if shared.input_closed() {
            return Ok(());
        }
        let (mut owner, endpoint) = candidate.activate(true).map_err(|failure| {
            clean = failure.cleanup.is_none_or(|c| c.failure().is_none());
            Error::Output(failure.cause)
        })?;
        // Declared after output: revoke both inputs/output before unwind can
        // wait for either device. The outer guard covers pre-activation errors.
        let mut inputs = inputs.take().unwrap();
        inputs.cancellation = Some(owner.video.cancellation());
        let result = (|| {
            owner.bind_corrections(reader).map_err(Error::Output)?;
            if let Some(c) = inputs.camera.as_mut() {
                if shared.input_closed() {
                    return Ok(());
                }
                c.device
                    .prepare(&c.selected, shared.generation)
                    .map_err(Error::Camera)?;
                if shared.input_closed() {
                    return Ok(());
                }
                c.device.start().map_err(Error::Camera)?;
                c.status()?;
            }
            if let Some(c) = inputs.capture.as_mut() {
                if shared.input_closed() {
                    return Ok(());
                }
                c.device
                    .prepare(&c.selection, shared.generation)
                    .map_err(Error::Capture)?;
                if shared.input_closed() {
                    return Ok(());
                }
                c.device.start().map_err(Error::Capture)?;
                c.status()?;
            }
            shared.publish(endpoint)?;
            while !shared.input_closed() {
                tap(&mut owner);
                if let Some(c) = inputs.capture.as_mut() {
                    for _ in 0..MAX_PACKETS {
                        if shared.input_closed() {
                            break;
                        }
                        let Some(packet) =
                            c.device.read(shared.microphone).map_err(Error::Capture)?
                        else {
                            break;
                        };
                        if shared.input_closed() {
                            break;
                        }
                        // Unsplit source packet, original first-sample QPC. No
                        // second capture queue and no time based on receipt.
                        owner
                            .push_audio(owner.generation, packet)
                            .map_err(Error::Output)?;
                        shared
                            .state
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .admitted += 1;
                    }
                    c.status()?;
                }
                if let Some(c) = inputs.camera.as_mut() {
                    c.pump()?;
                }
                for _ in 0..MAX_PACKETS {
                    let Some(packet) = shared.pop() else {
                        break;
                    };
                    owner
                        .push_audio(owner.generation, packet)
                        .map_err(Error::Output)?;
                }
                if shared.input_closed() {
                    break;
                }
                let event = inputs
                    .capture
                    .as_ref()
                    .map(|c| c.device.event())
                    .transpose()
                    .map_err(Error::Capture)?
                    .flatten();
                let camera_event = inputs
                    .camera
                    .as_ref()
                    .map(|c| c.device.event())
                    .transpose()
                    .map_err(Error::Camera)?
                    .flatten();
                let progress = owner
                    .wait_with_inputs(shared.stop.handle(), event, camera_event)
                    .map_err(Error::Output)?;
                shared.output(owner.snapshot());
                if matches!(progress, super::Progress::Stopped) {
                    break;
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            shared.close(Some(e));
        }
        // Revoke presentation before potentially slow capture cleanup. Capture
        // is stopped/destroyed before output/source retirement can claim idle.
        owner.video.cancel();
        clean &= inputs.finish();
        let cleanup = owner.stop();
        clean &= cleanup.failure().is_none();
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
    if let Some(inputs) = inputs.as_mut() {
        clean &= inputs.finish();
    }
    clean
}
