'use client';
import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {Button} from '@/components/ui/button';
type Field='role'|'organization'|'context'|'terminology'|'call_names';
type Values=Record<Field,string>;
type Preview={version:string;profile_hash:string;current:Partial<Values>;suggestions:Values};
const fields=[['role','岗位',120],['organization','组织',120],['context','背景',1000],['terminology','术语',1000],['call_names','点名称呼',240]] as const;
export function BackgroundSettings({revision,locked,names='',applyNames,onBusy,onApplied}:{revision:string;locked:boolean;names?:string;applyNames?:(expected:string,value:string)=>boolean;onBusy:(busy:boolean)=>void;onApplied:()=>Promise<void>}){
  const [preview,setPreview]=useState<Preview|null>(null),[draft,setDraft]=useState<Partial<Values>>({}),[selected,setSelected]=useState<Partial<Record<Field,boolean>>>({});
  const [message,setMessage]=useState(''),[busy,setBusy]=useState(false),[reload,setReload]=useState(0);
  const alive=useRef(false),working=useRef(false),epoch=useRef(0);
  useEffect(()=>{
    alive.current=true;const request=++epoch.current;setPreview(null);setSelected({});setMessage('');
    void invoke<Preview|null>('background_settings_preview').then(value=>{
      if(!alive.current||request!==epoch.current)return;
      if(value){setPreview({...value,current:{...value.current,call_names:names}});setDraft(value.suggestions);}
    }).catch(e=>{if(alive.current&&request===epoch.current)setMessage(String(e));});
    return()=>{alive.current=false;};
  },[revision,reload]);
  const apply=async()=>{
    if(locked||working.current||!preview)return;
    const chosen=fields.filter(([key])=>selected[key]);if(!chosen.length)return;
    if(chosen.some(([key,,max])=>(draft[key]||'').length>max)){setMessage('所选设置超过长度限制，请先精简。');return;}
    working.current=true;setBusy(true);onBusy(true);setMessage('');
    let committed=false;const request=epoch.current;
    try{
      const changes=chosen.filter(([field])=>field!=='call_names').map(([field])=>({field,expected:preview.current[field]||'',value:draft[field]||''}));
      await invoke('background_settings_apply',{version:preview.version,profileHash:preview.profile_hash,changes});
      committed=changes.length>0;
      // The database operation may finish after navigation; do not let an old
      // panel change local reminder state or refresh a newly mounted panel.
      if(!alive.current||request!==epoch.current)return;
      const namesApplied=!selected.call_names||!!applyNames?.(preview.current.call_names||'',draft.call_names||'');
      committed=changes.length>0||(!!selected.call_names&&namesApplied);
      if(alive.current){setPreview(p=>p?{...p,current:{...p.current,...Object.fromEntries(changes.map(c=>[c.field,c.value])),...(selected.call_names&&namesApplied?{call_names:draft.call_names||''}:{})}}:p);setSelected({});}
      await onApplied();
      if(alive.current){setSelected({});setMessage(namesApplied?'已应用所选设置。':`${changes.length?'场景设置已保存；':''}点名称呼已变化或无法保存，已保留当前值。请重新读取差异。`);}
    }catch(e){if(alive.current)setMessage(`${committed?'设置已保存，但刷新失败：':''}${String(e)}`);}
    finally{working.current=false;if(alive.current)setBusy(false);onBusy(false);}
  };
  if(!preview)return message?<p role="status">{message}<Button size="sm" variant="outline" onClick={()=>setReload(n=>n+1)}>重新读取差异</Button></p>:null;
  const shown=fields.filter(([key])=>(preview.suggestions[key]||'').trim()&&(key!=='call_names'||applyNames));
  if(!shown.length)return null;
  return <section className="space-y-3 border-t pt-3" aria-label="背景设置建议"><h4 className="font-medium">设置建议</h4>
    <div className="hidden sm:grid grid-cols-[5rem_1fr_1fr] gap-3 text-xs text-slate-500"><span/><span>当前值</span><span>建议值</span></div>
    {shown.map(([key,label,max])=><div key={key} className="grid sm:grid-cols-[5rem_1fr_1fr] gap-3 items-start">
      <label className="flex gap-2 items-center"><input aria-label={`应用${label}`} type="checkbox" checked={!!selected[key]} disabled={locked||busy} onChange={e=>setSelected(s=>({...s,[key]:e.target.checked}))}/>{label}</label>
      <p className="whitespace-pre-wrap break-words text-sm rounded bg-slate-50 p-2">{preview.current[key]||'—'}</p>
      <textarea aria-label={`${label}建议`} className="w-full rounded border p-2" rows={key==='context'?4:2} maxLength={max} disabled={locked||busy} value={draft[key]||''} onChange={e=>setDraft(d=>({...d,[key]:e.target.value}))}/>
    </div>)}
    <p className="text-xs text-slate-500">应用后的岗位、组织、背景和术语会用于后续获准发送的 AI 请求；点名称呼仅用于本机提醒。</p>
    <div className="flex flex-wrap gap-2"><Button size="sm" disabled={locked||busy||!shown.some(([key])=>selected[key])} onClick={()=>void apply()}>应用所选设置</Button><Button size="sm" variant="outline" disabled={locked||busy} onClick={()=>setReload(n=>n+1)}>重新读取差异</Button></div>
    {message&&<p role="status">{message}</p>}
  </section>;
}
