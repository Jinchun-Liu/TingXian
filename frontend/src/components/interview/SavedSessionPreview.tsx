'use client';
import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {parseSessionSnapshot,SessionSnapshot} from '@/lib/interview-session';
import {Button} from '@/components/ui/button';

export function SavedSessionPreview({id,onReview}:{id:number;onReview?:(snapshot:SessionSnapshot)=>void}){
  const [snapshot,setSnapshot]=useState<SessionSnapshot|null>(null),[error,setError]=useState(''),[busy,setBusy]=useState(false);
  const epoch=useRef(0);
  useEffect(()=>{epoch.current++;setSnapshot(null);setError('');setBusy(false);return()=>{epoch.current++;};},[id]);
  return <div className="tx-snapshot-preview"><Button size="sm" variant="outline" disabled={busy} onClick={async()=>{
    if(snapshot){setSnapshot(null);return;}const generation=epoch.current;setBusy(true);setError('');
    try{const value=parseSessionSnapshot(await invoke('tingxian_session_preview',{id}));if(generation===epoch.current)setSnapshot(value);}catch(e){if(generation===epoch.current)setError('读取失败：'+String(e));}finally{if(generation===epoch.current)setBusy(false);}
  }}>{snapshot?'收起详情':'查看详情'}</Button>
  {onReview&&<Button size="sm" variant="outline" disabled={busy} onClick={async()=>{const generation=epoch.current;setBusy(true);setError('');try{const value=snapshot??parseSessionSnapshot(await invoke('tingxian_session_preview',{id}));if(generation===epoch.current)onReview(value);}catch(e){if(generation===epoch.current)setError('读取失败：'+String(e));}finally{if(generation===epoch.current)setBusy(false);}}}>复盘这份会话</Button>}
  {snapshot&&<div className="tx-snapshot-body">{snapshot.history.map(t=><div key={t.id}><p>{t.role==='self'?'我':t.role==='manual'?'手动提问':'对方'} · {t.question}</p>{t.suggestion&&<p className="whitespace-pre-wrap">{t.suggestion}</p>}</div>)}
  {snapshot.input_drafts?.question&&<p className="whitespace-pre-wrap">{snapshot.input_drafts.self_edit?'未提交的本人修正':'未提交的问题'} · {snapshot.input_drafts.question}</p>}
  {snapshot.input_drafts?.confirmed_answer&&<p className="whitespace-pre-wrap">未确认的本人回答 · {snapshot.input_drafts.confirmed_answer}</p>}
  <Button size="sm" variant="outline" onClick={async()=>{try{
    const {content}=await invoke<{content:string}>('interview_session_export',{snapshot});const url=URL.createObjectURL(new Blob([content],{type:'application/json'}));
    const a=document.createElement('a');a.href=url;a.download='听弦-会话-'+id+'.json';a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);
  }catch(e){setError('导出失败：'+String(e));}}}>导出这份会话</Button></div>}{error&&<p role="alert">{error}</p>}</div>;
}
