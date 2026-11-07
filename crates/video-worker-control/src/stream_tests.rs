use super::*;
use std::{io::Write, os::windows::io::{AsRawHandle, FromRawHandle}, ptr::{null,null_mut}};
use windows_sys::Win32::{Foundation::*, Storage::FileSystem::*, System::Pipes::*};
#[link(name="kernel32")]
extern "system" { fn GetTickCount64()->u64; }
fn tick()->u64 {unsafe{GetTickCount64()}}
fn frame(id:u64,captured_ms:u64)->StreamFrame {StreamFrame{id,captured_ms,bgr:vec![31;STREAM_BGR_BYTES]}}
fn start(state:&Arc<Controller>,worker:BundledWorker)->(Run,StreamFeed) {
    state.begin_stream_if_current(Some(worker),EyeProvider::Synthetic,state.snapshot().generation).unwrap().unwrap()
}
fn eventually(mut test:impl FnMut()->bool) {
    let end=Instant::now()+Duration::from_secs(3);
    while !test(){assert!(Instant::now()<end,"bounded stream condition not reached");thread::sleep(Duration::from_millis(2));}
}
struct Client(fs::File);
impl Client {
    fn read(&self,bytes:usize)->Vec<u8> {
        let mut data=vec![0;bytes];let mut done=0;let end=Instant::now()+Duration::from_secs(3);
        while done<bytes {
            assert!(Instant::now()<end,"output read deadline");let mut available=0;
            assert_ne!(unsafe{PeekNamedPipe(self.0.as_raw_handle(),null_mut(),0,null_mut(),&mut available,null_mut())},0,"output pipe closed");
            if available==0 {thread::sleep(Duration::from_millis(1));continue;}
            let mut read=0;assert_ne!(unsafe{ReadFile(self.0.as_raw_handle(),data[done..].as_mut_ptr(),available.min((bytes-done) as u32),&mut read,null_mut())},0);
            assert_ne!(read,0);done+=read as usize;
        }
        data
    }
    fn connect(endpoint:NativeOutputEndpoint)->Self {
        let b=endpoint.activation_blob();let name:Vec<u16>=b[4..196].chunks_exact(2).map(|x|u16::from_le_bytes([x[0],x[1]])).collect();
        let raw=unsafe{CreateFileW(name.as_ptr(),GENERIC_READ|GENERIC_WRITE,0,null(),OPEN_EXISTING,0,null_mut())};
        assert_ne!(raw,INVALID_HANDLE_VALUE,"synthetic output connect failed");
        let mut client=Self(unsafe{fs::File::from_raw_handle(raw)});let mut pid=0;
        assert_ne!(unsafe{GetNamedPipeServerProcessId(raw,&mut pid)},0);assert_eq!(pid,u32::from_le_bytes(b[196..200].try_into().unwrap()));
        let mut hello=[0u8;40];hello[0..4].copy_from_slice(&0x31565854u32.to_le_bytes());hello[4..6].copy_from_slice(&1u16.to_le_bytes());
        hello[6..8].copy_from_slice(&1u16.to_le_bytes());hello[8..12].copy_from_slice(&32u32.to_le_bytes());
        client.0.write_all(&hello).unwrap();client.0.write_all(&b[204..236]).unwrap();
        let ack=client.read(40);assert_eq!(&ack[0..8],&[0x54,0x58,0x56,0x31,1,0,2,0]);
        let begin=client.read(40);assert_eq!(u16::from_le_bytes(begin[6..8].try_into().unwrap()),3);
        client
    }
    fn corrected(&self,id:u64,time:u64) {
        let h=self.read(40);assert_eq!(u16::from_le_bytes(h[6..8].try_into().unwrap()),4);
        assert_eq!(u64::from_le_bytes(h[24..32].try_into().unwrap()),id);
        assert_eq!(u64::from_le_bytes(h[32..40].try_into().unwrap()),time);
        let bytes=1280*720*3/2;assert_eq!(u32::from_le_bytes(h[8..12].try_into().unwrap()),bytes);
        let pixels=self.read(bytes as usize);assert!(pixels[..1280*720].iter().all(|v|*v==188));assert!(pixels[1280*720..].iter().all(|v|*v==128));
    }
}
fn endpoint_bytes()->[u8;236] {
    let mut b=[0;236];b[0..4].copy_from_slice(&1u32.to_le_bytes());
    let name=format!(r"\\.\pipe\TingXian.Video.{}","a".repeat(32));
    for (i,c) in name.encode_utf16().enumerate(){b[4+i*2..6+i*2].copy_from_slice(&c.to_le_bytes());}
    b[196..200].copy_from_slice(&1u32.to_le_bytes());b[204..236].fill(1);b
}
#[test]
fn stream_latest_queue_is_bounded_and_cancel_revokes_unclaimed_endpoint() {
    let (job,feed)=crate::stream::pair(EyeProvider::Synthetic);
    assert!(feed.submit(frame(1,1)).is_err());job.shared.admit(1);
    assert!(job.shared.publish(NativeOutputEndpoint::parse(&endpoint_bytes(),1).unwrap()));
    assert!(!feed.submit(frame(1,1)).unwrap());assert!(feed.submit(frame(2,2)).unwrap());assert!(feed.submit(frame(2,3)).is_err());
    assert!(feed.submit(StreamFrame{id:3,captured_ms:3,bgr:vec![0;1]}).is_err());
    assert!(feed.submit(frame(3,1)).is_err());assert_eq!(feed.status().queued,1);assert_eq!(feed.status().replaced,1);
    match job.shared.next(){crate::stream::Next::Frame(f)=>assert_eq!(f.id,2),_=>panic!("latest frame missing")}
    job.shared.settled(true);assert_eq!(feed.status().written,1);
    assert!(feed.finish());assert!(!feed.finish());assert!(feed.submit(frame(4,4)).is_err());
    job.shared.close();assert!(matches!(feed.take_endpoint(Duration::ZERO),Err(Reason::Abandoned)));assert_eq!(feed.status().queued,0);
}
#[test]
fn stream_three_process_restarts_deliver_corrected_pixels_original_time_and_safe_snapshot() {
    let state=Arc::new(Controller::default());
    for _ in 0..3 {
        let (run,feed)=start(&state,fixture("eye-stream.exe"));let running=thread::spawn(move||run.execute());
        let endpoint=feed.take_endpoint(Duration::from_secs(3)).expect("native readiness");assert_eq!(state.snapshot().phase,Phase::Ready);
        assert!(matches!(feed.take_endpoint(Duration::ZERO),Err(Reason::EyeOutput)));
        let snapshot=serde_json::to_string(&state.snapshot()).unwrap();assert!(!snapshot.contains("TingXian.Video")&&!snapshot.contains("capability"));
        let client=Client::connect(endpoint);let captured=tick();feed.submit(frame(17,captured)).unwrap();client.corrected(17,captured);
        assert!(feed.finish());let done=running.join().unwrap();assert_eq!(done.phase,Phase::Completed,"{done:?}");
        let report=done.stream.unwrap();assert_eq!((report.received,report.corrected,report.submitted),(1,1,1));
        assert!(!report.camera_started&&!report.audio_started&&!report.virtual_device_started);assert!(state.wait_idle(Duration::ZERO));
        assert!(feed.status().closed);assert!(feed.submit(frame(18,tick())).is_err());state.request_stop();assert!(state.snapshot().stream.is_none());
        let status=feed.status();assert_eq!((status.accepted,status.written,status.in_flight),(1,1,0));
    }
}
#[test]
fn stream_stop_before_dispatch_and_dropped_dispatch_never_start_and_stale_ticket_fails() {
    let state=Arc::new(Controller::default());let worker=fixture("eye-stream.exe");let folder=worker.path.parent().unwrap().to_owned();
    let (run,feed)=start(&state,worker);state.request_stop();assert!(feed.status().closed);assert_eq!(run.execute().phase,Phase::Off);assert!(!folder.join("launched").exists());
    let stale=state.snapshot().generation;state.request_stop();assert!(state.begin_stream_if_current(None,EyeProvider::Synthetic,stale).is_err());
    let (run,feed)=start(&state,fixture("eye-stream.exe"));drop(run);assert!(feed.status().closed);assert_eq!(state.snapshot().reason,Some(Reason::Abandoned));
}
#[test]
fn stream_cancel_during_prepare_and_blocked_input_retires_writer_before_slot_is_idle() {
    for name in ["eye-wait.exe","eye-slow.exe"] {
        let state=Arc::new(Controller::default());let worker=fixture(name);let folder=worker.path.parent().unwrap().to_owned();
        let (run,feed)=start(&state,worker);let running=thread::spawn(move||run.execute());
        if name=="eye-slow.exe" {
            let _endpoint=feed.take_endpoint(Duration::from_secs(3)).unwrap();feed.submit(frame(1,tick())).unwrap();reached(&folder,"processing");
            feed.submit(frame(2,tick())).unwrap();eventually(||feed.status().queued==0); // writer blocks while provider waits
            feed.submit(frame(3,tick())).unwrap();assert_eq!(feed.status().queued,1);
        } else {reached(&folder,"preparing");}
        let started=Instant::now();state.request_stop();assert!(feed.status().closed);assert_eq!(feed.status().queued,0);
        assert!(state.wait_idle(Duration::from_secs(3)));let done=running.join().unwrap();assert_eq!(done.phase,Phase::Off,"{done:?}");
        assert!(started.elapsed()<Duration::from_secs(2));assert!(done.stream.is_none());
        let status=feed.status();assert_eq!(status.in_flight,0);assert_eq!(status.accepted,status.written+status.replaced+status.discarded);
    }
}
#[test]
fn stream_provider_crash_fault_cleanup_and_timeout_remain_distinct() {
    for (name,reason) in [("eye-crash.exe",Reason::WorkerExit),("eye-fail.exe",Reason::EyeProcessing),("eye-cleanup.exe",Reason::ProviderCleanup)] {
        let state=Arc::new(Controller::default());let (run,feed)=start(&state,fixture(name));let running=thread::spawn(move||run.execute());
        let _endpoint=feed.take_endpoint(Duration::from_secs(3)).unwrap();
        if name=="eye-cleanup.exe" {feed.finish();} else {feed.submit(frame(1,tick())).unwrap();}
        let done=running.join().unwrap();assert_eq!(done.phase,Phase::Fault,"{done:?}");assert_eq!(done.reason,Some(reason));assert!(state.wait_idle(Duration::ZERO));assert!(feed.status().closed);
        if let Some(report)=done.stream {assert_eq!(report.submitted,0);assert_eq!(report.corrected,0);}
    }
    let state=Arc::new(Controller::default());let (run,feed)=start(&state,fixture("eye-hang.exe"));
    let done=run.execute_with_timeout(Duration::from_millis(200));assert_eq!(done.reason,Some(Reason::Timeout));assert!(feed.status().closed);assert!(state.wait_idle(Duration::ZERO));
}
#[test]
fn stream_report_rejects_wrong_identity_unbalanced_counts_and_media_claims() {
    let good=serde_json::json!({"protocol":3,"generation":5,"provider":"synthetic","width":1280,"height":720,
        "received":1,"rejected":0,"provider_dropped":0,"corrected":1,"output_dropped":0,"submitted":1,"outcome":"completed",
        "error":0,"cleanup_uncertain":false,"camera_started":false,"audio_started":false,"virtual_device_started":false});
    assert!(StreamReport::parse(&serde_json::to_vec(&good).unwrap(),EyeProvider::Synthetic,5).is_ok());
    for (key,value) in [("generation",serde_json::json!(6)),("submitted",serde_json::json!(2)),("camera_started",serde_json::json!(true)),("provider",serde_json::json!("ar087")),("cleanup_uncertain",serde_json::json!(true)),("path",serde_json::json!("private"))] {
        let mut bad=good.clone();bad[key]=value;assert_eq!(StreamReport::parse(&serde_json::to_vec(&bad).unwrap(),EyeProvider::Synthetic,5),Err(Reason::Protocol));
    }
    let endpoint=endpoint_bytes();assert!(NativeOutputEndpoint::parse(&endpoint,1).is_ok());assert!(NativeOutputEndpoint::parse(&endpoint,2).is_err());
    let mut invalid=endpoint;invalid[204..].fill(0);assert!(NativeOutputEndpoint::parse(&invalid,1).is_err());assert!(NativeOutputEndpoint::parse(&endpoint[..235],1).is_err());
}

