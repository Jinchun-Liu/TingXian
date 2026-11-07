use super::*;
use crate::av_timeline::{
    CorrectedFrame, PacketFlags, PcmPacket, RenderStatus, SincRenderer, Source, Timeline,
    NV12_BYTES,
};
use sha2::{Digest, Sha256};
use std::{fs, mem::size_of};

const BASE: u64 = 100_000_000;
fn time(offset: u64) -> Time {
    Time::from_qpc_100ns(BASE + offset)
}
pub(crate) fn pin(fixture: bool) -> PinnedAudioLibrary {
    let directory = std::env::var_os("TINGXIAN_AUDIO_TEST_BIN")
        .expect("Set TINGXIAN_AUDIO_TEST_BIN to the actual native build directory");
    let path = PathBuf::from(directory).join(if fixture {
        "TingXianAudioFixture.dll"
    } else {
        "TingXianAudioOutput.dll"
    });
    // Tests hash their own CI-built library. Product host must use a build pin,
    // not compute a trusted digest from a runtime/user-supplied DLL.
    let sha256 = format!("{:x}", Sha256::digest(fs::read(&path).unwrap()));
    PinnedAudioLibrary { path, sha256 }
}
pub(crate) fn pair() -> AudioPair {
    AudioPair {
        render_id: "fixture-render".into(),
        capture_id: "fixture-capture".into(),
    }
}
fn prepared(generation: u64) -> NativeAudio {
    let mut audio = NativeAudio::load(pin(true)).unwrap();
    let f = audio.prepare(&pair(), 2, generation).unwrap();
    assert_eq!(f.clock_frequency, 10_000_000);
    audio.start(true).unwrap();
    assert!(
        audio.reserve_at(480, time(0)).unwrap().is_none(),
        "one observation must not manufacture an output clock"
    );
    audio
}
type Step = unsafe extern "system" fn(*mut c_void, u32, u32) -> i32;
type Read = unsafe extern "system" fn(*mut c_void, *mut f32, u32, *mut u32) -> i32;
pub(crate) fn step(audio: &NativeAudio, frames: u32, failure: u32) {
    let p = unsafe { GetProcAddress(audio.library.module, c"TxAudioFixtureStep".as_ptr().cast()) }
        .unwrap();
    let function = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, Step>(p) };
    assert_eq!(unsafe { function(audio.raw.as_ptr(), frames, failure) }, 0);
}
pub(crate) fn read(audio: &NativeAudio) -> Vec<f32> {
    let p = unsafe { GetProcAddress(audio.library.module, c"TxAudioFixtureRead".as_ptr().cast()) }
        .unwrap();
    let function = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, Read>(p) };
    let mut output = vec![0.0f32; 1920];
    let mut count = 0;
    assert_eq!(
        unsafe { function(audio.raw.as_ptr(), output.as_mut_ptr(), 1920, &mut count) },
        0
    );
    output.truncate(count as usize);
    output
}
pub(crate) fn anchor(audio: &NativeAudio, qpc: u64) {
    type Anchor = unsafe extern "system" fn(*mut c_void, u64) -> i32;
    let p = unsafe {
        GetProcAddress(
            audio.library.module,
            c"TxAudioFixtureAnchor".as_ptr().cast(),
        )
    }
    .unwrap();
    let function =
        unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, Anchor>(p) };
    assert_eq!(unsafe { function(audio.raw.as_ptr(), qpc) }, 0);
}
fn voice(index: u64) -> f32 {
    (0.3 * (std::f64::consts::TAU * 1000.0 * index as f64 / 48000.0).sin()) as f32
}

#[test]
fn native_audio_bridge_pinned_production_has_no_fixture_and_refuses_unknown_pair() {
    let mut bad = pin(false);
    bad.sha256 = "00".repeat(32);
    assert!(matches!(
        NativeAudio::load(bad),
        Err(Error::Load(Reason::Integrity))
    ));
    let mut audio = NativeAudio::load(pin(false)).unwrap();
    assert!(unsafe {
        GetProcAddress(
            audio.library.module,
            c"TxAudioFixtureAnchor".as_ptr().cast(),
        )
    }
    .is_none());
    assert!(
        unsafe { GetProcAddress(audio.library.module, c"TxAudioFixtureStep".as_ptr().cast()) }
            .is_none()
    );
    assert!(
        unsafe { GetProcAddress(audio.library.module, c"TxAudioFixtureRead".as_ptr().cast()) }
            .is_none()
    );
    assert_eq!(audio.status().unwrap().phase, 0);
    assert!(audio.ready_event().unwrap().is_none());
    // Product DLL enumerates metadata, and rejects these nonexistent IDs before
    // IAudioClient activation. No real render/capture endpoint is selected.
    assert!(matches!(
        audio.prepare(&pair(), 2, 1),
        Err(Error::Native(_))
    ));
    assert_eq!(audio.status().unwrap().submitted_frames, 0);
}

