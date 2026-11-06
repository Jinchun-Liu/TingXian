import {useEffect,useRef,useState,useSyncExternalStore} from 'react';
import {invoke,isTauri} from '@tauri-apps/api/core';
import {interviewExit} from '@/lib/interview-exit';
import {RecoverySnapshot,RecoveryWriter} from '@/lib/interview-recovery';
export function useInterviewRecovery(read:()=>RecoverySnapshot){
  const latest=useRef(read);latest.current=read;
  const [writer]=useState(()=>new RecoveryWriter(invoke,()=>latest.current(),()=>crypto.randomUUID()));
  const view=useSyncExternalStore(writer.subscribe,writer.getSnapshot,writer.getServerSnapshot);
  useEffect(()=>{
    if(!isTauri())return;
    void writer.initialize().catch(()=>{});
    const timer=setInterval(()=>writer.tick(),1000);
    const unregister=interviewExit.registerCheckpoint(()=>writer.flush());
    return()=>{clearInterval(timer);unregister();void writer.flush().catch(()=>{});};
  },[writer]);
  return {writer,...view};
}
