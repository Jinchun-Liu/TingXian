//! Conservative durable exposure barrier. No reset API: neither endpoint yet
//! supplies a verifiable downstream drain/release proof. Local cleanup, elapsed
//! time, a new object, or process exit must not implicitly clear this record.
use crate::windows::{lock_parents, regular};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::windows::fs::OpenOptionsExt,
    path::Path,
};
use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Path,
    Io,
    Busy,
    UncertainDrain,
}
pub(super) struct Barrier {
    file: File,
    _directories: Vec<File>,
    armed: bool,
}
impl Barrier {
    pub(super) fn acquire(path: &Path) -> Result<Self, Error> {
        // Stable trusted native profile path, never a per-session name or a
        // webview preference. Lease parents before opening, rejecting reparse
        // points, ADS, hard links and concurrent writers/renames.
        let directories = lock_parents(path).map_err(|_| Error::Path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|e| match e.raw_os_error() {
                Some(32 | 33) => Error::Busy,
                _ => Error::Io,
            })?;
        // Any nonempty, malformed or partially written record fences restart.
        // Empty is safe only because all exposure is preceded by a durable arm.
        if regular(&file).map_err(|_| Error::Path)? != 0 {
            return Err(Error::UncertainDrain);
        }
        Ok(Self {
            file,
            _directories: directories,
            armed: false,
        })
    }
    pub(super) fn arm(&mut self, generation: u64) -> Result<(), Error> {
        if self.armed || generation == 0 {
            return Err(Error::UncertainDrain);
        }
        self.armed = true;
        let mut record = [0u8; 32];
        record[..8].copy_from_slice(b"TXAVB001");
        record[8..16].copy_from_slice(&generation.to_le_bytes());
        record[16..20].copy_from_slice(&std::process::id().to_le_bytes());
        self.file.write_all(&record).map_err(|_| Error::Io)?;
        self.file.sync_all().map_err(|_| Error::Io)?;
        Ok(())
    }
}