struct StopOnDrop(Arc<Controller>);
impl Drop for StopOnDrop {fn drop(&mut self){self.0.request_stop();self.0.wait_idle(Duration::from_secs(3));}}
fn await_preview(preview:&mut NativePreview) {
    let deadline=Instant::now()+Duration::from_secs(3);
    while !preview.poll(Duration::from_millis(20)).expect("native preview packet") {
        assert!(Instant::now()<deadline,"no corrected preview frame");
    }
}
#[test]
fn preview_real_worker_three_restarts_keep_pixels_time_and_host_generation() {
    let state=Arc::new(Controller::default());let _stop=StopOnDrop(state.clone());
    for _ in 0..3 {
        let (run,feed)=start(&state,fixture("eye-stream.exe"));let generation=state.snapshot().generation;
        let running=thread::spawn(move||run.execute());let mut preview=feed.open_preview(Duration::from_secs(3)).unwrap();
        assert!(matches!(feed.take_endpoint(Duration::ZERO),Err(Reason::EyeOutput)));
        preview.with_current(|f|assert!(f.is_none(),"no fabricated initial preview"));
        let captured=tick();feed.submit(frame(17,captured)).unwrap();await_preview(&mut preview);
        preview.with_current(|value|{
            let f=value.unwrap();assert_eq!((f.generation,f.id,f.captured_ms),(generation,17,captured));
            assert!(f.nv12[..1280*720].iter().all(|v|*v==188));assert!(f.nv12[1280*720..].iter().all(|v|*v==128));
        });
        preview.with_current(|f|assert_eq!(f.unwrap().id,17));
        let report=preview.status();assert_eq!((report.received,report.presented,report.repeated),(1,1,1));
        let json=serde_json::to_string(&report).unwrap();assert!(!json.contains("TingXian.Video")&&!json.contains("nv12")&&!json.contains("capability"));
        state.request_stop();preview.with_current(|f|assert!(f.is_none(),"late frame after controller stop"));
        assert_eq!(preview.status().phase,PreviewPhase::Stopped);assert!(state.wait_idle(Duration::from_secs(3)));
        assert_eq!(running.join().unwrap().phase,Phase::Off);drop(preview);
    }
}
#[test]
fn preview_expiry_and_replacement_do_not_relabel_capture_or_count_repeats_as_unique() {
    let state=Arc::new(Controller::default());let _stop=StopOnDrop(state.clone());
    let (run,feed)=start(&state,fixture("eye-stream.exe"));let running=thread::spawn(move||run.execute());
    let mut preview=feed.open_preview(Duration::from_secs(3)).unwrap();
    feed.submit(frame(1,tick())).unwrap();await_preview(&mut preview);
    let captured=tick();feed.submit(frame(2,captured)).unwrap();await_preview(&mut preview);
    preview.with_current(|f|{let f=f.unwrap();assert_eq!((f.id,f.captured_ms),(2,captured));});
    assert_eq!(preview.status().replaced,1);
    while tick()-captured<=250{thread::sleep(Duration::from_millis(2));}
    preview.with_current(|f|assert!(f.is_none(),"expired face must become neutral even without another packet"));
    assert_eq!((preview.status().expired,preview.status().presented),(1,1));
    feed.submit(frame(3,tick())).unwrap();await_preview(&mut preview);
    state.request_stop();preview.with_current(|f|assert!(f.is_none()));
    assert_eq!((preview.status().replaced,preview.status().discarded),(1,1),"stop discard is not a replacement or successful presentation");
    assert_eq!(running.join().unwrap().phase,Phase::Off);
}
#[test]
fn preview_native_handoff_serializes_stop_and_drop_disconnects_the_owned_worker() {
    let state=Arc::new(Controller::default());let _stop=StopOnDrop(state.clone());
    let (run,feed)=start(&state,fixture("eye-stream.exe"));let running=thread::spawn(move||run.execute());
    let mut preview=feed.open_preview(Duration::from_secs(3)).unwrap();
    feed.submit(frame(1,tick())).unwrap();await_preview(&mut preview);
    let (entered,entry)=std::sync::mpsc::channel();let (release,wait_release)=std::sync::mpsc::channel();
    let presenting=thread::spawn(move||{preview.with_current(|f|{assert!(f.is_some());entered.send(()).unwrap();wait_release.recv().unwrap();});preview});
    entry.recv_timeout(Duration::from_secs(2)).unwrap();
    let stopping_state=state.clone();let (stopped,stop_receipt)=std::sync::mpsc::channel();
    let stopping=thread::spawn(move||{stopping_state.request_stop();stopped.send(()).unwrap();});
    assert!(stop_receipt.recv_timeout(Duration::from_millis(30)).is_err(),"stop returned before in-flight handoff ended");
    release.send(()).unwrap();let mut preview=presenting.join().unwrap();stopping.join().unwrap();
    preview.with_current(|f|assert!(f.is_none()));assert_eq!(running.join().unwrap().phase,Phase::Off);
    let (run,feed)=start(&state,fixture("eye-stream.exe"));let running=thread::spawn(move||run.execute());
    let mut preview=feed.open_preview(Duration::from_secs(3)).unwrap();feed.submit(frame(1,tick())).unwrap();await_preview(&mut preview);
    drop(preview);feed.submit(frame(2,tick())).unwrap();
    let done=running.join().unwrap();assert_eq!(done.phase,Phase::Fault);assert_eq!(done.reason,Some(Reason::EyeProcessing));
    assert!(state.wait_idle(Duration::ZERO));
}

