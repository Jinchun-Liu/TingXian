use super::*;
use sha2::{Digest, Sha256};
use std::{fs, path::Path, sync::atomic::{AtomicU64, Ordering}, thread, time::{Instant, SystemTime, UNIX_EPOCH}};
static COUNTER: AtomicU64 = AtomicU64::new(0);
fn fixture(name: &str) -> BundledWorker {
    let bin = PathBuf::from(std::env::var_os("TINGXIAN_VIDEO_TEST_BIN").expect("explicit compiled fixture directory"));
    let root = PathBuf::from(std::env::var_os("TINGXIAN_VIDEO_TEST_ROOT").expect("explicit synthetic evidence root"));
    let folder = root.join(format!("case-{}-{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(), COUNTER.fetch_add(1, Ordering::SeqCst)));
    fs::create_dir_all(&folder).unwrap();
    let src = if name.starts_with("eye-") {
        PathBuf::from(std::env::var_os("TINGXIAN_EYE_TEST_BIN").expect("explicit eye fixture directory")).join("tingxian-eye-worker-fixture.exe")
    } else { bin.join(if name == "production.exe" { "tingxian-video-worker.exe" } else { "tingxian-video-worker-fixture.exe" }) };
    // Pin the known build BEFORE copying. Rehashing only the destination could
    // accidentally bless bytes changed by a copy/host intervention as trusted.
    let expected = format!("{:x}", Sha256::digest(fs::read(&src).unwrap()));
    let path = folder.join(name); fs::copy(src, &path).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(fs::read(&path).unwrap())), expected,
        "synthetic fixture copy differs from the original build");
    BundledWorker { sha256: expected, path }
}
fn launched(folder: &Path) {
    reached(folder,"launched");
}
fn reached(folder: &Path, stage: &str) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !folder.join(stage).exists() { assert!(Instant::now() < deadline, "worker never reached {stage}"); thread::sleep(Duration::from_millis(10)); }
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
    let stale=state.snapshot().generation;state.request_stop();
    assert!(state.begin_if_current(Some(fixture("valid.exe")),stale).is_err(),"a webview start queued before stop was admitted");
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
    let worker = fixture("valid.exe"); let path = worker.path.clone();
    let mut bytes = fs::read(&path).unwrap(); bytes[0] ^= 1; fs::write(&path, bytes).unwrap();
    assert_eq!(state.begin(Some(worker)).unwrap().unwrap().execute().reason, Some(Reason::Integrity));
    assert!(!path.parent().unwrap().join("launched").exists(), "a changed copy must not be repinned or executed");
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
    let worker=fixture("valid.exe"); let path=worker.path.clone();
    let queued=state.begin(Some(worker)).unwrap();state.request_stop();drop(queued);
    assert_eq!(state.snapshot().phase,Phase::Off,"dropping an already cancelled dispatch is not a crash");
    assert!(state.snapshot().reason.is_none());
    assert!(!path.parent().unwrap().join("launched").exists());
}

