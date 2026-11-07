//! Native corrected-frame receiver. No camera, device registration, UI or SDK
//! authority. Poll on the native media lane, never the webview/UI thread.
use crate::{stream::Shared, NativeOutputEndpoint, Reason, StreamFeed};
use crate::{output::NativeOutput,PinnedOutputLibrary,RelayPhase,RelayStatus,OutputSnapshot};
use serde::Serialize;
use std::{ptr::{null,null_mut}, sync::Arc, thread, time::{Duration,Instant}};
use windows_sys::Win32::{Foundation::*, Storage::FileSystem::*, System::{IO::*, Pipes::*, Threading::*}};

const NV12_BYTES:usize=1280*720*3/2;
const MAX_AGE_MS:u64=250;
const IO_BOUND:Duration=Duration::from_millis(500);
#[link(name="kernel32")]
extern "system" { fn GetTickCount64()->u64; }
fn tick()->u64 {unsafe{GetTickCount64()}}

// No Debug/Serialize: pixels must not become diagnostics or webview IPC.
pub struct PreviewFrame {
    /// Controller generation is bound by the ready message and single-use
    /// endpoint. A new child has a separate local Session epoch; retain both.
    pub generation:u64,pub worker_generation:u64,pub id:u64,pub captured_ms:u64,pub nv12:Vec<u8>
}
impl Drop for PreviewFrame {fn drop(&mut self){self.nv12.fill(0);}}
#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize)]
#[serde(rename_all="snake_case")]
pub enum PreviewPhase { Waiting, Receiving, Stopped, Fault }
#[derive(Clone,Copy,Debug,Serialize)]
pub struct PreviewStatus {
    pub phase:PreviewPhase,pub reason:Option<Reason>,pub received:u64,pub rejected:u64,
    pub replaced:u64,pub discarded:u64,pub expired:u64,pub presented:u64,pub repeated:u64,pub neutral:u64,
}
impl Default for PreviewStatus {
    fn default()->Self {Self{phase:PreviewPhase::Waiting,reason:None,received:0,rejected:0,
        replaced:0,discarded:0,expired:0,presented:0,repeated:0,neutral:0}}
}
struct Handle(HANDLE);
unsafe impl Send for Handle {}
impl Drop for Handle {fn drop(&mut self){unsafe{CloseHandle(self.0);}}}
fn handle(raw:HANDLE)->Result<Handle,Reason> {
    if raw.is_null()||raw==INVALID_HANDLE_VALUE{Err(Reason::EyeOutput)}else{Ok(Handle(raw))}
}
#[derive(Clone,Copy)]
struct Header {kind:u16,bytes:u32,generation:u64,id:u64,time:u64}
impl Header {
    fn parse(b:&[u8;40])->Result<Self,Reason> {
        if b[0..4]!=0x31565854u32.to_le_bytes()||b[4..6]!=1u16.to_le_bytes()||b[12..16]!=[0;4] {return Err(Reason::Protocol);}
        Ok(Self{kind:u16::from_le_bytes(b[6..8].try_into().unwrap()),bytes:u32::from_le_bytes(b[8..12].try_into().unwrap()),
            generation:u64::from_le_bytes(b[16..24].try_into().unwrap()),id:u64::from_le_bytes(b[24..32].try_into().unwrap()),time:u64::from_le_bytes(b[32..40].try_into().unwrap())})
    }
    fn control(self,kind:u16,generation:u64)->bool {self.kind==kind&&self.bytes==0&&self.generation==generation&&self.id==0&&self.time==0}
}

