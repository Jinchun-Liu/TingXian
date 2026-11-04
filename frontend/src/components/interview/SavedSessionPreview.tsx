'use client';
import {useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {parseSessionSnapshot,SessionSnapshot} from '@/lib/interview-session';
import {Button} from '@/components/ui/button';

export function SavedSessionPreview({id}:{id:number}){
  const [snapshot,setSnapshot]=useState<SessionSnapshot|null>(null),[error,setError]=useState(''),[busy,setBusy]=useState(false);
  return <div className="tx-snapshot-preview"><Button size="sm" variant="outline" disabled={busy} onClick={async()=>{
    if(snapshot){setSnapshot(null);return;}setBusy(true);setError('');
    try{setSnapshot(parseSessionSnapshot(await invoke('tingxian_session_preview',{id})));}catch(e){setError('读取失败：'+String(e));}finally{setBusy(false);}
  }}>{snapshot?'收起详情':'查看详情'}</Button>
  {snapshot&&<div className="tx-snapshot-body">{snapshot.history.map(t=><div key={t.id}><p>{t.role==='self'?'我':t.role==='manual'?'手动提问':'对方'} · {t.question}</p>{t.suggestion&&<p className="whitespace-pre-wrap">{t.suggestion}</p>}</div>)}
  <Button size="sm" variant="outline" onClick={async()=>{try{
    const {content}=await invoke<{content:string}>('interview_session_export',{snapshot});const url=URL.createObjectURL(new Blob([content],{type:'application/json'}));
    const a=document.createElement('a');a.href=url;a.download='听弦-会话-'+id+'.json';a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);
  }catch(e){setError('导出失败：'+String(e));}}}>导出这份会话</Button></div>}{error&&<p role="alert">{error}</p>}</div>;
}
