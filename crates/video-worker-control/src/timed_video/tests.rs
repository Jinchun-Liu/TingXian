use super::*;
use crate::av_timeline::{
    CorrectedFrame, DeviceClock, PacketFlags, PcmPacket, RenderStatus, SincRenderer, Source, Time,
    Timeline, RATE,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Storage::FileSystem::ReadFile,
    System::{
        Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
        Pipes::PeekNamedPipe,
        Threading::WaitForSingleObject,
    },
};
pub(crate) fn pin() -> PinnedTimedLibrary {
    let path = PathBuf::from(
        std::env::var_os("TINGXIAN_VIDEO_TEST_BIN").expect("actual native build directory"),
    )
    .join("TingXianVideoOutput.dll");
    PinnedTimedLibrary {
        sha256: format!("{:x}", Sha256::digest(fs::read(&path).unwrap())),
        path,
    }
}
pub(crate) fn now() -> Time {
    let (mut ticks, mut frequency) = (0, 0);
    assert_ne!(unsafe { QueryPerformanceCounter(&mut ticks) }, 0);
    assert_ne!(unsafe { QueryPerformanceFrequency(&mut frequency) }, 0);
    Time::from_qpc_ticks(ticks as u64, frequency as u64).unwrap()
}
fn wait(video: &NativeTimedVideo) {
    let event = video.event().unwrap();
    let result = unsafe { WaitForSingleObject(event.as_raw_handle(), 100) };
    assert!(result == WAIT_OBJECT_0 || result == WAIT_TIMEOUT);
}
pub(crate) fn ready(video: &NativeTimedVideo) {
    let until = Instant::now() + Duration::from_secs(3);
    while video.status().unwrap().phase != Phase::Ready {
        assert!(Instant::now() < until, "sender not ready");
        wait(video);
    }
}
fn receipt(video: &mut NativeTimedVideo, flight: &mut Flight) -> Admission {
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(value) = video.take(flight).unwrap() {
            return value;
        }
        assert!(Instant::now() < until, "receipt timeout");
        wait(video);
    }
}
pub(crate) struct Reader {
    pub(crate) child: Child,
    input: ChildStdin,
    output: ChildStdout,
}
impl Drop for Reader {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
impl Reader {
    pub(crate) fn connect(endpoint: TimedEndpoint) -> Self {
        let bin = PathBuf::from(std::env::var_os("TINGXIAN_VIDEO_TEST_BIN").unwrap());
        let mut command = Command::new(bin.join("tingxian-video-ipc-test.exe"));
        command
            .arg("--timed-relay-reader")
            .arg(bin.join("TingXianVideoSource.dll"))
            .env_clear()
            .env("SystemRoot", std::env::var_os("SystemRoot").unwrap())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .creation_flags(0x08000000);
        let mut child = command.spawn().expect("own hidden MF reader");
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let mut result = Self {
            child,
            input,
            output,
        };
        result.input.write_all(endpoint.activation_blob()).unwrap();
        result.received(b'R');
        result
    }
    fn received(&mut self, want: u8) {
        let raw = self.output.as_raw_handle();
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            assert!(Instant::now() < until, "MF reader deadline");
            let mut available = 0;
            assert_ne!(
                unsafe {
                    PeekNamedPipe(raw, null_mut(), 0, null_mut(), &mut available, null_mut())
                },
                0,
                "MF reader failed"
            );
            if available > 0 {
                let mut byte = 0;
                let mut count = 0;
                assert_ne!(
                    unsafe { ReadFile(raw, &mut byte, 1, &mut count, null_mut()) },
                    0
                );
                assert_eq!((count, byte), (1, want));
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
    pub(crate) fn sample(&mut self, id: u64, capture: u64, presentation: u64) {
        let mut bytes = vec![b'T'];
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(&capture.to_le_bytes());
        bytes.extend_from_slice(&presentation.to_le_bytes());
        bytes.extend_from_slice(&u32::from(id == 0).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        self.input.write_all(&bytes).unwrap();
        self.received(b'T');
    }
    pub(crate) fn neutral_after_stop(&mut self) {
        self.input.write_all(b"N").unwrap();
        self.received(b'N');
    }
}
fn simple_window<'a>(
    generation: u64,
    frame: Option<&'a CorrectedFrame>,
    presentation: Time,
    audio: &'a [f32],
) -> RenderedWindow<'a> {
    RenderedWindow {
        generation,
        capture: frame.map_or(presentation, |v| v.capture),
        presentation,
        audio,
        video: frame,
        status: if frame.is_some() {
            RenderStatus::Media
        } else {
            RenderStatus::MissingTimelineInput
        },
        saturated_samples: 0,
    }
}
#[test]
fn timed_bridge_requires_pin_and_explicit_intent_and_releases_module() {
    let mut bad = pin();
    bad.sha256 = "00".repeat(32);
    assert!(matches!(
        NativeTimedVideo::load(bad),
        Err(Error::Load(Reason::Integrity))
    ));
    let path = pin().path;
    let mut video = NativeTimedVideo::load(pin()).unwrap();
    assert_eq!(video.status().unwrap().phase, Phase::Idle);
    assert!(matches!(video.open(1, false), Err(Error::Permission)));
    let endpoint = video.open(1, true).unwrap();
    assert_eq!(
        u32::from_le_bytes(endpoint.activation_blob()[..4].try_into().unwrap()),
        2
    );
    assert!(
        NativeOutputEndpoint::parse(endpoint.activation_blob(), unsafe { GetCurrentProcessId() })
            .is_err(),
        "legacy parser silently accepted v2"
    );
    assert!(video.reserve().unwrap().is_none());
    assert!(fs::OpenOptions::new().write(true).open(&path).is_err());
    let status = video.stop().unwrap();
    assert!(status.local_clear && !status.external_drain_verified);
    assert!(matches!(video.open(2, true), Err(Error::Off)));
    drop(video);
    drop(
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("pin released after sender retirement"),
    );
}
#[test]
fn timed_bridge_timeline_window_reaches_real_mf_with_original_capture_and_plan() {
    let mut video = NativeTimedVideo::load(pin()).unwrap();
    let mut reader = Reader::connect(video.open(1, true).unwrap());
    ready(&video);
    let renderer = SincRenderer::default();
    let timeline = Timeline::default();
    let generation = timeline.begin(0, 7).unwrap();
    assert_eq!(generation, 1);
    let observed = now();
    let base = observed.qpc_100ns() - 1_000_000;
    for index in 0..10 {
        timeline
            .push_audio(
                generation,
                PcmPacket {
                    device: 7,
                    source: Source::SelfMicrophone,
                    first_sample: index * 480,
                    capture: Time::from_qpc_100ns(base + index * 100_000),
                    rate: RATE,
                    channels: 1,
                    flags: PacketFlags::default(),
                    samples: vec![0.2; 480],
                },
                observed,
            )
            .unwrap();
    }
    let capture = base + 800_000;
    let presentation = base + 1_200_000;
    let mut nv12 = vec![96; NV12_BYTES];
    nv12[..1280 * 720].fill(160);
    timeline
        .push_video(
            generation,
            CorrectedFrame {
                id: 41,
                capture: Time::from_qpc_100ns(capture),
                nv12: Arc::from(nv12),
            },
            observed,
        )
        .unwrap();
    let mut clock = DeviceClock::new(generation, 48000).unwrap();
    clock.observe(0, Time::from_qpc_100ns(base)).unwrap();
    clock.observe(4800, observed).unwrap();
    let interval = clock.interval_from_frame(5760, 480, observed).unwrap();
    let ticket = video.reserve().unwrap().unwrap();
    assert!(video.reserve().unwrap().is_none());
    let mut staged = None;
    let mut pcm = Vec::with_capacity(480);
    timeline
        .with_rendered_window(generation, interval, observed, &renderer, |window| {
            assert_eq!(window.status, RenderStatus::Media);
            assert_eq!(window.presentation.qpc_100ns(), presentation);
            pcm.extend_from_slice(window.audio);
            staged = Some(video.stage(ticket, &window).unwrap());
            true
        })
        .unwrap();
    assert_eq!(pcm.len(), 480);
    assert!(pcm.iter().all(|v| (v - 0.2).abs() < 0.001));
    let mut flight = staged.unwrap();
    assert_eq!(receipt(&mut video, &mut flight), Admission::Admitted);
    assert!(
        matches!(video.take(&mut flight), Err(Error::Owner)),
        "receipt consumed twice"
    );
    reader.sample(41, capture, presentation);
    video.stop().unwrap();
    reader.neutral_after_stop();
    // PCM was inspected in memory, not committed to a real audio device.
}
#[test]
fn timed_bridge_missing_timeline_input_delivers_scheduled_neutral() {
    let mut video = NativeTimedVideo::load(pin()).unwrap();
    let mut reader = Reader::connect(video.open(1, true).unwrap());
    ready(&video);
    let timeline = Timeline::default();
    let generation = timeline.begin(0, 7).unwrap();
    let renderer = SincRenderer::default();
    let observed = now();
    let mut clock = DeviceClock::new(generation, 48000).unwrap();
    clock
        .observe(0, Time::from_qpc_100ns(observed.qpc_100ns() - 100000))
        .unwrap();
    clock.observe(480, observed).unwrap();
    let interval = clock.interval_from_frame(1440, 480, observed).unwrap();
    let ticket = video.reserve().unwrap().unwrap();
    let mut staged = None;
    let mut presentation = 0;
    timeline
        .with_rendered_window(generation, interval, observed, &renderer, |window| {
            assert_eq!(window.status, RenderStatus::MissingTimelineInput);
            assert!(window.audio.iter().all(|v| *v == 0.0) && window.video.is_none());
            presentation = window.presentation.qpc_100ns();
            staged = Some(video.stage(ticket, &window).unwrap());
            true
        })
        .unwrap();
    assert_eq!(
        receipt(&mut video, &mut staged.unwrap()),
        Admission::Admitted
    );
    reader.sample(0, 0, presentation);
    video.stop().unwrap();
    reader.neutral_after_stop();
}
#[test]
fn timed_bridge_rejects_foreign_instance_and_keeps_unknown_io_distinct() {
    let mut first = NativeTimedVideo::load(pin()).unwrap();
    let _one = Reader::connect(first.open(1, true).unwrap());
    ready(&first);
    let foreign = first.reserve().unwrap().unwrap();
    first.stop().unwrap();
    drop(first);
    let mut video = NativeTimedVideo::load(pin()).unwrap();
    let reader = Reader::connect(video.open(1, true).unwrap());
    ready(&video);
    let observed = now();
    let window = simple_window(
        1,
        None,
        Time::from_qpc_100ns(observed.qpc_100ns() + 800000),
        &[0.0; 480],
    );
    assert!(matches!(video.stage(foreign, &window), Err(Error::Owner)));
    let own = video.reserve().unwrap().unwrap();
    drop(reader);
    let mut flight = video.stage(own, &window).unwrap();
    assert!(matches!(
        receipt(&mut video, &mut flight),
        Admission::Unknown(_)
    ));
    let stopped = video.stop().unwrap();
    assert_eq!(stopped.phase, Phase::Fault);
    assert!(stopped.local_clear && !stopped.external_drain_verified);
}
#[test]
fn timed_bridge_cancel_does_not_turn_a_late_admission_into_success() {
    let mut video = NativeTimedVideo::load(pin()).unwrap();
    let mut reader = Reader::connect(video.open(1, true).unwrap());
    ready(&video);
    let ticket = video.reserve().unwrap().unwrap();
    let time = now();
    let window = simple_window(
        1,
        None,
        Time::from_qpc_100ns(time.qpc_100ns() + 800000),
        &[0.0; 480],
    );
    let mut flight = video.stage(ticket, &window).unwrap();
    video.cancel();
    assert_eq!(receipt(&mut video, &mut flight), Admission::Cancelled);
    let stopped = video.stop().unwrap();
    assert!(!stopped.pending && stopped.local_clear && !stopped.external_drain_verified);
    reader.neutral_after_stop();
}
