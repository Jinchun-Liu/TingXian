use super::*;
use crate::joint::host::{Error as HostError, Host};
use crate::{
    Controller, EyeProvider, NativeCorrections, QpcStreamFeed, QpcStreamFrame, STREAM_BGR_BYTES,
};
use std::sync::{mpsc, Mutex};

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
    let (anchored, anchor_ready) = mpsc::channel();
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
                    let now = v::now().qpc_100ns();
                    a::anchor(&owner.audio, now);
                    anchor = Some(now);
                    consumed = 0;
                    anchored.send(now).unwrap();
                    resume.recv_timeout(Duration::from_secs(2)).unwrap();
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
    let mut mf = v::Reader::connect(input.take_endpoint(Duration::from_secs(3)).unwrap());
    assert!(matches!(
        input.take_endpoint(Duration::ZERO),
        Err(HostError::Source)
    ));
    let capture = source.frame(71);
    let base = anchor_ready.recv_timeout(Duration::from_secs(3)).unwrap();
    // Deterministic historical PCM around the actual fixture device anchor.
    // Five contiguous chunks fit the 50ms ingress; no future capture timestamp.
    for n in 0..5 {
        input
            .submit(input.generation(), pcm(base - 400_000 + n * 100_000, n))
            .unwrap();
    }
    release.send(()).unwrap();
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