#[test]
fn native_audio_bridge_renders_timeline_pcm_and_corresponding_video_tags() {
    let timeline = Timeline::default();
    let g = timeline.begin(0, 7).unwrap();
    let mut audio = prepared(g);
    let renderer = SincRenderer::default();
    let revoked = audio.revocation();
    let mut media = 0;
    let mut neutral = 0;
    for n in 1..=12u64 {
        let now = time(n * 100_000);
        let first = (n - 1) * 480;
        timeline
            .push_audio(
                g,
                PcmPacket {
                    device: 7,
                    source: Source::SelfMicrophone,
                    first_sample: first,
                    capture: time((n - 1) * 100_000),
                    rate: RATE,
                    channels: 1,
                    flags: PacketFlags::default(),
                    samples: (first..first + 480).map(voice).collect(),
                },
                now,
            )
            .unwrap();
        timeline
            .push_video(
                g,
                CorrectedFrame {
                    id: n,
                    capture: time((n - 1) * 100_000),
                    nv12: Arc::from(vec![n as u8; NV12_BYTES]),
                },
                now,
            )
            .unwrap();
        step(&audio, 480, 0);
        let mut ticket = audio.reserve_at(480, now).unwrap().unwrap();
        assert_eq!(ticket.first_frame(), (n + 1) * 480);
        let interval = ticket.take_interval().unwrap();
        let mut expected = Vec::new();
        let delivered = timeline
            .with_rendered_window(g, interval, now, &renderer, |window| {
                assert_eq!(window.presentation, time((n + 1) * 100_000));
                if window.status == RenderStatus::Media {
                    let picture = window.video.unwrap();
                    assert_eq!(picture.capture, window.capture);
                    assert_eq!(picture.id, n - 2);
                    assert!(picture.nv12.iter().all(|v| *v == picture.id as u8));
                    media += 1;
                } else {
                    assert!(window.video.is_none() && window.audio.iter().all(|v| *v == 0.0));
                    neutral += 1;
                }
                expected.extend_from_slice(window.audio);
                audio.commit(ticket, window.audio).is_ok()
            })
            .unwrap();
        let actual = read(&audio);
        assert_eq!(actual.len(), 960);
        for (i, channels) in actual.chunks_exact(2).enumerate() {
            assert_eq!(
                channels,
                &[expected[i], expected[i]],
                "actual native buffer must retain original samples"
            );
            if delivered && n >= 4 {
                assert!(
                    (channels[0] - voice((n - 3) * 480 + i as u64)).abs() < 0.0001,
                    "analytic tone oracle"
                );
            }
        }
    }
    assert_eq!((media, neutral), (10, 2));
    let status = audio.stop().unwrap();
    assert!(revoked.revoked() && status.client_reset && status.external_drain_verified);
    assert!(read(&audio).is_empty());
    assert_eq!(timeline.snapshot().metrics.output_frames, 4800);
    timeline.stop().unwrap();
    assert_eq!(timeline.snapshot().audio_frames, 0);
    println!("native bridge: 10 original-voice windows, 2 neutral windows, 5760 stereo frames checked; no device streams");
}

#[test]
fn native_audio_bridge_backpressure_does_not_consume_timeline_or_accept_an_empty_ticket() {
    let mut audio = prepared(1);
    step(&audio, 480, 0);
    for _ in 0..3 {
        let mut t = audio.reserve_at(480, time(100_000)).unwrap().unwrap();
        t.take_interval().unwrap();
        audio.commit(t, &[0.25; 480]).unwrap();
    }
    let before = audio.status().unwrap().submitted_frames;
    assert!(audio.reserve_at(480, time(100_000)).unwrap().is_none());
    assert_eq!(audio.status().unwrap().submitted_frames, before);
    step(&audio, 480, 0);
    assert!(audio.reserve_at(480, time(200_000)).unwrap().is_some());
}

#[test]
fn native_audio_bridge_faults_and_cleanup_errors_survive_ffi() {
    for failure in 1..=5 {
        let mut audio = prepared(1);
        step(&audio, 480, 0);
        let mut ticket = audio.reserve_at(480, time(100_000)).unwrap().unwrap();
        ticket.take_interval().unwrap();
        step(&audio, 0, failure);
        if failure <= 2 {
            assert!(audio.commit(ticket, &[0.2; 480]).is_err());
        }
        let status = audio.stop().unwrap();
        assert!(audio.revocation().revoked());
        assert_eq!(status.client_reset, failure != 3 && failure != 4);
        assert_eq!(
            status.external_drain_verified,
            failure != 3 && failure != 4 && failure != 5
        );
        if failure == 3 {
            assert!(status.stop_error < 0);
        }
        if failure == 4 {
            assert!(status.reset_error < 0);
        }
        assert!(audio.start(true).is_err());
        assert!(
            audio.prepare(&pair(), 2, 2).is_err(),
            "no object reuse after stop"
        );
        step(&audio, 0, 0);
        assert!(audio.stop().unwrap().client_reset, "cleanup retry");
    }
}

