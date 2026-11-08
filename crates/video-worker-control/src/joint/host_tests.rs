use super::*;
use crate::capture;
use crate::joint::host::{Error as HostError, Host};
use crate::{
    Controller, EyeProvider, NativeCorrections, QpcStreamFeed, QpcStreamFrame, STREAM_BGR_BYTES,
};
use std::sync::{mpsc, Mutex};

struct MemoryCapture {
    lane: thread::ThreadId,
    packets: mpsc::Receiver<PcmPacket>,
    calls: Arc<Mutex<Vec<&'static str>>>,
    phase: u32,
    frames: u64,
    count: u64,
    mode: i32,
    finish_gate: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    finished: bool,
}
impl MemoryCapture {
    fn note(&self, name: &'static str) {
        assert_eq!(thread::current().id(), self.lane);
        self.calls.lock().unwrap().push(name);
    }
}
impl capture::Device for MemoryCapture {
    fn inspect(&self, s: &capture::Selection) -> Result<(), capture::Error> {
        self.note("inspect");
        assert_eq!(
            (&*s.microphone_id, &*s.output_return_id),
            ("synthetic-mic", "fixture-capture")
        );
        if self.mode == 0 {
            Err(capture::Error::Native(-1))
        } else {
            Ok(())
        }
    }
    fn prepare(&mut self, _: &capture::Selection, generation: u64) -> Result<(), capture::Error> {
        self.note("prepare");
        assert!(generation > 0);
        self.phase = 1;
        if self.mode == 1 {
            Err(capture::Error::Native(-2))
        } else {
            Ok(())
        }
    }
    fn start(&mut self) -> Result<(), capture::Error> {
        self.note("start");
        assert_eq!(self.phase, 1);
        self.phase = 2;
        if self.mode == 2 {
            Err(capture::Error::Native(-3))
        } else {
            Ok(())
        }
    }
    fn read(&mut self, microphone: u64) -> Result<Option<PcmPacket>, capture::Error> {
        self.note("read");
        assert_eq!(self.phase, 2);
        assert_eq!(microphone, 7);
        if self.mode == 3 {
            return Err(capture::Error::Native(-4));
        }
        if self.mode == 5 {
            panic!("synthetic capture unwind");
        }
        if self.mode == 6 {
            if let Some((notice, gate)) = self.finish_gate.take() {
                notice.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(3)).unwrap();
            }
        }
        let p = self.packets.try_recv().ok();
        if let Some(p) = &p {
            self.frames += p.samples.len() as u64;
            self.count += 1;
        }
        Ok(p)
    }
    fn event(&self) -> Result<Option<BorrowedHandle<'_>>, capture::Error> {
        self.note("event");
        Ok(None)
    }
    fn status(&self) -> Result<capture::Status, capture::Error> {
        self.note("status");
        Ok(capture::Status {
            phase: self.phase,
            packets: self.count,
            delivered_frames: self.frames,
            reset: u32::from(self.phase == 3),
            stop_error: if self.mode == 4 { -5 } else { 0 },
            ..Default::default()
        })
    }
    fn finish(&mut self) -> capture::Cleanup {
        self.note("finish");
        assert!(!self.finished);
        self.finished = true;
        if let Some((notice, gate)) = self.finish_gate.take() {
            notice.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        self.phase = 3;
        capture::Cleanup {
            stop: Ok(()),
            status: self.status(),
            destroy: Ok(()),
            reset_required: true,
        }
    }
}
impl Drop for MemoryCapture {
    fn drop(&mut self) {
        self.note("drop");
        assert!(self.finished);
    }
}
fn capture_launch(
    mode: i32,
    finish_gate: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
) -> (
    capture::Launch,
    mpsc::Sender<PcmPacket>,
    Arc<Mutex<Vec<&'static str>>>,
) {
    let (send, packets) = mpsc::channel();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed = calls.clone();
    let caller = thread::current().id();
    (
        capture::Launch {
            selection: capture::Selection {
                microphone_id: "synthetic-mic".into(),
                output_return_id: "fixture-capture".into(),
            },
            factory: Box::new(move || {
                assert_ne!(thread::current().id(), caller);
                observed.lock().unwrap().push("create");
                Ok(Box::new(MemoryCapture {
                    lane: thread::current().id(),
                    packets,
                    calls: observed,
                    phase: 0,
                    frames: 0,
                    count: 0,
                    mode,
                    finish_gate,
                    finished: false,
                }))
            }),
        },
        send,
        calls,
    )
}

