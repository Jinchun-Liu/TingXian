use super::*;
use sha2::{Digest, Sha256};
use std::fs;
pub(crate) fn pin() -> PinnedCameraLibrary {
    let path = PathBuf::from(std::env::var_os("TINGXIAN_VIDEO_TEST_BIN").expect("CI native build"))
        .join("TingXianCameraInput.dll");
    let sha256 = format!("{:x}", Sha256::digest(fs::read(&path).unwrap()));
    PinnedCameraLibrary { path, sha256 }
}
fn packet() -> Packet {
    Packet {
        generation: 7,
        sequence: 41,
        qpc_100ns: 9_007_199_254_741_117,
        bytes: STREAM_BGR_BYTES as u32,
        discontinuity: 1,
    }
}
#[test]
fn camera_binding_preserves_exact_qpc_bgr_generation_and_discontinuity() {
    let values: Vec<u8> = (0..STREAM_BGR_BYTES).map(|x| (x % 251) as u8).collect();
    let f = decode(packet(), Pixels(values.clone()), 7).unwrap();
    assert_eq!(f.generation, 7);
    assert_eq!(f.sequence, 41);
    assert_eq!(f.capture.qpc_100ns(), packet().qpc_100ns);
    assert!(f.discontinuity);
    assert_eq!(f.bgr, values);
}
#[test]
fn camera_binding_rejects_invalid_generation_time_capacity_and_flags() {
    for mode in 0..8 {
        let mut raw = packet();
        let mut pixels = Pixels(vec![19; STREAM_BGR_BYTES]);
        match mode {
            0 => raw.generation += 1,
            1 => raw.sequence = 0,
            2 => raw.qpc_100ns = 0,
            3 => raw.qpc_100ns = u64::MAX,
            4 => raw.bytes -= 1,
            5 => raw.discontinuity = 2,
            6 => pixels.0.truncate(30),
            _ => raw.generation = 0,
        }
        assert!(
            matches!(decode(raw, pixels, 7), Err(Error::Protocol)),
            "mode {mode}"
        );
    }
}
#[test]
fn camera_production_abi_denied_and_invalid_without_enumeration_or_capture() {
    for _ in 0..3 {
        let mut camera = NativeCamera::load(pin()).unwrap();
        assert_eq!(camera.status().unwrap().phase, 0);
        let (lib, ptr) = camera.native().unwrap();
        let name = endpoint("not-a-real-camera").unwrap();
        assert_eq!(
            unsafe { (lib.prepare)(ptr, name.as_ptr(), name.len() as u32, 1, 0) },
            E_ACCESSDENIED
        );
        assert_eq!(
            unsafe { (lib.prepare)(ptr, name.as_ptr(), 0, 1, 1) },
            E_INVALIDARG
        );
        let mut pixels = vec![255; STREAM_BGR_BYTES];
        let mut raw = packet();
        assert_eq!(
            unsafe {
                (lib.read)(
                    ptr,
                    999,
                    pixels.as_mut_ptr(),
                    STREAM_BGR_BYTES as u32,
                    &mut raw,
                    32,
                )
            },
            E_INVALIDARG
        );
        assert!(pixels.iter().all(|p| *p == 0));
        assert_eq!(raw.sequence, 0);
        assert_eq!(camera.prepare("", 1), Err(Error::Bounds));
        assert_eq!(camera.start(), Err(Error::Off));
        assert!(matches!(camera.read(), Err(Error::Off)));
        camera.revoke().unwrap();
        assert!(camera.finish().failure().is_none());
        assert!(camera.finish().failure().is_none());
        assert_eq!(camera.status(), Err(Error::Off));
    }
}
#[test]
fn camera_integrity_and_uncertain_retirement_never_report_success() {
    let mut bad = pin();
    bad.sha256 = "00".repeat(32);
    assert!(matches!(
        NativeCamera::load(bad),
        Err(Error::Load(Reason::Integrity))
    ));
    for mode in 0..8 {
        let mut s = Status {
            phase: 5,
            ..Default::default()
        };
        let mut c = Cleanup {
            revoke: Ok(()),
            stop: Ok(()),
            status: Ok(s),
            destroy: Ok(()),
        };
        match mode {
            0 => {}
            1 => s.phase = 4,
            2 => s.cleanup_uncertain = 1,
            3 => s.queued = 1,
            4 => s.shutdown = E_ACCESSDENIED,
            5 => c.revoke = Err(Error::Native(E_FAIL)),
            6 => c.destroy = Err(Error::Native(E_FAIL)),
            _ => c.stop = Err(Error::Native(E_FAIL)),
        }
        c.status = Ok(s);
        assert_eq!(c.failure().is_none(), mode == 0);
    }
}
