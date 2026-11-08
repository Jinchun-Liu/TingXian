use super::*;
use sha2::{Digest, Sha256};
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
static SERIAL: AtomicU64 = AtomicU64::new(0);
fn sha(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
fn stage() -> Installed {
    let parent = PathBuf::from(
        std::env::var_os("TINGXIAN_VIDEO_TEST_ROOT").expect("explicit CI evidence root"),
    );
    let directory = parent.join(format!(
        "media-bundle-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        SERIAL.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir_all(&directory).unwrap();
    let mut manifest = format!("{}\n", bundle_contract::HEADER);
    for name in bundle_contract::FILES {
        let key = if name.starts_with("TingXianAudio") {
            "TINGXIAN_AUDIO_TEST_BIN"
        } else {
            "TINGXIAN_VIDEO_TEST_BIN"
        };
        let original = PathBuf::from(std::env::var_os(key).unwrap()).join(name);
        let bytes = fs::read(&original).unwrap();
        let hash = sha(&bytes);
        fs::copy(original, directory.join(name)).unwrap();
        assert_eq!(sha(&fs::read(directory.join(name)).unwrap()), hash);
        manifest += &format!("{hash}  {name}\n");
    }
    fs::write(directory.join(bundle_contract::MANIFEST), &manifest).unwrap();
    Installed {
        directory,
        manifest_sha256: sha(manifest.as_bytes()),
    }
}
#[test]
fn native_media_bundle_verifies_real_components_and_holds_file_leases_without_loading() {
    let install = stage();
    let directory = install.directory.clone();
    let pin = install.manifest_sha256.clone();
    let bundle = install.verify().unwrap();
    assert_eq!(bundle.report().state, State::IntegrityVerified);
    assert!(!bundle.report().media_started);
    assert_eq!(
        bundle.camera().path,
        directory.join(bundle_contract::FILES[0])
    );
    assert_eq!(
        bundle.microphone().path,
        directory.join(bundle_contract::FILES[1])
    );
    assert_eq!(
        bundle.audio().path,
        directory.join(bundle_contract::FILES[2])
    );
    assert_eq!(
        bundle.video().path,
        directory.join(bundle_contract::FILES[3])
    );
    let target = directory.join(bundle_contract::FILES[0]);
    assert!(
        fs::OpenOptions::new().write(true).open(&target).is_err(),
        "verified file was writable"
    );
    assert!(
        fs::rename(&target, directory.join("unexpected.dll")).is_err(),
        "verified path was replaceable"
    );
    drop(bundle);
    let mut bytes = fs::read(&target).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&target, bytes).unwrap();
    assert_eq!(
        inspect(&directory, Some(&pin)).state,
        State::IntegrityFailure
    );
}
#[test]
fn native_media_bundle_does_not_trust_mutable_manifest_or_invalid_pe() {
    let install = stage();
    let directory = install.directory;
    let manifest = directory.join(bundle_contract::MANIFEST);
    let old = fs::read_to_string(&manifest).unwrap();
    let target = directory.join(bundle_contract::FILES[1]);
    let before = fs::read(&target).unwrap();
    let mut changed = before.clone();
    changed[0] = 0;
    fs::write(target, &changed).unwrap();
    let replaced = old.replace(&sha(&before), &sha(&changed));
    fs::write(&manifest, &replaced).unwrap();
    assert_eq!(
        inspect(&directory, Some(&install.manifest_sha256)).state,
        State::IntegrityFailure,
        "mutable digest was trusted"
    );
    assert_eq!(
        inspect(&directory, Some(&sha(replaced.as_bytes()))).state,
        State::IntegrityFailure,
        "non-PE component was accepted"
    );
}
#[test]
fn native_media_bundle_missing_and_hardlinked_components_are_rejected() {
    let install = stage();
    let target = install.directory.join(bundle_contract::FILES[2]);
    let preserved = install.directory.parent().unwrap().join(format!(
        "preserved-{}",
        install.directory.file_name().unwrap().to_string_lossy()
    ));
    fs::rename(&target, &preserved).unwrap();
    assert_eq!(
        inspect(&install.directory, Some(&install.manifest_sha256)).state,
        State::IntegrityFailure
    );
    fs::hard_link(&preserved, &target).unwrap();
    assert_eq!(
        inspect(&install.directory, Some(&install.manifest_sha256)).state,
        State::IntegrityFailure
    );
}
#[test]
fn native_media_bundle_without_embedded_pin_remains_unavailable_and_idle() {
    assert_eq!(
        inspect(Path::new("missing/no-read"), None).state,
        State::NotBundled
    );
    assert_eq!(
        inspect(Path::new("missing/no-read"), Some("")).state,
        State::NotBundled
    );
    let report = inspect(Path::new("missing/no-read"), Some("invalid"));
    assert_eq!(report.state, State::IntegrityFailure);
    assert_eq!(report.verified_components, 0);
    assert!(!report.media_started);
}
#[test]
fn native_media_bundle_rejects_unlisted_payload_in_media_directory() {
    let install = stage();
    fs::write(install.directory.join("unreviewed.txt"), b"synthetic").unwrap();
    assert_eq!(
        inspect(&install.directory, Some(&install.manifest_sha256)).state,
        State::IntegrityFailure
    );
}
