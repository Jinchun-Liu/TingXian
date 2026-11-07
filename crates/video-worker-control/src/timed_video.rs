//! Explicit v2 video admission. One pending reservation, no pipe wait in stage,
//! no camera/device/SDK authority. An admission is not remote presentation.
use crate::{
    av_timeline::{RenderedWindow, NV12_BYTES},
    windows::{verify, Verified},
    BundledWorker, NativeOutputEndpoint, Reason,
};
use std::{
    ffi::c_void,
    os::windows::{ffi::OsStrExt, io::BorrowedHandle},
    path::PathBuf,
    ptr::{null_mut, NonNull},
    rc::Rc,
};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::*, Threading::GetCurrentProcessId},
};

pub struct PinnedTimedLibrary {
    pub path: PathBuf,
    pub sha256: String,
}
/// No Debug/Serialize: the transient capability is native-owner data only.
pub struct TimedEndpoint(NativeOutputEndpoint);
impl TimedEndpoint {
    pub fn activation_blob(&self) -> &[u8; 236] {
        self.0.activation_blob()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Load(Reason),
    Protocol,
    Native(i32),
    Owner,
    Bounds,
    Off,
    Permission,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Waiting,
    Ready,
    Reserved,
    Sending,
    Receipt,
    Stopping,
    Stopped,
    Fault,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Admitted,
    Rejected(i32),
    Unknown(i32),
    Cancelled,
}
#[derive(Clone, Copy, Debug)]
pub struct Status {
    pub phase: Phase,
    pub generation: u64,
    pub serial: u64,
    pub submitted: u64,
    pub admitted: u64,
    pub rejected: u64,
    pub cancelled: u64,
    pub local_clear: bool,
    pub external_drain_verified: bool,
    pub pending: bool,
    pub error: i32,
}
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct RawTicket {
    owner: u64,
    generation: u64,
    serial: u64,
}
#[repr(C)]
#[derive(Default)]
struct RawFrame {
    id: u64,
    capture: u64,
    presentation: u64,
    flags: u32,
    reserved: u32,
}
#[repr(C)]
#[derive(Default)]
struct RawReceipt {
    version: u32,
    outcome: u32,
    generation: u64,
    serial: u64,
    error: i32,
    reserved: u32,
}
#[repr(C)]
#[derive(Default)]
struct RawStatus {
    version: u32,
    phase: u32,
    generation: u64,
    serial: u64,
    submitted: u64,
    admitted: u64,
    rejected: u64,
    cancelled: u64,
    local_clear: u32,
    external_drain: u32,
    pending: u32,
    error: i32,
    reserved: [u32; 2],
}
const _: () = assert!(std::mem::size_of::<RawTicket>() == 24);
const _: () = assert!(std::mem::size_of::<RawFrame>() == 32);
const _: () = assert!(std::mem::size_of::<RawReceipt>() == 32);
const _: () = assert!(std::mem::size_of::<RawStatus>() == 80);
type Abi = unsafe extern "system" fn() -> u32;
type Create = unsafe extern "system" fn() -> *mut c_void;
type Open = unsafe extern "system" fn(*mut c_void, u64, *mut u8, u32) -> i32;
type Reserve = unsafe extern "system" fn(*mut c_void, u64, *mut RawTicket, u32) -> i32;
type Stage = unsafe extern "system" fn(
    *mut c_void,
    *const RawTicket,
    u32,
    *const RawFrame,
    u32,
    *const u8,
    u32,
) -> i32;
type Take =
    unsafe extern "system" fn(*mut c_void, *const RawTicket, u32, *mut RawReceipt, u32) -> i32;
type GetStatus = unsafe extern "system" fn(*mut c_void, *mut RawStatus, u32) -> i32;
type Event = unsafe extern "system" fn(*mut c_void, *mut HANDLE) -> i32;
type Cancel = unsafe extern "system" fn(*mut c_void);
type Action = unsafe extern "system" fn(*mut c_void) -> i32;
struct Library {
    module: HMODULE,
    _verified: Verified,
    create: Create,
    open: Open,
    reserve: Reserve,
    stage: Stage,
    take: Take,
    status: GetStatus,
    event: Event,
    cancel: Cancel,
    stop: Action,
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
    fn load(pin: PinnedTimedLibrary) -> Result<Self, Error> {
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
        let count =
            unsafe { GetModuleFileNameW(module.0, actual.as_mut_ptr(), actual.len() as u32) }
                as usize;
        if count == 0
            || count >= actual.len()
            || String::from_utf16_lossy(&actual[..count]).to_lowercase()
                != normalized.to_lowercase()
        {
            return Err(Error::Load(Reason::Integrity));
        }
        macro_rules! symbol {
            ($name:literal,$ty:ty) => {{
                let p = unsafe { GetProcAddress(module.0, concat!($name, "\0").as_ptr()) }
                    .ok_or(Error::Protocol)?;
                unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, $ty>(p) }
            }};
        }
        let abi = symbol!("TxTimedAbi", Abi);
        if unsafe { abi() } != 2 {
            return Err(Error::Protocol);
        }
        let result = Self {
            module: module.0,
            _verified: verified,
            create: symbol!("TxTimedCreate", Create),
            open: symbol!("TxTimedOpen", Open),
            reserve: symbol!("TxTimedReserve", Reserve),
            stage: symbol!("TxTimedStage", Stage),
            take: symbol!("TxTimedTake", Take),
            status: symbol!("TxTimedStatus", GetStatus),
            event: symbol!("TxTimedEvent", Event),
            cancel: symbol!("TxTimedCancel", Cancel),
            stop: symbol!("TxTimedStop", Action),
            destroy: symbol!("TxTimedDestroy", Action),
        };
        module.0 = null_mut();
        Ok(result)
    }
}
fn check(hr: i32) -> Result<(), Error> {
    if hr == 0 {
        Ok(())
    } else {
        Err(Error::Native(hr))
    }
}
pub struct Reservation {
    identity: Rc<()>,
    raw: RawTicket,
}
pub struct Flight {
    identity: Rc<()>,
    raw: RawTicket,
    consumed: bool,
}
#[derive(Clone, Copy)]
struct CancelTarget {
    raw: *mut c_void,
    cancel: Cancel,
}
/// Owner-lane-only revocation capability. It cannot start/output/wait and turns
/// inert BEFORE the DLL/instance is destroyed. Rc/Cell prevent cross-thread use.
#[derive(Clone)]
pub(crate) struct Cancellation(Rc<std::cell::Cell<Option<CancelTarget>>>);
impl Cancellation {
    pub(crate) fn cancel(&self) {
        if let Some(target) = self.0.get() {
            unsafe {
                (target.cancel)(target.raw);
            }
        }
    }
}
/// !Send/!Sync: constructed, operated and destroyed on the native owner lane.
/// A borrow of its wake event cannot outlive the library/owner. Drop retires the
/// sender before freeing either. Recreating this object does not verify drain.
pub struct NativeTimedVideo {
    raw: NonNull<c_void>,
    library: Library,
    identity: Rc<()>,
    generation: u64,
    stopped: bool,
    cancellation: Cancellation,
}
impl Drop for NativeTimedVideo {
    fn drop(&mut self) {
        self.cancellation.0.set(None);
        unsafe {
            (self.library.cancel)(self.raw.as_ptr());
            (self.library.stop)(self.raw.as_ptr());
            (self.library.destroy)(self.raw.as_ptr());
        }
    }
}
impl NativeTimedVideo {
    pub fn load(pin: PinnedTimedLibrary) -> Result<Self, Error> {
        let library = Library::load(pin)?;
        let raw = NonNull::new(unsafe { (library.create)() }).ok_or(Error::Load(Reason::Launch))?;
        let cancellation = Cancellation(Rc::new(std::cell::Cell::new(Some(CancelTarget {
            raw: raw.as_ptr(),
            cancel: library.cancel,
        }))));
        Ok(Self {
            raw,
            library,
            identity: Rc::new(()),
            generation: 0,
            stopped: false,
            cancellation,
        })
    }
    pub(crate) fn cancellation(&self) -> Cancellation {
        self.cancellation.clone()
    }
    pub fn open(&mut self, generation: u64, explicit_intent: bool) -> Result<TimedEndpoint, Error> {
        if !explicit_intent {
            return Err(Error::Permission);
        }
        if self.stopped || self.generation != 0 {
            return Err(Error::Off);
        }
        if generation == 0 {
            return Err(Error::Bounds);
        }
        let mut bytes = [0u8; 236];
        let hr =
            unsafe { (self.library.open)(self.raw.as_ptr(), generation, bytes.as_mut_ptr(), 236) };
        let endpoint = if hr == 0 {
            NativeOutputEndpoint::parse_version(&bytes, unsafe { GetCurrentProcessId() }, 2)
                .map_err(Error::Load)
        } else {
            Err(Error::Native(hr))
        };
        for b in &mut bytes {
            unsafe {
                std::ptr::write_volatile(b, 0);
            }
        }
        match endpoint {
            Ok(endpoint) => {
                self.generation = generation;
                Ok(TimedEndpoint(endpoint))
            }
            Err(error) => {
                let _ = self.stop();
                Err(error)
            }
        }
    }
    /// Wait with audio readiness and the owner's stop event. Auto reset/single
    /// waiter; an event is only a notification, so always inspect state/receipt.
    pub fn event(&self) -> Result<BorrowedHandle<'_>, Error> {
        let mut event = null_mut();
        check(unsafe { (self.library.event)(self.raw.as_ptr(), &mut event) })?;
        if event.is_null() || event == INVALID_HANDLE_VALUE {
            return Err(Error::Protocol);
        }
        Ok(unsafe { BorrowedHandle::borrow_raw(event) })
    }
    pub fn reserve(&mut self) -> Result<Option<Reservation>, Error> {
        if self.stopped || self.generation == 0 {
            return Err(Error::Off);
        }
        let mut raw = RawTicket::default();
        let hr =
            unsafe { (self.library.reserve)(self.raw.as_ptr(), self.generation, &mut raw, 24) };
        if hr == 1 {
            return Ok(None);
        }
        check(hr)?;
        if raw.owner == 0 || raw.generation != self.generation || raw.serial == 0 {
            let _ = self.stop();
            return Err(Error::Protocol);
        }
        Ok(Some(Reservation {
            identity: self.identity.clone(),
            raw,
        }))
    }
    /// Bounded NV12 copy only. The native sender performs pipe I/O outside the
    /// timeline callback. Retain original capture from the selected picture.
    pub fn stage(
        &mut self,
        ticket: Reservation,
        window: &RenderedWindow<'_>,
    ) -> Result<Flight, Error> {
        if !Rc::ptr_eq(&ticket.identity, &self.identity) {
            return Err(Error::Owner);
        }
        if self.stopped {
            return Err(Error::Off);
        }
        if window.generation != self.generation {
            return Err(Error::Owner);
        }
        let raw = RawFrame {
            id: window.video.map_or(0, |f| f.id),
            capture: window.video.map_or(0, |f| f.capture.qpc_100ns()),
            presentation: window.presentation.qpc_100ns(),
            flags: u32::from(window.video.is_none()),
            reserved: 0,
        };
        let pixels = window.video.map(|f| f.nv12.as_ref());
        if pixels.is_some_and(|p| p.len() != NV12_BYTES) {
            self.stopped = true;
            self.cancel();
            return Err(Error::Bounds);
        }
        let hr = unsafe {
            (self.library.stage)(
                self.raw.as_ptr(),
                &ticket.raw,
                24,
                &raw,
                32,
                pixels.map_or(std::ptr::null(), |p| p.as_ptr()),
                pixels.map_or(0, |p| p.len() as u32),
            )
        };
        if let Err(error) = check(hr) {
            self.stopped = true;
            self.cancel();
            return Err(error);
        }
        Ok(Flight {
            identity: ticket.identity,
            raw: ticket.raw,
            consumed: false,
        })
    }
    pub fn take(&mut self, flight: &mut Flight) -> Result<Option<Admission>, Error> {
        if !Rc::ptr_eq(&flight.identity, &self.identity) || flight.consumed {
            return Err(Error::Owner);
        }
        let mut raw = RawReceipt::default();
        let hr = unsafe { (self.library.take)(self.raw.as_ptr(), &flight.raw, 24, &mut raw, 32) };
        if hr == 1 {
            return Ok(None);
        }
        check(hr)?;
        flight.consumed = true;
        if raw.version != 2
            || raw.reserved != 0
            || raw.generation != self.generation
            || raw.serial != flight.raw.serial
        {
            return Err(Error::Protocol);
        }
        let result = match (raw.outcome, raw.error) {
            (1, 0) => Admission::Admitted,
            (2, hr) if hr < 0 => Admission::Rejected(hr),
            (3, hr) if hr < 0 => Admission::Unknown(hr),
            (4, hr) if hr < 0 => Admission::Cancelled,
            _ => return Err(Error::Protocol),
        };
        Ok(Some(result))
    }
    pub fn cancel(&self) {
        unsafe {
            (self.library.cancel)(self.raw.as_ptr());
        }
    }
    pub fn stop(&mut self) -> Result<Status, Error> {
        self.stopped = true;
        self.cancel();
        check(unsafe { (self.library.stop)(self.raw.as_ptr()) })?;
        self.status()
    }
    pub fn status(&self) -> Result<Status, Error> {
        let mut s = RawStatus::default();
        check(unsafe { (self.library.status)(self.raw.as_ptr(), &mut s, 80) })?;
        if s.version != 2
            || s.reserved != [0, 0]
            || s.local_clear > 1
            || s.external_drain != 0
            || s.pending > 1
            || s.admitted > s.submitted
        {
            return Err(Error::Protocol);
        }
        let phase = match s.phase {
            0 => Phase::Idle,
            1 => Phase::Waiting,
            2 => Phase::Ready,
            3 => Phase::Reserved,
            4 => Phase::Sending,
            5 => Phase::Receipt,
            6 => Phase::Stopping,
            7 => Phase::Stopped,
            8 => Phase::Fault,
            _ => return Err(Error::Protocol),
        };
        Ok(Status {
            phase,
            generation: s.generation,
            serial: s.serial,
            submitted: s.submitted,
            admitted: s.admitted,
            rejected: s.rejected,
            cancelled: s.cancelled,
            local_clear: s.local_clear != 0,
            external_drain_verified: false,
            pending: s.pending != 0,
            error: s.error,
        })
    }
}
#[cfg(test)]
pub(crate) mod tests;
