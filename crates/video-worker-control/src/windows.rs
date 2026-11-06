use super::{BundledWorker, Inspection, Reason, Work, Report, EyeJob, EyeReport, EyeOutcome};
use sha2::{Digest, Sha256};
use std::{ffi::{c_void, OsStr}, fs::{File, OpenOptions}, io::Read,
    mem::{size_of, size_of_val, zeroed}, os::windows::{ffi::OsStrExt, fs::{MetadataExt, OpenOptionsExt}, io::AsRawHandle},
    path::Component, ptr::{null, null_mut}, sync::atomic::{AtomicBool, Ordering}, time::{Duration, Instant}};
use windows_sys::Win32::{Foundation::*, Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::*, System::{JobObjects::*, Pipes::CreatePipe, Threading::*}};
#[cfg(test)]
#[path = "launch_diagnostics.rs"]
mod launch_diagnostics;

struct Handle(HANDLE);
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Handle {
    fn take(raw: HANDLE) -> Result<Self, Reason> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE { Err(Reason::Launch) } else { Ok(Self(raw)) }
    }
    fn duplicate(&self, access: u32) -> Result<Self, Reason> {
        Self::duplicate_raw(self.0, access)
    }
    fn duplicate_raw(handle: HANDLE, access: u32) -> Result<Self, Reason> {
        let mut raw = null_mut();
        if unsafe { DuplicateHandle(GetCurrentProcess(), handle, GetCurrentProcess(), &mut raw, access, 1, 0) } == 0 {
            return Err(Reason::Launch);
        }
        Self::take(raw)
    }
}
impl Drop for Handle { fn drop(&mut self) { unsafe { CloseHandle(self.0); } } }
pub(super) struct Signal { event: Handle, cancelled: AtomicBool }
impl Signal {
    pub(super) fn new() -> Result<Self, Reason> {
        Ok(Self { event: Handle::take(unsafe { CreateEventW(null(), 1, 0, null()) })?, cancelled: AtomicBool::new(false) })
    }
    pub(super) fn stop(&self) { self.cancelled.store(true, Ordering::SeqCst); unsafe { SetEvent(self.event.0); } }
    pub(super) fn stopped(&self) -> bool { self.cancelled.load(Ordering::SeqCst) }
}
fn wide(value: &OsStr) -> Result<Vec<u16>, Reason> {
    let mut text: Vec<u16> = value.encode_wide().collect();
    if text.contains(&0) || text.contains(&(b'"' as u16)) { return Err(Reason::Integrity); }
    text.push(0); Ok(text)
}
// Keep the EXE open without write/delete sharing from hashing through process exit.
// Directory leases prevent parent-path replacement during this interval too.
struct Verified { _directories: Vec<File>, _file: File }
fn lock_parents(target: &std::path::Path) -> Result<Vec<File>, Reason> {
    if !target.is_absolute() { return Err(Reason::Integrity); }
    let mut path = std::path::PathBuf::new(); let mut directories = Vec::new();
    for part in target.components() {
        match part {
            Component::Prefix(prefix) if matches!(prefix.kind(), std::path::Prefix::Disk(_)) => path.push(part),
            Component::RootDir => path.push(part),
            Component::Normal(name) => {
                if name.to_string_lossy().contains(':') { return Err(Reason::Integrity); }
                path.push(part);
                if path == target { break; }
                let dir = OpenOptions::new().read(true).share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                    .open(&path).map_err(|_|Reason::Integrity)?;
                let meta = dir.metadata().map_err(|_|Reason::Integrity)?;
                if !meta.is_dir() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 { return Err(Reason::Integrity); }
                directories.push(dir);
            }
            _ => return Err(Reason::Integrity),
        }
    }
    Ok(directories)
}
fn regular(file: &File) -> Result<u64, Reason> {
    let meta = file.metadata().map_err(|_|Reason::Integrity)?;
    if !meta.is_file() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 { return Err(Reason::Integrity); }
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 || info.nNumberOfLinks != 1 {
        return Err(Reason::Integrity);
    }
    Ok(meta.len())
}
fn verify(worker: &BundledWorker) -> Result<Verified, Reason> {
    if worker.sha256.len() != 64 || !worker.sha256.bytes().all(|c|c.is_ascii_hexdigit()) { return Err(Reason::Integrity); }
    let directories = lock_parents(&worker.path)?;
    let mut file = OpenOptions::new().read(true).share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(&worker.path).map_err(|_|Reason::Integrity)?;
    if regular(&file)? > 32 * 1024 * 1024 { return Err(Reason::Integrity); }
    let mut hash = Sha256::new(); let mut buffer = [0u8; 65536];
    loop { let bytes = file.read(&mut buffer).map_err(|_|Reason::Integrity)?; if bytes == 0 { break; } hash.update(&buffer[..bytes]); }
    if format!("{:x}", hash.finalize()) != worker.sha256.to_ascii_lowercase() { return Err(Reason::Integrity); }
    Ok(Verified { _directories: directories, _file: file })
}
struct EyeFiles { _directories: Vec<File>, input: File, output: File }
impl EyeFiles {
    fn open(job: &EyeJob) -> Result<Self, Reason> {
        let bytes = job.frame_bytes()? * u64::from(job.frames);
        let mut directories = lock_parents(&job.input).map_err(|_|Reason::EyeInput)?;
        let input = OpenOptions::new().read(true).share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(&job.input).map_err(|_|Reason::EyeInput)?;
        if regular(&input).map_err(|_|Reason::EyeInput)? != bytes { return Err(Reason::EyeInput); }
        directories.extend(lock_parents(&job.output).map_err(|_|Reason::EyeOutput)?);
        let output = OpenOptions::new().read(true).write(true).create_new(true).share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(&job.output).map_err(|_|Reason::EyeOutput)?;
        if regular(&output).map_err(|_|Reason::EyeOutput)? != 0 { return Err(Reason::EyeOutput); }
        Ok(Self { _directories: directories, input, output })
    }
}
struct Attributes { storage: Vec<usize>, initialized: bool }
impl Attributes {
    fn pointer(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST { self.storage.as_mut_ptr().cast() }
    fn new(handles: &mut [HANDLE]) -> Result<Self, Reason> {
        let mut bytes = 0;
        unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes); }
        if bytes == 0 { return Err(Reason::Launch); }
        let mut attrs = Self { storage: vec![0; (bytes + size_of::<usize>() - 1)/size_of::<usize>()], initialized: false };
        if unsafe { InitializeProcThreadAttributeList(attrs.pointer(), 1, 0, &mut bytes) } == 0 { return Err(Reason::Launch); }
        attrs.initialized = true;
        if unsafe { UpdateProcThreadAttribute(attrs.pointer(), 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            handles.as_mut_ptr().cast(), std::mem::size_of_val(handles), null_mut(), null()) } == 0 { return Err(Reason::Launch); }
        Ok(attrs)
    }
}
impl Drop for Attributes { fn drop(&mut self) { if self.initialized { unsafe { DeleteProcThreadAttributeList(self.pointer()); } } } }
struct Process { process: Handle, job: Handle, output: Handle, _verified: Verified, eye_files: Option<EyeFiles> }
impl Process {
    fn terminate(&self) -> Result<(), Reason> {
        if unsafe { WaitForSingleObject(self.process.0, 0) } == WAIT_OBJECT_0 { return Ok(()); }
        if unsafe { TerminateJobObject(self.job.0, 1223) } == 0 ||
            unsafe { WaitForSingleObject(self.process.0, 1000) } != WAIT_OBJECT_0 { return Err(Reason::Cleanup); }
        Ok(())
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        // Exact owned job only; never enumerate or kill applications by name/PID.
        if unsafe { WaitForSingleObject(self.process.0, 0) } != WAIT_OBJECT_0 {
            unsafe { TerminateJobObject(self.job.0, 1223); WaitForSingleObject(self.process.0, 1000); }
        }
    }
}
fn launch(worker: &BundledWorker, signal: &Signal, work: &Work, generation: u64) -> Result<Process, Reason> {
    let verified = verify(worker)?;
    if signal.stopped() { return Err(Reason::Abandoned); }
    let eye_files = match work { Work::Inspect => None, Work::Eye(job) => Some(EyeFiles::open(job)?) };
    let event = signal.event.duplicate(SYNCHRONIZE)?;
    let mut owner_raw = null_mut();
    if unsafe { DuplicateHandle(GetCurrentProcess(), GetCurrentProcess(), GetCurrentProcess(), &mut owner_raw,
        SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 1, 0) } == 0 { return Err(Reason::Launch); }
    let owner = Handle::take(owner_raw)?;
    let security = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: null_mut(), bInheritHandle: 1 };
    let (mut read, mut write) = (null_mut(), null_mut());
    if unsafe { CreatePipe(&mut read, &mut write, &security, 4096) } == 0 { return Err(Reason::Launch); }
    let output = Handle::take(read)?; let writer = Handle::take(write)?;
    if unsafe { SetHandleInformation(output.0, HANDLE_FLAG_INHERIT, 0) } == 0 { return Err(Reason::Launch); }
    let job = Handle::take(unsafe { CreateJobObjectW(null(), null()) })?;
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
    limits.BasicLimitInformation.ActiveProcessLimit = 1;
    if unsafe { SetInformationJobObject(job.0, JobObjectExtendedLimitInformation,
        (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(), size_of_val(&limits) as u32) } == 0 { return Err(Reason::Launch); }
    let mut inherited = vec![owner.0, event.0, writer.0];
    let mut eye_handles = Vec::new();
    if let Some(files) = &eye_files {
        eye_handles.push(Handle::duplicate_raw(files.input.as_raw_handle(), FILE_READ_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE)?);
        eye_handles.push(Handle::duplicate_raw(files.output.as_raw_handle(), FILE_WRITE_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE)?);
        inherited.extend(eye_handles.iter().map(|h|h.0));
    }
    let mut attributes = Attributes::new(&mut inherited)?;
    let exe = wide(worker.path.as_os_str())?;
    let parent = worker.path.parent().ok_or(Reason::Integrity)?;
    let cwd = wide(parent.as_os_str())?;
    let args = match work {
        Work::Inspect => format!("--inspect-v1 {} {}", owner.0 as usize, event.0 as usize),
        Work::Eye(job) => format!("--eye-offline-v1 {} {} {} {} {} {} {} {} {}", owner.0 as usize, event.0 as usize,
            writer.0 as usize, eye_handles[0].0 as usize, eye_handles[1].0 as usize, job.width, job.height, job.frames, generation),
    };
    let mut command: Vec<u16> = format!("\"{}\" {}", worker.path.display(), args).encode_utf16().chain(Some(0)).collect();
    // No API keys, app config, PATH, proxy, or private profile settings are inherited.
    let mut environment = Vec::<u16>::new();
    for name in ["SystemRoot", "TEMP", "TMP", "WINDIR"] {
        if let Some(value) = std::env::var_os(name) {
            environment.extend(OsStr::new(name).encode_wide()); environment.push(b'=' as u16);
            environment.extend(value.encode_wide()); environment.push(0);
        }
    }
    if environment.is_empty() { environment.push(0); } environment.push(0);
    let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    if matches!(work, Work::Inspect) {
        startup.StartupInfo.hStdOutput = writer.0; startup.StartupInfo.hStdError = writer.0;
    } // Eye reports use an explicit handle; vendor stdout cannot corrupt protocol.
    startup.lpAttributeList = attributes.pointer();
    let mut info: PROCESS_INFORMATION = unsafe { zeroed() };
    if signal.stopped() { return Err(Reason::Abandoned); }
    #[cfg(test)]
    let diagnostic = launch_diagnostics::before(worker, &verified._file);
    if unsafe { CreateProcessW(exe.as_ptr(), command.as_mut_ptr(), null(), null(), 1,
        EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
        environment.as_ptr().cast::<c_void>(), cwd.as_ptr(), &startup.StartupInfo, &mut info) } == 0 {
        #[cfg(test)] {
            let error = unsafe { GetLastError() };
            eprintln!("CreateProcessW failed: {error}");
            launch_diagnostics::failed(worker, &verified._file, diagnostic, error);
        }
        return Err(Reason::Launch);
    }
    let process = Handle::take(info.hProcess)?; let thread = Handle::take(info.hThread)?;
    if unsafe { AssignProcessToJobObject(job.0, process.0) } == 0 {
        if unsafe { TerminateProcess(process.0, 1223) } == 0 ||
            unsafe { WaitForSingleObject(process.0, 1000) } != WAIT_OBJECT_0 { return Err(Reason::Cleanup); }
        return Err(Reason::Launch);
    }
    let owned = Process { process, job, output, _verified: verified, eye_files };
    if signal.stopped() { owned.terminate()?; return Err(Reason::Abandoned); }
    if unsafe { ResumeThread(thread.0) } == u32::MAX { owned.terminate()?; return Err(Reason::Launch); }
    Ok(owned)
}
pub(super) fn execute(worker: &BundledWorker, signal: &Signal, work: &Work, generation: u64, timeout: Duration) -> Result<Report, Reason> {
    let process = launch(worker, signal, work, generation)?;
    let started = Instant::now();
    let mut stop_deadline = None;
    loop {
        match unsafe { WaitForSingleObject(process.process.0, 20) } {
            WAIT_OBJECT_0 => break,
            WAIT_TIMEOUT => {},
            _ => return Err(Reason::Cleanup),
        }
        if signal.stopped() && stop_deadline.is_none() { stop_deadline = Some(Instant::now() + Duration::from_secs(1)); }
        if started.elapsed() >= timeout || stop_deadline.is_some_and(|d|Instant::now() >= d) {
            process.terminate()?;
            return Err(if stop_deadline.is_some() { Reason::ForcedStop } else { Reason::Timeout });
        }
    }
    let mut code = 0;
    if unsafe { GetExitCodeProcess(process.process.0, &mut code) } == 0 { return Err(Reason::Cleanup); }
    if code != 0 { return Err(Reason::WorkerExit); }
    // The sole job process has exited; parent writer was closed at launch return.
    // No descendant can retain stdout. At most 4097 bytes are ever read/parsed.
    let mut bytes = Vec::new(); let mut buffer = [0u8; 1024];
    loop {
        let mut read = 0;
        let remaining=(4097-bytes.len()).min(buffer.len());
        let ok = unsafe { ReadFile(process.output.0, buffer.as_mut_ptr(), remaining as u32, &mut read, null_mut()) };
        if ok == 0 { if unsafe { GetLastError() } == ERROR_BROKEN_PIPE { break; } return Err(Reason::Protocol); }
        if read == 0 { break; }
        bytes.extend_from_slice(&buffer[..read as usize]);
        if bytes.len() > 4096 { return Err(Reason::Protocol); }
    }
    match work {
        Work::Inspect => Inspection::parse(&bytes).map(Report::Inspection),
        Work::Eye(job) => {
            let report = EyeReport::parse(&bytes, job, generation)?;
            if report.outcome == EyeOutcome::Cancelled && !signal.stopped() { return Err(Reason::Protocol); }
            let length = process.eye_files.as_ref().ok_or(Reason::Protocol)?.output.metadata().map_err(|_|Reason::EyeOutput)?.len();
            let reported = u64::from(report.corrected + report.neutral) * job.frame_bytes()?;
            if length != reported && report.error != 10 { return Err(Reason::Protocol); }
            Ok(Report::Eye(report))
        }
    }
}
