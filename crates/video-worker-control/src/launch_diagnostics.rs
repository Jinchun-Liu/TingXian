//! Test-only, explicit local diagnosis. Never compiled into the app/controller
//! example and disabled in ordinary CI. Does not retry or alter process creation.
use super::*;
use serde_json::{json, Value};
use std::io::{Seek, SeekFrom, Write};
use windows_sys::Win32::System::ApplicationInstallationAndServicing::{
    CreateActCtxW, ReleaseActCtx, ACTCTXW,
};
use windows_sys::Win32::System::WindowsProgramming::ACTCTX_FLAG_RESOURCE_NAME_VALID;

fn locked_file(file: &File) -> std::io::Result<Value> {
    let mut reader = file.try_clone()?;
    reader.seek(SeekFrom::Start(0))?;
    // Hash through the SAME pinned handle, not a fresh pathname lookup.
    let mut hash = Sha256::new(); let mut buffer = [0u8;65536]; let mut bytes = 0u64;
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 { break; }
        bytes += n as u64;
        if bytes > 32*1024*1024 { return Err(std::io::Error::other("diagnostic size limit")); }
        hash.update(&buffer[..n]);
    }
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut path = vec![0u16;32768];
    let len = unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), path.as_mut_ptr(), path.len() as u32, 0) };
    if len == 0 || len as usize >= path.len() { return Err(std::io::Error::last_os_error()); }
    Ok(json!({"bytes":bytes,"sha256":format!("{:x}",hash.finalize()),
        "resolved_path":String::from_utf16_lossy(&path[..len as usize]),
        "volume_serial":info.dwVolumeSerialNumber,"file_index_high":info.nFileIndexHigh,
        "file_index_low":info.nFileIndexLow,"links":info.nNumberOfLinks}))
}
fn snapshot(worker: &BundledWorker, file: &File) -> Value {
    let locked = locked_file(file).unwrap_or_else(|error|json!({"read_error":error.raw_os_error()}));
    // Only parse resource 1. Never ActivateActCtx, load a DLL or execute a helper.
    let activation = match wide(worker.path.as_os_str()) {
        Ok(path) => {
            let mut context: ACTCTXW = unsafe { zeroed() };
            context.cbSize = size_of::<ACTCTXW>() as u32;
            context.dwFlags = ACTCTX_FLAG_RESOURCE_NAME_VALID;
            context.lpSource = path.as_ptr(); context.lpResourceName = 1usize as *const u16;
            let handle = unsafe { CreateActCtxW(&context) };
            if handle == INVALID_HANDLE_VALUE { json!({"created":false,"error":unsafe{GetLastError()}}) }
            else { unsafe { ReleaseActCtx(handle); } json!({"created":true,"error":0}) }
        }
        Err(_) => json!({"invalid_path":true}),
    };
    json!({"locked_file":locked,"activation_context":activation})
}
pub(super) fn before(worker: &BundledWorker, file: &File) -> Option<Value> {
    if std::env::var_os("TINGXIAN_VIDEO_LAUNCH_DIAGNOSTICS").as_deref() != Some(OsStr::new("1")) { return None; }
    Some(snapshot(worker,file))
}
pub(super) fn failed(worker: &BundledWorker, file: &File, before: Option<Value>, error: u32) {
    let Some(before) = before else { return; };
    let result = (|| -> std::io::Result<()> {
        let after = snapshot(worker,file);
        let record = json!({"schema_version":1,"create_process_error":error,
            "expected_source_sha256":worker.sha256,"requested_path":worker.path,
            "before":before,"after":after,"activation_context_activated":false,
            "process_retried":false,"security_settings_changed":false});
        let target = worker.path.parent().ok_or_else(||std::io::Error::other("missing test directory"))?
            .join("launch-diagnostics.json");
        let mut output = OpenOptions::new().write(true).create_new(true).open(target)?;
        output.write_all(&serde_json::to_vec_pretty(&record)?)
    })();
    if let Err(error) = result { eprintln!("test-only launch evidence write failed: {:?}",error.raw_os_error()); }
}
