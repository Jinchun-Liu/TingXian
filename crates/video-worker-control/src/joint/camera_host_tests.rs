use super::*;
use crate::camera;
struct MediaSource {
    state: Arc<Controller>,
    run: Option<thread::JoinHandle<crate::Snapshot>>,
}
impl MediaSource {
    fn new() -> (Self, QpcStreamFeed, NativeCorrections) {
        let state = Arc::new(Controller::default());
        let (job, feed) = state
            .begin_qpc_stream_if_current(
                Some(crate::tests::fixture("eye-stream.exe")),
                EyeProvider::Synthetic,
                0,
            )
            .unwrap()
            .unwrap();
        let run = thread::spawn(move || job.execute());
        let reader = feed.open_corrections(Duration::from_secs(3)).unwrap();
        (
            Self {
                state,
                run: Some(run),
            },
            feed,
            reader,
        )
    }
}
impl Drop for MediaSource {
    fn drop(&mut self) {
        self.state.request_stop();
        self.run.take().unwrap().join().unwrap();
    }
}
struct CameraMemory {
    lane: thread::ThreadId,
    phase: u32,
    generation: u64,
    count: u64,
    mode: i32,
    frames: mpsc::Receiver<camera::Frame>,
    calls: Arc<Mutex<Vec<&'static str>>>,
    mic: Arc<Mutex<Vec<&'static str>>>,
    gate: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    revoked: bool,
    finished: bool,
}
impl CameraMemory {
    fn note(&self, name: &'static str) {
        assert_eq!(thread::current().id(), self.lane);
        self.calls.lock().unwrap().push(name);
    }
}
impl camera::Device for CameraMemory {
    fn prepare(&mut self, selected: &str, generation: u64) -> Result<(), camera::Error> {
        self.note("prepare");
        assert_eq!(selected, "synthetic-hardware-camera");
        assert!(generation > 0);
        self.phase = 2;
        self.generation = generation;
        if self.mode == 7 {
            let (notice, gate) = self.gate.take().unwrap();
            notice.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        if self.mode == 0 {
            Err(camera::Error::Native(-1))
        } else {
            Ok(())
        }
    }
    fn start(&mut self) -> Result<(), camera::Error> {
        self.note("start");
        assert_eq!(self.phase, 2);
        self.phase = 3;
        if self.mode == 1 {
            Err(camera::Error::Native(-2))
        } else {
            Ok(())
        }
    }
    fn read(&mut self) -> Result<Option<camera::Frame>, camera::Error> {
        self.note("read");
        assert_eq!(self.phase, 3);
        assert!(!self.revoked);
        if self.mode == 2 {
            return Err(camera::Error::Native(-3));
        }
        if self.mode == 4 {
            panic!("synthetic camera unwind");
        }
        if self.mode == 5 {
            if let Some((notice, gate)) = self.gate.take() {
                notice.send(()).unwrap();
                gate.recv_timeout(Duration::from_secs(3)).unwrap();
            }
        }
        let frame = self.frames.try_recv().ok();
        if frame.is_some() {
            self.note("frame");
            self.count += 1;
        }
        Ok(frame)
    }
    fn event(&self) -> Result<Option<BorrowedHandle<'_>>, camera::Error> {
        self.note("event");
        Ok(None)
    }
    fn status(&self) -> Result<camera::Status, camera::Error> {
        Ok(camera::Status {
            version: 1,
            phase: self.phase,
            delivered: self.count,
            closed: u32::from(self.revoked),
            cleanup_uncertain: u32::from(self.mode == 3),
            ..Default::default()
        })
    }
    fn revoke(&mut self) -> Result<(), camera::Error> {
        self.note("revoke");
        assert!(
            !self.mic.lock().unwrap().contains(&"finish"),
            "camera gate must close before microphone waits"
        );
        self.revoked = true;
        self.phase = 4;
        Ok(())
    }
    fn finish(&mut self) -> camera::Cleanup {
        self.note("finish");
        assert!(!self.finished && self.revoked);
        assert!(
            self.mic.lock().unwrap().contains(&"finish"),
            "microphone must retire before camera waits"
        );
        if let Some((notice, gate)) = self.gate.take() {
            notice.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        self.finished = true;
        self.phase = 5;
        camera::Cleanup {
            revoke: Ok(()),
            stop: Ok(()),
            status: self.status(),
            destroy: Ok(()),
        }
    }
}
impl Drop for CameraMemory {
    fn drop(&mut self) {
        self.note("drop");
        assert!(self.finished);
    }
}
fn camera_launch(
    mode: i32,
    mic: Arc<Mutex<Vec<&'static str>>>,
    gate: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
) -> (
    camera::Launch,
    mpsc::Sender<camera::Frame>,
    Arc<Mutex<Vec<&'static str>>>,
) {
    let (send, frames) = mpsc::channel();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed = calls.clone();
    let caller = thread::current().id();
    (
        camera::Launch {
            selected: "synthetic-hardware-camera".into(),
            factory: Box::new(move || {
                assert_ne!(thread::current().id(), caller);
                observed.lock().unwrap().push("create");
                Ok(Box::new(CameraMemory {
                    lane: thread::current().id(),
                    phase: 0,
                    generation: 0,
                    count: 0,
                    mode,
                    frames,
                    calls: observed,
                    mic,
                    gate,
                    revoked: false,
                    finished: false,
                }))
            }),
        },
        send,
        calls,
    )
}
fn camera_frame(generation: u64) -> camera::Frame {
    let bgr = vec![31; STREAM_BGR_BYTES];
    camera::Frame {
        generation,
        sequence: 81,
        capture: Time::from_qpc_100ns(v::now().qpc_100ns() - 300_000),
        discontinuity: true,
        bgr,
    }
}
#[test]
fn media_camera_permission_generation_and_foreign_feed_reject_before_factories() {
    for mode in 0..3 {
        let (source, feed, reader) = MediaSource::new();
        let (foreign, other_feed, _other_reader) = MediaSource::new();
        let host = Host::default();
        let path = root().join("output.barrier");
        let (mic, _pcm, trace) = capture_launch(-1, None);
        let (camera, _frames, cam_trace) = camera_launch(-1, trace.clone(), None);
        let selected_feed = if mode == 2 { other_feed } else { feed };
        let result = host.start_media_tapped(
            config(path.clone()),
            mic,
            camera,
            selected_feed,
            source.state.clone(),
            reader,
            if mode == 1 { 9 } else { 0 },
            mode != 0,
            |_| {},
        );
        let error = match mode {
            0 => HostError::Permission,
            1 => HostError::Stale,
            _ => HostError::Source,
        };
        assert!(matches!(result, Err(e) if e == error));
        assert!(trace.lock().unwrap().is_empty() && cam_trace.lock().unwrap().is_empty());
        assert!(!path.exists() && !host.snapshot().busy);
        drop(foreign);
    }
}
#[test]
fn media_camera_failures_and_unwind_revoke_both_inputs_and_latch_uncertain_cleanup() {
    for mode in 0..5 {
        let (source, feed, reader) = MediaSource::new();
        let host = Host::default();
        let path = root().join("output.barrier");
        let (mic, _pcm, trace) = capture_launch(-1, None);
        let (camera, _frames, cam_trace) = camera_launch(mode, trace.clone(), None);
        let input = host
            .start_media_tapped(
                config(path.clone()),
                mic,
                camera,
                feed,
                source.state.clone(),
                reader,
                0,
                true,
                |_| {},
            )
            .unwrap();
        if mode == 3 {
            let _endpoint = input.take_endpoint(Duration::from_secs(3)).unwrap();
            host.request_stop();
        }
        assert_eq!(host.wait_idle(Duration::from_secs(10)), mode < 3);
        let s = host.snapshot();
        assert!(!s.busy && s.camera_submitted == 0);
        assert_eq!(s.cleanup_uncertain, mode >= 3);
        if mode == 4 {
            assert_eq!(s.failure, Some(HostError::Thread));
        } else {
            assert!(matches!(s.failure, Some(HostError::Camera(_))), "{s:?}");
        }
        assert!(s.capture_cleanup.unwrap().failure().is_none());
        assert_eq!(s.camera_cleanup.unwrap().failure().is_some(), mode == 3);
        let cam = cam_trace.lock().unwrap();
        assert_eq!(cam.first(), Some(&"create"));
        assert_eq!(cam.last(), Some(&"drop"));
        assert_eq!(cam.iter().filter(|s| **s == "finish").count(), 1);
        drop(cam);
        assert_eq!(trace.lock().unwrap().last(), Some(&"drop"));
        drop(input);
        drop(host);
        fenced(path);
    }
}
#[test]
fn media_camera_frame_returning_after_stop_is_not_submitted_to_eye() {
    let (source, feed, reader) = MediaSource::new();
    let host = Host::default();
    let path = root().join("output.barrier");
    let (mic, _pcm, trace) = capture_launch(-1, None);
    let (notice, ready) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let (camera, frames, _cam_trace) = camera_launch(5, trace, Some((notice, gate)));
    let input = host
        .start_media_tapped(
            config(path.clone()),
            mic,
            camera,
            feed,
            source.state.clone(),
            reader,
            0,
            true,
            |_| {},
        )
        .unwrap();
    let mut mf = v::Reader::connect(input.take_endpoint(Duration::from_secs(3)).unwrap());
    ready.recv_timeout(Duration::from_secs(3)).unwrap();
    frames.send(camera_frame(input.generation())).unwrap();
    host.request_stop();
    assert!(!host.wait_idle(Duration::from_millis(20)));
    release.send(()).unwrap();
    assert!(host.wait_idle(Duration::from_secs(10)));
    let s = host.snapshot();
    assert_eq!(s.camera.unwrap().delivered, 1);
    assert_eq!(s.camera_submitted, 0);
    assert_eq!(s.output.unwrap().committed_windows, 0);
    mf.neutral_after_stop();
    drop(input);
    drop(host);
    fenced(path);
}

#[test]
fn media_camera_source_stopped_during_prepare_cannot_start_either_input() {
    let (source, feed, reader) = MediaSource::new();
    let host = Host::default();
    let path = root().join("output.barrier");
    let (mic, _pcm, trace) = capture_launch(-1, None);
    let (notice, ready) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let (camera, _frames, cam_trace) = camera_launch(7, trace.clone(), Some((notice, gate)));
    let input = host
        .start_media_tapped(
            config(path.clone()),
            mic,
            camera,
            feed,
            source.state.clone(),
            reader,
            0,
            true,
            |_| {},
        )
        .unwrap();
    ready.recv_timeout(Duration::from_secs(3)).unwrap();
    source.state.request_stop();
    assert!(!host.wait_idle(Duration::from_millis(20)));
    release.send(()).unwrap();
    assert!(host.wait_idle(Duration::from_secs(10)));
    assert!(!cam_trace.lock().unwrap().contains(&"start"));
    assert!(!trace.lock().unwrap().contains(&"prepare"));
    assert_eq!(host.snapshot().camera_submitted, 0);
    assert!(input.take_endpoint(Duration::ZERO).is_err());
    drop(input);
    drop(host);
    fenced(path);
}
#[test]
fn media_camera_to_actual_eye_worker_and_mf_keeps_original_qpc_and_voice() {
    let (source, feed, reader) = MediaSource::new();
    let host = Host::default();
    let path = root().join("output.barrier");
    let (mic, pcm_send, trace) = capture_launch(-1, None);
    let (cleaning, cleanup_ready) = mpsc::channel();
    let (finish, finish_gate) = mpsc::channel();
    let (camera, frames, cam_trace) =
        camera_launch(-1, trace.clone(), Some((cleaning, finish_gate)));
    let (proof, proven) = mpsc::channel();
    let (resume, resume_gate) = mpsc::channel();
    let mut base = None;
    let mut consumed = 0;
    let mut reported = false;
    let mut pts = 0;
    let input = host
        .start_media_tapped(
            config(path.clone()),
            mic,
            camera,
            feed,
            source.state.clone(),
            reader,
            0,
            true,
            move |owner| {
                if base.is_none() {
                    // The real MF consumer is connected before the first camera
                    // packet. Wait only after this owner actually pumped that
                    // packet, then anchor: setup time is not device consumption.
                    if !cam_trace.lock().unwrap().contains(&"frame") {
                        return;
                    }
                    let until = Instant::now() + Duration::from_secs(3);
                    while owner.snapshot().eye.unwrap().received == 0 {
                        assert!(Instant::now() < until, "camera correction missing");
                        thread::sleep(Duration::from_millis(1));
                    }
                    let now = v::now().qpc_100ns();
                    a::anchor(&owner.audio, now);
                    base = Some(now);
                    let mut first = pcm(now - 1_100_000, 0);
                    first.samples.resize(4800, 0.2);
                    first.flags.discontinuity = true;
                    pcm_send.send(first).unwrap();
                    pcm_send.send(pcm(now - 100_000, 10)).unwrap();
                } else {
                    assert_ne!(
                        owner.snapshot().phase,
                        Phase::WaitingVideo,
                        "synthetic device must not consume before output starts"
                    );
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
                    resume_gate.recv_timeout(Duration::from_secs(3)).unwrap();
                }
            },
        )
        .unwrap();
    let endpoint = input.take_endpoint(Duration::from_secs(3)).unwrap();
    let mut mf = v::Reader::connect(endpoint);
    let frame = camera_frame(input.generation());
    let capture_time = frame.capture.qpc_100ns();
    frames.send(frame).unwrap();
    let (pts, values) = proven.recv_timeout(Duration::from_secs(3)).unwrap_or_else(|e| {
        let before = host.snapshot();
        host.request_stop();
        let _ = resume.send(());
        let _ = finish.send(());
        let clean = host.wait_idle(Duration::from_secs(10));
        panic!("full camera/voice proof missing: {e:?}; before={before:?}; clean={clean}; after={:?}", host.snapshot());
    });
    assert_eq!(values.len(), 960);
    assert!(values.iter().all(|v| (*v - 0.2).abs() < 0.00001));
    mf.sample_eye(81, capture_time, pts);
    let s = host.snapshot();
    assert_eq!(s.camera_submitted, 1);
    assert_eq!(s.camera_gaps, 1);
    assert_eq!(s.admitted_packets, 2);
    host.request_stop();
    resume.send(()).unwrap();
    cleanup_ready.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(trace.lock().unwrap().contains(&"finish"));
    assert!(!host.wait_idle(Duration::from_millis(20)) && host.snapshot().busy);
    finish.send(()).unwrap();
    assert!(host.wait_idle(Duration::from_secs(10)));
    assert!(host.snapshot().camera_cleanup.unwrap().failure().is_none());
    mf.neutral_after_stop();
    drop(input);
    drop(host);
    fenced(path);
}
