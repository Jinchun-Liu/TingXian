//! Dedicated virtual device MTA, separate from the continuous media lane.
//! Explicit native authority only. Default/status/resume have no side effects.
use super::{Error as NativeError, NativeDevice, PinnedTimedLibrary, Status};
use crate::joint::host::Input;
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    ptr::null,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Permission,
    Stale,
    Suspended,
    Busy,
    Closed,
    Thread,
    Apartment(i32),
    Native(NativeError),
    CleanupUncertain,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Off,
    Preparing,
    Available,
    Stopping,
    CleanupBlocked,
    Retired,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub generation: u64,
    pub input_generation: Option<u64>,
    pub busy: bool,
    pub suspended: bool,
    pub cleanup_uncertain: bool,
    pub phase: Phase,
    pub failure: Option<Error>,
    pub last_cleanup_error: Option<Error>,
    pub cleanup_attempts: u64,
    pub native: Option<Status>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            generation: 0,
            input_generation: None,
            busy: false,
            suspended: false,
            cleanup_uncertain: false,
            phase: Phase::Off,
            failure: None,
            last_cleanup_error: None,
            cleanup_attempts: 0,
            native: None,
        }
    }
}
type Revoke = Arc<dyn Fn() + Send + Sync>;
// Internal fault-injection seam. No WebView factory or alternate production mode.
trait Device {
    fn cancellation(&self) -> Revoke;
    fn start(&mut self) -> Result<(), NativeError>;
    fn status(&self) -> Result<Status, NativeError>;
    fn close(&mut self) -> Result<(), NativeError>;
}
impl Device for NativeDevice {
    fn cancellation(&self) -> Revoke {
        let token = self.cancellation();
        Arc::new(move || token.cancel())
    }
    fn start(&mut self) -> Result<(), NativeError> {
        self.start(true, Duration::from_secs(3))
    }
    fn status(&self) -> Result<Status, NativeError> {
        self.status()
    }
    fn close(&mut self) -> Result<(), NativeError> {
        self.close()
    }
}
type Factory = Box<dyn FnOnce(Arc<Input>) -> Result<Box<dyn Device>, NativeError> + Send>;
struct State {
    report: Snapshot,
    closed: bool,
    retry: u64,
    finished: bool,
    cancel: Option<Revoke>,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    input: Arc<Input>,
}
impl Shared {
    fn revoke(&self, retry: bool) {
        let cancel = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.closed = true;
            if retry {
                s.retry = s.retry.saturating_add(1);
            }
            if s.report.phase != Phase::CleanupBlocked {
                s.report.phase = Phase::Stopping;
            }
            self.changed.notify_all();
            s.cancel.clone()
        };
        // Callback reenters this exact Input; never call under the state lock.
        self.input.request_stop();
        if let Some(cancel) = cancel {
            cancel();
        }
    }
    fn closed(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).closed
    }
    fn failure(&self, error: Error) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.report.failure.get_or_insert(error);
    }
    fn cancel_when_closed(&self) {
        if self.input.is_closed() {
            self.revoke(false);
        }
    }
}
struct Active {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}
#[derive(Default)]
struct Inner {
    generation: u64,
    suspended: bool,
    uncertain: bool,
    active: Option<Active>,
    last: Snapshot,
}
#[derive(Default)]
pub struct Host {
    inner: Mutex<Inner>,
}
impl Host {
    /// Native caller must first validate installed COM source identity/version.
    /// This grants no authority to restore output or select a different source.
    pub fn start(
        &self,
        pin: PinnedTimedLibrary,
        input: Arc<Input>,
        expected_generation: u64,
        authorized: bool,
    ) -> Result<(), Error> {
        self.start_inner(
            input,
            expected_generation,
            authorized,
            Box::new(move |input| Ok(Box::new(NativeDevice::load(pin, input)?))),
        )
    }
    fn start_inner(
        &self,
        input: Arc<Input>,
        expected_generation: u64,
        authorized: bool,
        factory: Factory,
    ) -> Result<(), Error> {
        if !authorized {
            return Err(Error::Permission);
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if expected_generation != inner.generation {
            return Err(Error::Stale);
        }
        if inner.suspended {
            return Err(Error::Suspended);
        }
        if inner.uncertain {
            return Err(Error::CleanupUncertain);
        }
        if inner.active.is_some() {
            return Err(Error::Busy);
        }
        if input.is_closed() {
            return Err(Error::Closed);
        }
        let generation = inner
            .generation
            .checked_add(1)
            .ok_or(Error::CleanupUncertain)?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                report: Snapshot {
                    generation,
                    input_generation: Some(input.generation()),
                    busy: true,
                    phase: Phase::Preparing,
                    ..Snapshot::default()
                },
                closed: false,
                retry: 0,
                finished: false,
                cancel: None,
            }),
            changed: Condvar::new(),
            input,
        });
        let running = shared.clone();
        let thread = thread::Builder::new()
            .name("tingxian-virtual-camera".into())
            .spawn(move || {
                let clean = match catch_unwind(AssertUnwindSafe(|| drive(&running, factory))) {
                    Ok(clean) => clean,
                    Err(_) => {
                        running.failure(Error::Thread);
                        running.revoke(false);
                        false
                    }
                };
                let mut s = running.state.lock().unwrap_or_else(|e| e.into_inner());
                s.cancel = None;
                s.report.cleanup_uncertain |= !clean;
                s.report.phase = if clean {
                    Phase::Retired
                } else {
                    Phase::CleanupBlocked
                };
                s.finished = true;
                running.changed.notify_all();
            })
            .map_err(|_| {
                shared.input.request_stop();
                Error::Thread
            })?;
        inner.generation = generation;
        inner.active = Some(Active {
            shared,
            thread: Some(thread),
        });
        Ok(())
    }
    pub fn snapshot(&self) -> Snapshot {
        let i = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut r = i
            .active
            .as_ref()
            .map(|a| {
                a.shared
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .report
            })
            .unwrap_or(i.last);
        r.generation = i.generation;
        r.suspended = i.suspended;
        r.busy = i.active.is_some();
        r.cleanup_uncertain |= i.uncertain;
        r
    }
    fn stop(&self, suspend: bool) {
        let shared = {
            let mut i = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            i.suspended |= suspend;
            i.generation = i.generation.saturating_add(1);
            i.active.as_ref().map(|a| a.shared.clone())
        };
        // A repeated explicit stop is also one cleanup retry, never a restart.
        if let Some(s) = shared {
            s.revoke(true);
        }
    }
    pub fn request_stop(&self) {
        self.stop(false);
    }
    pub fn suspend(&self) {
        self.stop(true);
    }
    pub fn resume(&self) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .suspended = false;
    }
    /// Actual device thread join only. Does not assert external consumer drain.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let until = Instant::now() + timeout;
        loop {
            let retired = {
                let mut i = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                match &mut i.active {
                    None => return !i.uncertain,
                    Some(a) if a.thread.as_ref().is_some_and(|t| t.is_finished()) => {
                        Some((a.shared.clone(), a.thread.take().unwrap()))
                    }
                    _ => None,
                }
            };
            if let Some((shared, thread)) = retired {
                let joined = thread.join().is_ok();
                let (finished, report) = {
                    let s = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                    (s.finished, s.report)
                };
                let mut i = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                i.uncertain |= !joined || !finished || report.cleanup_uncertain;
                i.last = report;
                i.active = None;
                return !i.uncertain;
            }
            if Instant::now() >= until {
                return false;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.request_stop();
    }
}
struct Apartment;
impl Apartment {
    fn new() -> Result<Self, Error> {
        let hr = unsafe { CoInitializeEx(null(), COINIT_MULTITHREADED as u32) };
        if hr < 0 {
            Err(Error::Apartment(hr))
        } else {
            Ok(Self)
        }
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
fn drive(shared: &Arc<Shared>, factory: Factory) -> bool {
    shared.cancel_when_closed();
    if shared.closed() {
        return true;
    }
    let _apartment = match Apartment::new() {
        Ok(a) => a,
        Err(e) => {
            shared.failure(e);
            shared.revoke(false);
            return true;
        }
    };
    let mut device = match factory(shared.input.clone()) {
        Ok(d) => d,
        Err(e) => {
            shared.failure(Error::Native(e));
            shared.revoke(false);
            return true;
        }
    };
    {
        let mut s = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        s.cancel = Some(device.cancellation());
    }
    // Independent of Windows Start: input/source failure or explicit stop can
    // cancel the native ticket while the owning MTA is inside a slow OS call.
    let watched = shared.clone();
    let monitor = thread::Builder::new()
        .name("tingxian-device-cancel".into())
        .spawn(move || loop {
            watched.cancel_when_closed();
            if watched.closed() {
                watched.revoke(false);
                return;
            }
            let state = watched.state.lock().unwrap_or_else(|e| e.into_inner());
            let _ = watched
                .changed
                .wait_timeout(state, Duration::from_millis(10))
                .unwrap_or_else(|e| e.into_inner());
        });
    let result = catch_unwind(AssertUnwindSafe(|| {
        if monitor.is_err() {
            return Err(Error::Thread);
        }
        shared.cancel_when_closed();
        if shared.closed() {
            shared.revoke(false);
            return Ok(());
        }
        device.start().map_err(Error::Native)?;
        loop {
            shared.cancel_when_closed();
            if shared.closed() {
                return Ok(());
            }
            let status = device.status().map_err(Error::Native)?;
            if status.state != 2 || status.revoked != 0 {
                return Err(Error::Closed);
            }
            let mut s = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            s.report.native = Some(status);
            if !s.closed {
                s.report.phase = Phase::Available;
            }
            let _ = shared
                .changed
                .wait_timeout(s, Duration::from_millis(20))
                .unwrap_or_else(|e| e.into_inner());
        }
    }));
    match result {
        Ok(Err(error)) => shared.failure(error),
        Err(_) => shared.failure(Error::Thread),
        _ => {}
    }
    shared.revoke(false);
    if let Ok(monitor) = monitor {
        if monitor.join().is_err() {
            shared.failure(Error::Thread);
        }
    }
    // Retain the device, callback, DLL and COM apartment on THIS live thread if
    // cleanup fails. Retry only after another explicit stop; no automatic loop.
    loop {
        let retry = shared.state.lock().unwrap_or_else(|e| e.into_inner()).retry;
        let result = catch_unwind(AssertUnwindSafe(|| device.close()));
        let error = match result {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(Error::Native(e)),
            Err(_) => Some(Error::Thread),
        };
        let mut s = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        s.report.cleanup_attempts += 1;
        if let Some(error) = error {
            s.report.last_cleanup_error = Some(error);
            s.report.cleanup_uncertain = true;
            s.report.phase = Phase::CleanupBlocked;
            shared.changed.notify_all();
            drop(
                shared
                    .changed
                    .wait_while(s, |s| s.retry == retry)
                    .unwrap_or_else(|e| e.into_inner()),
            );
        } else {
            s.report.cleanup_uncertain = false;
            return true;
        }
    }
}

#[cfg(test)]
mod tests;