// Stack OVERLAPPED and its buffer stay alive until completion, including after
// cancellation. A cancellation request alone never proves I/O retirement.
fn transfer(pipe:&Handle,write:bool,bytes:&mut [u8],lease:&Shared,deadline:Instant)->Result<(),Reason> {
    let event=handle(unsafe{CreateEventW(null(),1,0,null())})?;
    let mut done=0;
    while done<bytes.len() {
        if lease.closed(){return Err(Reason::Abandoned);}
        if Instant::now()>=deadline{return Err(Reason::Timeout);}
        unsafe{ResetEvent(event.0);}
        let mut operation:OVERLAPPED=unsafe{std::mem::zeroed()};operation.hEvent=event.0;
        let mut count=0;
        let ok=unsafe {if write{WriteFile(pipe.0,bytes[done..].as_ptr(),(bytes.len()-done) as u32,&mut count,&mut operation)}
            else{ReadFile(pipe.0,bytes[done..].as_mut_ptr(),(bytes.len()-done) as u32,&mut count,&mut operation)}};
        if ok==0 {
            if unsafe{GetLastError()}!=ERROR_IO_PENDING{return Err(Reason::EyeOutput);}
            let failure=loop {
                if lease.closed(){break Some(Reason::Abandoned);}
                if Instant::now()>=deadline{break Some(Reason::Timeout);}
                match unsafe{WaitForSingleObject(event.0,5)} {
                    WAIT_OBJECT_0=>break None,WAIT_TIMEOUT=>{},_=>break Some(Reason::EyeOutput),
                }
            };
            if let Some(reason)=failure {
                unsafe{CancelIoEx(pipe.0,&operation);GetOverlappedResult(pipe.0,&operation,&mut count,1);}
                return Err(reason);
            }
            if unsafe{GetOverlappedResult(pipe.0,&operation,&mut count,0)}==0{return Err(Reason::EyeOutput);}
        }
        if count==0||count as usize>bytes.len()-done{return Err(Reason::Protocol);}
        done+=count as usize;
    }
    if lease.closed(){Err(Reason::Abandoned)}else{Ok(())}
}

