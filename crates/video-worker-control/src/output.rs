//! Native-only relay adapter. Reuses our C++ publisher/ACL, not a second wire
//! protocol. No registration, camera/SDK, user-configured DLL path, or auto start.
use crate::{windows::{verify,Verified},stream::{Shared,RevokeOutput},BundledWorker,NativeOutputEndpoint,PreviewFrame,Reason};
use serde::Serialize;
use std::{ffi::c_void,os::windows::ffi::OsStrExt,path::PathBuf,ptr::{null_mut,NonNull},sync::Arc};
use windows_sys::Win32::{Foundation::*,System::{LibraryLoader::*,Threading::GetCurrentProcessId}};

// A build-pinned own library supplied by the native host. Never deserialize from
// a webview/settings file. Pins are not vendor-runtime authorization.
pub struct PinnedOutputLibrary {pub path:PathBuf,pub sha256:String}
#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize)]
#[serde(rename_all="snake_case")]
pub enum RelayPhase { Off, Opening, Waiting, Forwarding, Stopping, Stopped, Fault }
#[derive(Clone,Copy,Debug,Serialize)]
pub struct RelayStatus {
    pub phase:RelayPhase,pub generation:u64,pub accepted:u64,pub rejected:u64,pub dropped:u64,pub submitted:u64,
    pub queued:u32,pub busy:bool,pub error:i32,
}
#[derive(Clone,Copy,Debug,Serialize)]
pub struct OutputSnapshot {pub intent:u64,pub status:Option<RelayStatus>}
#[repr(C)]
#[derive(Default)]
struct RawStatus {version:u32,state:u32,generation:u64,accepted:u64,rejected:u64,dropped:u64,submitted:u64,queued:u32,busy:u32,error:i32,reserved:u32}
type Abi=unsafe extern "system" fn()->u32;
type Create=unsafe extern "system" fn()->*mut c_void;
type Open=unsafe extern "system" fn(*mut c_void,u64,*mut u8,u32)->i32;
type Submit=unsafe extern "system" fn(*mut c_void,u64,u64,u64,*const u8,u32)->i32;
type Action=unsafe extern "system" fn(*mut c_void);
type Status=unsafe extern "system" fn(*mut c_void,*mut RawStatus,u32)->i32;
struct Library {module:HMODULE,_verified:Verified,create:Create,open:Open,submit:Submit,cancel:Action,close:Action,status:Status,destroy:Action}
// All calls are pinned to this live module. Its C ABI serializes lifetime calls
// on the owning lane and explicitly supports concurrent Cancel/Status.
unsafe impl Send for Library {}
unsafe impl Sync for Library {}
impl Drop for Library {fn drop(&mut self){unsafe{FreeLibrary(self.module);}}}
struct Module(HMODULE);
impl Drop for Module {fn drop(&mut self){if !self.0.is_null(){unsafe{FreeLibrary(self.0);}}}}
impl Library {
    fn load(pin:PinnedOutputLibrary)->Result<Self,Reason> {
        let verified=verify(&BundledWorker{path:pin.path.clone(),sha256:pin.sha256})?;
        let normalized=pin.path.to_str().ok_or(Reason::Integrity)?.replace('/',"\\");
        let path:Vec<u16>=std::ffi::OsStr::new(&normalized).encode_wide().chain(Some(0)).collect();
        if path[..path.len()-1].contains(&0){return Err(Reason::Integrity);}
        // Full path for our DLL, system-only dependency lookup, no PATH mutation.
        let mut module=Module(unsafe{LoadLibraryExW(path.as_ptr(),null_mut(),LOAD_LIBRARY_SEARCH_SYSTEM32)});
        if module.0.is_null(){return Err(Reason::Launch);}
        let mut actual=vec![0u16;32768];let count=unsafe{GetModuleFileNameW(module.0,actual.as_mut_ptr(),actual.len() as u32)} as usize;
        if count==0||count>=actual.len()||String::from_utf16_lossy(&actual[..count]).to_lowercase()!=normalized.to_lowercase(){return Err(Reason::Integrity);}
        macro_rules! symbol {($name:literal,$type:ty)=>{{
            let p=unsafe{GetProcAddress(module.0,concat!($name,"\0").as_ptr())}.ok_or(Reason::Protocol)?;
            // Exact ABI v1 export signatures from output_bridge.h, no variadics.
            unsafe{std::mem::transmute::<unsafe extern "system" fn()->isize,$type>(p)}
        }}}
        let abi=symbol!("TxOutputAbi",Abi);if unsafe{abi()}!=1||std::mem::size_of::<RawStatus>()!=64{return Err(Reason::Protocol);}
        let library=Self{module:module.0,_verified:verified,create:symbol!("TxOutputCreate",Create),open:symbol!("TxOutputOpen",Open),
            submit:symbol!("TxOutputSubmit",Submit),cancel:symbol!("TxOutputCancel",Action),close:symbol!("TxOutputClose",Action),
            status:symbol!("TxOutputGetStatus",Status),destroy:symbol!("TxOutputDestroy",Action)};
        module.0=null_mut();Ok(library)
    }
}
struct Core {raw:NonNull<c_void>,library:Library}
unsafe impl Send for Core {}
unsafe impl Sync for Core {}
impl RevokeOutput for Core {fn revoke(&self){unsafe{(self.library.cancel)(self.raw.as_ptr());}}}
impl Drop for Core {fn drop(&mut self){unsafe{(self.library.destroy)(self.raw.as_ptr());}}}
pub(crate) struct NativeOutput {core:Arc<Core>,generation:u64}
impl Drop for NativeOutput {fn drop(&mut self){self.core.revoke();unsafe{(self.core.library.close)(self.core.raw.as_ptr());}}}
impl NativeOutput {
    pub(crate) fn stop(&mut self)->Result<RelayStatus,Reason> {
        self.core.revoke();unsafe{(self.core.library.close)(self.core.raw.as_ptr());}self.status()
    }
    pub(crate) fn open(pin:PinnedOutputLibrary,generation:u64,lease:&Shared)->Result<(Self,NativeOutputEndpoint),Reason> {
        let library=Library::load(pin)?;let raw=NonNull::new(unsafe{(library.create)()}).ok_or(Reason::Launch)?;
        let core=Arc::new(Core{raw,library});let output=Self{core,generation};
        let mut endpoint=[0u8;236];
        let hr=unsafe{(output.core.library.open)(output.core.raw.as_ptr(),generation,endpoint.as_mut_ptr(),236)};
        let parsed=if hr>=0{NativeOutputEndpoint::parse(&endpoint,unsafe{GetCurrentProcessId()})}else{Err(Reason::EyeOutput)};
        for b in &mut endpoint{unsafe{std::ptr::write_volatile(b,0);}}
        let endpoint=parsed?;
        let revoke:Arc<dyn RevokeOutput>=output.core.clone();
        if !lease.attach_output(Arc::downgrade(&revoke)){return Err(Reason::Abandoned);}
        Ok((output,endpoint))
    }
    pub(crate) fn submit(&self,frame:&PreviewFrame)->Result<(),Reason> {
        if frame.nv12.len()!=1280*720*3/2{return Err(Reason::EyeOutput);}
        let hr=unsafe{(self.core.library.submit)(self.core.raw.as_ptr(),self.generation,frame.id,frame.captured_ms,frame.nv12.as_ptr(),frame.nv12.len() as u32)};
        if hr>=0{Ok(())}else{Err(Reason::EyeOutput)}
    }
    pub(crate) fn status(&self)->Result<RelayStatus,Reason> {
        let mut s=RawStatus::default();
        if unsafe{(self.core.library.status)(self.core.raw.as_ptr(),&mut s,64)}<0||s.version!=1||s.reserved!=0||s.generation!=self.generation||s.queued>1||s.busy>1{return Err(Reason::Protocol);}
        let phase=match s.state{0=>RelayPhase::Off,1=>RelayPhase::Opening,2=>RelayPhase::Waiting,3=>RelayPhase::Forwarding,4=>RelayPhase::Stopping,5=>RelayPhase::Stopped,6=>RelayPhase::Fault,_=>return Err(Reason::Protocol)};
        Ok(RelayStatus{phase,generation:s.generation,accepted:s.accepted,rejected:s.rejected,dropped:s.dropped,submitted:s.submitted,queued:s.queued,busy:s.busy!=0,error:s.error})
    }
}
