use super::*;
use crate::{Controller, EyeProvider, QpcStreamFeed, QpcStreamFrame, Reason, STREAM_BGR_BYTES};
struct Link {
    state: Arc<Controller>,
    feed: QpcStreamFeed,
    run: Option<thread::JoinHandle<crate::Snapshot>>,
}
impl Link {
    fn stop(&mut self) -> crate::Snapshot {
        self.state.request_stop();
        self.run.take().unwrap().join().unwrap()
    }
}
impl Drop for Link {
    fn drop(&mut self) {
        self.state.request_stop();
        if let Some(run) = self.run.take() {
            let _ = run.join();
        }
    }
}
fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(Instant::now() < deadline, "eye/joint bounded condition");
        thread::sleep(Duration::from_millis(1));
    }
}
fn bound(mode: &str) -> (JointOutput, v::Reader, Link, PathBuf) {
    let state = Arc::new(Controller::default());
    let (run, feed) = state
        .begin_qpc_stream_if_current(Some(crate::tests::fixture(mode)), EyeProvider::Synthetic, 0)
        .unwrap()
        .unwrap();
    let run = thread::spawn(move || run.execute());
    let corrections = feed.open_corrections(Duration::from_secs(3)).unwrap();
    let path = root().join("output.barrier");
    let (mut owner, endpoint) = JointOutput::prepare(config(path.clone()), true).unwrap();
    let reader = v::Reader::connect(endpoint);
    v::ready(&owner.video);
    owner.bind_corrections(corrections).unwrap();
    assert_eq!(owner.advance().unwrap(), Progress::Waiting);
    assert_eq!(
        owner.audio.status().unwrap().submitted_frames,
        0,
        "no prime/audio before first correction"
    );
    (
        owner,
        reader,
        Link {
            state,
            feed,
            run: Some(run),
        },
        path,
    )
}
fn input(link: &Link, id: u64) -> u64 {
    let bgr = vec![31; STREAM_BGR_BYTES];
    let capture = v::now().qpc_100ns() - 300_000; // Explicit synthetic capture history, no timestamp refresh.
    link.feed
        .submit(QpcStreamFrame {
            id,
            capture: Time::from_qpc_100ns(capture),
            bgr,
        })
        .unwrap();
    capture
}
fn stage(owner: &mut JointOutput, link: &Link) -> (u64, u64) {
    let capture = input(link, 71);
    until(|| owner.snapshot().eye.unwrap().queued);
    let base = v::now().qpc_100ns();
    a::anchor(&owner.audio, base);
    for n in 0..10 {
        owner.push_audio(owner.generation, packet(base, n)).unwrap();
    }
    assert_eq!(owner.advance().unwrap(), Progress::Waiting);
    a::step(&owner.audio, 96, 0);
    while v::now().qpc_100ns() < base + 20_000 {
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(owner.advance().unwrap(), Progress::Staged);
    assert!(
        !owner.pending.as_ref().unwrap().neutral,
        "actual correction must be fresh for the fixed timeline delay"
    );
    assert_eq!(
        owner.audio.status().unwrap().submitted_frames,
        960,
        "prime only before video receipt"
    );
    assert_eq!(owner.snapshot().eye.unwrap().owner_in_flight, 0);
    (capture, base + 200_000)
}
fn stopped_with_eye(owner: &JointOutput) {
    local_stopped(owner);
    let status = owner.snapshot().eye.unwrap();
    assert!(status.revoked && !status.queued);
    assert_eq!(status.owner_in_flight, 0);
    assert_eq!(owner.snapshot().cleanup.unwrap().eye, Some(Ok(())));
    assert!(a::read(&owner.audio).is_empty());
}
#[test]
fn joint_eye_actual_worker_to_mf_and_pcm_then_worker_stop_revokes_both() {
    let (mut owner, mut reader, mut link, path) = bound("eye-stream.exe");
    let (capture, pts) = stage(&mut owner, &link);
    commit(&mut owner);
    assert_eq!(owner.snapshot().committed_windows, 1);
    assert_eq!(owner.snapshot().neutral_windows, 0);
    let samples = a::read(&owner.audio);
    assert_eq!(samples.len(), 960);
    assert!(samples.iter().all(|x| (*x - 0.2).abs() < 0.00001));
    reader.sample_eye(71, capture, pts);
    link.state.request_stop();
    assert!(owner.snapshot().eye.unwrap().revoked);
    assert_eq!(
        owner.video.status().unwrap().phase,
        timed_video::Phase::Stopping,
        "video cancels without another owner tick"
    );
    assert!(
        owner.snapshot().cleanup.is_none(),
        "request is not confirmed audio/thread cleanup"
    );
    assert_eq!(owner.advance(), Err(Error::Eye(Reason::Abandoned)));
    stopped_with_eye(&owner);
    reader.neutral_after_stop();
    assert_eq!(owner.snapshot().committed_windows, 1);
    link.stop();
    drop(owner);
    fenced(path);
}
#[test]
fn joint_eye_stop_before_voice_commit_discards_the_pending_window() {
    let (mut owner, mut reader, mut link, path) = bound("eye-stream.exe");
    stage(&mut owner, &link);
    link.state.request_stop();
    assert_eq!(owner.advance(), Err(Error::Eye(Reason::Abandoned)));
    assert_eq!(owner.snapshot().committed_windows, 0);
    assert_eq!(owner.audio.status().unwrap().submitted_frames, 960);
    stopped_with_eye(&owner);
    reader.neutral_after_stop();
    link.stop();
    drop(owner);
    fenced(path);
}
#[test]
fn joint_eye_provider_failure_and_capture_expiry_stop_without_starting_audio() {
    for mode in ["eye-fail.exe", "eye-stream.exe"] {
        let (mut owner, mut reader, mut link, path) = bound(mode);
        input(&link, 1);
        if mode == "eye-stream.exe" {
            until(|| owner.snapshot().eye.unwrap().received == 1);
        }
        until(|| owner.snapshot().eye.unwrap().revoked);
        assert!(matches!(owner.advance(), Err(Error::Eye(_))));
        assert_eq!(owner.audio.status().unwrap().submitted_frames, 0);
        stopped_with_eye(&owner);
        reader.neutral_after_stop();
        link.stop();
        drop(owner);
        fenced(path);
    }
}
#[test]
fn joint_eye_latest_slot_is_bounded_and_manual_video_cannot_bypass_it() {
    let (mut owner, mut reader, mut link, path) = bound("eye-stream.exe");
    for id in 1..=3 {
        input(&link, id);
        until(|| owner.snapshot().eye.unwrap().received == id);
    }
    let status = owner.snapshot().eye.unwrap();
    assert!(status.queued);
    assert_eq!((status.received, status.replaced, status.taken), (3, 2, 0));
    let frame = || CorrectedFrame {
        id: 99,
        capture: v::now(),
        nv12: Arc::from(vec![160; NV12_BYTES]),
    };
    assert_eq!(
        owner.push_video(owner.generation + 1, frame()),
        Err(Error::Stale)
    );
    assert!(!owner.snapshot().eye.unwrap().revoked);
    assert_eq!(
        owner.push_video(owner.generation, frame()),
        Err(Error::Source)
    );
    stopped_with_eye(&owner);
    assert_eq!(owner.snapshot().eye.unwrap().discarded, 1);
    reader.neutral_after_stop();
    link.stop();
    drop(owner);
    fenced(path);
}
#[test]
fn joint_eye_partial_packet_stop_retires_reader_without_pipe_wait_on_device_lane() {
    let peer = crate::eye_qpc::tests::partial_peer();
    let corrections = peer.feed.open_corrections(Duration::from_secs(1)).unwrap();
    let path = root().join("output.barrier");
    let (mut owner, endpoint) = JointOutput::prepare(config(path.clone()), true).unwrap();
    let mut reader = v::Reader::connect(endpoint);
    v::ready(&owner.video);
    owner.bind_corrections(corrections).unwrap();
    thread::sleep(Duration::from_millis(20));
    assert_eq!(owner.advance().unwrap(), Progress::Waiting);
    assert_eq!(owner.audio.status().unwrap().submitted_frames, 0);
    owner.stop();
    stopped_with_eye(&owner);
    assert!(peer.feed.status().closed);
    reader.neutral_after_stop();
    drop(owner);
    fenced(path);
}
#[test]
fn joint_eye_first_frame_deadline_never_primes_audio() {
    let (mut owner, mut reader, mut link, path) = bound("eye-stream.exe");
    until(|| owner.snapshot().eye.unwrap().revoked);
    assert_eq!(owner.advance(), Err(Error::Eye(Reason::Timeout)));
    assert_eq!(owner.audio.status().unwrap().submitted_frames, 0);
    stopped_with_eye(&owner);
    reader.neutral_after_stop();
    link.stop();
    drop(owner);
    fenced(path);
}
