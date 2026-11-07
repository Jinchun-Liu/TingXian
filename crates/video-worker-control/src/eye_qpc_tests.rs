use super::*;
use crate::{
    av_timeline::{CorrectedFrame, Timeline},
    stream::StreamJob,
    Controller, EyeProvider, Phase, STREAM_BGR_BYTES,
};
use std::{
    fs::File,
    io::{Read, Write},
    os::windows::io::FromRawHandle,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread::JoinHandle,
};
use windows_sys::Win32::{
    Foundation::*,
    Storage::FileSystem::{PIPE_ACCESS_DUPLEX, FILE_FLAG_FIRST_PIPE_INSTANCE},
    System::{Pipes::*, RemoteDesktop::ProcessIdToSessionId, Threading::GetCurrentProcessId},
};
static NEXT: AtomicU64 = AtomicU64::new(1);
fn input(id: u64, capture: Time) -> QpcStreamFrame {
    QpcStreamFrame {
        id,
        capture,
        bgr: vec![31; STREAM_BGR_BYTES],
    }
}
fn until(mut predicate: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(Instant::now() < end, "bounded QPC condition");
        thread::sleep(Duration::from_millis(2));
    }
}
fn start(state: &Arc<Controller>, name: &str) -> (crate::Run, QpcStreamFeed) {
    state
        .begin_qpc_stream_if_current(
            Some(crate::tests::fixture(name)),
            EyeProvider::Synthetic,
            state.snapshot().generation,
        )
        .unwrap()
        .unwrap()
}

#[test]
fn qpc_real_worker_three_restarts_preserve_capture_and_enter_the_bounded_timeline() {
    let state = Arc::new(Controller::default());
    for _ in 0..3 {
        let (run, feed) = start(&state, "eye-delay.exe");
        let running = thread::spawn(move || run.execute());
        let mut reader = feed.open_corrections(Duration::from_secs(3)).unwrap();
        assert!(matches!(
            feed.open_corrections(Duration::ZERO),
            Err(Reason::EyeOutput)
        ));
        let capture = now().unwrap();
        feed.submit(input(71, capture)).unwrap();
        until(|| reader.poll(Duration::from_millis(10)).unwrap());
        let timeline = Timeline::default();
        let g = timeline.begin(0, 7).unwrap();
        let controller_generation = state.snapshot().generation;
        reader.with_current(|f| {
            let f = f.expect("current corrected frame");
            assert_eq!(
                (f.id, f.capture, f.generation),
                (71, capture, controller_generation)
            );
            assert!(f.completed.qpc_100ns() >= capture.qpc_100ns() + 300_000);
            assert!(f.nv12[..1280 * 720].iter().all(|x| *x == 188));
            assert!(f.nv12[1280 * 720..].iter().all(|x| *x == 128));
            assert!(timeline
                .push_video(
                    g,
                    CorrectedFrame {
                        id: f.id,
                        capture: f.capture,
                        nv12: Arc::from(f.nv12.clone())
                    },
                    now().unwrap()
                )
                .unwrap());
        });
        assert_eq!(
            (
                timeline.snapshot().video_frames,
                timeline.snapshot().metrics.admitted_video
            ),
            (1, 1)
        );
        // The timeline has its own lifecycle. This check does not pretend that
        // reader shutdown automatically revokes a separately prepared endpoint.
        timeline.stop().unwrap();
        assert!(feed.finish());
        let done = running.join().unwrap();
        assert_eq!(done.phase, Phase::Completed, "{done:?}");
        let report = done.stream.unwrap();
        assert_eq!(
            (report.protocol, report.received, report.submitted),
            (4, 1, 1)
        );
        assert!(feed.status().closed);
        reader.with_current(|f| assert!(f.is_none()));
        assert_eq!(reader.status().phase, PreviewPhase::Stopped);
        let json = serde_json::to_string(&state.snapshot()).unwrap();
        assert!(
            !json.contains("TingXian.Video")
                && !json.contains("capability")
                && !json.contains("nv12")
        );
        state.request_stop();
    }
}
#[test]
fn qpc_real_worker_stop_during_inference_and_provider_failure_never_return_raw_pixels() {
    for mode in ["eye-slow.exe", "eye-fail.exe"] {
        let state = Arc::new(Controller::default());
        let worker = crate::tests::fixture(mode);
        let directory = worker.path.parent().unwrap().to_owned();
        let (run, feed) = state
            .begin_qpc_stream_if_current(Some(worker), EyeProvider::Synthetic, 0)
            .unwrap()
            .unwrap();
        let running = thread::spawn(move || run.execute());
        let mut reader = feed.open_corrections(Duration::from_secs(3)).unwrap();
        feed.submit(input(1, now().unwrap())).unwrap();
        until(|| directory.join("processing").exists());
        if mode == "eye-slow.exe" {
            state.request_stop();
        }
        let done = running.join().unwrap();
        if mode == "eye-slow.exe" {
            assert_eq!(done.phase, Phase::Off);
        } else {
            assert_eq!(done.phase, Phase::Fault);
            assert_eq!(done.reason, Some(Reason::EyeProcessing));
        }
        reader.with_current(|f| assert!(f.is_none()));
        assert!(feed.status().closed);
        assert!(state.wait_idle(Duration::ZERO));
    }
}