pub struct NativePreview {
    pipe:Option<Handle>,lease:Arc<Shared>,generation:u64,worker_generation:Option<u64>,last_id:u64,last_time:u64,
    frame:Option<PreviewFrame>,last_presented:u64,status:PreviewStatus,
    output:Option<NativeOutput>,output_intent:u64,last_output:Option<RelayStatus>,
}
impl Drop for NativePreview {fn drop(&mut self){let _=self.disable_output();}}
impl StreamFeed {
    /// Consumes this stream's sole endpoint. The preview owner can separately
    /// enable a pinned output relay; it never gives this endpoint to two clients.
    pub fn open_preview(&self,timeout:Duration)->Result<NativePreview,Reason> {
        let endpoint=self.take_endpoint(timeout)?;
        NativePreview::connect(endpoint,self.shared.clone())
    }
}
impl NativePreview {
    fn connect(endpoint:NativeOutputEndpoint,lease:Arc<Shared>)->Result<Self,Reason> {
        let generation=lease.while_open(|open,g|if open{Some(g)}else{None}).ok_or(Reason::Abandoned)?;
        let b=endpoint.activation_blob();
        let name:Vec<u16>=b[4..196].chunks_exact(2).map(|c|u16::from_le_bytes([c[0],c[1]])).collect();
        let pipe=handle(unsafe{CreateFileW(name.as_ptr(),FILE_READ_DATA|FILE_WRITE_DATA|FILE_READ_ATTRIBUTES|SYNCHRONIZE,
            0,null(),OPEN_EXISTING,FILE_FLAG_OVERLAPPED|SECURITY_SQOS_PRESENT|SECURITY_IDENTIFICATION,null_mut())})?;
        let mut pid=0;let mut session=0;
        if unsafe{GetNamedPipeServerProcessId(pipe.0,&mut pid)}==0||unsafe{GetNamedPipeServerSessionId(pipe.0,&mut session)}==0
            ||pid!=u32::from_le_bytes(b[196..200].try_into().unwrap())||session!=u32::from_le_bytes(b[200..204].try_into().unwrap()){return Err(Reason::Protocol);}
        let deadline=Instant::now()+IO_BOUND;
        let mut hello=[0u8;40];hello[0..4].copy_from_slice(&0x31565854u32.to_le_bytes());
        hello[4..6].copy_from_slice(&1u16.to_le_bytes());hello[6..8].copy_from_slice(&1u16.to_le_bytes());hello[8..12].copy_from_slice(&32u32.to_le_bytes());
        transfer(&pipe,true,&mut hello,&lease,deadline)?;
        let mut capability=b[204..236].to_vec();
        let sent=transfer(&pipe,true,&mut capability,&lease,deadline);
        for byte in &mut capability{unsafe{std::ptr::write_volatile(byte,0);}}
        sent?;
        let mut ack=[0;40];transfer(&pipe,false,&mut ack,&lease,deadline)?;
        if !Header::parse(&ack)?.control(2,0){return Err(Reason::Protocol);}
        Ok(Self{pipe:Some(pipe),lease,generation,worker_generation:None,last_id:0,last_time:0,frame:None,last_presented:0,status:PreviewStatus::default(),output:None,output_intent:0,last_output:None})
    }
    /// Explicit native host intent only. Never accepts a DLL pin from WebView.
    /// This opens a transport endpoint, not a device or meeting connection.
    pub fn enable_output(&mut self,pin:PinnedOutputLibrary,expected_intent:u64)->Result<NativeOutputEndpoint,Reason> {
        self.refresh();
        if expected_intent!=self.output_intent||self.output.is_some()||self.pipe.is_none()||self.worker_generation.is_none(){return Err(Reason::EyeOutput);}
        self.output_intent=self.output_intent.checked_add(1).ok_or(Reason::EyeOutput)?;
        let (output,endpoint)=NativeOutput::open(pin,self.output_intent,&self.lease)?;
        self.output=Some(output);self.last_output=None;Ok(endpoint)
    }
    /// Stops just the relay, leaving valid local preview/input alive. Carry the
    /// new intent on subsequent explicit enable; queued earlier enables fail.
    pub fn disable_output(&mut self)->Result<OutputSnapshot,Reason> {
        self.output_intent=self.output_intent.saturating_add(1);
        if let Some(mut output)=self.output.take(){self.last_output=Some(output.stop()?);}
        Ok(OutputSnapshot{intent:self.output_intent,status:self.last_output})
    }
    pub fn output_status(&mut self)->Result<OutputSnapshot,Reason> {
        self.refresh();
        Ok(OutputSnapshot{intent:self.output_intent,status:if let Some(output)=&self.output{Some(output.status()?)}else{self.last_output}})
    }
    fn clear(&mut self,replaced:bool) {
        if self.frame.as_ref().is_some_and(|f|f.id!=self.last_presented){
            if replaced{self.status.replaced+=1;}else{self.status.discarded+=1;}
        }
        self.frame=None;
    }
    fn end(&mut self,reason:Option<Reason>) {
        // Revoke and retire the independent relay before discarding preview.
        let cleanup=if self.output.is_some(){self.disable_output().err()}else{None};
        self.clear(false);self.pipe=None;self.status.reason=reason;
        self.status.reason=reason.or(cleanup);
        self.status.phase=if self.status.reason.is_some(){PreviewPhase::Fault}else{PreviewPhase::Stopped};
    }
    fn refresh(&mut self) {
        if self.lease.closed(){self.end(self.status.reason);}
        else {
            let now=tick();
            if self.frame.as_ref().is_some_and(|f|f.captured_ms>now||now-f.captured_ms>MAX_AGE_MS){self.status.expired+=1;self.clear(false);}
        }
    }
    /// Receives at most one bounded packet. Idle is Ok(false), not a fault.
    /// Partial/invalid packets fail closed. Call from a dedicated native lane.
    pub fn poll(&mut self,wait:Duration)->Result<bool,Reason> {
        self.refresh();if self.pipe.is_none(){return self.status.reason.map_or(Ok(false),Err);}
        if let Some(output)=&self.output {
            let failure=match output.status(){Ok(s) if s.phase==RelayPhase::Fault=>Some(Reason::EyeOutput),Err(e)=>Some(e),_=>None};
            if let Some(reason)=failure{self.end(Some(reason));return Err(reason);}
        }
        let result=self.receive(wait.min(IO_BOUND));
        if let Err(reason)=result {
            if self.lease.closed(){self.end(self.status.reason);return Ok(false);}
            self.status.rejected+=1;self.end(Some(reason));
        }
        result
    }
    fn receive(&mut self,wait:Duration)->Result<bool,Reason> {
        let pipe=self.pipe.as_ref().ok_or(Reason::EyeOutput)?;
        let idle_deadline=Instant::now()+wait;
        loop {
            if self.lease.closed(){return Err(Reason::Abandoned);}
            let mut available=0;
            if unsafe{PeekNamedPipe(pipe.0,null_mut(),0,null_mut(),&mut available,null_mut())}==0{return Err(Reason::EyeOutput);}
            if available>0{break;}
            if Instant::now()>=idle_deadline{return Ok(false);}
            thread::sleep(Duration::from_millis(2));
        }
        let deadline=Instant::now()+IO_BOUND;
        let mut raw=[0;40];transfer(pipe,false,&mut raw,&self.lease,deadline)?;
        let h=Header::parse(&raw)?;
        if h.kind==3 {
            if self.worker_generation.is_some()||h.generation==0||!h.control(3,h.generation){return Err(Reason::Protocol);}
            self.worker_generation=Some(h.generation);self.status.phase=PreviewPhase::Receiving;return Ok(false);
        }
        if h.kind==5 {
            if self.worker_generation!=Some(h.generation)||!h.control(5,h.generation){return Err(Reason::Protocol);}
            self.end(None);return Ok(false);
        }
        if self.worker_generation!=Some(h.generation)||h.kind!=4||h.bytes as usize!=NV12_BYTES
            ||h.id==0||h.id<=self.last_id||h.time<self.last_time {return Err(Reason::Protocol);}
        let mut frame=PreviewFrame{generation:self.generation,worker_generation:h.generation,id:h.id,captured_ms:h.time,nv12:vec![0;NV12_BYTES]};
        transfer(pipe,false,&mut frame.nv12,&self.lease,deadline)?;
        self.last_id=h.id;self.last_time=h.time;self.status.received+=1;
        let now=tick();
        if h.time>now||now-h.time>MAX_AGE_MS {self.status.rejected+=1;self.status.expired+=1;self.clear(false);return Ok(false);}
        if let Some(output)=&self.output {
            self.lease.while_open(|open,_|if open{output.submit(&frame)}else{Err(Reason::Abandoned)})?;
        }
        self.clear(true);self.frame=Some(frame);Ok(true)
    }
    pub fn status(&mut self)->PreviewStatus {self.refresh();self.status}
    /// Native handoff only. None means render a neutral placeholder. The short,
    /// synchronous callback runs under the stream lease; it must not call back
    /// into Controller/StreamFeed or block. Never enqueue the borrowed old frame
    /// for later presentation. These counters are not GUI/meeting FPS evidence.
    pub fn with_current<T>(&mut self,present:impl FnOnce(Option<&PreviewFrame>)->T)->T {
        self.refresh();let lease=self.lease.clone();
        lease.while_open(|open,_|{
            if !open{self.end(self.status.reason);}
            // Recheck age after acquiring the lease, including delayed callers.
            let now=tick();
            if self.frame.as_ref().is_some_and(|f|f.captured_ms>now||now-f.captured_ms>MAX_AGE_MS){self.status.expired+=1;self.clear(false);}
            let result=present(self.frame.as_ref());
            if let Some(f)=&self.frame {
                if f.id==self.last_presented{self.status.repeated+=1;}else{self.status.presented+=1;self.last_presented=f.id;}
            }else{self.status.neutral+=1;}
            result
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use std::{fs::File,io::{Read,Write},os::windows::io::FromRawHandle,sync::{atomic::{AtomicU64,Ordering},mpsc},thread::JoinHandle};
    static NEXT:AtomicU64=AtomicU64::new(1);
    fn header(kind:u16,bytes:u32,generation:u64,id:u64,time:u64)->Vec<u8> {
        let mut b=vec![0;40];b[..4].copy_from_slice(&0x31565854u32.to_le_bytes());b[4..6].copy_from_slice(&1u16.to_le_bytes());
        b[6..8].copy_from_slice(&kind.to_le_bytes());b[8..12].copy_from_slice(&bytes.to_le_bytes());b[16..24].copy_from_slice(&generation.to_le_bytes());
        b[24..32].copy_from_slice(&id.to_le_bytes());b[32..40].copy_from_slice(&time.to_le_bytes());b
    }
    struct Peer { _job:crate::stream::StreamJob,feed:StreamFeed,done:Option<JoinHandle<()>>,release:mpsc::Sender<()> }
    impl Drop for Peer {fn drop(&mut self){self.feed.shared.close();let _=self.release.send(());if let Some(done)=self.done.take(){done.join().unwrap();}}}
    fn peer(payload:Vec<u8>,bad_ack:bool,bad_session:bool)->Peer {
        let pid=unsafe{GetCurrentProcessId()};let mut session=0;assert_ne!(unsafe{ProcessIdToSessionId(pid,&mut session)},0);
        let name=format!(r"\\.\pipe\TingXian.Video.{:016x}{:016x}",pid,NEXT.fetch_add(1,Ordering::SeqCst));
        let wide:Vec<u16>=name.encode_utf16().chain(Some(0)).collect();
        let raw=unsafe{CreateNamedPipeW(wide.as_ptr(),PIPE_ACCESS_DUPLEX|FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE|PIPE_READMODE_BYTE|PIPE_WAIT|PIPE_REJECT_REMOTE_CLIENTS,1,2*1024*1024,4096,0,null())};
        assert_ne!(raw,INVALID_HANDLE_VALUE);
        let owned=handle(raw).unwrap();
        let (release,wait)=mpsc::channel();
        let done=thread::spawn(move||{
            if unsafe{ConnectNamedPipe(owned.0,null_mut())}==0&&unsafe{GetLastError()}!=ERROR_PIPE_CONNECTED{return;}
            let raw=owned.0;std::mem::forget(owned);let mut file=unsafe{File::from_raw_handle(raw)};
            let mut hello=[0;72];if file.read_exact(&mut hello).is_err(){return;}
            assert_eq!(&hello[..40],header(1,32,0,0,0));assert_eq!(&hello[40..],&[1;32]);
            let mut ack=header(2,0,0,0,0);if bad_ack{ack[0]=0;}
            if file.write_all(&ack).is_err(){return;}
            if file.write_all(&payload).is_err(){return;}
            let _=wait.recv_timeout(Duration::from_secs(3));
        });
        let mut bytes=[0u8;236];bytes[..4].copy_from_slice(&1u32.to_le_bytes());
        for (i,c) in wide.iter().enumerate(){bytes[4+i*2..6+i*2].copy_from_slice(&c.to_le_bytes());}
        bytes[196..200].copy_from_slice(&pid.to_le_bytes());bytes[200..204].copy_from_slice(&(if bad_session{session+1}else{session}).to_le_bytes());bytes[204..].fill(1);
        let (job,feed)=crate::stream::pair(crate::EyeProvider::Synthetic);job.shared.admit(41);
        assert!(job.shared.publish(NativeOutputEndpoint::parse(&bytes,pid).unwrap()));
        Peer{_job:job,feed,done:Some(done),release}
    }
    #[test]
    fn preview_rejects_wrong_peer_session_and_ack_before_accepting_any_pixels() {
        for (bad_ack,bad_session) in [(true,false),(false,true)] {
            let peer=peer(vec![],bad_ack,bad_session);
            assert!(matches!(peer.feed.open_preview(Duration::from_secs(1)),Err(Reason::Protocol)));
        }
    }
    #[test]
    fn preview_protocol_faults_and_partial_payload_clear_the_previous_frame() {
        let mut partial=header(4,NV12_BYTES as u32,7,2,tick());partial.extend_from_slice(&[1,2,3]);
        let mut bad_magic=header(4,NV12_BYTES as u32,7,2,tick());bad_magic[0]=0;
        for (mut bad,reason) in [
            (header(3,0,7,0,0),Reason::Protocol), // repeated begin
            (header(4,NV12_BYTES as u32,8,2,tick()),Reason::Protocol),
            (header(4,u32::MAX,7,2,tick()),Reason::Protocol),
            (header(4,NV12_BYTES as u32,7,1,tick()),Reason::Protocol),
            (header(5,0,8,0,0),Reason::Protocol),(bad_magic,Reason::Protocol),
            (vec![0x54],Reason::Timeout),(partial,Reason::Timeout),
        ] {
            let captured=tick();if bad.len()>=40&&bad[6..8]==4u16.to_le_bytes(){bad[32..40].copy_from_slice(&captured.to_le_bytes());}
            let mut bytes=header(3,0,7,0,0);bytes.extend(header(4,NV12_BYTES as u32,7,1,captured));bytes.extend(vec![188;NV12_BYTES]);bytes.extend(bad);
            let peer=peer(bytes,false,false);let mut preview=peer.feed.open_preview(Duration::from_secs(1)).unwrap();
            assert!(!preview.poll(Duration::from_millis(50)).unwrap());assert!(preview.poll(Duration::from_millis(50)).unwrap());
            preview.with_current(|f|assert!(f.is_some()));
            assert_eq!(preview.poll(Duration::from_millis(50)),Err(reason));
            preview.with_current(|f|assert!(f.is_none()));assert_eq!(preview.status().phase,PreviewPhase::Fault);
            assert_eq!(preview.status().presented,1);
            peer.feed.shared.close();assert_eq!(preview.status().reason,Some(reason),"later teardown concealed the original protocol fault");
        }
    }
    #[test]
    fn preview_future_or_expired_frames_are_neutral_and_end_is_terminal() {
        for time in [tick().saturating_sub(1000),tick()+10000] {
            let mut bytes=header(3,0,7,0,0);bytes.extend(header(4,NV12_BYTES as u32,7,1,time));bytes.extend(vec![188;NV12_BYTES]);bytes.extend(header(5,0,7,0,0));
            let peer=peer(bytes,false,false);let mut preview=peer.feed.open_preview(Duration::from_secs(1)).unwrap();
            preview.poll(Duration::from_millis(50)).unwrap();assert!(!preview.poll(Duration::from_millis(50)).unwrap());
            preview.with_current(|f|assert!(f.is_none()));assert_eq!((preview.status().received,preview.status().rejected,preview.status().expired),(1,1,1));
            assert!(!preview.poll(Duration::from_millis(50)).unwrap());assert_eq!(preview.status().phase,PreviewPhase::Stopped);
        }
    }
    #[test]
    fn preview_stop_interrupts_a_pending_partial_packet_and_never_reopens() {
        let mut bytes=header(3,0,7,0,0);bytes.push(0x54);
        let peer=peer(bytes,false,false);let mut preview=peer.feed.open_preview(Duration::from_secs(1)).unwrap();preview.poll(Duration::from_millis(50)).unwrap();
        let lease=peer.feed.shared.clone();let stop=thread::spawn(move||{thread::sleep(Duration::from_millis(40));lease.close();});
        let started=Instant::now();assert!(!preview.poll(Duration::from_millis(50)).unwrap());assert!(started.elapsed()<Duration::from_millis(400));
        stop.join().unwrap();preview.with_current(|f|assert!(f.is_none()));assert_eq!(preview.status().phase,PreviewPhase::Stopped);
        assert!(!preview.poll(Duration::ZERO).unwrap());
    }
}