#[test]
fn media_host_capture_failures_retire_on_owner_and_cleanup_failure_stays_latched() {
    for mode in 0..5 {
        let (source, reader) = SourceRun::new(Arc::new(Controller::default()));
        let host = Host::default();
        let path = root().join("output.barrier");
        let (capture, _send, calls) = capture_launch(mode, None);
        let input = host
            .start_capture_tapped(
                config(path.clone()),
                capture,
                source.state.clone(),
                reader,
                |_| {},
            )
            .unwrap();
        if mode == 4 {
            let _endpoint = input.take_endpoint(Duration::from_secs(3)).unwrap();
            input.request_stop();
        }
        assert_eq!(host.wait_idle(Duration::from_secs(10)), mode != 4);
        let s = host.snapshot();
        assert!(!s.busy);
        assert!(matches!(s.failure, Some(HostError::Capture(_))), "{s:?}");
        assert_eq!(s.cleanup_uncertain, mode == 4);
        assert_eq!(s.capture_cleanup.unwrap().failure().is_some(), mode == 4);
        let trace = calls.lock().unwrap();
        assert_eq!(trace.first(), Some(&"create"));
        assert_eq!(trace.last(), Some(&"drop"));
        assert_eq!(trace.iter().filter(|s| **s == "finish").count(), 1);
        if mode == 0 {
            assert!(!path.exists() && !trace.contains(&"prepare"));
        }
        if mode == 1 {
            assert!(!trace.contains(&"start"));
        }
        host.resume();
        assert_eq!(host.wait_idle(Duration::ZERO), mode != 4);
        drop(trace);
        drop(input);
        drop(host);
        if mode != 0 {
            fenced(path);
        }
    }
}

#[test]
fn media_host_capture_permission_unknown_source_and_three_unexposed_lifetimes() {
    let host = Host::default();
    let path = root().join("output.barrier");
    for n in 0..3 {
        let (source, reader) = SourceRun::new(Arc::new(Controller::default()));
        let config_capture = capture::Config {
            library: capture::tests::pin(),
            microphone_id: "tingxian-nonexistent-microphone".into(),
        };
        let input = host.start_captured(
            config(path.clone()),
            config_capture,
            source.state.clone(),
            reader,
            host.snapshot().generation,
            n != 0,
        );
        if n == 0 {
            assert!(matches!(input, Err(HostError::Permission)));
        } else {
            let input = input.unwrap();
            assert!(host.wait_idle(Duration::from_secs(10)));
            assert!(matches!(
                host.snapshot().failure,
                Some(HostError::Capture(_))
            ));
            assert_eq!(host.snapshot().capture.unwrap().packets, 0);
            assert_eq!(
                input.submit(input.generation(), pcm(v::now().qpc_100ns(), 0)),
                Err(HostError::Source)
            );
        }
        assert!(!host.snapshot().busy && !path.exists());
    }
}

#[test]
fn media_host_capture_unwind_destroys_input_and_never_claims_clean_idle() {
    let (source, reader) = SourceRun::new(Arc::new(Controller::default()));
    let host = Host::default();
    let path = root().join("output.barrier");
    let (capture, _send, calls) = capture_launch(5, None);
    let input = host
        .start_capture_tapped(
            config(path.clone()),
            capture,
            source.state.clone(),
            reader,
            |_| {},
        )
        .unwrap();
    assert!(!host.wait_idle(Duration::from_secs(10)));
    let s = host.snapshot();
    assert!(!s.busy && s.cleanup_uncertain);
    assert_eq!(s.failure, Some(HostError::Thread));
    assert!(s.capture_cleanup.unwrap().failure().is_none());
    let trace = calls.lock().unwrap();
    assert_eq!(trace.last(), Some(&"drop"));
    assert_eq!(trace.iter().filter(|s| **s == "finish").count(), 1);
    drop(trace);
    drop(input);
    drop(host);
    fenced(path);
}

