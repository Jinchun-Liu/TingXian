'use client';
import {useState,useEffect,useRef} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {indexedDBService} from '@/services/indexedDBService';
import {Button} from '@/components/ui/button';
type Entry={id:string;title:string;kind:'meeting'|'recovery'};
type Detail={title:string;transcripts:{timestamp:string;text:string}[]};
export function PreviousMeetings({initialId}:{initialId?:string}){
  const [rows,setRows]=useState<Entry[]>([]),[detail,setDetail]=useState<Detail|null>(null),[message,setMessage]=useState(''),[loading,setLoading]=useState(true);
  const [revision,setRevision]=useState(0),seq=useRef(0);
  async function open(entry:Entry){
    const version=++seq.current;setDetail(null);setMessage('');
    try{
      const data=entry.kind==='meeting'?await invoke<Detail>('api_get_meeting',{meetingId:entry.id,authToken:null}):
        {title:entry.title,transcripts:await indexedDBService.getTranscripts(entry.id,true)};
      if(version===seq.current)setDetail(data);
    }catch(e){if(version===seq.current)setMessage('无法打开这条记录：'+String(e));}
  }
  useEffect(()=>{
    let live=true;setLoading(true);setMessage('');
    async function read(){
      const [sql,cache]=await Promise.allSettled([invoke<{id:string;title:string}[]>('api_get_meetings',{authToken:null}),indexedDBService.getAllMeetings(true)]);
      if(!live)return;
      setRows([...(sql.status==='fulfilled'?sql.value:[]).map(r=>({...r,kind:'meeting' as const})),...(cache.status==='fulfilled'?cache.value:[]).map(r=>({id:r.meetingId,title:r.title,kind:'recovery' as const}))]);
      if(sql.status==='rejected'||cache.status==='rejected')setMessage('部分旧记录未能读取，请重试。');
      setLoading(false);
      if(initialId)void open({kind:'meeting',id:initialId,title:''});
    }
    void read();return()=>{live=false;seq.current++;};
  },[initialId,revision]);
  return <section aria-label="以往会议" className="space-y-2 border-t pt-3">
    <h3>以往会议</h3>{message&&<p role="alert">{message} <button onClick={()=>setRevision(n=>n+1)}>重试</button></p>}
    {loading?<p>正在读取…</p>:!rows.length&&!message?<p>暂无旧会议</p>:null}
    {rows.map(r=><button type="button" key={r.kind+r.id} className="block text-left underline" onClick={()=>void open(r)}>{r.title}{r.kind==='recovery'?' · 未保存转写':''}</button>)}
    {detail&&<article className="space-y-2"><h4>{detail.title}</h4>{detail.transcripts.map((t,i)=><p key={i}><time>{t.timestamp}</time> {t.text}</p>)}{!detail.transcripts.length&&<p>这次会议没有转写</p>}
      <Button variant="outline" size="sm" onClick={()=>{
        const url=URL.createObjectURL(new Blob([JSON.stringify(detail,null,2)],{type:'application/json'}));const a=document.createElement('a');
        a.href=url;a.download='听弦-旧会议.json';a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);
      }}>导出会议转写</Button></article>}
  </section>;
}
