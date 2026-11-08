use super::*;
use crate::av_timeline::{PacketFlags, Source, NV12_BYTES, RATE};
use crate::{audio::tests as a, timed_video::tests as v};
use barrier::Barrier;
use std::os::windows::process::CommandExt;
use std::{
    fs,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};
static NEXT: AtomicU64 = AtomicU64::new(1);
#[path = "eye_tests.rs"]
mod eye_tests;
fn root() -> PathBuf {
    let root = PathBuf::from(
        std::env::var_os("TINGXIAN_VIDEO_TEST_ROOT").expect("isolated test directory"),
    )
    .join(format!(
        "joint-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
fn config(path: PathBuf) -> Config {
    Config {
        audio: a::pin(true),
        video: v::pin(),
        pair: a::pair(),
        channels: 2,
        selected_microphone: 7,
        barrier_path: path,
    }
}
struct Signal(HANDLE);
impl Signal {
    fn new() -> Self {
        let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        assert!(!event.is_null());
        Self(event)
    }
    fn handle(&self) -> BorrowedHandle<'_> {
        unsafe { BorrowedHandle::borrow_raw(self.0) }
    }
    fn set(&self) {
        assert_ne!(unsafe { SetEvent(self.0) }, 0);
    }
}
impl Drop for Signal {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn packet(base: u64, n: u64) -> PcmPacket {
    PcmPacket {
        device: 7,
        source: Source::SelfMicrophone,
        first_sample: n * 480,
        capture: Time::from_qpc_100ns(base - 1_000_000 + n * 100_000),
        rate: RATE,
        channels: 1,
        flags: PacketFlags::default(),
        samples: vec![0.2; 480],
    }
}
fn staged(media: bool) -> (JointOutput, v::Reader, PathBuf, u64) {
    let path = root().join("output.barrier");
    let (mut owner, endpoint) = JointOutput::prepare(config(path.clone()), true).unwrap();
    let reader = v::Reader::connect(endpoint);
    v::ready(&owner.video);
    // Prepare synthetic pixels before anchoring the memory device to real QPC.
    let mut pixels = vec![160; NV12_BYTES];
    pixels[1280 * 720..].fill(96);
    let base = v::now().qpc_100ns();
    a::anchor(&owner.audio, base);
    if media {
        for n in 0..10 {
            owner.push_audio(owner.generation, packet(base, n)).unwrap();
        }
        owner
            .push_video(
                owner.generation,
                CorrectedFrame {
                    id: 41,
                    capture: Time::from_qpc_100ns(base - 200_000),
                    nv12: Arc::from(pixels),
                },
            )
            .unwrap();
    }
    assert_eq!(owner.advance().unwrap(), Progress::Waiting); // Prime + first clock only.
    a::step(&owner.audio, 96, 0);
    while v::now().qpc_100ns() < base + 20_000 {
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(owner.advance().unwrap(), Progress::Staged);
    assert_eq!(owner.snapshot().pending_samples, 480);
    assert_eq!(
        owner.audio.status().unwrap().submitted_frames,
        960,
        "only prime, not voice"
    );
    assert!(a::read(&owner.audio).iter().all(|x| *x == 0.0));
    (owner, reader, path, base + 200_000)
}
fn commit(owner: &mut JointOutput) {
    let stop = Signal::new();
    let until = Instant::now() + Duration::from_secs(1);
    loop {
        match owner.wait(stop.handle()).unwrap() {
            Progress::Committed => return,
            Progress::Waiting => assert!(Instant::now() < until),
            other => panic!("unexpected {other:?}"),
        }
    }
}
fn local_stopped(owner: &JointOutput) {
    let s = owner.snapshot();
    assert_eq!(s.pending_samples, 0);
    let c = s.cleanup.unwrap();
    assert!(c.timeline.is_ok());
    assert_eq!(c.audio.unwrap().phase, 3);
    assert!(c.audio.unwrap().revoked);
    assert!(c.video.unwrap().local_clear);
    assert!(!c.video.unwrap().external_drain_verified);
    assert!(c.restart_blocked);
}
fn fenced(path: PathBuf) {
    assert!(matches!(
        Barrier::acquire(&path),
        Err(BarrierError::UncertainDrain)
    ));
    assert!(matches!(
        JointOutput::prepare(config(path), true).map_err(|e| e.cause),
        Err(Error::Barrier(BarrierError::UncertainDrain))
    ));
}

#[test]
fn joint_native_media_admission_gates_pcm_and_mf_pts() {
    let (mut owner, mut reader, path, pts) = staged(true);
    commit(&mut owner);
    let written = a::read(&owner.audio);
    assert_eq!(written.len(), 960);
    assert!(written.iter().all(|v| (*v - 0.2).abs() < 0.00001));
    assert!(written.chunks_exact(2).all(|p| p[0] == p[1]));
    assert_eq!(owner.audio.status().unwrap().submitted_frames, 1440);
    assert_eq!(owner.snapshot().committed_windows, 1);
    assert_eq!(owner.snapshot().neutral_windows, 0);
    reader.sample(41, pts - 400_000, pts); // Actual separate-process MF sample/pixels/PTS.
    owner.stop();
    local_stopped(&owner);
    reader.neutral_after_stop();
    assert!(a::read(&owner.audio).is_empty());
    assert_eq!(owner.advance(), Err(Error::Off));
    drop(owner);
    fenced(path);
}
#[test]
fn joint_missing_input_commits_only_neutral_and_silence() {
    let (mut owner, mut reader, path, pts) = staged(false);
    commit(&mut owner);
    assert_eq!(owner.snapshot().neutral_windows, 1);
    assert_eq!(a::read(&owner.audio), vec![0.0; 960]);
    reader.sample(0, 0, pts);
    owner.stop();
    local_stopped(&owner);
    drop(owner);
    fenced(path);
}
#[test]
fn joint_three_contiguous_windows_keep_device_pts_and_one_pending_slot() {
    let (mut owner, mut reader, path, pts) = staged(true);
    commit(&mut owner);
    let base = pts - 200_000;
    let mut pixels = vec![160; NV12_BYTES];
    pixels[1280 * 720..].fill(96);
    let pixels: Arc<[u8]> = Arc::from(pixels);
    for i in 1..=2u64 {
        while v::now().qpc_100ns() < base + 20_000 + i * 100_000 {
            thread::sleep(Duration::from_millis(1));
        }
        a::step(&owner.audio, 480, 0);
        owner
            .push_audio(owner.generation, packet(base, 9 + i))
            .unwrap();
        owner
            .push_video(
                owner.generation,
                CorrectedFrame {
                    id: 41 + i,
                    capture: Time::from_qpc_100ns(base - 200_000 + i * 100_000),
                    nv12: pixels.clone(),
                },
            )
            .unwrap();
        assert_eq!(owner.advance().unwrap(), Progress::Staged);
        assert_eq!(owner.snapshot().pending_samples, 480);
        assert_eq!(
            owner.pending.as_ref().unwrap().presentation.qpc_100ns(),
            pts + i * 100_000
        );
        commit(&mut owner);
        assert!(a::read(&owner.audio)
            .iter()
            .all(|v| (*v - 0.2).abs() < 0.00001));
    }
    assert_eq!(owner.snapshot().committed_windows, 3);
    assert_eq!(owner.snapshot().neutral_windows, 0);
    assert_eq!(owner.audio.status().unwrap().submitted_frames, 2400);
    reader.sample(43, base, pts + 200_000);
    owner.stop();
    local_stopped(&owner);
    drop(owner);
    fenced(path);
}
#[test]
fn joint_stalled_priming_clock_cannot_wait_past_prime_budget() {
    let path = root().join("output.barrier");
    let (mut owner, endpoint) = JointOutput::prepare(config(path.clone()), true).unwrap();
    let mut reader = v::Reader::connect(endpoint);
    v::ready(&owner.video);
    a::anchor(&owner.audio, v::now().qpc_100ns());
    assert_eq!(owner.advance().unwrap(), Progress::Waiting);
    let deadline = owner.prime_deadline.unwrap();
    while v::now().qpc_100ns() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(owner.advance(), Err(Error::Deadline));
    assert_eq!(owner.snapshot().committed_windows, 0);
    local_stopped(&owner);
    reader.neutral_after_stop();
    drop(owner);
    fenced(path);
}
#[test]
fn joint_prepare_failure_retains_cleanup_and_durable_fence() {
    let path = root().join("output.barrier");
    let candidate = Candidate::inspect(config(path.clone())).unwrap();
    a::step(&candidate.audio, 0, 7); // Prepare fails AFTER crossing the resource boundary.
    let failure = match candidate.activate(true) {
        Err(f) => f,
        Ok(_) => panic!("failed preparation accepted"),
    };
    assert!(matches!(
        failure.cause,
        Error::Audio(audio::Error::Native(_))
    ));
    let cleanup = failure.cleanup.unwrap();
    assert!(cleanup.restart_blocked);
    assert!(cleanup.video.unwrap().local_clear);
    assert_eq!(cleanup.audio.unwrap().submitted_frames, 0);
    fenced(path);
}
#[test]
fn joint_preflight_invalid_pair_does_not_poison_unexposed_profile() {
    let path = root().join("output.barrier");
    let mut c = config(path.clone());
    c.pair.render_id = "not-an-endpoint".into();
    let failure = match JointOutput::prepare(c, true) {
        Err(f) => f,
        Ok(_) => panic!("invalid pair accepted"),
    };
    assert!(matches!(
        failure.cause,
        Error::Audio(audio::Error::Native(_))
    ));
    assert!(
        failure.cleanup.is_none(),
        "unexposed preflight must not prepare either endpoint"
    );
    assert!(
        !path.exists(),
        "invalid selection permanently poisoned the profile"
    );
    let (mut owner, _endpoint) = JointOutput::prepare(config(path.clone()), true).unwrap();
    owner.stop();
    drop(owner);
    fenced(path);
}
#[test]
fn joint_preflight_candidate_is_idle_and_activation_rechecks_unplug() {
    let path = root().join("output.barrier");
    let candidate = Candidate::inspect(config(path.clone())).unwrap();
    assert!(!path.exists());
    assert_eq!(candidate.audio.status().unwrap().phase, 0);
    assert_eq!(candidate.audio.status().unwrap().submitted_frames, 0);
    assert_eq!(
        candidate.video.status().unwrap().phase,
        timed_video::Phase::Idle
    );
    assert!(a::read(&candidate.audio).is_empty());
    a::step(&candidate.audio, 0, 6); // Endpoint becomes inactive after inspection.
    let failure = match candidate.activate(true) {
        Err(f) => f,
        Ok(_) => panic!("stale inventory started"),
    };
    assert!(matches!(
        failure.cause,
        Error::Audio(audio::Error::Native(_))
    ));
    assert!(failure.cleanup.is_none());
    assert!(!path.exists());
    let candidate = Candidate::inspect(config(path.clone())).unwrap();
    assert!(matches!(
        candidate.activate(false),
        Err(PrepareFailure {
            cause: Error::Permission,
            cleanup: None
        })
    ));
    assert!(!path.exists());
}
#[test]
fn joint_preflight_product_unknown_pair_is_metadata_only_and_keeps_existing_fence() {
    let path = root().join("output.barrier");
    fs::write(&path, b"retained unresolved exposure").unwrap();
    let mut c = config(path.clone());
    c.audio = a::pin(false);
    assert!(matches!(
        Candidate::inspect(c),
        Err(Error::Audio(audio::Error::Native(_)))
    ));
    assert_eq!(fs::read(&path).unwrap(), b"retained unresolved exposure");
    // A valid candidate cannot remove a prior record either.
    let candidate = Candidate::inspect(config(path.clone())).unwrap();
    assert!(matches!(
        candidate.activate(true),
        Err(PrepareFailure {
            cause: Error::Barrier(BarrierError::UncertainDrain),
            cleanup: None
        })
    ));
    assert_eq!(fs::read(&path).unwrap(), b"retained unresolved exposure");
}
#[test]
fn joint_stop_event_wins_over_pending_receipt() {
    let (mut owner, mut reader, path, _) = staged(true);
    let stop = Signal::new();
    stop.set();
    assert_eq!(owner.wait(stop.handle()).unwrap(), Progress::Stopped);
    assert_eq!(owner.snapshot().committed_windows, 0);
    local_stopped(&owner);
    reader.neutral_after_stop();
    drop(owner);
    fenced(path);
}
#[test]
fn joint_late_admission_never_commits_expired_audio() {
    let (mut owner, mut reader, path, pts) = staged(true);
    let until = Instant::now() + Duration::from_secs(1);
    while owner.video.status().unwrap().admitted == 0 {
        assert!(Instant::now() < until);
        let event = owner.video.event().unwrap();
        unsafe {
            WaitForSingleObject(event.as_raw_handle(), 1);
        }
    }
    while v::now().qpc_100ns() + GUARD < pts {
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(owner.advance(), Err(Error::Deadline));
    assert_eq!(owner.audio.status().unwrap().submitted_frames, 960);
    local_stopped(&owner);
    reader.neutral_after_stop();
    drop(owner);
    fenced(path);
}
#[test]
fn joint_audio_write_failure_revokes_admitted_video_and_retains_error() {
    let (mut owner, mut reader, path, _) = staged(true);
    a::step(&owner.audio, 0, 2); // Next actual memory-backend write fails.
    let stop = Signal::new();
    loop {
        match owner.wait(stop.handle()) {
            Ok(Progress::Waiting) => {}
            Err(Error::Audio(audio::Error::Native(hr))) => {
                assert!(hr < 0);
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(owner.snapshot().committed_windows, 0);
    assert!(
        owner
            .snapshot()
            .cleanup
            .unwrap()
            .audio
            .unwrap()
            .operation_error
            < 0
    );
    local_stopped(&owner);
    reader.neutral_after_stop();
    drop(owner);
    fenced(path);
}
#[test]
fn joint_audio_fault_callback_cancels_video_before_joint_cleanup() {
    let path = root().join("output.barrier");
    let (mut owner, endpoint) = JointOutput::prepare(config(path.clone()), true).unwrap();
    let mut reader = v::Reader::connect(endpoint);
    v::ready(&owner.video);
    a::anchor(&owner.audio, v::now().qpc_100ns());
    assert_eq!(owner.advance().unwrap(), Progress::Waiting);
    a::step(&owner.audio, 0, 1);
    // Directly exercise the adapter's synchronous callback before joint.fail.
    assert!(owner.audio.reserve(480).is_err());
    assert!(matches!(
        owner.video.status().unwrap().phase,
        timed_video::Phase::Stopping | timed_video::Phase::Stopped
    ));
    assert_eq!(owner.advance(), Err(Error::Revoked));
    local_stopped(&owner);
    reader.neutral_after_stop();
    drop(owner);
    fenced(path);
}
#[test]
fn joint_dead_video_peer_never_unlocks_voice() {
    let path = root().join("output.barrier");
    let (mut owner, endpoint) = JointOutput::prepare(config(path.clone()), true).unwrap();
    let mut reader = v::Reader::connect(endpoint);
    v::ready(&owner.video);
    a::anchor(&owner.audio, v::now().qpc_100ns());
    owner.advance().unwrap();
    reader.child.kill().unwrap();
    reader.child.wait().unwrap();
    a::step(&owner.audio, 96, 0);
    thread::sleep(Duration::from_millis(2));
    let first = owner.advance();
    let error = if let Err(e) = first {
        e
    } else {
        let stop = Signal::new();
        loop {
            match owner.wait(stop.handle()) {
                Err(e) => break e,
                Ok(Progress::Waiting) => {}
                other => panic!("unexpected {other:?}"),
            }
        }
    };
    assert!(
        matches!(
            error,
            Error::Admission(Admission::Unknown(_)) | Error::Video(_)
        ),
        "{error:?}"
    );
    assert_eq!(owner.snapshot().committed_windows, 0);
    assert_eq!(owner.audio.status().unwrap().submitted_frames, 960);
    local_stopped(&owner);
    drop(owner);
    fenced(path);
}
#[test]
fn joint_source_and_generation_checks_do_not_admit_loopback() {
    let path = root().join("output.barrier");
    let (mut owner, endpoint) = JointOutput::prepare(config(path.clone()), true).unwrap();
    let mut reader = v::Reader::connect(endpoint);
    v::ready(&owner.video);
    let base = v::now().qpc_100ns();
    assert_eq!(
        owner.push_audio(owner.generation + 1, packet(base, 0)),
        Err(Error::Stale)
    );
    assert_eq!(owner.phase, Phase::WaitingVideo);
    let mut bad = packet(base, 0);
    bad.source = Source::SystemLoopback;
    assert_eq!(
        owner.push_audio(owner.generation, bad),
        Err(Error::Timeline(av_timeline::Error::Source))
    );
    local_stopped(&owner);
    reader.neutral_after_stop();
    drop(owner);
    fenced(path);
}
#[test]
fn joint_cleanup_preserves_reset_failure_and_stops_other_lane() {
    let (mut owner, mut reader, path, _) = staged(false);
    a::step(&owner.audio, 0, 4);
    let cleanup = owner.stop();
    assert!(cleanup.audio.unwrap().reset_error < 0);
    assert!(!cleanup.audio.unwrap().client_reset);
    local_stopped(&owner);
    reader.neutral_after_stop();
    drop(owner);
    fenced(path);
}
#[test]
fn joint_stop_event_reports_reset_failure_instead_of_completed_stop() {
    let (mut owner, mut reader, path, _) = staged(false);
    a::step(&owner.audio, 0, 4);
    let stop = Signal::new();
    stop.set();
    let result = owner.wait(stop.handle());
    let cleanup = owner.snapshot().cleanup.unwrap();
    let error = cleanup.audio.unwrap().reset_error;
    assert!(error < 0);
    assert_eq!(result, Err(Error::Audio(audio::Error::Native(error))));
    assert_eq!(owner.snapshot().phase, Phase::Fault);
    assert_eq!(owner.snapshot().failure, result.err());
    assert_eq!(owner.snapshot().committed_windows, 0);
    local_stopped(&owner);
    reader.neutral_after_stop();
    drop(owner);
    fenced(path);
}
#[test]
fn joint_denied_intent_does_not_create_barrier_or_start() {
    let path = root().join("output.barrier");
    assert!(matches!(
        JointOutput::prepare(config(path.clone()), false).map_err(|e| e.cause),
        Err(Error::Permission)
    ));
    assert!(!path.exists());
    let video = NativeTimedVideo::load(v::pin()).unwrap();
    let token = video.cancellation();
    drop(video);
    token.cancel(); // Inert after DLL retirement.
}

fn child(path: &std::path::Path, action: &str) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "joint::tests::joint_barrier_child",
            "--nocapture",
        ])
        .env_clear()
        .env("SystemRoot", std::env::var_os("SystemRoot").unwrap())
        .env("TX_JOINT_CHILD_PATH", path)
        .env("TX_JOINT_CHILD_ACTION", action)
        .stdin(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
        .unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(s) = child.try_wait().unwrap() {
            assert!(s.success());
            break;
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            panic!("barrier child deadline");
        }
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn joint_barrier_child() {
    let Some(path) = std::env::var_os("TX_JOINT_CHILD_PATH") else {
        return;
    };
    let path = PathBuf::from(path);
    match std::env::var("TX_JOINT_CHILD_ACTION").unwrap().as_str() {
        "busy" => assert!(matches!(Barrier::acquire(&path), Err(BarrierError::Busy))),
        "arm-crash" => {
            let mut barrier = Barrier::acquire(&path).unwrap();
            barrier.arm(42).unwrap();
            println!("barrier flushed; exit without destructors");
            std::process::exit(0);
        }
        _ => panic!("unknown isolated test action"),
    }
}
#[test]
fn joint_barrier_exclusive_parent_lease_and_crash_persist() {
    let dir = root();
    let path = dir.join("output.barrier");
    let barrier = Barrier::acquire(&path).unwrap();
    assert!(matches!(Barrier::acquire(&path), Err(BarrierError::Busy)));
    child(&path, "busy");
    assert!(fs::rename(&dir, dir.with_extension("moved")).is_err());
    drop(barrier); // No exposure, empty record may be opened again.
    child(&path, "arm-crash");
    assert_eq!(fs::read(&path).unwrap().len(), 32);
    fenced(path);
}
#[test]
fn joint_barrier_rejects_corruption_and_hardlinks_without_overwrite() {
    let dir = root();
    let corrupt = dir.join("corrupt.barrier");
    fs::write(&corrupt, b"x").unwrap();
    assert!(matches!(
        Barrier::acquire(&corrupt),
        Err(BarrierError::UncertainDrain)
    ));
    assert_eq!(fs::read(&corrupt).unwrap(), b"x");
    let original = dir.join("original.barrier");
    fs::write(&original, []).unwrap();
    let linked = dir.join("linked.barrier");
    fs::hard_link(&original, &linked).unwrap();
    assert!(matches!(Barrier::acquire(&linked), Err(BarrierError::Path)));
    assert!(fs::read(&original).unwrap().is_empty());
    assert!(matches!(
        Barrier::acquire(std::path::Path::new("relative.barrier")),
        Err(BarrierError::Path)
    ));
}