fn output_pin()->PinnedOutputLibrary {
    let path=PathBuf::from(std::env::var_os("TINGXIAN_VIDEO_TEST_BIN").unwrap()).join("TingXianVideoOutput.dll");
    PinnedOutputLibrary{sha256:format!("{:x}",Sha256::digest(fs::read(&path).expect("built own output library"))),path}
}
fn prime_preview(state:&Arc<Controller>)->(StreamFeed,NativePreview,thread::JoinHandle<Snapshot>) {
    let (run,feed)=start(state,fixture("eye-stream.exe"));let running=thread::spawn(move||run.execute());
    let mut preview=feed.open_preview(Duration::from_secs(3)).unwrap();
    feed.submit(frame(1,tick())).unwrap();await_preview(&mut preview);
    (feed,preview,running)
}
#[test]
fn fanout_output_requires_explicit_intent_and_three_restarts_leave_preview_alive() {
    let state=Arc::new(Controller::default());let _stop=StopOnDrop(state.clone());
    let (feed,mut preview,running)=prime_preview(&state);assert!(preview.output_status().unwrap().status.is_none());
    for index in 0..3 {
        let intent=preview.output_status().unwrap().intent;
        let endpoint=preview.enable_output(output_pin(),intent).unwrap();
        let client=Client::connect(endpoint);
        assert!(matches!(preview.enable_output(output_pin(),intent),Err(Reason::EyeOutput)));
        let id=2+index*2;let captured=tick();feed.submit(frame(id,captured)).unwrap();await_preview(&mut preview);
        preview.with_current(|f|assert_eq!((f.unwrap().id,f.unwrap().captured_ms),(id,captured)));
        client.corrected(id,captured);
        eventually(||preview.output_status().unwrap().status.unwrap().submitted==1);
        let stats=preview.disable_output().unwrap();assert_eq!(stats.status.unwrap().phase,RelayPhase::Off);
        assert_eq!((stats.status.unwrap().accepted,stats.status.unwrap().submitted),(1,1));
        assert!(matches!(preview.enable_output(output_pin(),intent),Err(Reason::EyeOutput)),"old output enable ticket revived a disabled lane");
        assert_eq!(state.snapshot().phase,Phase::Ready);
        feed.submit(frame(id+1,tick())).unwrap();await_preview(&mut preview);preview.with_current(|f|assert_eq!(f.unwrap().id,id+1));
    }
    state.request_stop();assert_eq!(running.join().unwrap().phase,Phase::Off);
}
#[test]
fn fanout_slow_receiver_is_bounded_and_disable_retires_pending_writes() {
    let state=Arc::new(Controller::default());let _stop=StopOnDrop(state.clone());let (feed,mut preview,running)=prime_preview(&state);
    let client=Client::connect(preview.enable_output(output_pin(),0).unwrap());
    feed.submit(frame(2,tick())).unwrap();await_preview(&mut preview);let header=client.read(40);assert_eq!(u16::from_le_bytes(header[6..8].try_into().unwrap()),4);
    for id in 3..=4 {feed.submit(frame(id,tick())).unwrap();await_preview(&mut preview);}
    let status=preview.output_status().unwrap().status.unwrap();assert!(status.busy);assert_eq!(status.queued,1);assert_eq!(status.accepted,3);
    let started=Instant::now();let stopped=preview.disable_output().unwrap().status.unwrap();
    assert!(started.elapsed()<Duration::from_secs(1));assert!(!stopped.busy);assert_eq!(stopped.queued,0);
    assert_eq!(stopped.accepted,stopped.submitted+stopped.dropped);
    feed.submit(frame(5,tick())).unwrap();await_preview(&mut preview);preview.with_current(|f|assert_eq!(f.unwrap().id,5));
    state.request_stop();assert_eq!(running.join().unwrap().phase,Phase::Off);
}
#[test]
fn fanout_controller_stop_revokes_output_without_a_preview_poll_or_drop() {
    let state=Arc::new(Controller::default());let _stop=StopOnDrop(state.clone());let (feed,mut preview,running)=prime_preview(&state);
    let client=Client::connect(preview.enable_output(output_pin(),0).unwrap());
    let captured=tick();feed.submit(frame(2,captured)).unwrap();await_preview(&mut preview);client.corrected(2,captured);
    state.request_stop();
    eventually(||unsafe{PeekNamedPipe(client.0.as_raw_handle(),null_mut(),0,null_mut(),null_mut(),null_mut())}==0);
    preview.with_current(|f|assert!(f.is_none()));
    assert_eq!(preview.output_status().unwrap().status.unwrap().phase,RelayPhase::Off);
    let intent=preview.output_status().unwrap().intent;
    assert!(matches!(preview.enable_output(output_pin(),intent),Err(Reason::EyeOutput)));
    assert_eq!(running.join().unwrap().phase,Phase::Off);
}
#[test]
fn fanout_library_pin_is_checked_before_load_and_held_until_output_is_retired() {
    let state=Arc::new(Controller::default());let _stop=StopOnDrop(state.clone());let (feed,mut preview,running)=prime_preview(&state);
    let pin=output_pin();let mut wrong=output_pin();wrong.sha256="0".repeat(64);
    assert!(matches!(preview.enable_output(wrong,0),Err(Reason::Integrity)));
    assert!(preview.output_status().unwrap().status.is_none());assert_eq!(state.snapshot().phase,Phase::Ready);
    let _endpoint=preview.enable_output(output_pin(),1).unwrap();
    assert!(fs::OpenOptions::new().write(true).open(&pin.path).is_err(),"loaded library must remain write-locked");
    preview.disable_output().unwrap();
    let reopened=fs::OpenOptions::new().write(true).open(&pin.path).expect("output retirement releases own library pin");drop(reopened);
    feed.submit(frame(2,tick())).unwrap();await_preview(&mut preview);
    state.request_stop();assert_eq!(running.join().unwrap().phase,Phase::Off);
}
#[test]
fn fanout_bad_client_capability_faults_without_replaying_preview_or_input() {
    let state=Arc::new(Controller::default());let _stop=StopOnDrop(state.clone());let (feed,mut preview,running)=prime_preview(&state);
    let endpoint=preview.enable_output(output_pin(),0).unwrap();let b=endpoint.activation_blob();
    let name:Vec<u16>=b[4..196].chunks_exact(2).map(|x|u16::from_le_bytes([x[0],x[1]])).collect();
    let raw=unsafe{CreateFileW(name.as_ptr(),GENERIC_READ|GENERIC_WRITE,0,null(),OPEN_EXISTING,0,null_mut())};assert_ne!(raw,INVALID_HANDLE_VALUE);
    let mut client=unsafe{fs::File::from_raw_handle(raw)};
    let mut hello=[0u8;72];hello[..4].copy_from_slice(&0x31565854u32.to_le_bytes());hello[4..6].copy_from_slice(&1u16.to_le_bytes());
    hello[6..8].copy_from_slice(&1u16.to_le_bytes());hello[8..12].copy_from_slice(&32u32.to_le_bytes());client.write_all(&hello).unwrap();
    eventually(||preview.output_status().unwrap().status.unwrap().phase==RelayPhase::Fault);
    assert_eq!(preview.poll(Duration::ZERO),Err(Reason::EyeOutput));preview.with_current(|f|assert!(f.is_none()));
    let done=running.join().unwrap();assert_eq!(done.phase,Phase::Fault);assert!(feed.status().closed);
}

