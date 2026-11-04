'use client';
import {useCallback,useEffect,useRef,useState} from 'react';
import {addressedOnly,initialMeeting,MeetingAddress,meetingModes,MeetingSetup,readMeeting} from '@/lib/meeting-mode';
const storageKey='tingxian-meeting-v1';

export function useMeetingMode(clearQueue:()=>void) {
  const [setup,setSetup]=useState<MeetingSetup>(initialMeeting);
  const current=useRef(initialMeeting);
  const gate=useRef(new MeetingAddress());
  const [notice,setNotice]=useState('');
  const timer=useRef<ReturnType<typeof setTimeout>>();
  useEffect(()=>{
    try {const saved=readMeeting(JSON.parse(localStorage.getItem(storageKey)||'null'));current.current=saved;setSetup(saved);}catch{}
    return ()=>clearTimeout(timer.current);
  },[]);
  const reset=useCallback(()=>{gate.current.reset();clearTimeout(timer.current);setNotice('');},[]);
  const change=(next:MeetingSetup)=>{
    clearQueue();reset();current.current=next;setSetup(next);
    try{localStorage.setItem(storageKey,JSON.stringify(next));}catch{}
  };
  const decide=useCallback((text:string)=>{
    const result=gate.current.decide(current.current,text);
    if(result.alert){
      setNotice('正在叫你 · '+result.alert);clearTimeout(timer.current);
      timer.current=setTimeout(()=>setNotice(''),12000);
    }
    return result.answer;
  },[]);
  return {setup,current,change,notice,decide,reset,interrupt:()=>gate.current.reset()};
}
export function MeetingControls({setup,change}:{setup:MeetingSetup;change:(value:MeetingSetup)=>void}){
  return <div className="meeting-controls">
    <label>参会模式<select aria-label="参会模式" value={setup.mode} onChange={e=>change({...setup,mode:e.target.value as MeetingSetup['mode']})}>
      {Object.entries(meetingModes).map(([id,name])=><option key={id} value={id}>{name}</option>)}
    </select></label>
    {addressedOnly(setup.mode)&&<label>点名称呼<input aria-label="点名称呼" maxLength={240} placeholder="如：小林、林老师、Alex" value={setup.names} onChange={e=>change({...setup,names:e.target.value})}/>
      {!setup.names.trim()&&<small>未填写时只转写。可手动选题回答。</small>}</label>}
  </div>;
}
export function CallNotice({text}:{text?:string|null}){
  return <div aria-live="assertive" aria-atomic="true" className={text?'meeting-call':'sr-only'}>{text}</div>;
}
