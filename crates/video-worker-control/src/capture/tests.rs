use super::*;
use sha2::{Digest, Sha256};
use std::fs;
pub(crate) fn pin() -> PinnedCaptureLibrary {
    let path = PathBuf::from(std::env::var_os("TINGXIAN_AUDIO_TEST_BIN").expect("CI native build"))
        .join("TingXianAudioCapture.dll");
    let sha256 = format!("{:x}", Sha256::digest(fs::read(&path).unwrap()));
    PinnedCaptureLibrary { path, sha256 }
}
fn packet() -> Packet {
    Packet {
        generation: 13,
        first_frame: 9917,
        qpc_100ns: 9_007_199_254_741_117,
        frames: 4800,
        flags: 1,
    }
}
#[test]
fn capture_bridge_preserves_whole_packet_original_qpc_position_and_startup_flag() {
    let samples: Vec<_> = (0..4800).map(|i| (i % 197) as f32 / 197.0).collect();
    let p = decode(packet(), &samples, 13, 7).unwrap();
    assert_eq!(p.samples, samples);
    assert_eq!(p.capture.qpc_100ns(), packet().qpc_100ns);
    assert_eq!(p.first_sample, 9917);
    assert_eq!(p.source, Source::SelfMicrophone);
    assert_eq!((p.device, p.rate, p.channels), (7, 48000, 1));
    assert!(p.flags.discontinuity && !p.flags.silent && !p.flags.timestamp_error);
    let mut raw = packet();
    raw.flags = 2;
    let silence = decode(raw, &[0.0; 4800], 13, 7).unwrap();
    assert!(silence.flags.silent && silence.samples.iter().all(|s| *s == 0.0));
}
#[test]
fn capture_bridge_rejects_malformed_generation_flags_and_samples() {
    for case in 0..10 {
        let mut raw = packet();
        let mut samples = vec![0.2; 4800];
        let mut mic = 7;
        match case {
            0 => raw.generation += 1,
            1 => raw.frames = 0,
            2 => raw.frames = 4801,
            3 => raw.first_frame = u64::MAX,
            4 => raw.qpc_100ns = 0,
            5 => raw.flags = 4,
            6 => raw.flags = 8,
            7 => samples[3] = f32::NAN,
            8 => raw.flags = 2,
            _ => mic = 0,
        }
        assert!(
            matches!(decode(raw, &samples, 13, mic), Err(Error::Protocol)),
            "case {case}"
        );
    }
}
#[test]
fn capture_real_abi_rejects_unknown_and_denied_without_initializing() {
    let mut input = NativeCapture::load(pin()).unwrap();
    let selected = Selection {
        microphone_id: "tingxian-nonexistent-microphone".into(),
        output_return_id: "tingxian-nonexistent-return".into(),
    };
    assert_eq!(input.status().unwrap().phase, 0);
    assert!(input.inspect(&selected).is_err());
    let (mic, ret) = (
        endpoint(&selected.microphone_id).unwrap(),
        endpoint(&selected.output_return_id).unwrap(),
    );
    let (lib, raw) = input.native().unwrap();
    let mut f = Format::default();
    let denied = unsafe {
        (lib.prepare)(
            raw,
            mic.as_ptr(),
            mic.len() as u32,
            ret.as_ptr(),
            ret.len() as u32,
            1,
            0,
            &mut f,
            16,
        )
    };
    assert_eq!(denied, E_ACCESSDENIED);
    assert!(input.prepare(&selected, 1).is_err());
    let s = input.status().unwrap();
    assert_eq!((s.phase, s.packets, s.delivered_frames), (0, 0, 0));
    assert_eq!(input.start(), Err(Error::Off));
    assert!(matches!(input.read(7), Err(Error::Off)));
    let c = input.finish();
    assert!(!c.reset_required && c.failure().is_none());
    assert!(input.finish().failure().is_none());
    assert_eq!(input.status(), Err(Error::Off));
}
#[test]
fn capture_integrity_and_cleanup_cannot_report_false_success() {
    let mut bad = pin();
    bad.sha256 = "00".repeat(32);
    assert!(matches!(
        NativeCapture::load(bad),
        Err(Error::Load(Reason::Integrity))
    ));
    for mode in 0..6 {
        let mut s = Status {
            phase: 3,
            reset: 1,
            ..Status::default()
        };
        let mut destroy = Ok(());
        match mode {
            0 => {
                s.stop_error = 1;
                s.reset_error = 1
            }
            1 => s.stop_error = 2,
            2 => s.reset = 0,
            3 => s.phase = 2,
            4 => s.reset_error = E_ACCESSDENIED,
            _ => destroy = Err(Error::Native(E_FAIL)),
        }
        let c = Cleanup {
            stop: Ok(()),
            status: Ok(s),
            destroy,
            reset_required: true,
        };
        assert_eq!(c.failure().is_none(), mode == 0);
    }
}
