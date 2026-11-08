//! Native virtual camera lifetime, separate from timed frame admission.
//! The caller owns an MTA lane; no UI command, restored start or registration.
use super::{Error, Library, PinnedTimedLibrary};
use crate::joint::host::Input;
use std::{
    ffi::c_void,
    ptr::NonNull,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};
use windows_sys::Win32::System::LibraryLoader::GetProcAddress;

type Revoke = unsafe extern "system" fn(*mut c_void);
type Create = unsafe extern "system" fn(Revoke, *mut c_void) -> *mut c_void;
type Start = unsafe extern "system" fn(*mut c_void, *const u8, u32, u32) -> i32;
type Action = unsafe extern "system" fn(*mut c_void) -> i32;
type Cancel = unsafe extern "system" fn(*mut c_void);
type Read = unsafe extern "system" fn(*mut c_void, *mut Status, u32) -> i32;
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Status {
    pub version: u32,
    /// 0 idle, 1 starting, 2 Windows available, 3 stopping, 4 fault.
    /// Available does not prove a consumer received frames.
    pub state: u32,
    pub operation: i32,
    pub cleanup: i32,
    pub revoked: u32,
    pub used: u32,
}
const _: () = assert!(std::mem::size_of::<Status>() == 24);
struct Api {
    create: Create,
    start: Start,
    cancel: Cancel,
    read: Read,
    stop: Action,
    destroy: Action,
}
impl Api {
    fn load(library: &Library) -> Result<Self, Error> {
        macro_rules! symbol {
            ($name:literal, $ty:ty) => {{
                let p = unsafe { GetProcAddress(library.module, concat!($name, "\0").as_ptr()) }
                    .ok_or(Error::Protocol)?;
                unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, $ty>(p) }
            }};
        }
        let abi = symbol!("TxDeviceAbi", unsafe extern "system" fn() -> u32);
        if unsafe { abi() } != 1 {
            return Err(Error::Protocol);
        }
        Ok(Self {
            create: symbol!("TxDeviceCreate", Create),
            start: symbol!("TxDeviceStart", Start),
            cancel: symbol!("TxDeviceCancel", Cancel),
            read: symbol!("TxDeviceStatusGet", Read),
            stop: symbol!("TxDeviceStop", Action),
            destroy: symbol!("TxDeviceDestroy", Action),
        })
    }
}
struct Context(Box<dyn Fn() + Send + Sync>);
unsafe extern "system" fn revoke(raw: *mut c_void) {
    // Only internally constructed, non-panicking native revocation closures.
    // extern system cannot unwind into C++; the Box survives failed cleanup.
    unsafe {
        ((*(raw as *const Context)).0)();
    }
}
#[derive(Clone, Copy)]
struct Target {
    raw: *mut c_void,
    cancel: Cancel,
}
// The only cross-thread native operation is explicitly thread-safe Cancel.
unsafe impl Send for Target {}
#[derive(Clone)]
pub struct Cancellation(Arc<Mutex<Option<Target>>>);
impl Cancellation {
    pub fn cancel(&self) {
        let guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = *guard {
            unsafe {
                (t.cancel)(t.raw);
            }
        }
    }
}
struct Owned {
    raw: NonNull<c_void>,
    api: Api,
    _context: Box<Context>,
    _library: Library,
}
/// !Send/!Sync. Stop/destroy on the creating MTA. A cancellation clone has no
/// start authority and becomes inert before destruction/unload. Failed destroy
/// retains the complete instance/module/context, including on Rust Drop.
pub struct NativeDevice {
    owned: Option<Box<Owned>>,
    cancellation: Cancellation,
    input: Option<Arc<Input>>,
    _owner: Rc<()>,
}
impl NativeDevice {
    pub fn load(pin: PinnedTimedLibrary, input: Arc<Input>) -> Result<Self, Error> {
        let exact = input.clone();
        let mut device = Self::load_with_revoke(pin, Box::new(move || exact.request_stop()))?;
        device.input = Some(input);
        Ok(device)
    }
    fn load_with_revoke(
        pin: PinnedTimedLibrary,
        callback: Box<dyn Fn() + Send + Sync>,
    ) -> Result<Self, Error> {
        let library = Library::load(pin)?;
        let api = Api::load(&library)?;
        let mut context = Box::new(Context(callback));
        let raw =
            NonNull::new(unsafe { (api.create)(revoke, (&mut *context as *mut Context).cast()) })
                .ok_or(Error::Off)?;
        let cancellation = Cancellation(Arc::new(Mutex::new(Some(Target {
            raw: raw.as_ptr(),
            cancel: api.cancel,
        }))));
        Ok(Self {
            owned: Some(Box::new(Owned {
                raw,
                api,
                _context: context,
                _library: library,
            })),
            cancellation,
            input: None,
            _owner: Rc::new(()),
        })
    }
    pub fn cancellation(&self) -> Cancellation {
        self.cancellation.clone()
    }
    /// Gets the v2 endpoint from this exact Input lease, never from WebView data.
    /// Separate device owner lane prevents Windows Start from blocking audio.
    pub fn start(&mut self, explicit_intent: bool, timeout: Duration) -> Result<(), Error> {
        if !explicit_intent {
            self.cancellation.cancel();
            return Err(Error::Permission);
        }
        let input = self.input.as_ref().ok_or(Error::Off)?;
        let endpoint = match input.take_endpoint(timeout) {
            Ok(value) => value,
            Err(_) => {
                self.cancellation.cancel();
                return Err(Error::Off);
            }
        };
        let owned = self.owned.as_ref().ok_or(Error::Off)?;
        super::check(unsafe {
            (owned.api.start)(
                owned.raw.as_ptr(),
                endpoint.activation_blob().as_ptr(),
                236,
                1,
            )
        })
    }
    pub fn status(&self) -> Result<Status, Error> {
        let o = self.owned.as_ref().ok_or(Error::Off)?;
        let mut status = Status::default();
        super::check(unsafe { (o.api.read)(o.raw.as_ptr(), &mut status, 24) })?;
        if status.version != 1 || status.state > 4 || status.revoked > 1 || status.used > 1 {
            return Err(Error::Protocol);
        }
        Ok(status)
    }
    pub fn stop(&mut self) -> Result<(), Error> {
        self.cancellation.cancel();
        let Some(o) = &self.owned else {
            return Ok(());
        };
        super::check(unsafe { (o.api.stop)(o.raw.as_ptr()) })
    }
    pub fn close(&mut self) -> Result<(), Error> {
        self.stop()?;
        let mut guard = self
            .cancellation
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(o) = &self.owned {
            super::check(unsafe { (o.api.destroy)(o.raw.as_ptr()) })?;
        }
        *guard = None;
        drop(guard);
        self.owned = None;
        self.input = None;
        Ok(())
    }
}
impl Drop for NativeDevice {
    fn drop(&mut self) {
        if self.close().is_err() {
            // No unload or freed callback after uncertain OS cleanup. Explicit
            // close can retry; abandoned Drop intentionally retains until exit.
            *self
                .cancellation
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
            if let Some(o) = self.owned.take() {
                std::mem::forget(o);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn real_device_bridge_is_inert_and_cancel_survives_owner_teardown() {
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let mut device = NativeDevice::load_with_revoke(
            super::super::tests::pin(),
            Box::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
        assert_eq!(
            device.status().unwrap(),
            Status {
                version: 1,
                ..Status::default()
            }
        );
        let token = device.cancellation();
        let other = token.clone();
        std::thread::spawn(move || other.cancel()).join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(device.status().unwrap().revoked, 1);
        device.close().unwrap();
        drop(device);
        token.cancel();
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn real_device_bridge_permission_denial_revokes_without_activating_device() {
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let mut device = NativeDevice::load_with_revoke(
            super::super::tests::pin(),
            Box::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
        assert_eq!(device.start(false, Duration::ZERO), Err(Error::Permission));
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(device.status().unwrap().used, 0);
        device.close().unwrap();
    }
}
