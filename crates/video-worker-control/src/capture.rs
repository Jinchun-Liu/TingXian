//! Optional pinned microphone input. Native owner only; no UI authority or ASR.
use crate::{
    av_timeline::{PacketFlags, PcmPacket, Source, Time, MAX_CAPTURE_FRAMES, RATE},
    windows::{verify, Verified},
    BundledWorker, Reason,
};
use std::{
    ffi::c_void,
    marker::PhantomData,
    os::windows::{ffi::OsStrExt, io::BorrowedHandle},
    path::PathBuf,
    ptr::{null_mut, NonNull},
    rc::Rc,
};
use windows_sys::Win32::{Foundation::*, System::LibraryLoader::*};

pub struct PinnedCaptureLibrary {
    pub path: PathBuf,
    pub sha256: String,
}
/// Supplied by trusted native selection, never deserialized as a DLL authority.
pub struct Config {
    pub library: PinnedCaptureLibrary,
    pub microphone_id: String,
}
pub(crate) struct Selection {
    pub microphone_id: String,
    pub output_return_id: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Load(Reason),
    Permission,
    Off,
    Bounds,
    Protocol,
    Native(i32),
}
#[repr(C)]
#[derive(Default)]
struct Format {
    version: u32,
    rate: u32,
    channels: u32,
    buffer_frames: u32,
}
#[repr(C)]
#[derive(Default)]
struct Packet {
    generation: u64,
    first_frame: u64,
    qpc_100ns: u64,
    frames: u32,
    flags: u32,
}
#[repr(C)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub(crate) version: u32,
    pub phase: u32,
    pub delivered_frames: u64,
    pub discarded_frames: u64,
    pub packets: u64,
    pub operation_error: i32,
    pub release_error: i32,
    pub stop_error: i32,
    pub reset_error: i32,
    pub reset: u32,
    pub startup_discontinuities: u32,
    pub(crate) reserved: [u32; 2],
}
const _: () = assert!(std::mem::size_of::<Format>() == 16);
const _: () = assert!(std::mem::size_of::<Packet>() == 32);
const _: () = assert!(std::mem::size_of::<Status>() == 64);
#[derive(Clone, Copy, Debug)]
pub struct Cleanup {
    pub stop: Result<(), Error>,
    pub status: Result<Status, Error>,
    pub destroy: Result<(), Error>,
    pub reset_required: bool,
}
impl Cleanup {
    pub fn failure(&self) -> Option<Error> {
        if let Err(e) = self.stop {
            return Some(e);
        }
        if let Err(e) = self.destroy {
            return Some(e);
        }
        let s = match self.status {
            Ok(s) => s,
            Err(e) => return Some(e),
        };
        for hr in [s.stop_error, s.reset_error] {
            if hr != 0 && hr != 1 {
                return Some(Error::Native(hr));
            }
        }
        if !matches!(s.phase, 3 | 4) || (self.reset_required && s.reset != 1) {
            return Some(Error::Protocol);
        }
        None
    }
}
pub(crate) trait Device {
    fn inspect(&self, selection: &Selection) -> Result<(), Error>;
    fn prepare(&mut self, selection: &Selection, generation: u64) -> Result<(), Error>;
    fn start(&mut self) -> Result<(), Error>;
    fn read(&mut self, microphone: u64) -> Result<Option<PcmPacket>, Error>;
    fn event(&self) -> Result<Option<BorrowedHandle<'_>>, Error>;
    fn status(&self) -> Result<Status, Error>;
    fn finish(&mut self) -> Cleanup;
}
pub(crate) type Factory = Box<dyn FnOnce() -> Result<Box<dyn Device>, Error> + Send>;
pub(crate) struct Launch {
    pub factory: Factory,
    pub selection: Selection,
}
impl Config {
    pub(crate) fn launch(self, output_return_id: String) -> Launch {
        Launch {
            selection: Selection {
                microphone_id: self.microphone_id,
                output_return_id,
            },
            factory: Box::new(move || {
                NativeCapture::load(self.library).map(|v| Box::new(v) as Box<dyn Device>)
            }),
        }
    }
}
type Abi = unsafe extern "system" fn() -> u32;
type Create = unsafe extern "system" fn() -> *mut c_void;
type Inspect = unsafe extern "system" fn(*mut c_void, *const u16, u32, *const u16, u32) -> i32;
type Prepare = unsafe extern "system" fn(
    *mut c_void,
    *const u16,
    u32,
    *const u16,
    u32,
    u64,
    u32,
    *mut Format,
    u32,
) -> i32;
type Start = unsafe extern "system" fn(*mut c_void, u64) -> i32;
type Read = unsafe extern "system" fn(*mut c_void, u64, *mut f32, u32, *mut Packet, u32) -> i32;
type Action = unsafe extern "system" fn(*mut c_void) -> i32;
type GetStatus = unsafe extern "system" fn(*mut c_void, *mut Status, u32) -> i32;
type Event = unsafe extern "system" fn(*mut c_void, *mut HANDLE) -> i32;
struct Library {
    module: HMODULE,
    _verified: Verified,
    create: Create,
    inspect: Inspect,
    prepare: Prepare,
    start: Start,
    read: Read,
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
    fn load(pin: PinnedCaptureLibrary) -> Result<Self, Error> {
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
        let abi = symbol!("TxCaptureAbi", Abi);
        if unsafe { abi() } != 1 {
            return Err(Error::Protocol);
        }
        let lib = Self {
            module: module.0,
            _verified: verified,
            create: symbol!("TxCaptureCreate", Create),
            inspect: symbol!("TxCaptureInspect", Inspect),
            prepare: symbol!("TxCapturePrepare", Prepare),
            start: symbol!("TxCaptureStart", Start),
            read: symbol!("TxCaptureRead", Read),
            stop: symbol!("TxCaptureStop", Action),
            status: symbol!("TxCaptureStatusRead", GetStatus),
            event: symbol!("TxCaptureReadyEvent", Event),
            destroy: symbol!("TxCaptureDestroy", Action),
        };
        module.0 = null_mut();
        Ok(lib)
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
    if v.is_empty() || v.len() > 1024 || v.contains(&0) {
        Err(Error::Bounds)
    } else {
        Ok(v)
    }
}
struct Scratch([f32; MAX_CAPTURE_FRAMES]);
impl Drop for Scratch {
    fn drop(&mut self) {
        self.0.fill(0.0);
    }
}
fn decode(
    raw: Packet,
    samples: &[f32],
    generation: u64,
    microphone: u64,
) -> Result<PcmPacket, Error> {
    if generation == 0
        || raw.generation != generation
        || microphone == 0
        || raw.frames == 0
        || raw.frames as usize > MAX_CAPTURE_FRAMES
        || raw.frames as usize > samples.len()
        || raw.first_frame.checked_add(raw.frames as u64).is_none()
        || raw.qpc_100ns == 0
        || raw.flags & !3 != 0
    {
        return Err(Error::Protocol);
    }
    let values = &samples[..raw.frames as usize];
    if values.iter().any(|v| !v.is_finite())
        || (raw.flags & 2 != 0 && values.iter().any(|v| *v != 0.0))
    {
        return Err(Error::Protocol);
    }
    Ok(PcmPacket {
        device: microphone,
        source: Source::SelfMicrophone,
        first_sample: raw.first_frame,
        capture: Time::from_qpc_100ns(raw.qpc_100ns),
        rate: RATE,
        channels: 1,
        flags: PacketFlags {
            silent: raw.flags & 2 != 0,
            discontinuity: raw.flags & 1 != 0,
            timestamp_error: false,
        },
        samples: values.to_vec(),
    })
}
/// !Send/!Sync: load, prepare, read, stop and destroy on the common owner lane.
struct NativeCapture {
    raw: Option<NonNull<c_void>>,
    library: Option<Library>,
    generation: u64,
    prepared: bool,
    started: bool,
    cleanup: Option<Cleanup>,
    _owner: PhantomData<Rc<()>>,
}
impl NativeCapture {
    fn load(pin: PinnedCaptureLibrary) -> Result<Self, Error> {
        let lib = Library::load(pin)?;
        let raw = NonNull::new(unsafe { (lib.create)() }).ok_or(Error::Load(Reason::Launch))?;
        Ok(Self {
            raw: Some(raw),
            library: Some(lib),
            generation: 0,
            prepared: false,
            started: false,
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
impl Device for NativeCapture {
    fn inspect(&self, selection: &Selection) -> Result<(), Error> {
        let (mic, ret) = (
            endpoint(&selection.microphone_id)?,
            endpoint(&selection.output_return_id)?,
        );
        let (lib, raw) = self.native()?;
        check(unsafe {
            (lib.inspect)(
                raw,
                mic.as_ptr(),
                mic.len() as u32,
                ret.as_ptr(),
                ret.len() as u32,
            )
        })
    }
    fn prepare(&mut self, selection: &Selection, generation: u64) -> Result<(), Error> {
        if generation == 0 || self.prepared || self.cleanup.is_some() {
            return Err(Error::Off);
        }
        let (mic, ret) = (
            endpoint(&selection.microphone_id)?,
            endpoint(&selection.output_return_id)?,
        );
        let (lib, raw) = self.native()?;
        let mut f = Format::default();
        check(unsafe {
            (lib.prepare)(
                raw,
                mic.as_ptr(),
                mic.len() as u32,
                ret.as_ptr(),
                ret.len() as u32,
                generation,
                1,
                &mut f,
                16,
            )
        })?;
        self.prepared = true;
        if f.version != 1
            || f.rate != RATE
            || f.channels != 1
            || !(1..=MAX_CAPTURE_FRAMES as u32).contains(&f.buffer_frames)
        {
            return Err(Error::Protocol);
        }
        self.generation = generation;
        Ok(())
    }
    fn start(&mut self) -> Result<(), Error> {
        if !self.prepared || self.started {
            return Err(Error::Off);
        }
        let (lib, raw) = self.native()?;
        check(unsafe { (lib.start)(raw, self.generation) })?;
        self.started = true;
        Ok(())
    }
    fn read(&mut self, microphone: u64) -> Result<Option<PcmPacket>, Error> {
        if !self.started {
            return Err(Error::Off);
        }
        let (lib, ptr) = self.native()?;
        let mut scratch = Scratch([0.0; MAX_CAPTURE_FRAMES]);
        let mut raw = Packet::default();
        let hr = unsafe {
            (lib.read)(
                ptr,
                self.generation,
                scratch.0.as_mut_ptr(),
                MAX_CAPTURE_FRAMES as u32,
                &mut raw,
                32,
            )
        };
        if hr == 1 {
            return Ok(None);
        }
        check(hr)?;
        decode(raw, &scratch.0, self.generation, microphone).map(Some)
    }
    fn event(&self) -> Result<Option<BorrowedHandle<'_>>, Error> {
        if !self.started {
            return Err(Error::Off);
        }
        let (lib, raw) = self.native()?;
        let mut h = null_mut();
        check(unsafe { (lib.event)(raw, &mut h) })?;
        if h.is_null() || h == INVALID_HANDLE_VALUE {
            return Err(Error::Protocol);
        }
        Ok(Some(unsafe { BorrowedHandle::borrow_raw(h) }))
    }
    fn status(&self) -> Result<Status, Error> {
        let (lib, raw) = self.native()?;
        let mut s = Status::default();
        check(unsafe { (lib.status)(raw, &mut s, 64) })?;
        if s.version != 1
            || s.phase > 4
            || s.reset > 1
            || s.startup_discontinuities > 1
            || s.reserved != [0, 0]
        {
            return Err(Error::Protocol);
        }
        Ok(s)
    }
    fn finish(&mut self) -> Cleanup {
        if let Some(c) = self.cleanup {
            return c;
        }
        let before = self.status();
        let required = self.prepared || before.is_ok_and(|s| s.phase != 0);
        let (lib, raw) = self.native().expect("first finish owns instance");
        let stop = check(unsafe { (lib.stop)(raw) });
        let status = self.status();
        let destroy = check(unsafe { (self.library.as_ref().unwrap().destroy)(raw) });
        self.raw = None;
        if destroy.is_err() {
            // Never unload code under an instance whose destruction is unproved.
            // Host latches uncertain cleanup and disallows further sessions.
            std::mem::forget(self.library.take());
        }
        self.started = false;
        let cleanup = Cleanup {
            stop,
            status: before.and(status),
            destroy,
            reset_required: required,
        };
        self.cleanup = Some(cleanup);
        cleanup
    }
}
impl Drop for NativeCapture {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[cfg(test)]
pub(crate) mod tests;
