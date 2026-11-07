//! Bounded inherited-pipe transport owned by the controller's existing Job.
use super::*;
use crate::{stream::{Next, Shared, StreamJob}, Controller, NativeOutputEndpoint, StreamReport};
use std::{sync::Arc, thread::{self, JoinHandle}};
use windows_sys::Win32::System::{IO::CancelSynchronousIo, Pipes::PeekNamedPipe};

const READY_MAGIC:u32=0x52455854;
const READY_BYTES:usize=260;
const MAX_WIRE:usize=READY_BYTES+4096;

fn write_all(pipe:&Handle,mut bytes:&[u8])->bool {
    while !bytes.is_empty() {
        let mut count=0;
        if unsafe {WriteFile(pipe.0,bytes.as_ptr(),bytes.len() as u32,&mut count,null_mut())}==0||count==0 {return false;}
        bytes=&bytes[count as usize..];
    }
    true
}
fn send_input(pipe:Handle,shared:Arc<Shared>,generation:u64) {
    loop {
        let frame=match shared.next() {Next::Frame(frame)=>Some(frame),Next::Finish=>None,Next::Closed=>return};
        let mut header=[0u8;40];
        header[0..4].copy_from_slice(&0x31495854u32.to_le_bytes());
        header[4..6].copy_from_slice(&1u16.to_le_bytes());
        header[6..8].copy_from_slice(&(if frame.is_some(){1u16}else{2}).to_le_bytes());
        header[16..24].copy_from_slice(&generation.to_le_bytes());
        if let Some(frame)=&frame {
            header[8..12].copy_from_slice(&(frame.bgr.len() as u32).to_le_bytes());
            header[24..32].copy_from_slice(&frame.id.to_le_bytes());
            header[32..40].copy_from_slice(&frame.captured_ms.to_le_bytes());
        }
        let header_written=!shared.closed()&&write_all(&pipe,&header);
        match frame {
            Some(frame)=>{
                let complete=header_written&&!shared.closed()&&write_all(&pipe,&frame.bgr);
                shared.settled(complete);if !complete{return;}
            }
            None=>return,
        }
    }
}
struct Owned {
    process:Process,writer:Option<JoinHandle<()>>,shared:Arc<Shared>,
}
impl Owned {
    fn retire(&mut self)->Result<(),Reason> {
        self.shared.close();
        let dead=self.process.terminate();
        let deadline=Instant::now()+Duration::from_secs(1);
        if let Some(writer)=&self.writer {
            // Cancellation is a request, not proof of retirement. Closing the
            // sole child's read end and observing the writer finish are both
            // required; cancellation races are retried only within this bound.
            while !writer.is_finished()&&Instant::now()<deadline {
                unsafe {CancelSynchronousIo(writer.as_raw_handle());}
                thread::sleep(Duration::from_millis(2));
            }
            if !writer.is_finished() {return Err(Reason::Cleanup);}
            if self.writer.take().unwrap().join().is_err() {return Err(Reason::Cleanup);}
        }
        dead
    }
}
impl Drop for Owned { fn drop(&mut self) {let _=self.retire();} }
#[derive(Default)]
struct Wire { bytes:Vec<u8>,announced:bool }
impl Drop for Wire { fn drop(&mut self){for b in &mut self.bytes {unsafe{std::ptr::write_volatile(b,0);}}} }
impl Wire {
    fn ready(&mut self,generation:u64,pid:u32)->Result<Option<NativeOutputEndpoint>,Reason> {
        if self.announced||self.bytes.len()<4 {return Ok(None);}
        let b=&self.bytes;
        if u32::from_le_bytes(b[0..4].try_into().unwrap())!=READY_MAGIC {return Ok(None);}
        if b.len()<READY_BYTES {return Ok(None);}
        if u16::from_le_bytes(b[4..6].try_into().unwrap())!=1
            ||u16::from_le_bytes(b[6..8].try_into().unwrap())!=1
            ||u32::from_le_bytes(b[8..12].try_into().unwrap())!=236
            ||b[12..16]!=[0;4]||u64::from_le_bytes(b[16..24].try_into().unwrap())!=generation {return Err(Reason::Protocol);}
        let endpoint=NativeOutputEndpoint::parse(&b[24..READY_BYTES],pid)?;
        let remaining=self.bytes.len()-READY_BYTES;
        self.bytes.copy_within(READY_BYTES..,0);
        self.bytes[remaining..].fill(0);self.bytes.truncate(remaining);self.announced=true;
        Ok(Some(endpoint))
    }
    fn pump(&mut self,pipe:&Handle)->Result<(),Reason> {
        let mut available=0;
        if unsafe {PeekNamedPipe(pipe.0,null_mut(),0,null_mut(),&mut available,null_mut())}==0 {
            return if unsafe{GetLastError()}==ERROR_BROKEN_PIPE {Ok(())} else {Err(Reason::Protocol)};
        }
        if available==0{return Ok(());}
        let limit=if self.announced{4096}else{MAX_WIRE};
        if self.bytes.len()+available as usize>limit {return Err(Reason::Protocol);}
        let old=self.bytes.len();self.bytes.resize(old+available as usize,0);
        let mut read=0;
        if unsafe {ReadFile(pipe.0,self.bytes[old..].as_mut_ptr(),available,&mut read,null_mut())}==0||read!=available {return Err(Reason::Protocol);}
        Ok(())
    }
}
pub(super) fn execute(mut process:Process,signal:&Signal,job:&StreamJob,generation:u64,timeout:Duration,controller:&Controller)->Result<StreamReport,Reason> {
    let input=process.input.take().ok_or(Reason::Launch)?;
    let shared=job.shared.clone();
    let writer=thread::Builder::new().name("eye-stream-input".into()).spawn(move||send_input(input,shared,generation)).map_err(|_|Reason::Launch)?;
    let mut owned=Owned{process,writer:Some(writer),shared:job.shared.clone()};
    let result=monitor(&owned.process,signal,job,generation,timeout,controller);
    // Never settle the controller's slot before its owned process and writer.
    owned.retire()?;result
}
fn monitor(process:&Process,signal:&Signal,job:&StreamJob,generation:u64,timeout:Duration,controller:&Controller)->Result<StreamReport,Reason> {
    let mut wire=Wire::default();let started=Instant::now();let mut stop_deadline=None;
    loop {
        wire.pump(&process.output)?;
        if let Some(endpoint)=wire.ready(generation,unsafe{GetProcessId(process.process.0)})? {
            if !controller.publish_stream_endpoint(generation,endpoint) {return Err(Reason::Abandoned);}
        }
        match unsafe {WaitForSingleObject(process.process.0,10)} {
            WAIT_OBJECT_0=>break,WAIT_TIMEOUT=>{},_=>return Err(Reason::Cleanup),
        }
        if signal.stopped()&&stop_deadline.is_none(){stop_deadline=Some(Instant::now()+Duration::from_secs(1));}
        if stop_deadline.is_some_and(|d|Instant::now()>=d){return Err(Reason::ForcedStop);}
        if started.elapsed()>=timeout||(!wire.announced&&started.elapsed()>=Duration::from_secs(30)){return Err(Reason::Timeout);}
    }
    // Child exit closes its last report writer. Drain remaining bytes before
    // parsing, but never revive readiness after it has already exited.
    wire.pump(&process.output)?;
    let _=wire.ready(generation,unsafe{GetProcessId(process.process.0)})?;
    let mut code=0;
    if unsafe{GetExitCodeProcess(process.process.0,&mut code)}==0{return Err(Reason::Cleanup);}
    if code!=0{return Err(Reason::WorkerExit);}
    let report=StreamReport::parse(&wire.bytes,job.provider,generation)?;
    if (report.outcome==EyeOutcome::Completed&&!wire.announced)
        ||(report.outcome==EyeOutcome::Cancelled&&!signal.stopped()){return Err(Reason::Protocol);}
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ready()->Vec<u8> {
        let mut b=vec![0;260];b[0..4].copy_from_slice(&READY_MAGIC.to_le_bytes());
        b[4..6].copy_from_slice(&1u16.to_le_bytes());b[6..8].copy_from_slice(&1u16.to_le_bytes());
        b[8..12].copy_from_slice(&236u32.to_le_bytes());b[16..24].copy_from_slice(&7u64.to_le_bytes());
        b[24..28].copy_from_slice(&1u32.to_le_bytes());
        let name=format!(r"\\.\pipe\TingXian.Video.{}","a".repeat(32));
        for (i,c) in name.encode_utf16().enumerate(){b[28+i*2..30+i*2].copy_from_slice(&c.to_le_bytes());}
        b[220..224].copy_from_slice(&123u32.to_le_bytes());b[228..260].fill(1);b
    }
    #[test]
    fn stream_ready_checks_wire_identity_and_consumes_capability_once() {
        let mut wire=Wire{bytes:ready(),announced:false};
        wire.bytes.extend_from_slice(b"{\"protocol\":3}");
        assert!(wire.ready(7,123).unwrap().is_some());assert!(wire.ready(7,123).unwrap().is_none());
        assert_eq!(wire.bytes,b"{\"protocol\":3}");
        for offset in [4,6,8,12,16,24,28,220] {
            let mut bad=Wire{bytes:ready(),announced:false};bad.bytes[offset]^=2;
            assert!(matches!(bad.ready(7,123),Err(Reason::Protocol)),"header offset {offset}");
        }
        let mut partial=Wire{bytes:ready()[..259].to_vec(),announced:false};assert!(partial.ready(7,123).unwrap().is_none());
        assert!(!partial.announced);
    }
}
