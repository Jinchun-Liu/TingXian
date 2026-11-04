'use client';
import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {listen,UnlistenFn} from '@tauri-apps/api/event';
import {AnswerConversation} from '@/components/interview/AnswerConversation';
import type {DisplayTurn,PresentationPreferences} from '@/lib/interview-presentation';
type Snapshot={epoch:string;version:number;history:DisplayTurn[];preferences:PresentationPreferences;selected_id:string|null;call_notice?:string};
export default function InterviewDisplay(){
  const [snapshot,setSnapshot]=useState<Snapshot|null>(null),[message,setMessage]=useState('');
  const latest=useRef<Snapshot|null>(null);
  useEffect(()=>{
    let disposed=false,receivedLive=false,stop:UnlistenFn|undefined;
    const receive=(s:Snapshot|null)=>{if(disposed||!s)return;if(latest.current?.epoch===s.epoch&&latest.current.version>s.version)return;latest.current=s;setSnapshot(s);};
    void (async()=>{
      const off=await listen<Snapshot>('interview-presentation',e=>{receivedLive=true;receive(e.payload);});
      if(disposed){off();return;}stop=off;
      const initial=await invoke<Snapshot|null>('interview_presentation_get');
      // A delayed initialization response can belong to a previous owner
      // epoch. Once any live event has arrived, it is the authoritative state.
      if(!receivedLive)receive(initial);
    })().catch(e=>setMessage(String(e)));
    return()=>{disposed=true;stop?.();};
  },[]);
  const [picked,setPicked]=useState<{epoch:string;id:string|null}|null>(null);
  return <main className="interview-display">
    <div aria-live="assertive" className={snapshot?.call_notice ? "meeting-call" : "sr-only"}>{snapshot?.call_notice}</div>
    {message&&<p role="alert">{message}</p>}
    {snapshot?<AnswerConversation key={snapshot.epoch} history={snapshot.history} preferences={snapshot.preferences} selectedId={picked?.epoch===snapshot.epoch?picked.id:snapshot.selected_id} onSelect={id=>setPicked({epoch:snapshot.epoch,id})}/>:<p className="m-auto text-slate-400">等待主窗口同步对话…</p>}
  </main>;
}