#[test]
fn fanout_supervised_worker_through_preview_and_own_dll_reaches_real_mf_samples() {
    use std::{os::windows::process::CommandExt,process::{Child,ChildStdin,ChildStdout,Command,Stdio}};
    struct Reader {child:Child,input:ChildStdin,output:ChildStdout}
    impl Drop for Reader {fn drop(&mut self){if self.child.try_wait().ok().flatten().is_none(){let _=self.child.kill();let _=self.child.wait();}}}
    impl Reader {
        fn received(&mut self,want:u8) {
            let raw=self.output.as_raw_handle();let deadline=Instant::now()+Duration::from_secs(3);
            loop {
                assert!(Instant::now()<deadline,"MF reader acknowledgment deadline");let mut available=0;
                assert_ne!(unsafe{PeekNamedPipe(raw,null_mut(),0,null_mut(),&mut available,null_mut())},0,"MF reader exited before acknowledgment");
                if available>0{let mut byte=0;let mut read=0;assert_ne!(unsafe{ReadFile(raw,&mut byte,1,&mut read,null_mut())},0);assert_eq!((read,byte),(1,want));return;}
                thread::sleep(Duration::from_millis(2));
            }
        }
        fn check(&mut self,kind:u8){self.input.write_all(&[kind]).unwrap();self.received(kind);}
    }
    let state=Arc::new(Controller::default());let _stop=StopOnDrop(state.clone());let (feed,mut preview,running)=prime_preview(&state);
    let endpoint=preview.enable_output(output_pin(),0).unwrap();
    let bin=PathBuf::from(std::env::var_os("TINGXIAN_VIDEO_TEST_BIN").unwrap());
    let mut command=Command::new(bin.join("tingxian-video-ipc-test.exe"));
    command.arg("--relay-reader").arg(bin.join("TingXianVideoSource.dll")).env_clear()
        .env("SystemRoot",std::env::var_os("SystemRoot").unwrap()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).creation_flags(0x08000000);
    let mut child=command.spawn().expect("own MF fixture launch");
    let input=child.stdin.take().unwrap();let output=child.stdout.take().unwrap();let mut reader=Reader{child,input,output};
    reader.input.write_all(endpoint.activation_blob()).unwrap();reader.received(b'R');
    for id in 2..=3 {
        let captured=tick();feed.submit(frame(id,captured)).unwrap();await_preview(&mut preview);
        preview.with_current(|f|assert_eq!((f.unwrap().id,f.unwrap().captured_ms),(id,captured)));
        reader.check(b'P');
    }
    state.request_stop();reader.check(b'N'); // no poll/drop needed to neutralize MF
    preview.with_current(|f|assert!(f.is_none()));reader.input.write_all(b"Q").unwrap();
    eventually(||reader.child.try_wait().unwrap().is_some());assert!(reader.child.wait().unwrap().success());
    assert_eq!(running.join().unwrap().phase,Phase::Off);
}
