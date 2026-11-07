//! Owner-thread native audio adapter. No capture, webview command, automatic
//! start, virtual-device installation or claim of paired video output.
use crate::{
    av_timeline::{
        DeviceClock, Error as ClockError, OutputInterval, Time, MAX_PACKET_FRAMES, RATE,
    },
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
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows_sys::Win32::{Foundation::*, System::LibraryLoader::*};

/// Native build pin. Never deserialize a webview/settings-provided DLL or digest.
pub struct PinnedAudioLibrary {
    pub path: PathBuf,
    pub sha256: String,
}
pub struct AudioPair {
    pub render_id: String,
    pub capture_id: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Load(Reason),
    Protocol,
    Clock(ClockError),
    Native(i32),
    Owner,
    Bounds,
    Off,
}
#[derive(Clone, Copy, Debug)]
pub struct Format {
    pub channels: u32,
    pub buffer_frames: u32,
    pub clock_frequency: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct Status {
    pub phase: u32,
    pub submitted_frames: u64,
    pub operation_error: i32,
    pub stop_error: i32,
    pub reset_error: i32,
    pub client_reset: bool,
    pub external_drain_verified: bool,
    pub revoked: bool,
}
/// Notification only; a joint owner additionally attaches an owner-lane video
/// cancellation capability before prepare. This flag is not a drain proof.
pub struct Revocation(AtomicBool);
struct Callback {
    state: Arc<Revocation>,
    video: Option<crate::timed_video::Cancellation>,
}
impl Revocation {
    pub fn revoked(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[repr(C)]
#[derive(Default)]
struct RawFormat {
    version: u32,
    rate: u32,
    channels: u32,
    buffer_frames: u32,
    clock_frequency: u64,
}
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct RawClock {
    position: u64,
    qpc_100ns: u64,
    padding: u32,
    reserved: u32,
}
#[repr(C)]
#[derive(Default)]
struct RawTicket {
    generation: u64,
    serial: u64,
    first_frame: u64,
    frames: u32,
    reserved: u32,
    clock: RawClock,
}
#[repr(C)]
#[derive(Default)]
struct RawStatus {
    version: u32,
    phase: u32,
    submitted_frames: u64,
    operation_error: i32,
    stop_error: i32,
    reset_error: i32,
    client_reset: u32,
    external_drain: u32,
    revoked: u32,
    reserved: [u32; 2],
}
const _: () = assert!(std::mem::size_of::<RawFormat>() == 24);
const _: () = assert!(std::mem::size_of::<RawClock>() == 24);
const _: () = assert!(std::mem::size_of::<RawTicket>() == 56);
const _: () = assert!(std::mem::size_of::<RawStatus>() == 48);
type Abi = unsafe extern "system" fn() -> u32;
type Revoke = unsafe extern "system" fn(*mut c_void);
type Create = unsafe extern "system" fn(Revoke, *mut c_void) -> *mut c_void;
type Prepare = unsafe extern "system" fn(
    *mut c_void,
    *const u16,
    u32,
    *const u16,
    u32,
    u32,
    u64,
    *mut RawFormat,
    u32,
) -> i32;
type Start = unsafe extern "system" fn(*mut c_void, u64, u32) -> i32;
type Observe = unsafe extern "system" fn(*mut c_void, u64, *mut RawClock, u32) -> i32;
type Reserve = unsafe extern "system" fn(*mut c_void, u64, u32, *mut RawTicket, u32) -> i32;
type Commit =
    unsafe extern "system" fn(*mut c_void, *const RawTicket, u32, u32, *const f32, u32) -> i32;
type Action = unsafe extern "system" fn(*mut c_void) -> i32;
type GetStatus = unsafe extern "system" fn(*mut c_void, *mut RawStatus, u32) -> i32;
type ReadyEvent = unsafe extern "system" fn(*mut c_void, *mut HANDLE) -> i32;
struct Library {
    module: HMODULE,
    _verified: Verified,
    create: Create,
    prepare: Prepare,
    start: Start,
    observe: Observe,
    reserve: Reserve,
    commit: Commit,
    stop: Action,
    status: GetStatus,
    destroy: Action,
    ready_event: ReadyEvent,
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
    fn load(pin: PinnedAudioLibrary) -> Result<Self, Error> {
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
            ($name:literal, $ty:ty) => {{
                let p = unsafe { GetProcAddress(module.0, concat!($name, "\0").as_ptr()) }
                    .ok_or(Error::Protocol)?;
                unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, $ty>(p) }
            }};
        }
        let abi = symbol!("TxAudioAbi", Abi);
        if unsafe { abi() } != 1 {
            return Err(Error::Protocol);
        }
        let library = Self {
            module: module.0,
            _verified: verified,
            create: symbol!("TxAudioCreate", Create),
            prepare: symbol!("TxAudioPrepare", Prepare),
            start: symbol!("TxAudioStart", Start),
            observe: symbol!("TxAudioObserve", Observe),
            reserve: symbol!("TxAudioReserve", Reserve),
            commit: symbol!("TxAudioCommit", Commit),
            stop: symbol!("TxAudioStop", Action),
            status: symbol!("TxAudioGetStatus", GetStatus),
            destroy: symbol!("TxAudioDestroy", Action),
            ready_event: symbol!("TxAudioReadyEvent", ReadyEvent),
        };
        module.0 = null_mut();
        Ok(library)
    }
}
unsafe extern "system" fn revoke(context: *mut c_void) {
    // Session invokes this only on its creator MTA lane, before audio cleanup.
    // No allocation/wait or audio reentry. The video capability is inert if its
    // owner is already destroyed; it cannot retain an unloaded function pointer.
    let callback = unsafe { &*(context as *const Callback) };
    callback.state.0.store(true, Ordering::Release);
    if let Some(video) = callback.video.as_ref() {
        video.cancel();
    }
}
fn check(hr: i32) -> Result<(), Error> {
    if hr == 0 {
        Ok(())
    } else {
        Err(Error::Native(hr))
    }
}
fn endpoint(value: &str) -> Result<Vec<u16>, Error> {
    let wide: Vec<u16> = value.encode_utf16().collect();
    if wide.is_empty() || wide.len() > 1024 || wide.contains(&0) {
        return Err(Error::Bounds);
    }
    Ok(wide)
}
pub(crate) fn native_now() -> Result<Time, Error> {
    let (mut counter, mut frequency) = (0i64, 0i64);
    if unsafe { QueryPerformanceCounter(&mut counter) } == 0
        || unsafe { QueryPerformanceFrequency(&mut frequency) } == 0
        || counter < 0
        || frequency <= 0
    {
        return Err(Error::Clock(ClockError::Clock));
    }
    Time::from_qpc_ticks(counter as u64, frequency as u64).map_err(Error::Clock)
}

/// Ticket fields cannot be forged by safe callers. A ticket belongs to one
/// instance and may only be committed once; it retains no audio/video samples.
pub struct Reservation {
    owner: Rc<()>,
    raw: RawTicket,
    interval: Option<OutputInterval>,
    _owner_thread: PhantomData<Rc<()>>,
}
impl Reservation {
    pub fn take_interval(&mut self) -> Option<OutputInterval> {
        self.interval.take()
    }
    pub fn first_frame(&self) -> u64 {
        self.raw.first_frame
    }
    pub fn frames(&self) -> usize {
        self.raw.frames as usize
    }
}

/// !Send/!Sync by construction. Construct and drop on the same native MTA lane;
/// a UI/controller may observe the separate revocation flag but cannot call FFI.
pub struct NativeAudio {
    raw: NonNull<c_void>,
    library: Library,
    callback: Box<Callback>,
    generation: u64,
    format: Option<Format>,
    clock: Option<DeviceClock>,
    stopped: bool,
    identity: Rc<()>,
    _owner_thread: PhantomData<Rc<()>>,
}
impl Drop for NativeAudio {
    fn drop(&mut self) {
        unsafe {
            (self.library.stop)(self.raw.as_ptr());
            (self.library.destroy)(self.raw.as_ptr());
        }
        // library and callback are still live until after the native owner is gone.
    }
}
impl NativeAudio {
    /// Loading our pinned module/creating an idle owner starts no audio client.
    pub fn load(pin: PinnedAudioLibrary) -> Result<Self, Error> {
        let library = Library::load(pin)?;
        let mut callback = Box::new(Callback {
            state: Arc::new(Revocation(AtomicBool::new(false))),
            video: None,
        });
        let raw = NonNull::new(unsafe {
            (library.create)(revoke, (&mut *callback as *mut Callback).cast())
        })
        .ok_or(Error::Load(Reason::Launch))?;
        Ok(Self {
            raw,
            library,
            callback,
            generation: 0,
            format: None,
            clock: None,
            stopped: false,
            identity: Rc::new(()),
            _owner_thread: PhantomData,
        })
    }
    /// Explicit native selection only; may initialize an output client. Caller
    /// must have output authorization and a joint owner's durable drain barrier.
    pub fn prepare(
        &mut self,
        pair: &AudioPair,
        channels: u32,
        generation: u64,
    ) -> Result<Format, Error> {
        if self.stopped || self.format.is_some() || generation == 0 {
            return Err(Error::Off);
        }
        let render = endpoint(&pair.render_id)?;
        let capture = endpoint(&pair.capture_id)?;
        let mut raw = RawFormat::default();
        check(unsafe {
            (self.library.prepare)(
                self.raw.as_ptr(),
                render.as_ptr(),
                render.len() as u32,
                capture.as_ptr(),
                capture.len() as u32,
                channels,
                generation,
                &mut raw,
                24,
            )
        })?;
        if raw.version != 1
            || raw.rate != RATE
            || raw.channels != channels
            || !(1..=2).contains(&channels)
            || !(960..=4800).contains(&raw.buffer_frames)
            || !(1..=1_000_000_000).contains(&raw.clock_frequency)
        {
            let _ = self.stop();
            return Err(Error::Protocol);
        }
        let format = Format {
            channels,
            buffer_frames: raw.buffer_frames,
            clock_frequency: raw.clock_frequency,
        };
        self.generation = generation;
        self.clock = Some(DeviceClock::new(generation, raw.clock_frequency).map_err(Error::Clock)?);
        self.format = Some(format);
        Ok(format)
    }
    pub fn start(&mut self, explicit_intent: bool) -> Result<(), Error> {
        if self.stopped || self.format.is_none() {
            return Err(Error::Off);
        }
        check(unsafe {
            (self.library.start)(
                self.raw.as_ptr(),
                self.generation,
                u32::from(explicit_intent),
            )
        })
    }
    pub fn revocation(&self) -> Arc<Revocation> {
        self.callback.state.clone()
    }
    pub(crate) fn couple_video(
        &mut self,
        video: crate::timed_video::Cancellation,
    ) -> Result<(), Error> {
        if self.stopped || self.format.is_some() || self.callback.video.is_some() {
            return Err(Error::Off);
        }
        self.callback.video = Some(video);
        Ok(())
    }
    /// Borrow only while this owner lives. Wait together with the joint owner's
    /// stop signal, then release the borrow before reserve/commit. No polling loop.
    pub fn ready_event(&self) -> Result<Option<BorrowedHandle<'_>>, Error> {
        if self.stopped {
            return Err(Error::Off);
        }
        let mut event = null_mut();
        check(unsafe { (self.library.ready_event)(self.raw.as_ptr(), &mut event) })?;
        if event.is_null() {
            Ok(None)
        } else if event == INVALID_HANDLE_VALUE {
            Err(Error::Protocol)
        } else {
            Ok(Some(unsafe { BorrowedHandle::borrow_raw(event) }))
        }
    }
    fn observe(&mut self, raw: RawClock) -> Result<(), Error> {
        if raw.reserved != 0 || raw.padding > self.format.ok_or(Error::Off)?.buffer_frames {
            return Err(Error::Protocol);
        }
        self.clock
            .as_mut()
            .ok_or(Error::Off)?
            .observe(raw.position, Time::from_qpc_100ns(raw.qpc_100ns))
            .map_err(Error::Clock)
    }
    /// None means clock warmup or backpressure; no source PCM is consumed. The
    /// owner waits for the next device event, not a busy loop or wall-clock guess.
    pub fn reserve(&mut self, frames: usize) -> Result<Option<Reservation>, Error> {
        self.reserve_using(frames, native_now)
    }
    // Only the isolated native fixture uses a deterministic clock. Product code
    // samples QPC after the device observation, never a pre-call/UI timestamp.
    #[cfg(test)]
    fn reserve_at(&mut self, frames: usize, now: Time) -> Result<Option<Reservation>, Error> {
        self.reserve_using(frames, || Ok(now))
    }
    fn reserve_using(
        &mut self,
        frames: usize,
        now: impl FnOnce() -> Result<Time, Error>,
    ) -> Result<Option<Reservation>, Error> {
        if self.stopped || self.clock.is_none() || self.callback.state.revoked() {
            return Err(Error::Off);
        }
        if frames == 0 || frames > MAX_PACKET_FRAMES {
            return Err(Error::Bounds);
        }
        let result = self.reserve_inner(frames, now);
        if result.is_err() {
            let _ = self.stop();
        }
        result
    }
    fn reserve_inner(
        &mut self,
        frames: usize,
        now: impl FnOnce() -> Result<Time, Error>,
    ) -> Result<Option<Reservation>, Error> {
        if !self.clock.as_ref().unwrap().is_ready() {
            let mut raw = RawClock::default();
            check(unsafe {
                (self.library.observe)(self.raw.as_ptr(), self.generation, &mut raw, 24)
            })?;
            self.observe(raw)?;
            if !self.clock.as_ref().unwrap().is_ready() {
                return Ok(None);
            }
        }
        let mut raw = RawTicket::default();
        let hr = unsafe {
            (self.library.reserve)(
                self.raw.as_ptr(),
                self.generation,
                frames as u32,
                &mut raw,
                56,
            )
        };
        if hr == 1 {
            return Ok(None);
        }
        check(hr)?;
        if raw.generation != self.generation
            || raw.serial == 0
            || raw.frames as usize != frames
            || raw.reserved != 0
        {
            return Err(Error::Protocol);
        }
        self.observe(raw.clock)?;
        let now = now()?;
        let interval = self
            .clock
            .as_ref()
            .unwrap()
            .interval_from_frame(raw.first_frame, frames, now)
            .map_err(Error::Clock)?;
        Ok(Some(Reservation {
            owner: self.identity.clone(),
            raw,
            interval: Some(interval),
            _owner_thread: PhantomData,
        }))
    }
    /// Only pass the original-voice window admitted by Timeline. This method
    /// does not authorize capture or independently prove its upstream source.
    pub fn commit(&mut self, reservation: Reservation, samples: &[f32]) -> Result<(), Error> {
        if !Rc::ptr_eq(&reservation.owner, &self.identity) {
            return Err(Error::Owner);
        }
        if self.stopped || self.callback.state.revoked() {
            return Err(Error::Off);
        }
        if reservation.interval.is_some() || samples.len() != reservation.frames() {
            let _ = self.stop();
            return Err(Error::Bounds);
        }
        let result = check(unsafe {
            (self.library.commit)(
                self.raw.as_ptr(),
                &reservation.raw,
                56,
                0,
                samples.as_ptr(),
                samples.len() as u32,
            )
        });
        if result.is_err() {
            let _ = self.stop();
        }
        result
    }
    /// Cleanup HRESULTs remain visible even if client_reset or external_drain
    /// is false. Returning a Status is not itself successful end-to-end stop.
    pub fn stop(&mut self) -> Result<Status, Error> {
        self.stopped = true;
        self.clock = None;
        let hr = unsafe { (self.library.stop)(self.raw.as_ptr()) };
        let status = self.status()?;
        if hr != 0 && status.stop_error == 0 && status.reset_error == 0 {
            return Err(Error::Native(hr));
        }
        Ok(status)
    }
    pub fn status(&self) -> Result<Status, Error> {
        let mut s = RawStatus::default();
        check(unsafe { (self.library.status)(self.raw.as_ptr(), &mut s, 48) })?;
        if s.version != 1
            || s.phase > 4
            || s.reserved != [0, 0]
            || s.client_reset > 1
            || s.external_drain > 1
            || s.revoked > 1
            || (s.external_drain == 1 && s.client_reset == 0)
        {
            return Err(Error::Protocol);
        }
        Ok(Status {
            phase: s.phase,
            submitted_frames: s.submitted_frames,
            operation_error: s.operation_error,
            stop_error: s.stop_error,
            reset_error: s.reset_error,
            client_reset: s.client_reset == 1,
            external_drain_verified: s.external_drain == 1,
            revoked: s.revoked == 1,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests;
