import {useEffect,useRef} from 'react';
import {listen} from '@tauri-apps/api/event';
export type SessionCommand='start'|'stop'|'pause'|'resume'|'toggle'|'cancel'|'clear'|'audio'|'history';
/** Only mounted by the main conversation owner. Every surface uses the same guards and transport. */
export function useSessionCommands(dispatch:(command:SessionCommand)=>Promise<void>){
  const current=useRef(dispatch),running=useRef(false);
  current.current=dispatch;
  const send=useRef(async(command:SessionCommand)=>{
    if(running.current)return;
    running.current=true;
    try{await current.current(command);}finally{running.current=false;}
  }).current;
  useEffect(()=>{
    let alive=true;
    const off=listen<SessionCommand>('tingxian-session-command',e=>{if(alive)void send(e.payload);});
    return()=>{alive=false;void off.then(f=>f()).catch(()=>{});};
  },[send]);
  return send;
}
