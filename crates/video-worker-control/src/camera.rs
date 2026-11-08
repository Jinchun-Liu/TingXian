//! Optional native camera input. Trusted selection only; no UI/media authority.
use crate::{
    av_timeline::Time,
    windows::{verify, Verified},
    BundledWorker, Reason, STREAM_BGR_BYTES,
};
use std::{
    ffi::c_void,
    marker::PhantomData,
    os::windows::{ffi::OsStrExt, io::BorrowedHandle},
    path::PathBuf,
    ptr::{null_mut, NonNull},
    rc::Rc,
};
use windows_sys::Win32::System::Threading::WaitForSingleObject;
use windows_sys::Win32::{Foundation::*, System::LibraryLoader::*};
pub struct PinnedCameraLibrary {
    pub path: PathBuf,
    pub sha256: String,
}
/// Native-only, not Deserialize. No restored setting may invoke this implicitly.
pub struct Config {
    pub library: PinnedCameraLibrary,
    pub symbolic_link: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Load(Reason),
    Off,
    Bounds,
    Protocol,
    Native(i32),
}
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Packet {
    generation: u64,
    sequence: u64,
    qpc_100ns: u64,
    bytes: u32,
    discontinuity: u32,
}
#[repr(C)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub(crate) version: u32,
    pub phase: u32,
    pub accepted: u64,
    pub delivered: u64,
    pub stale: u64,
    pub superseded: u64,
    pub cleared: u64,
    pub late: u64,
    pub discontinuities: u64,
    pub queued: u32,
    pub closed: u32,
    pub cleanup_uncertain: u32,
    pub(crate) reserved: u32,
    pub operation: i32,
    pub frame_error: i32,
    pub flush: i32,
    pub shutdown: i32,
    pub release: i32,
    pub(crate) reserved2: u32,
}
const _: () = assert!(std::mem::size_of::<Packet>() == 32);
const _: () = assert!(std::mem::size_of::<Status>() == 104);
#[derive(Clone, Copy, Debug)]
pub struct Cleanup {
    pub revoke: Result<(), Error>,
    pub stop: Result<(), Error>,
    pub status: Result<Status, Error>,
    pub destroy: Result<(), Error>,
}
impl Cleanup {
    pub fn failure(&self) -> Option<Error> {
        for r in [self.revoke, self.stop, self.destroy] {
            if let Err(e) = r {
                return Some(e);
            }
        }
        let s = match self.status {
            Ok(s) => s,
            Err(e) => return Some(e),
        };
        for hr in [s.flush, s.shutdown, s.release] {
            if hr != 0 {
                return Some(Error::Native(hr));
            }
        }
        if s.phase != 5 || s.cleanup_uncertain != 0 || s.queued != 0 {
            return Some(Error::Protocol);
        }
        None
    }
}
pub(crate) struct Frame {
    pub generation: u64,
    pub sequence: u64,
    pub capture: Time,
    pub discontinuity: bool,
    pub bgr: Vec<u8>,
}
impl Drop for Frame {
    fn drop(&mut self) {
        self.bgr.fill(0);
    }
}
pub(crate) trait Device {
    fn prepare(&mut self, selected: &str, generation: u64) -> Result<(), Error>;
    fn start(&mut self) -> Result<(), Error>;
    fn read(&mut self) -> Result<Option<Frame>, Error>;
    fn event(&self) -> Result<Option<BorrowedHandle<'_>>, Error>;
    fn status(&self) -> Result<Status, Error>;
    fn revoke(&mut self) -> Result<(), Error>;
    fn finish(&mut self) -> Cleanup;
}
pub(crate) struct Launch {
    pub factory: Box<dyn FnOnce() -> Result<Box<dyn Device>, Error> + Send>,
    pub selected: String,
}
impl Config {
    pub(crate) fn launch(self) -> Launch {
        Launch {
            selected: self.symbolic_link,
            factory: Box::new(move || {
                NativeCamera::load(self.library).map(|v| Box::new(v) as Box<dyn Device>)
            }),
        }
    }
}
type Abi = unsafe extern "system" fn() -> u32;
type Create = unsafe extern "system" fn() -> *mut c_void;
type Prepare = unsafe extern "system" fn(*mut c_void, *const u16, u32, u64, u32) -> i32;
type Start = unsafe extern "system" fn(*mut c_void, u64) -> i32;
type Read = unsafe extern "system" fn(*mut c_void, u64, *mut u8, u32, *mut Packet, u32) -> i32;
type Action = unsafe extern "system" fn(*mut c_void) -> i32;
type GetStatus = unsafe extern "system" fn(*mut c_void, *mut Status, u32) -> i32;
type Event = unsafe extern "system" fn(*mut c_void, *mut HANDLE) -> i32;
struct Library {
    module: HMODULE,
    _verified: Verified,
    create: Create,
    prepare: Prepare,
    start: Start,
    read: Read,
    revoke: Action,
    stop: Action,
    status: GetStatus,
    event: Event,
    destroy: Action,
}
impl Drop for Library {
    fn drop(&mut self) {
        unsafe {
            FreeLibrary(self.module);
        }
    }
}
struct Module(HMODULE);
impl Drop for Module {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                FreeLibrary(self.0);
            }
        }
    }
}
impl Library {
    fn load(pin: PinnedCameraLibrary) -> Result<Self, Error> {
        let verified = verify(&BundledWorker {
            path: pin.path.clone(),
            sha256: pin.sha256,
        })
        .map_err(Error::Load)?;
        let normalized = pin
            .path
            .to_str()
            .ok_or(Error::Load(Reason::Integrity))?
            .replace('/', "\\");
        let path: Vec<u16> = std::ffi::OsStr::new(&normalized)
            .encode_wide()
            .chain(Some(0))
            .collect();
        if path[..path.len() - 1].contains(&0) {
            return Err(Error::Load(Reason::Integrity));
        }
        let mut module = Module(unsafe {
            LoadLibraryExW(path.as_ptr(), null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32)
        });
        if module.0.is_null() {
            return Err(Error::Load(Reason::Launch));
        }
        let mut actual = vec![0u16; 32768];
        let n = unsafe { GetModuleFileNameW(module.0, actual.as_mut_ptr(), actual.len() as u32) }
            as usize;
        if n == 0
            || n >= actual.len()
            || String::from_utf16_lossy(&actual[..n]).to_lowercase() != normalized.to_lowercase()
        {
            return Err(Error::Load(Reason::Integrity));
        }
        macro_rules! symbol {
            ($name:literal, $ty:ty) => {{
                let p = unsafe { GetProcAddress(module.0, concat!($name, "\0").as_ptr()) }
                    .ok_or(Error::Protocol)?;
                unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, $ty>(p) }
            }};
        }
        let abi = symbol!("TxCameraAbi", Abi);
        if unsafe { abi() } != 1 {
            return Err(Error::Protocol);
        }
        let library = Self {
            module: module.0,
            _verified: verified,
            create: symbol!("TxCameraCreate", Create),
            prepare: symbol!("TxCameraPrepare", Prepare),
            start: symbol!("TxCameraStart", Start),
            read: symbol!("TxCameraRead", Read),
            revoke: symbol!("TxCameraRevoke", Action),
            stop: symbol!("TxCameraStop", Action),
            status: symbol!("TxCameraStatusRead", GetStatus),
            event: symbol!("TxCameraReadyEvent", Event),
            destroy: symbol!("TxCameraDestroy", Action),
        };
        module.0 = null_mut();
        Ok(library)
    }
}
fn check(hr: i32) -> Result<(), Error> {
    if hr == 0 {
        Ok(())
    } else {
        Err(Error::Native(hr))
    }
}
fn endpoint(s: &str) -> Result<Vec<u16>, Error> {
    let v: Vec<_> = s.encode_utf16().collect();
    if v.is_empty() || v.len() > 32767 || v.contains(&0) {
        Err(Error::Bounds)
    } else {
        Ok(v)
    }
}
struct Pixels(Vec<u8>);
impl Drop for Pixels {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}
fn decode(raw: Packet, mut pixels: Pixels, generation: u64) -> Result<Frame, Error> {
    if !generation.eq(&raw.generation)
        || generation == 0
        || raw.sequence == 0
        || raw.qpc_100ns == 0
        || raw.qpc_100ns > i64::MAX as u64
        || raw.bytes as usize != STREAM_BGR_BYTES
        || pixels.0.len() != STREAM_BGR_BYTES
        || raw.discontinuity > 1
    {
        return Err(Error::Protocol);
    }
    Ok(Frame {
        generation,
        sequence: raw.sequence,
        capture: Time::from_qpc_100ns(raw.qpc_100ns),
        discontinuity: raw.discontinuity == 1,
        bgr: std::mem::take(&mut pixels.0),
    })
}
/// !Send/!Sync. Creation, reads, revocation and destruction stay on media MTA.
struct NativeCamera {
    raw: Option<NonNull<c_void>>,
    library: Option<Library>,
    generation: u64,
    started: bool,
    previous_sequence: u64,
    previous_qpc: u64,
    revoked: Option<Result<(), Error>>,
    cleanup: Option<Cleanup>,
    _owner: PhantomData<Rc<()>>,
}
impl NativeCamera {
    fn load(pin: PinnedCameraLibrary) -> Result<Self, Error> {
        let lib = Library::load(pin)?;
        let raw = NonNull::new(unsafe { (lib.create)() }).ok_or(Error::Load(Reason::Launch))?;
        Ok(Self {
            raw: Some(raw),
            library: Some(lib),
            generation: 0,
            started: false,
            previous_sequence: 0,
            previous_qpc: 0,
            revoked: None,
            cleanup: None,
            _owner: PhantomData,
        })
    }
    fn native(&self) -> Result<(&Library, *mut c_void), Error> {
        Ok((
            self.library.as_ref().ok_or(Error::Off)?,
            self.raw.ok_or(Error::Off)?.as_ptr(),
        ))
    }
}
impl Device for NativeCamera {
    fn prepare(&mut self, selected: &str, generation: u64) -> Result<(), Error> {
        if generation == 0 || self.generation != 0 || self.revoked.is_some() {
            return Err(Error::Off);
        }
        let link = endpoint(selected)?;
        let (lib, raw) = self.native()?;
        check(unsafe { (lib.prepare)(raw, link.as_ptr(), link.len() as u32, generation, 1) })?;
        self.generation = generation;
        Ok(())
    }
    fn start(&mut self) -> Result<(), Error> {
        if self.generation == 0 || self.started || self.revoked.is_some() {
            return Err(Error::Off);
        }
        let (lib, raw) = self.native()?;
        check(unsafe { (lib.start)(raw, self.generation) })?;
        self.started = true;
        Ok(())
    }
    fn read(&mut self) -> Result<Option<Frame>, Error> {
        if !self.started || self.revoked.is_some() {
            return Err(Error::Off);
        }
        let (lib, raw) = self.native()?;
        let mut event = null_mut();
        check(unsafe { (lib.event)(raw, &mut event) })?;
        if event.is_null() || event == INVALID_HANDLE_VALUE {
            return Err(Error::Protocol);
        }
        match unsafe { WaitForSingleObject(event, 0) } {
            WAIT_TIMEOUT => return Ok(None),
            WAIT_OBJECT_0 => {}
            _ => return Err(Error::Protocol),
        }
        let mut pixels = Pixels(vec![0; STREAM_BGR_BYTES]);
        let mut packet = Packet::default();
        let hr = unsafe {
            (lib.read)(
                raw,
                self.generation,
                pixels.0.as_mut_ptr(),
                STREAM_BGR_BYTES as u32,
                &mut packet,
                32,
            )
        };
        if hr == 1 {
            return Ok(None);
        }
        check(hr)?;
        let frame = decode(packet, pixels, self.generation)?;
        if frame.sequence <= self.previous_sequence
            || frame.capture.qpc_100ns() <= self.previous_qpc
        {
            return Err(Error::Protocol);
        }
        self.previous_sequence = frame.sequence;
        self.previous_qpc = frame.capture.qpc_100ns();
        Ok(Some(frame))
    }
    fn event(&self) -> Result<Option<BorrowedHandle<'_>>, Error> {
        if !self.started || self.revoked.is_some() {
            return Err(Error::Off);
        }
        let (lib, raw) = self.native()?;
        let mut event = null_mut();
        check(unsafe { (lib.event)(raw, &mut event) })?;
        if event.is_null() || event == INVALID_HANDLE_VALUE {
            return Err(Error::Protocol);
        }
        Ok(Some(unsafe { BorrowedHandle::borrow_raw(event) }))
    }
    fn status(&self) -> Result<Status, Error> {
        let (lib, raw) = self.native()?;
        let mut s = Status::default();
        check(unsafe { (lib.status)(raw, &mut s, 104) })?;
        if s.version != 1
            || s.phase > 6
            || s.queued > 1
            || s.closed > 1
            || s.cleanup_uncertain > 1
            || s.reserved != 0
            || s.reserved2 != 0
        {
            return Err(Error::Protocol);
        }
        Ok(s)
    }
    fn revoke(&mut self) -> Result<(), Error> {
        if let Some(result) = self.revoked {
            return result;
        }
        let (lib, raw) = self.native()?;
        let result = check(unsafe { (lib.revoke)(raw) });
        self.revoked = Some(result);
        result
    }
    fn finish(&mut self) -> Cleanup {
        if let Some(c) = self.cleanup {
            return c;
        }
        let revoke = self.revoke();
        let (lib, raw) = self.native().expect("first finish owns camera");
        let stop = check(unsafe { (lib.stop)(raw) });
        let status = self.status();
        let destroy = check(unsafe { (self.library.as_ref().unwrap().destroy)(raw) });
        self.raw = None;
        if destroy.is_err() {
            std::mem::forget(self.library.take());
        }
        self.started = false;
        let cleanup = Cleanup {
            revoke,
            stop,
            status,
            destroy,
        };
        self.cleanup = Some(cleanup);
        cleanup
    }
}
impl Drop for NativeCamera {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
#[cfg(test)]
pub(crate) mod tests;
