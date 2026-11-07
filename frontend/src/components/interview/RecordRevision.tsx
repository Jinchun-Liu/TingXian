'use client';
import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {Button} from '@/components/ui/button';
import {ChoiceMenu,ChoiceOption} from '@/components/ui/choice-menu';
import {DocumentRecord,RecordSegment,UserAssertion} from '@/lib/interview-documents';
type Revision=UserAssertion & {source:{text:string;location:string;hash:string}};
type History={versions:Revision[];next_before:number|null};
const activities={unknown:'未标注',planned:'计划中',in_progress:'进行中',completed:'本人声明已完成'};
export function RecordRevision({document,segment,locked,onSaved}:{document:DocumentRecord;segment:RecordSegment;locked:boolean;onSaved?:(record:UserAssertion)=>void}){
  const [current,setCurrent]=useState(segment.user_assertion),[editing,setEditing]=useState(false),[claim,setClaim]=useState(''),[reason,setReason]=useState('');
  const [activity,setActivity]=useState<UserAssertion['activity']>('unknown'),[confirmed,setConfirmed]=useState(false),[history,setHistory]=useState<History|null>(null),[error,setError]=useState(''),[busy,setBusy]=useState(false);
  const epoch=useRef(0),running=useRef(false);
  useEffect(()=>{epoch.current++;return()=>{epoch.current++;};},[document.root,document.hash,segment.source_id]);
  const source={source_id:segment.source_id,path:document.path,root:document.root,hash:document.hash};
  const edit=()=>{setClaim(current?.claim || segment.text);setActivity(current?.activity || 'unknown');setReason('');setConfirmed(false);setError('');setEditing(true);};
  const save=async()=>{
    if(locked||running.current||!confirmed||!claim.trim()||!reason.trim())return;
    running.current=true;setBusy(true);setError('');const request=epoch.current;
    try{
      const record=await invoke<Revision>('profile_record_revise',{update:{source,expected_revision:current?.revision || 0,claim,activity,reason,confirmed}});
      if(request===epoch.current){setCurrent(record);setEditing(false);setHistory(null);onSaved?.(record);}
    }catch(e){if(request===epoch.current)setError(String(e));}finally{if(request===epoch.current){running.current=false;setBusy(false);}}
  };
  const versions=async(before:number|null=null)=>{
    if(running.current)return;running.current=true;setBusy(true);setError('');const request=epoch.current;
    try{const next=await invoke<History>('profile_record_history',{source,before});if(request===epoch.current)setHistory(old=>({versions:before?[...(old?.versions || []),...next.versions]:next.versions,next_before:next.next_before}));}
    catch(e){if(request===epoch.current)setError(String(e));}finally{if(request===epoch.current){running.current=false;setBusy(false);}}
  };
  return <section className="space-y-2 rounded border bg-white p-3">
    <p className="text-slate-500">{segment.location}</p>
    {current?<><p className="whitespace-pre-wrap">{current.claim}</p><p className="text-slate-500">本人声明 · v{current.revision} · {activities[current.activity]}</p><details><summary className="cursor-pointer">原文</summary><p className="mt-2 whitespace-pre-wrap">{segment.text}</p></details></>:<p className="whitespace-pre-wrap">{segment.text}</p>}
    {!editing&&<div className="flex gap-2"><Button variant="outline" size="sm" disabled={locked||busy||document.excluded||document.path==='.背景/综合背景'} onClick={edit}>修订这条</Button>{current&&<Button variant="ghost" size="sm" disabled={busy} onClick={()=>versions()}>修订历史</Button>}</div>}
    {editing&&<div className="space-y-3">
      <label className="block">本人声明<textarea aria-label="本人声明" maxLength={1200} disabled={locked||busy} className="mt-1 min-h-24 w-full rounded border p-2" value={claim} onChange={e=>{setClaim(e.target.value);setConfirmed(false);}}/></label>
      <ChoiceMenu aria-label="完成状态" value={activity} disabled={locked||busy} onValueChange={v=>{setActivity(v as UserAssertion['activity']);setConfirmed(false);}}>{Object.entries(activities).map(([value,label])=><ChoiceOption key={value} value={value}>{label}</ChoiceOption>)}</ChoiceMenu>
      <label className="block">修订依据<textarea aria-label="修订依据" maxLength={300} disabled={locked||busy} className="mt-1 w-full rounded border p-2" value={reason} onChange={e=>{setReason(e.target.value);setConfirmed(false);}}/></label>
      <label className="flex items-center gap-2"><input type="checkbox" aria-label="确认本人修订" disabled={locked||busy} checked={confirmed} onChange={e=>setConfirmed(e.target.checked)}/>这是我确认的声明，原文保留</label>
      <div className="flex gap-2"><Button size="sm" disabled={locked||busy||!confirmed||!claim.trim()||!reason.trim()} onClick={save}>保存修订</Button><Button size="sm" variant="ghost" disabled={busy} onClick={()=>setEditing(false)}>取消</Button></div>
    </div>}
    {history&&<div className="space-y-2 border-t pt-2">{history.versions.map(v=><details key={v.revision}><summary className="cursor-pointer">v{v.revision} · {new Date(v.recorded_at).toLocaleString('zh-CN')}</summary><p className="whitespace-pre-wrap">{v.claim}</p><p>{activities[v.activity]} · {v.reason}</p></details>)}{history.next_before!==null&&<Button size="sm" variant="ghost" disabled={busy} onClick={()=>versions(history.next_before)}>更早版本</Button>}</div>}
    {error&&<p role="alert" className="text-amber-700">{error}</p>}
  </section>;
}
