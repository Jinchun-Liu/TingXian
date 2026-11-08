use super::*;
use crate::{
    joint::{
        host::Host as MediaHost,
        tests::{config, host_tests::SourceRun, root},
    },
    Controller,
};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};
use windows_sys::Win32::System::Com::{CoGetApartmentType, APTTYPE_MTA};

struct Trace {
    calls: Mutex<Vec<(&'static str, thread::ThreadId)>>,
    cancelled: AtomicBool,
    fail_cleanup: AtomicBool,
    closes: AtomicUsize,
    drops: AtomicUsize,
}
impl Trace {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            cancelled: AtomicBool::new(false),
            fail_cleanup: AtomicBool::new(false),
            closes: AtomicUsize::new(0),
            drops: AtomicUsize::new(0),
        })
    }
    fn note(&self, name: &'static str) {
        let (mut kind, mut qualifier) = (0, 0);
        assert_eq!(
            unsafe { CoGetApartmentType(&mut kind, &mut qualifier) },
            0,
            "COM must survive through device destruction"
        );
        assert_eq!(kind, APTTYPE_MTA);
        self.calls
            .lock()
            .unwrap()
            .push((name, thread::current().id()));
    }
    fn assert_owner(&self) {
        let calls = self.calls.lock().unwrap();
        assert!(!calls.is_empty());
        assert!(
            calls
                .iter()
                .all(|(_, id)| *id == calls[0].1 && *id != thread::current().id()),
            "device calls left the owner lane"
        );
    }
}
struct MemoryDevice {
    input: Arc<Input>,
    trace: Arc<Trace>,
    start_gate: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    close_gate: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    fail_start: bool,
    panic_start: bool,
    closed: bool,
}
impl Device for MemoryDevice {
    fn cancellation(&self) -> Revoke {
        let trace = self.trace.clone();
        let input = self.input.clone();
        Arc::new(move || {
            trace.cancelled.store(true, Ordering::SeqCst);
            input.request_stop();
        })
    }
    fn start(&mut self) -> Result<(), NativeError> {
        self.trace.note("start");
        let _endpoint = self
            .input
            .take_endpoint(Duration::from_secs(3))
            .map_err(|_| NativeError::Off)?;
        if let Some((notice, gate)) = self.start_gate.take() {
            notice.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        assert!(!self.panic_start, "injected owner operation panic");
        if self.fail_start {
            Err(NativeError::Native(-7))
        } else if self.trace.cancelled.load(Ordering::SeqCst) {
            Err(NativeError::Native(0x800704c7u32 as i32))
        } else {
            Ok(())
        }
    }
    fn status(&self) -> Result<Status, NativeError> {
        Ok(Status {
            version: 1,
            state: 2,
            revoked: u32::from(self.trace.cancelled.load(Ordering::SeqCst)),
            used: 1,
            ..Status::default()
        })
    }
    fn close(&mut self) -> Result<(), NativeError> {
        self.trace.note("close");
        self.trace.closes.fetch_add(1, Ordering::SeqCst);
        if let Some((notice, gate)) = self.close_gate.take() {
            notice.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        if self.trace.fail_cleanup.load(Ordering::SeqCst) {
            return Err(NativeError::Native(-9));
        }
        self.closed = true;
        Ok(())
    }
}
impl Drop for MemoryDevice {
    fn drop(&mut self) {
        self.trace.note("drop");
        assert!(self.closed, "owner was destroyed before successful cleanup");
        self.trace.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn factory(
    trace: Arc<Trace>,
    start_gate: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    close_gate: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    fail_start: bool,
    panic_start: bool,
) -> Factory {
    Box::new(move |input| {
        trace.note("create");
        Ok(Box::new(MemoryDevice {
            input,
            trace,
            start_gate,
            close_gate,
            fail_start,
            panic_start,
            closed: false,
        }))
    })
}
struct Media {
    host: MediaHost,
    input: Arc<Input>,
    source: SourceRun,
}
impl Media {
    fn new() -> Self {
        let (source, reader) = SourceRun::new(Arc::new(Controller::default()));
        let host = MediaHost::default();
        let input = Arc::new(
            host.start(
                config(root().join("output.barrier")),
                source.state.clone(),
                reader,
                0,
                true,
            )
            .unwrap(),
        );
        Self {
            host,
            input,
            source,
        }
    }
    fn retired(&self) {
        assert!(
            self.host.wait_idle(Duration::from_secs(10)),
            "real synthetic media/source did not retire"
        );
        assert!(self.input.is_closed());
    }
}
impl Drop for Media {
    fn drop(&mut self) {
        self.host.request_stop();
        let _ = self.host.wait_idle(Duration::from_secs(10));
    }
}
fn until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(Instant::now() < deadline, "condition deadline");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn device_host_authorization_generation_and_resume_are_inert() {
    let m = Media::new();
    let host = Host::default();
    let trace = Trace::new();
    assert_eq!(host.snapshot(), Snapshot::default());
    assert_eq!(
        host.start_inner(
            m.input.clone(),
            0,
            false,
            factory(trace.clone(), None, None, false, false)
        ),
        Err(Error::Permission)
    );
    assert_eq!(
        host.start_inner(
            m.input.clone(),
            4,
            true,
            factory(trace.clone(), None, None, false, false)
        ),
        Err(Error::Stale)
    );
    host.suspend();
    assert_eq!(
        host.start_inner(
            m.input.clone(),
            1,
            true,
            factory(trace.clone(), None, None, false, false)
        ),
        Err(Error::Suspended)
    );
    host.resume();
    assert!(!host.snapshot().busy && !host.snapshot().suspended);
    assert!(trace.calls.lock().unwrap().is_empty());
    assert!(
        !m.input.is_closed(),
        "rejected requests must not stop independent media"
    );
    assert!(host.wait_idle(Duration::ZERO));
    host.start_inner(
        m.input.clone(),
        1,
        true,
        factory(trace.clone(), None, None, false, false),
    )
    .unwrap();
    until(|| host.snapshot().phase == Phase::Available);
    host.request_stop();
    assert!(host.wait_idle(Duration::from_secs(5)));
    m.retired();
    trace.assert_owner();
}

#[test]
fn device_host_stop_while_factory_waits_prevents_device_start() {
    let m = Media::new();
    let host = Host::default();
    let trace = Trace::new();
    let (notice, entered) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let t = trace.clone();
    host.start_inner(
        m.input.clone(),
        0,
        true,
        Box::new(move |input| {
            notice.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(3)).unwrap();
            factory(t, None, None, false, false)(input)
        }),
    )
    .unwrap();
    entered.recv_timeout(Duration::from_secs(3)).unwrap();
    host.request_stop();
    assert!(m.input.is_closed());
    assert!(!host.wait_idle(Duration::from_millis(20)));
    release.send(()).unwrap();
    assert!(host.wait_idle(Duration::from_secs(5)));
    m.retired();
    assert!(!trace
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|(name, _)| *name == "start"));
    assert_eq!(trace.drops.load(Ordering::SeqCst), 1);
    trace.assert_owner();
}

#[test]
fn device_host_source_stop_cancels_blocked_start_before_mta_returns() {
    let m = Media::new();
    let host = Host::default();
    let trace = Trace::new();
    let (notice, entered) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    host.start_inner(
        m.input.clone(),
        0,
        true,
        factory(trace.clone(), Some((notice, gate)), None, false, false),
    )
    .unwrap();
    entered.recv_timeout(Duration::from_secs(3)).unwrap();
    m.source.state.request_stop();
    until(|| trace.cancelled.load(Ordering::SeqCst));
    assert!(!host.wait_idle(Duration::from_millis(20)));
    assert_eq!(
        trace.closes.load(Ordering::SeqCst),
        0,
        "OS start has not returned yet"
    );
    release.send(()).unwrap();
    assert!(host.wait_idle(Duration::from_secs(5)));
    m.retired();
    let s = host.snapshot();
    assert_eq!(s.phase, Phase::Retired);
    assert!(s.failure.is_some() && !s.cleanup_uncertain);
    trace.assert_owner();
}

#[test]
fn device_host_media_stop_and_slow_cleanup_keep_owner_busy_until_join() {
    let m = Media::new();
    let host = Host::default();
    let trace = Trace::new();
    let (notice, entered) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    host.start_inner(
        m.input.clone(),
        0,
        true,
        factory(trace.clone(), None, Some((notice, gate)), false, false),
    )
    .unwrap();
    until(|| host.snapshot().phase == Phase::Available);
    m.host.request_stop();
    entered.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(trace.cancelled.load(Ordering::SeqCst));
    assert!(!host.wait_idle(Duration::from_millis(20)) && host.snapshot().busy);
    assert_eq!(trace.drops.load(Ordering::SeqCst), 0);
    assert_eq!(
        host.start_inner(
            m.input.clone(),
            1,
            true,
            factory(trace.clone(), None, None, false, false)
        ),
        Err(Error::Busy)
    );
    release.send(()).unwrap();
    assert!(host.wait_idle(Duration::from_secs(5)));
    m.retired();
    assert_eq!(trace.drops.load(Ordering::SeqCst), 1);
    trace.assert_owner();
}

#[test]
fn device_host_failed_cleanup_retains_live_apartment_until_explicit_retry() {
    let m = Media::new();
    let host = Host::default();
    let trace = Trace::new();
    trace.fail_cleanup.store(true, Ordering::SeqCst);
    host.start_inner(
        m.input.clone(),
        0,
        true,
        factory(trace.clone(), None, None, false, false),
    )
    .unwrap();
    until(|| host.snapshot().phase == Phase::Available);
    host.request_stop();
    until(|| host.snapshot().phase == Phase::CleanupBlocked);
    assert!(!host.wait_idle(Duration::from_millis(30)));
    assert_eq!(
        trace.closes.load(Ordering::SeqCst),
        1,
        "cleanup must not auto-retry"
    );
    assert_eq!(trace.drops.load(Ordering::SeqCst), 0);
    let s = host.snapshot();
    assert!(s.busy && s.cleanup_uncertain);
    assert_eq!(
        host.start_inner(
            m.input.clone(),
            s.generation,
            true,
            factory(trace.clone(), None, None, false, false)
        ),
        Err(Error::Busy)
    );
    trace.fail_cleanup.store(false, Ordering::SeqCst);
    host.request_stop();
    assert!(host.wait_idle(Duration::from_secs(5)));
    m.retired();
    let s = host.snapshot();
    assert!(!s.busy && !s.cleanup_uncertain);
    assert_eq!(s.cleanup_attempts, 2);
    assert_eq!(
        s.last_cleanup_error,
        Some(Error::Native(NativeError::Native(-9)))
    );
    assert_eq!(trace.drops.load(Ordering::SeqCst), 1);
    trace.assert_owner();
}

#[test]
fn device_host_operation_failure_or_panic_closes_both_lanes_before_com_teardown() {
    for panic_start in [false, true] {
        let m = Media::new();
        let host = Host::default();
        let trace = Trace::new();
        host.start_inner(
            m.input.clone(),
            0,
            true,
            factory(trace.clone(), None, None, !panic_start, panic_start),
        )
        .unwrap();
        assert!(host.wait_idle(Duration::from_secs(5)));
        m.retired();
        let s = host.snapshot();
        assert!(!s.cleanup_uncertain && s.phase == Phase::Retired);
        assert_eq!(
            s.failure,
            Some(if panic_start {
                Error::Thread
            } else {
                Error::Native(NativeError::Native(-7))
            })
        );
        assert!(trace.cancelled.load(Ordering::SeqCst));
        assert_eq!(trace.drops.load(Ordering::SeqCst), 1);
        trace.assert_owner();
    }
}

#[test]
fn device_host_real_native_library_cancel_before_start_never_activates_a_device() {
    let m = Media::new();
    let host = Host::default();
    let (notice, entered) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    host.start_inner(
        m.input.clone(),
        0,
        true,
        Box::new(move |input| {
            let device = NativeDevice::load(crate::timed_video::tests::pin(), input)?;
            assert_eq!(
                device.status()?,
                Status {
                    version: 1,
                    ..Status::default()
                }
            );
            notice.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(3)).unwrap();
            Ok(Box::new(device))
        }),
    )
    .unwrap();
    entered.recv_timeout(Duration::from_secs(3)).unwrap();
    host.request_stop();
    assert!(m.input.is_closed() && !host.wait_idle(Duration::from_millis(20)));
    release.send(()).unwrap();
    assert!(host.wait_idle(Duration::from_secs(5)));
    m.retired();
    let s = host.snapshot();
    assert_eq!(s.failure, None);
    assert_eq!(s.cleanup_attempts, 1);
    assert_eq!(s.phase, Phase::Retired);
}
