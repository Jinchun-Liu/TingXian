use super::*;
use sha2::{Digest, Sha256};
use std::{fs, path::Path, sync::atomic::{AtomicU64, Ordering}, thread, time::{Instant, SystemTime, UNIX_EPOCH}};
static COUNTER: AtomicU64 = AtomicU64::new(0);
fn fixture(name: &str) -> BundledWorker {
    let bin = PathBuf::from(std::env::var_os("TINGXIAN_VIDEO_TEST_BIN").expect("explicit compiled fixture directory"));
    let root = PathBuf::from(std::env::var_os("TINGXIAN_VIDEO_TEST_ROOT").expect("explicit synthetic evidence root"));
    let folder = root.join(format!("case-{}-{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(), COUNTER.fetch_add(1, Ordering::SeqCst)));
    fs::create_dir_all(&folder).unwrap();
    let src = bin.join(if name == "production.exe" { "tingxian-video-worker.exe" } else { "tingxian-video-worker-fixture.exe" });
    let path = folder.join(name); fs::copy(src, &path).unwrap();
    BundledWorker { sha256: format!("{:x}", Sha256::digest(fs::read(&path).unwrap())), path }
}
fn launched(folder: &Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !folder.join("launched").exists() { assert!(Instant::now() < deadline, "worker never launched"); thread::sleep(Duration::from_millis(10)); }
}
#[test]
fn real_inspection_is_unavailable_without_runtime_and_reports_only_prerequisites() {
    let state = Arc::new(Controller::default());
    let snapshot = state.begin(Some(fixture("production.exe"))).unwrap().unwrap().execute();
    assert_eq!(snapshot.phase, Phase::Unavailable);
    assert_eq!(snapshot.reason, Some(Reason::RuntimeNotPinned));
    assert!(!snapshot.inspection.unwrap().media_started);
    assert!(state.wait_idle(Duration::ZERO));
}
#[test]
fn stop_before_dispatched_launch_never_creates_a_process() {
    let state = Arc::new(Controller::default()); let worker = fixture("valid.exe"); let folder = worker.path.parent().unwrap().to_owned();
    let run = state.begin(Some(worker)).unwrap().unwrap(); state.request_stop();
    let stopped = run.execute(); assert_eq!(stopped.phase, Phase::Off); assert!(stopped.inspection.is_none());
    assert!(!folder.join("launched").exists());
}
#[test]
fn running_stop_suppresses_late_report_and_explicit_retry_works() {
    let state = Arc::new(Controller::default()); let worker = fixture("wait.exe"); let folder = worker.path.parent().unwrap().to_owned();
    let run = state.begin(Some(worker)).unwrap().unwrap(); let running = thread::spawn(move ||run.execute());
    launched(&folder); assert!(state.begin(None).is_err()); state.request_stop();
    let stopped = running.join().unwrap(); assert_eq!(stopped.phase, Phase::Off); assert!(stopped.inspection.is_none());
    assert!(state.wait_idle(Duration::from_secs(1)));
    assert_eq!(state.begin(Some(fixture("valid.exe"))).unwrap().unwrap().execute().phase, Phase::Unavailable);
}
#[test]
fn exit_suspension_blocks_starts_until_cancel_and_never_restores_work() {
    let state = Arc::new(Controller::default()); let worker = fixture("wait.exe"); let folder = worker.path.parent().unwrap().to_owned();
    let run = state.begin(Some(worker)).unwrap().unwrap(); let running = thread::spawn(move ||run.execute());
    launched(&folder); state.suspend(); assert!(state.begin(None).is_err());
    assert!(state.wait_idle(Duration::from_secs(3))); running.join().unwrap();
    state.resume(); assert_eq!(state.snapshot().phase, Phase::Off);
    assert!(state.wait_idle(Duration::ZERO));
}
#[test]
fn wrong_hash_and_hard_link_are_rejected_before_execution() {
    let state = Arc::new(Controller::default()); let mut worker = fixture("valid.exe"); let path = worker.path.clone();
    worker.sha256 = "0".repeat(64);
    assert_eq!(state.begin(Some(worker)).unwrap().unwrap().execute().reason, Some(Reason::Integrity));
    assert!(!path.parent().unwrap().join("launched").exists());
    let worker = fixture("valid.exe"); fs::hard_link(&worker.path, worker.path.with_extension("alias")).unwrap();
    assert_eq!(state.begin(Some(worker)).unwrap().unwrap().execute().reason, Some(Reason::Integrity));
}
#[test]
fn worker_crash_and_invalid_report_remain_visible_faults() {
    for (name, reason) in [("crash.exe", Reason::WorkerExit), ("invalid.exe", Reason::Protocol)] {
        let state = Arc::new(Controller::default());
        assert_eq!(state.begin(Some(fixture(name))).unwrap().unwrap().execute().reason, Some(reason));
        assert!(state.wait_idle(Duration::ZERO));
    }
}
#[test]
fn hung_or_overproducing_worker_is_reaped_and_not_reported_as_clean_stop() {
    for name in ["hang.exe", "oversize.exe"] {
        let state = Arc::new(Controller::default());
        let snapshot = state.begin(Some(fixture(name))).unwrap().unwrap().execute_with_timeout(Duration::from_millis(250));
        assert_eq!(snapshot.phase, Phase::Fault);
        if name == "hang.exe" { assert_eq!(snapshot.reason, Some(Reason::Timeout)); }
        else { assert!(matches!(snapshot.reason, Some(Reason::Protocol | Reason::Timeout)), "oversized output must be rejected or bounded by timeout"); }
        assert!(state.wait_idle(Duration::ZERO));
    }
}
#[test]
fn explicit_stop_of_unresponsive_worker_records_forced_cleanup() {
    let state = Arc::new(Controller::default()); let worker = fixture("hang.exe"); let folder = worker.path.parent().unwrap().to_owned();
    let run = state.begin(Some(worker)).unwrap().unwrap(); let running = thread::spawn(move ||run.execute());
    launched(&folder); state.request_stop();
    let snapshot = running.join().unwrap(); assert_eq!(snapshot.reason, Some(Reason::ForcedStop));
    assert!(state.wait_idle(Duration::ZERO));
}
#[test]
fn child_has_no_app_secret_and_cannot_spawn_descendants() {
    let state = Arc::new(Controller::default());
    std::env::set_var("TINGXIAN_TEST_SECRET", "synthetic-value-no-credential");
    let snapshot = state.begin(Some(fixture("descendant.exe"))).unwrap().unwrap().execute();
    std::env::remove_var("TINGXIAN_TEST_SECRET");
    assert_eq!(snapshot.reason, Some(Reason::RuntimeNotPinned));
}
#[test]
fn dropped_dispatch_releases_slot_without_starting() {
    let state = Arc::new(Controller::default()); let worker = fixture("valid.exe"); let path = worker.path.clone();
    drop(state.begin(Some(worker)).unwrap().unwrap());
    assert_eq!(state.snapshot().reason, Some(Reason::Abandoned));
    assert!(!path.parent().unwrap().join("launched").exists());
    assert!(state.wait_idle(Duration::ZERO));
}