#[test]
fn media_host_capture_packet_returning_after_stop_never_reaches_timeline() {
    let (source, reader) = SourceRun::new(Arc::new(Controller::default()));
    let host = Host::default();
    let path = root().join("output.barrier");
    let (notice, entered) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let (capture, send, _calls) = capture_launch(6, Some((notice, gate)));
    let input = host
        .start_capture_tapped(
            config(path.clone()),
            capture,
            source.state.clone(),
            reader,
            |_| {},
        )
        .unwrap();
    let mut mf = v::Reader::connect(input.take_endpoint(Duration::from_secs(3)).unwrap());
    entered.recv_timeout(Duration::from_secs(3)).unwrap();
    send.send(pcm(v::now().qpc_100ns() - 100_000, 0)).unwrap();
    host.request_stop();
    assert!(!host.wait_idle(Duration::from_millis(20)));
    release.send(()).unwrap();
    assert!(host.wait_idle(Duration::from_secs(10)));
    let s = host.snapshot();
    assert_eq!(s.capture.unwrap().delivered_frames, 480);
    assert_eq!(s.admitted_packets, 0);
    assert_eq!(s.output.unwrap().committed_windows, 0);
    assert_eq!(s.queued_frames, 0);
    mf.neutral_after_stop();
    drop(input);
    drop(host);
    fenced(path);
}

#[test]
fn media_host_capture_delivers_unsplit_original_voice_and_waits_for_capture_destruction() {
    let (source, reader) = SourceRun::new(Arc::new(Controller::default()));
    let host = Host::default();
    let path = root().join("output.barrier");
    let (cleanup, cleanup_ready) = mpsc::channel();
    let (finish, finish_gate) = mpsc::channel();
    let (capture, send, calls) = capture_launch(-1, Some((cleanup, finish_gate)));
    let (resume, ready) = mpsc::channel();
    let (proof, proven) = mpsc::channel();
    let mut base = None;
    let mut consumed = 0;
    let mut reported = false;
    let mut pts = 0;
    let input = host
        .start_capture_tapped(
            config(path.clone()),
            capture,
            source.state.clone(),
            reader,
            move |owner| {
                if base.is_none() {
                    let until = Instant::now() + Duration::from_secs(2);
                    while owner.snapshot().eye.unwrap().received == 0 {
                        assert!(Instant::now() < until);
                        thread::sleep(Duration::from_millis(1));
                    }
                    let now = v::now().qpc_100ns();
                    a::anchor(&owner.audio, now);
                    base = Some(now);
                    // Seed synthetic input on this lane immediately after its
                    // clock anchor; a main-thread round trip is not device time.
                    let mut first = pcm(now - 1_100_000, 0);
                    first.samples.resize(4800, 0.2);
                    first.flags.discontinuity = true;
                    send.send(first).unwrap();
                    send.send(pcm(now - 100_000, 10)).unwrap();
                } else {
                    let target = (v::now().qpc_100ns() - base.unwrap()) * 48000 / 10_000_000;
                    let delta = target.saturating_sub(consumed);
                    assert!(delta <= 4800);
                    a::step(&owner.audio, delta as u32, 0);
                    consumed = target;
                }
                if let Some(p) = &owner.pending {
                    if !p.neutral {
                        pts = p.presentation.qpc_100ns();
                    }
                }
                if owner.snapshot().committed_windows > 0 && !reported {
                    assert_eq!(owner.snapshot().neutral_windows, 0);
                    reported = true;
                    proof.send((pts, a::read(&owner.audio))).unwrap();
                    ready.recv_timeout(Duration::from_secs(2)).unwrap();
                }
            },
        )
        .unwrap();
    let mut mf = v::Reader::connect(input.take_endpoint(Duration::from_secs(3)).unwrap());
    assert_eq!(
        input.submit(input.generation(), pcm(v::now().qpc_100ns(), 0)),
        Err(HostError::Source)
    );
    let capture_time = source.frame(81);
    let (pts, values) = proven.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(values.len(), 960);
    assert!(values.iter().all(|v| (*v - 0.2).abs() < 0.00001));
    mf.sample_eye(81, capture_time, pts);
    assert_eq!(host.snapshot().capture.unwrap().delivered_frames, 5280);
    assert_eq!(host.snapshot().admitted_packets, 2);
    assert_eq!(host.snapshot().queued_frames, 0);
    host.request_stop();
    resume.send(()).unwrap();
    cleanup_ready.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(!host.wait_idle(Duration::from_millis(20)));
    assert!(host.snapshot().busy);
    finish.send(()).unwrap();
    assert!(host.wait_idle(Duration::from_secs(10)));
    assert!(host.snapshot().capture_cleanup.unwrap().failure().is_none());
    let trace = calls.lock().unwrap();
    assert_eq!(trace.iter().filter(|s| **s == "finish").count(), 1);
    assert_eq!(trace.last(), Some(&"drop"));
    mf.neutral_after_stop();
    drop(input);
    drop(host);
    fenced(path);
}

