//! Native-only component trust. No DLL load, endpoint enumeration or media authority.
use crate::{
    bundle_contract::{self, Pins},
    windows::{verify, Verified},
    BundledWorker, Reason,
};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    NotBundled,
    IntegrityVerified,
    IntegrityFailure,
    UnsupportedPlatform,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Report {
    pub state: State,
    pub verified_components: u32,
    // Integrity never grants runtime/media/device readiness.
    pub media_started: bool,
}
impl Report {
    pub fn unavailable(state: State) -> Self {
        Self {
            state,
            verified_components: 0,
            media_started: false,
        }
    }
}
/// Paths/hash come from native installation/build inputs, never WebView settings.
pub struct Installed {
    pub directory: PathBuf,
    pub manifest_sha256: String,
}
/// Keep these file/directory leases through native preparation. Every eventual
/// component loader still rechecks its own pin/path/ABI; inspection is not start.
pub struct Bundle {
    directory: PathBuf,
    pins: Pins,
    _files: Vec<Verified>,
}
impl Installed {
    pub fn verify(self) -> Result<Bundle, Reason> {
        let manifest = verify(&BundledWorker {
            path: self.directory.join(bundle_contract::MANIFEST),
            sha256: self.manifest_sha256,
        })?;
        let data = manifest.read_bounded(1024)?;
        let pins = Pins::parse(std::str::from_utf8(&data).map_err(|_| Reason::Integrity)?)
            .map_err(|_| Reason::Integrity)?;
        for entry in std::fs::read_dir(&self.directory).map_err(|_| Reason::Integrity)? {
            let entry = entry.map_err(|_| Reason::Integrity)?;
            if !entry
                .file_name()
                .to_str()
                .is_some_and(bundle_contract::allowed_name)
            {
                return Err(Reason::Integrity);
            }
        }
        let mut files = vec![manifest];
        for (i, name) in bundle_contract::FILES.iter().enumerate() {
            let file = verify(&BundledWorker {
                path: self.directory.join(name),
                sha256: pins.0[i].clone(),
            })?;
            if !bundle_contract::x64_dll(&file.read_bounded(32 * 1024 * 1024)?) {
                return Err(Reason::Integrity);
            }
            files.push(file);
        }
        Ok(Bundle {
            directory: self.directory,
            pins,
            _files: files,
        })
    }
}
impl Bundle {
    pub fn report(&self) -> Report {
        Report {
            state: State::IntegrityVerified,
            verified_components: 4,
            media_started: false,
        }
    }
    fn path(&self, i: usize) -> PathBuf {
        self.directory.join(bundle_contract::FILES[i])
    }
    pub fn camera(&self) -> crate::camera::PinnedCameraLibrary {
        crate::camera::PinnedCameraLibrary {
            path: self.path(0),
            sha256: self.pins.0[0].clone(),
        }
    }
    pub fn microphone(&self) -> crate::capture::PinnedCaptureLibrary {
        crate::capture::PinnedCaptureLibrary {
            path: self.path(1),
            sha256: self.pins.0[1].clone(),
        }
    }
    pub fn audio(&self) -> crate::audio::PinnedAudioLibrary {
        crate::audio::PinnedAudioLibrary {
            path: self.path(2),
            sha256: self.pins.0[2].clone(),
        }
    }
    pub fn video(&self) -> crate::timed_video::PinnedTimedLibrary {
        crate::timed_video::PinnedTimedLibrary {
            path: self.path(3),
            sha256: self.pins.0[3].clone(),
        }
    }
}
pub fn inspect(directory: &Path, embedded_pin: Option<&str>) -> Report {
    let Some(pin) = embedded_pin.filter(|s| !s.is_empty()) else {
        return Report::unavailable(State::NotBundled);
    };
    match (Installed {
        directory: directory.to_owned(),
        manifest_sha256: pin.into(),
    })
    .verify()
    {
        Ok(bundle) => bundle.report(),
        Err(_) => Report::unavailable(State::IntegrityFailure),
    }
}

#[cfg(test)]
mod tests;