fn eye_job(worker: &BundledWorker) -> EyeJob {
    let folder = worker.path.parent().unwrap();
    let input = folder.join("synthetic.bgr"); fs::write(&input, vec![17u8;24*3]).unwrap();
    EyeJob { input, output: folder.join("new-output.bgr"), width:4, height:2, frames:3, provider:EyeProvider::Synthetic }
}
fn begin_eye(state: &Arc<Controller>, worker: BundledWorker, job: EyeJob) -> Run {
    state.begin_eye_if_current(Some(worker),job,state.snapshot().generation).unwrap().unwrap()
}
#[test]
fn eye_corrected_bytes_use_restricted_files_and_explicit_generations() {
    let state = Arc::new(Controller::default());
    for _ in 0..3 {
        let worker=fixture("eye-valid.exe"); let job=eye_job(&worker); let output=job.output.clone();
        std::env::set_var("TINGXIAN_TEST_SECRET","synthetic-value-no-credential");
        let done=begin_eye(&state,worker,job).execute();
        std::env::remove_var("TINGXIAN_TEST_SECRET");
        assert_eq!(done.phase,Phase::Completed,"{done:?}");
        let report=done.eye.unwrap(); assert_eq!(report.corrected,3); assert_eq!(report.neutral,0);
        assert_eq!(fs::read(output).unwrap(),vec![200u8;72]);
        assert!(done.inspection.is_none()); assert!(state.wait_idle(Duration::ZERO));
        state.request_stop(); assert_eq!(state.snapshot().phase,Phase::Off); assert!(state.snapshot().eye.is_none());
    }
}
#[test]
fn eye_fault_neutralizes_and_does_not_accept_raw_provider_bytes() {
    let state=Arc::new(Controller::default()); let worker=fixture("eye-fail.exe"); let job=eye_job(&worker); let output=job.output.clone();
    let done=begin_eye(&state,worker,job).execute();
    assert_eq!(done.phase,Phase::Fault); assert_eq!(done.reason,Some(Reason::EyeProcessing));
    let report=done.eye.unwrap(); assert_eq!(report.corrected,0); assert_eq!(report.neutral,1); assert_eq!(report.error,4);
    assert_eq!(fs::read(output).unwrap(),vec![32u8;24],"raw input or stale tail escaped failure");
}
#[test]
fn eye_stop_during_prepare_or_process_discards_late_results() {
    for name in ["eye-wait.exe","eye-slow.exe"] {
        let state=Arc::new(Controller::default()); let worker=fixture(name); let job=eye_job(&worker);
        let folder=worker.path.parent().unwrap().to_owned(); let output=job.output.clone();
        let run=begin_eye(&state,worker,job); let running=thread::spawn(move ||run.execute());
        reached(&folder,if name=="eye-wait.exe" {"preparing"} else {"processing"}); state.request_stop();
        let done=running.join().unwrap(); assert_eq!(done.phase,Phase::Off,"{done:?}");
        assert!(done.eye.is_none()); assert!(state.wait_idle(Duration::ZERO));
        assert!(fs::read(output).unwrap().is_empty());
    }
}
#[test]
fn eye_queued_cancel_does_not_open_files_or_run_and_stale_ticket_is_rejected() {
    let state=Arc::new(Controller::default()); let worker=fixture("eye-valid.exe"); let job=eye_job(&worker);
    let folder=worker.path.parent().unwrap().to_owned(); let output=job.output.clone();
    let run=begin_eye(&state,worker,job); state.request_stop();
    assert_eq!(run.execute().phase,Phase::Off);
    assert!(!output.exists() && !folder.join("launched").exists());
    let worker=fixture("eye-valid.exe"); let job=eye_job(&worker); let stale=state.snapshot().generation;
    state.request_stop(); assert!(state.begin_eye_if_current(Some(worker),job,stale).is_err());
}
#[test]
fn eye_input_redirection_wrong_length_and_existing_output_are_refused() {
    for mode in 0..3 {
        let state=Arc::new(Controller::default()); let worker=fixture("eye-valid.exe"); let job=eye_job(&worker);
        let folder=worker.path.parent().unwrap().to_owned(); let output=job.output.clone();
        if mode==0 { fs::write(&job.input,vec![17u8;71]).unwrap(); }
        if mode==1 { fs::hard_link(&job.input,job.input.with_extension("alias")).unwrap(); }
        if mode==2 { fs::write(&job.output,b"preserve-existing").unwrap(); }
        let done=begin_eye(&state,worker,job).execute();
        assert_eq!(done.reason,Some(if mode==2 {Reason::EyeOutput} else {Reason::EyeInput}));
        assert!(!folder.join("launched").exists());
        if mode==2 { assert_eq!(fs::read(output).unwrap(),b"preserve-existing"); }
    }
}
#[test]
fn eye_hang_crash_and_uncertain_provider_cleanup_remain_faults() {
    for (name,reason) in [("eye-hang.exe",Reason::Timeout),("eye-crash.exe",Reason::WorkerExit),("eye-cleanup.exe",Reason::ProviderCleanup)] {
        let state=Arc::new(Controller::default()); let worker=fixture(name); let job=eye_job(&worker);
        let done=begin_eye(&state,worker,job).execute_with_timeout(Duration::from_millis(600));
        assert_eq!(done.phase,Phase::Fault,"{done:?}"); assert_eq!(done.reason,Some(reason));
        assert!(state.wait_idle(Duration::ZERO),"owned process was not reaped");
    }
}
#[test]
fn eye_explicit_stop_of_hung_provider_is_a_visible_forced_stop() {
    let state=Arc::new(Controller::default()); let worker=fixture("eye-hang.exe"); let job=eye_job(&worker);
    let folder=worker.path.parent().unwrap().to_owned(); let run=begin_eye(&state,worker,job);
    let running=thread::spawn(move ||run.execute()); reached(&folder,"preparing");
    state.request_stop(); let done=running.join().unwrap();
    assert_eq!(done.phase,Phase::Fault); assert_eq!(done.reason,Some(Reason::ForcedStop));
    assert!(done.eye.is_none() && state.wait_idle(Duration::ZERO));
}
#[test]
fn eye_stop_winning_completion_lock_cannot_publish_a_previously_sampled_success() {
    let state=Arc::new(Controller::default()); let worker=fixture("eye-valid.exe"); let job=eye_job(&worker);
    let run=begin_eye(&state,worker,job);
    let stale_cancelled=run.signal.stopped(); assert!(!stale_cancelled);
    let report=EyeReport{protocol:2,generation:run.id,provider:EyeProvider::Synthetic,width:4,height:2,expected_frames:3,
        corrected:3,neutral:0,outcome:EyeOutcome::Completed,error:0,cleanup_uncertain:false,
        camera_started:false,audio_started:false,external_output_started:false};
    state.request_stop();
    state.complete(run.id,stale_cancelled,Ok(Report::Eye(report)));
    let done=state.snapshot(); assert_eq!(done.phase,Phase::Off); assert!(done.eye.is_none());
    assert!(state.wait_idle(Duration::ZERO));
}
#[test]
fn eye_owner_exit_reaps_actual_worker_job() {
    if let Some(path)=std::env::var_os("TINGXIAN_EYE_OWNER_CASE") {
        let path=PathBuf::from(path); let folder=path.parent().unwrap();
        let job=EyeJob{input:folder.join("synthetic.bgr"),output:folder.join("new-output.bgr"),width:4,height:2,frames:3,provider:EyeProvider::Synthetic};
        let worker=BundledWorker{sha256:format!("{:x}",Sha256::digest(fs::read(&path).unwrap())),path};
        let state=Arc::new(Controller::default());
        let _=begin_eye(&state,worker,job).execute();
        panic!("owner should have been terminated by the exact spawned Child handle");
    }
    let worker=fixture("eye-owner.exe"); let _job=eye_job(&worker); let folder=worker.path.parent().unwrap().to_owned();
    let mut child=std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact","tests::eye_owner_exit_reaps_actual_worker_job","--nocapture"])
        .env("TINGXIAN_EYE_OWNER_CASE",&worker.path)
        .stdout(fs::File::create(folder.join("owner.stdout")).unwrap())
        .stderr(fs::File::create(folder.join("owner.stderr")).unwrap()).spawn().unwrap();
    // Own the child through the entire failure path too; never leave a test owner.
    let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        reached(&folder,"preparing");
        let deadline=Instant::now()+Duration::from_secs(3);
        let pid=loop {
            if let Ok(text)=fs::read_to_string(folder.join("launched")) { if let Ok(pid)=text.parse::<u32>() {break pid;} }
            assert!(Instant::now()<deadline); thread::sleep(Duration::from_millis(10));
        };
        use windows_sys::Win32::{Foundation::*,System::Threading::*,Storage::FileSystem::SYNCHRONIZE};
        let observed=unsafe{OpenProcess(SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION,0,pid)};
        assert!(!observed.is_null());
        struct Observation(HANDLE);
        impl Drop for Observation {fn drop(&mut self) {unsafe{CloseHandle(self.0)};}}
        let observed=Observation(observed);
        // Observation handle has NO terminate access; only the exact Child is killed.
        assert_eq!(unsafe{WaitForSingleObject(observed.0,0)},WAIT_TIMEOUT);
        child.kill().unwrap(); child.wait().unwrap();
        let result=unsafe{WaitForSingleObject(observed.0,2000)};
        assert_eq!(result,WAIT_OBJECT_0,"native worker survived the owner job lifetime");
    }));
    let _=child.kill(); let _=child.wait();
    if let Err(panic)=result {std::panic::resume_unwind(panic);}
}