fn header(kind: u16, bytes: u32, generation: u64, id: u64) -> Vec<u8> {
    let mut b = vec![0; 40];
    b[..4].copy_from_slice(&0x31565854u32.to_le_bytes());
    b[4..6].copy_from_slice(&3u16.to_le_bytes());
    b[6..8].copy_from_slice(&kind.to_le_bytes());
    b[8..12].copy_from_slice(&bytes.to_le_bytes());
    b[16..24].copy_from_slice(&generation.to_le_bytes());
    b[24..32].copy_from_slice(&id.to_le_bytes());
    b
}
fn frame(id: u64, capture: Time, completed: Time) -> Vec<u8> {
    let mut b = header(8, (NV12_BYTES + 16) as u32, 7, id);
    b.extend(capture.qpc_100ns().to_le_bytes());
    b.extend(completed.qpc_100ns().to_le_bytes());
    b.extend(vec![188; 1280 * 720]);
    b.extend(vec![128; NV12_BYTES - 1280 * 720]);
    b
}
struct Peer {
    _job: StreamJob,
    feed: QpcStreamFeed,
    done: Option<JoinHandle<()>>,
    release: mpsc::Sender<()>,
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.feed.inner.shared.close();
        let _ = self.release.send(());
        if let Some(done) = self.done.take() {
            done.join().unwrap();
        }
    }
}
fn peer(
    payload: Vec<u8>,
    capture: Time,
    bad_ack: bool,
    bad_session: bool,
    split_delay: Option<usize>,
) -> Peer {
    let pid = unsafe { GetCurrentProcessId() };
    let mut session = 0;
    assert_ne!(unsafe { ProcessIdToSessionId(pid, &mut session) }, 0);
    let name = format!(
        r"\\.\pipe\TingXian.Video.{:016x}{:016x}",
        pid,
        NEXT.fetch_add(1, Ordering::SeqCst)
    );
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let raw = unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            2 * 1024 * 1024,
            4096,
            0,
            std::ptr::null(),
        )
    };
    assert_ne!(raw, INVALID_HANDLE_VALUE);
    let owned = crate::preview::handle(raw).unwrap();
    let (release, wait) = mpsc::channel();
    let done = thread::spawn(move || {
        if unsafe { ConnectNamedPipe(owned.0, null_mut()) } == 0
            && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED
        {
            return;
        }
        let raw = owned.0;
        std::mem::forget(owned);
        let mut file = unsafe { File::from_raw_handle(raw) };
        let mut hello = [0; 72];
        if file.read_exact(&mut hello).is_err() {
            return;
        }
        assert_eq!(&hello[..40], header(1, 32, 0, 0));
        assert_eq!(&hello[40..], &[1; 32]);
        let mut ack = header(2, 0, 0, 0);
        if bad_ack {
            ack[4] = 1;
        }
        if file.write_all(&ack).is_err() {
            return;
        }
        if let Some(split) = split_delay {
            if file.write_all(&payload[..split]).is_err() {
                return;
            }
            thread::sleep(Duration::from_millis(300));
            if file.write_all(&payload[split..]).is_err() {
                return;
            }
        } else if file.write_all(&payload).is_err() {
            return;
        }
        let _ = wait.recv_timeout(Duration::from_secs(3));
    });
    let mut bytes = [0u8; 236];
    bytes[..4].copy_from_slice(&3u32.to_le_bytes());
    for (i, c) in wide.iter().enumerate() {
        bytes[4 + i * 2..6 + i * 2].copy_from_slice(&c.to_le_bytes());
    }
    bytes[196..200].copy_from_slice(&pid.to_le_bytes());
    bytes[200..204].copy_from_slice(&(session + u32::from(bad_session)).to_le_bytes());
    bytes[204..].fill(1);
    let (job, inner) = crate::stream::pair_protocol(EyeProvider::Synthetic, Protocol::Qpc);
    job.shared.admit(41);
    assert!(job
        .shared
        .publish(NativeOutputEndpoint::parse_version(&bytes, pid, 3).unwrap()));
    let feed = QpcStreamFeed::new(inner);
    feed.submit(input(1, capture)).unwrap();
    feed.submit(input(2, capture)).unwrap();
    Peer {
        _job: job,
        feed,
        done: Some(done),
        release,
    }
}
#[test]
fn qpc_receiver_checks_peer_session_and_exact_ack_version() {
    for (bad_ack, bad_session) in [(true, false), (false, true)] {
        let p = peer(vec![], now().unwrap(), bad_ack, bad_session, None);
        assert!(matches!(
            p.feed.open_corrections(Duration::from_secs(1)),
            Err(Reason::Protocol)
        ));
        assert!(p.feed.status().closed);
    }
}
#[test]
fn qpc_receiver_malformed_packets_clear_prior_frame_and_revoke_input() {
    for case in 0..12 {
        let capture = now().unwrap();
        let mut bad = frame(2, capture, capture);
        let expected = if case >= 10 {
            Reason::Timeout
        } else {
            Reason::Protocol
        };
        match case {
            0 => bad[4] = 1,
            1 => bad[6] = 4,
            2 => bad[8..12].copy_from_slice(&u32::MAX.to_le_bytes()),
            3 => bad[12] = 1,
            4 => bad[16] = 8,
            5 => bad[24] = 1,
            6 => bad[32] = 1,
            7 => {
                bad[40..48].copy_from_slice(&(capture.qpc_100ns() + 1).to_le_bytes());
                bad[48..56].copy_from_slice(&(capture.qpc_100ns() + 2).to_le_bytes());
            } // mismatched admitted capture
            8 => bad[48..56].copy_from_slice(&(capture.qpc_100ns() - 1).to_le_bytes()),
            9 => bad[48..56].copy_from_slice(&(capture.qpc_100ns() + 100_000_000).to_le_bytes()),
            10 => bad.truncate(1),
            11 => bad.truncate(59),
            _ => unreachable!(),
        }
        let mut payload = header(3, 0, 7, 0);
        payload.extend(frame(1, capture, capture));
        payload.extend(bad);
        let p = peer(payload, capture, false, false, None);
        let mut reader = p.feed.open_corrections(Duration::from_secs(1)).unwrap();
        assert!(!reader.poll(Duration::from_millis(20)).unwrap());
        assert!(reader.poll(Duration::from_millis(20)).unwrap());
        reader.with_current(|f| assert!(f.is_some()));
        assert_eq!(
            reader.poll(Duration::from_millis(20)),
            Err(expected),
            "case {case}"
        );
        reader.with_current(|f| assert!(f.is_none()));
        assert!(p.feed.status().closed);
        assert_eq!(reader.status().reason, Some(expected));
        assert_eq!(reader.status().phase, PreviewPhase::Fault);
    }
}
#[test]
fn qpc_freshness_is_rechecked_after_full_payload_and_before_handoff() {
    for delayed_transfer in [false, true] {
        let capture = now().unwrap();
        let mut payload = header(3, 0, 7, 0);
        payload.extend(frame(1, capture, capture));
        let p = peer(
            payload,
            capture,
            false,
            false,
            delayed_transfer.then_some(40 + 56 + 1),
        );
        let mut reader = p.feed.open_corrections(Duration::from_secs(1)).unwrap();
        reader.poll(Duration::from_millis(20)).unwrap();
        assert_eq!(
            reader.poll(Duration::from_millis(20)).unwrap(),
            !delayed_transfer
        );
        if !delayed_transfer {
            thread::sleep(Duration::from_millis(260));
        }
        reader.with_current(|f| assert!(f.is_none()));
        assert_eq!(reader.status().expired, 1);
    }
}
#[test]
fn qpc_stop_serializes_native_handoff_and_drop_closes_input() {
    let capture = now().unwrap();
    let mut payload = header(3, 0, 7, 0);
    payload.extend(frame(1, capture, capture));
    let p = peer(payload, capture, false, false, None);
    let mut reader = p.feed.open_corrections(Duration::from_secs(1)).unwrap();
    reader.poll(Duration::from_millis(20)).unwrap();
    assert!(reader.poll(Duration::from_millis(20)).unwrap());
    let lease = p.feed.inner.shared.clone();
    let (inside, entered) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let active = thread::spawn(move || {
        reader.with_current(|f| {
            assert!(f.is_some());
            inside.send(()).unwrap();
            resume.recv().unwrap();
        });
        reader
    });
    entered.recv_timeout(Duration::from_secs(1)).unwrap();
    let (stopped, done) = mpsc::channel();
    let stop = thread::spawn(move || {
        lease.close();
        stopped.send(()).unwrap();
    });
    assert!(done.recv_timeout(Duration::from_millis(20)).is_err());
    release.send(()).unwrap();
    done.recv_timeout(Duration::from_secs(1)).unwrap();
    stop.join().unwrap();
    let mut reader = active.join().unwrap();
    reader.with_current(|f| assert!(f.is_none()));
    assert_eq!(reader.status().phase, PreviewPhase::Stopped);
    drop(reader);
    assert!(p.feed.status().closed);
}
#[test]
fn qpc_input_and_provenance_are_bounded_and_never_relabel_unknown_capture() {
    let capture = now().unwrap();
    let p = peer(vec![], capture, false, false, None);
    // Complete the handshake so the test server cannot remain blocked in accept.
    let reader = p.feed.open_corrections(Duration::from_secs(1)).unwrap();
    for id in 3..=67 {
        p.feed.submit(input(id, now().unwrap())).unwrap();
    }
    assert_eq!(p.feed.status().queued, 1);
    assert_eq!(p.feed.status().accepted, 67);
    assert!(!p
        .feed
        .inner
        .shared
        .verify_capture(1, capture.qpc_100ns())
        .unwrap());
    assert_eq!(
        p.feed.inner.shared.verify_capture(68, capture.qpc_100ns()),
        Err(Reason::Protocol)
    );
    assert!(p.feed.submit(input(68, Time::from_qpc_100ns(0))).is_err());
    assert!(p
        .feed
        .submit(input(
            68,
            Time::from_qpc_100ns(now().unwrap().qpc_100ns() + 10_000_000)
        ))
        .is_err());
    drop(reader);
    assert!(p.feed.status().closed);
    assert_eq!(p.feed.status().queued, 0);
}
#[test]
fn qpc_final_report_requires_selected_protocol_and_preserves_media_denials() {
    let report = crate::StreamReport {
        protocol: 4,
        generation: 7,
        provider: EyeProvider::Synthetic,
        width: 1280,
        height: 720,
        received: 1,
        rejected: 0,
        provider_dropped: 0,
        corrected: 1,
        output_dropped: 0,
        submitted: 1,
        outcome: crate::EyeOutcome::Completed,
        error: 0,
        cleanup_uncertain: false,
        camera_started: false,
        audio_started: false,
        virtual_device_started: false,
    };
    let bytes = serde_json::to_vec(&report).unwrap();
    assert!(
        crate::StreamReport::parse_protocol(&bytes, EyeProvider::Synthetic, 7, Protocol::Qpc)
            .is_ok()
    );
    assert!(crate::StreamReport::parse(&bytes, EyeProvider::Synthetic, 7).is_err());
    for (protocol, media) in [(3, false), (4, true)] {
        let mut bad = report.clone();
        bad.protocol = protocol;
        bad.audio_started = media;
        assert!(crate::StreamReport::parse_protocol(
            &serde_json::to_vec(&bad).unwrap(),
            EyeProvider::Synthetic,
            7,
            Protocol::Qpc
        )
        .is_err());
    }
}