#[test]
fn media_host_source_stop_revokes_unclaimed_endpoint_and_ingress_before_owner_tick() {
    let (source, reader) = SourceRun::new(Arc::new(Controller::default()));
    let host = Host::default();
    let path = root().join("output.barrier");
    let (started, ready) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let mut held = false;
    let input = host
        .start_tapped(
            config(path.clone()),
            source.state.clone(),
            reader,
            0,
            true,
            move |_| {
                if !held {
                    held = true;
                    started.send(()).unwrap();
                    resume.recv_timeout(Duration::from_secs(2)).unwrap();
                }
            },
        )
        .unwrap();
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    source.state.request_stop();
    assert!(matches!(
        input.take_endpoint(Duration::ZERO),
        Err(HostError::Closed)
    ));
    assert_eq!(
        input.submit(input.generation(), pcm(v::now().qpc_100ns(), 0)),
        Err(HostError::Closed)
    );
    assert_eq!(host.snapshot().queued_frames, 0);
    assert!(!host.wait_idle(Duration::ZERO));
    release.send(()).unwrap();
    assert!(host.wait_idle(Duration::from_secs(10)));
    assert_eq!(host.snapshot().output.unwrap().committed_windows, 0);
    drop(input);
    drop(host);
    fenced(path);
}

struct SourceRun {
    state: Arc<Controller>,
    feed: QpcStreamFeed,
    run: Option<thread::JoinHandle<crate::Snapshot>>,
}
impl SourceRun {
    fn new(state: Arc<Controller>) -> (Self, NativeCorrections) {
        let generation = state.snapshot().generation;
        let (run, feed) = state
            .begin_qpc_stream_if_current(
                Some(crate::tests::fixture("eye-stream.exe")),
                EyeProvider::Synthetic,
                generation,
            )
            .unwrap()
            .unwrap();
        let run = thread::spawn(move || run.execute());
        let reader = feed.open_corrections(Duration::from_secs(3)).unwrap();
        (
            Self {
                state,
                feed,
                run: Some(run),
            },
            reader,
        )
    }
    fn frame(&self, id: u64) -> u64 {
        let pixels = vec![31; STREAM_BGR_BYTES];
        let capture = v::now().qpc_100ns() - 300_000;
        self.feed
            .submit(QpcStreamFrame {
                id,
                capture: Time::from_qpc_100ns(capture),
                bgr: pixels,
            })
            .unwrap();
        capture
    }
}
impl Drop for SourceRun {
    fn drop(&mut self) {
        self.state.request_stop();
        if let Some(run) = self.run.take() {
            run.join().unwrap();
        }
    }
}
fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(Instant::now() < deadline, "bounded host observation");
        thread::sleep(Duration::from_millis(1));
    }
}
fn pcm(capture: u64, sequence: u64) -> PcmPacket {
    PcmPacket {
        device: 7,
        source: Source::SelfMicrophone,
        first_sample: sequence * 480,
        capture: Time::from_qpc_100ns(capture),
        rate: RATE,
        channels: 1,
        flags: PacketFlags::default(),
        samples: vec![0.2; 480],
    }
}
#[test]
fn media_host_default_suspend_resume_does_not_create_output() {
    let host = Host::default();
    assert!(!host.snapshot().busy);
    assert!(host.wait_idle(Duration::ZERO));
    host.suspend();
    let generation = host.snapshot().generation;
    assert!(host.snapshot().suspended);
    host.resume();
    assert_eq!(host.snapshot().generation, generation);
    assert!(!host.snapshot().busy && host.snapshot().output.is_none());
}
#[test]
fn media_host_denied_stale_and_unrelated_source_never_prepare_devices() {
    for mode in 0..4 {
        let (source, reader) = SourceRun::new(Arc::new(Controller::default()));
        let host = Host::default();
        let path = root().join("output.barrier");
        if mode == 2 {
            host.suspend();
        }
        let controller = if mode == 3 {
            Arc::new(Controller::default())
        } else {
            source.state.clone()
        };
        let result = host.start(
            config(path.clone()),
            controller.clone(),
            reader,
            if mode == 1 {
                1
            } else {
                host.snapshot().generation
            },
            mode != 0,
        );
        let expected = match mode {
            0 => HostError::Permission,
            1 => HostError::Stale,
            2 => HostError::Suspended,
            _ => HostError::Source,
        };
        assert!(matches!(result,Err(e) if e==expected));
        assert!(!path.exists() && !host.snapshot().busy);
        if mode == 3 {
            assert_eq!(controller.snapshot().generation, 0);
        }
    }
}
#[test]
fn media_host_owned_thread_delivers_pcm_and_mf_then_waits_for_actual_retirement() {
    let (source, reader) = SourceRun::new(Arc::new(Controller::default()));
    let host = Arc::new(Host::default());
    let path = root().join("output.barrier");
    let proof = Arc::new(Mutex::new((0u64, Vec::<f32>::new())));
    let capture_proof = proof.clone();
    let (committed, noticed) = mpsc::channel();
    let (authority, input_ready) = mpsc::sync_channel::<Arc<crate::joint::host::Input>>(1);
    let (release, resume) = mpsc::channel();
    let mut anchor = None;
    let mut consumed = 0u64;
    let mut reported = false;
    let mut planned = 0u64;
    let input = host
        .start_tapped(
            config(path.clone()),
            source.state.clone(),
            reader,
            0,
            true,
            move |owner| {
                if anchor.is_none() {
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while owner.snapshot().eye.unwrap().received == 0 {
                        assert!(
                            Instant::now() < deadline,
                            "correction not received before synthetic device anchor"
                        );
                        thread::sleep(Duration::from_millis(1));
                    }
                    let ingress = input_ready.recv_timeout(Duration::from_secs(2)).unwrap();
                    let now = v::now().qpc_100ns();
                    a::anchor(&owner.audio, now);
                    anchor = Some(now);
                    consumed = 0;
                    // Keep the five real ingress submissions, but do not spend
                    // the synthetic device's bootstrap lease on test IPC.
                    for n in 0..5 {
                        ingress
                            .submit(ingress.generation(), pcm(now - 400_000 + n * 100_000, n))
                            .unwrap();
                    }
                } else if let Some(base) = anchor {
                    let target = (v::now().qpc_100ns() - base) * 48_000 / 10_000_000;
                    let delta = target.saturating_sub(consumed);
                    assert!(delta <= 4800);
                    a::step(&owner.audio, delta as u32, 0);
                    consumed = target;
                }
                if let Some(pending) = &owner.pending {
                    if !pending.neutral {
                        planned = pending.presentation.qpc_100ns();
                    }
                }
                if owner.snapshot().committed_windows > 0 && !reported {
                    assert_eq!(owner.snapshot().neutral_windows, 0);
                    *capture_proof.lock().unwrap() = (planned, a::read(&owner.audio));
                    reported = true;
                    committed.send(()).unwrap();
                    resume.recv_timeout(Duration::from_secs(2)).unwrap();
                }
            },
        )
        .unwrap();
    let input = Arc::new(input);
    authority.send(input.clone()).unwrap();
    let mut mf = v::Reader::connect(input.take_endpoint(Duration::from_secs(3)).unwrap());
    assert!(matches!(
        input.take_endpoint(Duration::ZERO),
        Err(HostError::Source)
    ));
    let capture = source.frame(71);
    let _ = noticed.recv_timeout(Duration::from_secs(2));
    let deadline = Instant::now() + Duration::from_secs(3);
    while proof.lock().unwrap().0 == 0 {
        let snapshot = host.snapshot();
        assert!(
            snapshot.failure.is_none() && Instant::now() < deadline,
            "missing owned-thread output: {snapshot:?}; source={:?}",
            source.state.snapshot()
        );
        thread::sleep(Duration::from_millis(1));
    }
    let (pts, samples) = proof.lock().unwrap().clone();
    assert_eq!(samples.len(), 960);
    assert!(samples.iter().all(|s| (*s - 0.2).abs() < 0.00001));
    mf.sample_eye(71, capture, pts);
    input.request_stop();
    assert!(
        !host.wait_idle(Duration::from_millis(20)),
        "blocked owner is not retired"
    );
    assert!(host.snapshot().busy && host.snapshot().queued_frames == 0);
    release.send(()).unwrap();
    assert!(host.wait_idle(Duration::from_secs(10)));
    let s = host.snapshot();
    assert!(!s.busy && !s.cleanup_uncertain);
    assert!(s.output.unwrap().cleanup.unwrap().restart_blocked);
    assert_eq!(
        input.submit(input.generation(), pcm(v::now().qpc_100ns(), 99)),
        Err(HostError::Closed)
    );
    mf.neutral_after_stop();
    drop(input);
    drop(host);
    fenced(path);
}
#[test]
fn media_host_audio_ingress_is_bounded_without_silent_speech_replacement() {
    for tiny in [false, true] {
        let (source, reader) = SourceRun::new(Arc::new(Controller::default()));
        let host = Host::default();
        let path = root().join("output.barrier");
        let (started, ready) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let mut held = false;
        let input = host
            .start_tapped(
                config(path.clone()),
                source.state.clone(),
                reader,
                0,
                true,
                move |_| {
                    if !held {
                        held = true;
                        started.send(()).unwrap();
                        resume.recv_timeout(Duration::from_secs(2)).unwrap();
                    }
                },
            )
            .unwrap();
        let mut mf = v::Reader::connect(input.take_endpoint(Duration::from_secs(3)).unwrap());
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        let count = if tiny { 16 } else { 5 };
        for n in 0..count {
            let mut p = pcm(v::now().qpc_100ns(), n);
            if tiny {
                p.samples.truncate(1);
            }
            input.submit(input.generation(), p).unwrap();
        }
        let mut over = pcm(v::now().qpc_100ns(), 99);
        if tiny {
            over.samples.truncate(1);
        }
        assert_eq!(
            input.submit(input.generation(), over),
            Err(HostError::Backpressure)
        );
        let s = host.snapshot();
        assert_eq!(s.queued_frames, 0);
        assert_eq!(s.cleared_frames, if tiny { 16 } else { 2400 });
        assert_eq!(s.admitted_packets, count);
        assert_eq!(s.failure, Some(HostError::Backpressure));
        assert!(!host.wait_idle(Duration::ZERO));
        release.send(()).unwrap();
        assert!(host.wait_idle(Duration::from_secs(10)));
        assert_eq!(host.snapshot().output.unwrap().committed_windows, 0);
        mf.neutral_after_stop();
        drop(input);
        drop(host);
        fenced(path);
    }
}
#[test]
fn media_host_reset_failure_blocks_clean_quit_and_resume_does_not_erase_it() {
    let (source, reader) = SourceRun::new(Arc::new(Controller::default()));
    let host = Host::default();
    let path = root().join("output.barrier");
    let (started, ready) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let mut held = false;
    let input = host
        .start_tapped(
            config(path.clone()),
            source.state.clone(),
            reader,
            0,
            true,
            move |owner| {
                if !held {
                    held = true;
                    a::step(&owner.audio, 0, 4);
                    started.send(()).unwrap();
                    resume.recv_timeout(Duration::from_secs(2)).unwrap();
                }
            },
        )
        .unwrap();
    let mut mf = v::Reader::connect(input.take_endpoint(Duration::from_secs(3)).unwrap());
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    host.suspend();
    assert!(!host.wait_idle(Duration::ZERO));
    release.send(()).unwrap();
    assert!(!host.wait_idle(Duration::from_secs(10)));
    assert!(!host.snapshot().busy && host.snapshot().cleanup_uncertain);
    assert!(
        matches!(host.snapshot().failure,Some(HostError::Output(Error::Audio(audio::Error::Native(hr)))) if hr<0)
    );
    host.resume();
    assert!(!host.wait_idle(Duration::ZERO));
    assert!(host.snapshot().cleanup_uncertain);
    mf.neutral_after_stop();
    drop(input);
    drop(host);
    fenced(path);
}
#[test]
fn media_host_old_input_drop_cannot_stop_a_later_worker_generation() {
    let state = Arc::new(Controller::default());
    let (mut old, reader) = SourceRun::new(state.clone());
    let host = Host::default();
    let path = root().join("output.barrier");
    let input = host
        .start(config(path.clone()), state.clone(), reader, 0, true)
        .unwrap();
    let mut mf = v::Reader::connect(input.take_endpoint(Duration::from_secs(3)).unwrap());
    input.request_stop();
    assert!(host.wait_idle(Duration::from_secs(10)));
    old.run.take().unwrap().join().unwrap();
    let (mut next, next_reader) = SourceRun::new(state.clone());
    let generation = state.snapshot().generation;
    drop(input);
    host.request_stop();
    assert_eq!(state.snapshot().generation, generation);
    assert!(!next.feed.status().closed);
    next.frame(91);
    let mut next_reader = next_reader;
    until(|| next_reader.poll(Duration::from_millis(10)).unwrap());
    next_reader.with_current(|f| assert_eq!(f.unwrap().id, 91));
    mf.neutral_after_stop();
    drop(next_reader);
    state.request_stop();
    next.run.take().unwrap().join().unwrap();
    drop(host);
    fenced(path);
}
