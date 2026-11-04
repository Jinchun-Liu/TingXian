'use client';
import {useEffect,useState} from 'react';
import {invoke,isTauri} from '@tauri-apps/api/core';
import {Button} from '@/components/ui/button';
export function StorageGate({children}:{children:React.ReactNode}){
  const [ready,setReady]=useState(false),[error,setError]=useState(''),[attempt,setAttempt]=useState(0);
  useEffect(()=>{
    let live=true;
    if(!isTauri()){setReady(true);return;}
    void invoke('tingxian_storage_ready').then(()=>{if(live)setReady(true);}).catch(e=>{if(live)setError(String(e));});
    return()=>{live=false;};
  },[attempt]);
  if(ready)return <>{children}</>;
  return <main className="grid min-h-dvh place-content-center gap-4 p-5"><h1 className="text-xl">听弦</h1>{error?<><p role="alert">本机数据库未能打开：{error}</p><Button onClick={()=>{setError('');setAttempt(n=>n+1);}}>重新打开</Button></>:<p role="status">正在打开本机资料…</p>}</main>;
}