#[test]
fn native_audio_bridge_rejects_late_ticket_and_keeps_recreated_owners_distinct() {
    let mut first = prepared(1);
    step(&first, 480, 0);
    let mut old = first.reserve_at(480, time(100_000)).unwrap().unwrap();
    old.take_interval().unwrap();
    first.stop().unwrap();
    drop(first);
    let mut second = prepared(1);
    step(&second, 480, 0);
    assert_eq!(second.commit(old, &[0.1; 480]), Err(Error::Owner));
    let mut t = second.reserve_at(480, time(100_000)).unwrap().unwrap();
    t.take_interval().unwrap();
    step(&second, 900, 0);
    assert!(second.commit(t, &[0.1; 480]).is_err());
    assert!(second.revocation().revoked() && read(&second).is_empty());
}

#[test]
fn native_audio_bridge_abi_bounds_and_thread_affinity_are_enforced() {
    let audio = NativeAudio::load(pin(true)).unwrap();
    let mut status = RawStatus::default();
    assert!(
        unsafe {
            (audio.library.status)(
                audio.raw.as_ptr(),
                &mut status,
                (size_of::<RawStatus>() - 1) as u32,
            )
        } < 0
    );
    let get_status = audio.library.status;
    let raw = audio.raw.as_ptr() as usize;
    let hr = std::thread::spawn(move || {
        let mut status = RawStatus::default();
        unsafe { get_status(raw as *mut c_void, &mut status, 48) }
    })
    .join()
    .unwrap();
    assert_eq!(hr as u32, 0x8001010e); // RPC_E_WRONG_THREAD, not an unsafe Session read.
    let flag = audio.revocation();
    drop(audio);
    assert!(
        flag.revoked(),
        "callback context lives until native destruction completes"
    );
}

#[test]
fn native_audio_bridge_explicit_intent_and_payload_failure_close_the_session() {
    let mut audio = NativeAudio::load(pin(true)).unwrap();
    audio.prepare(&pair(), 2, 1).unwrap();
    assert!(audio.start(false).is_err());
    assert_eq!(audio.status().unwrap().submitted_frames, 0);
    audio.start(true).unwrap();
    assert!(audio.reserve_at(480, time(0)).unwrap().is_none());
    step(&audio, 480, 0);
    let mut ticket = audio.reserve_at(480, time(100_000)).unwrap().unwrap();
    ticket.take_interval().unwrap();
    let mut samples = [0.2; 480];
    samples[17] = f32::NAN;
    assert!(audio.commit(ticket, &samples).is_err());
    assert!(audio.revocation().revoked() && read(&audio).is_empty());
}

#[test]
fn native_audio_bridge_full_scale_voice_reaches_native_buffer_without_fault() {
    let timeline = Timeline::default();
    let g = timeline.begin(0, 7).unwrap();
    let mut audio = prepared(g);
    let renderer = SincRenderer::default();
    for n in 1..=3u64 {
        let now = time(n * 100_000);
        let first = (n - 1) * 480;
        timeline
            .push_audio(
                g,
                PcmPacket {
                    device: 7,
                    source: Source::SelfMicrophone,
                    first_sample: first,
                    capture: time((n - 1) * 100_000),
                    rate: RATE,
                    channels: 1,
                    flags: PacketFlags::default(),
                    samples: (first..first + 480)
                        .map(|i| if i % 48 < 24 { 1.0 } else { -1.0 })
                        .collect(),
                },
                now,
            )
            .unwrap();
        timeline
            .push_video(
                g,
                CorrectedFrame {
                    id: n,
                    capture: time((n - 1) * 100_000),
                    nv12: Arc::from(vec![40; NV12_BYTES]),
                },
                now,
            )
            .unwrap();
        step(&audio, 480, 0);
        let mut ticket = audio.reserve_at(480, now).unwrap().unwrap();
        let interval = ticket.take_interval().unwrap();
        let mut expected = Vec::new();
        let media = timeline
            .with_rendered_window(g, interval, now, &renderer, |window| {
                if n == 3 {
                    assert!(window.saturated_samples > 0);
                }
                expected.extend_from_slice(window.audio);
                audio.commit(ticket, window.audio).is_ok()
            })
            .unwrap();
        assert_eq!(media, n == 3);
        let actual = read(&audio);
        for (i, channels) in actual.chunks_exact(2).enumerate() {
            assert_eq!(channels, &[expected[i], expected[i]]);
        }
    }
    assert!(timeline.snapshot().metrics.saturated_samples > 0);
    assert!(!audio.revocation().revoked());
    assert_eq!(audio.status().unwrap().phase, 2);
}
