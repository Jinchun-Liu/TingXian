'use client';
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';

export type RequestConfig = {model:string;connect_timeout_secs:number;first_answer_warning_secs:number;total_timeout_secs:number};
const defaults:RequestConfig={model:'deepseek-flash',connect_timeout_secs:5,first_answer_warning_secs:8,total_timeout_secs:20};
type Draft={model:string;connect_timeout_secs:string;first_answer_warning_secs:string;total_timeout_secs:string};
const draftOf=(c:RequestConfig):Draft=>({model:c.model,connect_timeout_secs:String(c.connect_timeout_secs),first_answer_warning_secs:String(c.first_answer_warning_secs),total_timeout_secs:String(c.total_timeout_secs)});
const fields=[['connect_timeout_secs','连接超时（秒）',1,30],['first_answer_warning_secs','未收到建议的提示时间（秒）',1,119],['total_timeout_secs','单次总截止（秒）',5,120]] as const;

export function RequestSettings({locked,onSaved}:{locked:boolean;onSaved?:(config:RequestConfig)=>void}) {
  const [draft,setDraft]=useState<Draft>(()=>draftOf(defaults));
  const [ready,setReady]=useState(false);
  const [saving,setSaving]=useState(false);
  const [message,setMessage]=useState('正在读取回答请求设置…');
  const [reload,setReload]=useState(0);
  const revision=useRef(0);
  useEffect(()=>{
    let current=true;const request=++revision.current;setReady(false);setMessage('正在读取回答请求设置…');
    invoke<RequestConfig>('interview_request_config_get').then(config=>{if(current && request===revision.current){setDraft(draftOf(config));setReady(true);setMessage('');}}).catch(e=>{if(current && request===revision.current)setMessage(`读取失败：${String(e)}。可重试或恢复默认后保存。`);});
    return ()=>{current=false;};
  },[reload]);
  const save=async()=>{
    setMessage('');const numbers=fields.map(([key])=>Number(draft[key]));
    if(numbers.some((value,i)=>!/^\d+$/.test(draft[fields[i][0]]) || !Number.isSafeInteger(value))){setMessage('超时时间须填写整数秒，不能留空。');return;}
    const config:RequestConfig={model:draft.model.trim(),connect_timeout_secs:numbers[0],first_answer_warning_secs:numbers[1],total_timeout_secs:numbers[2]};
    if(config.connect_timeout_secs<1 || config.connect_timeout_secs>30 || config.total_timeout_secs<5 || config.total_timeout_secs>120 || config.connect_timeout_secs>config.total_timeout_secs || config.first_answer_warning_secs<1 || config.first_answer_warning_secs>=config.total_timeout_secs){setMessage('连接超时须为1–30秒且不超过总截止；总截止须为5–120秒；等待提示须早于总截止。');return;}
    setSaving(true);
    try{const saved=await invoke<RequestConfig>('interview_request_config_save',{config});setDraft(draftOf(saved));onSaved?.(saved);setMessage('已保存，下次回答使用新设置。');}
    catch(e){setMessage(`保存失败：${String(e)}`);}
    finally{setSaving(false);}
  };
  return <div className="border-t pt-3 space-y-3">
    <h3 className="text-sm font-medium">回答请求设置</h3>
    <p className="text-xs text-slate-500">请求固定发送到 DeepSeek 官方服务，默认模型为 deepseek-flash。请填写自己账号可用的模型 ID；不同模型可能有不同能力和费用。超时保留部分答案；保存设置不会发起请求。</p>
    <label className="block text-sm">模型 ID<input aria-label="回答模型 ID" className="block w-full border rounded p-2 mt-1" value={draft.model} maxLength={64} disabled={locked || saving || !ready} onChange={e=>setDraft(d=>({...d,model:e.target.value}))} /></label>
    <div className="grid sm:grid-cols-3 gap-3">{fields.map(([key,label,min,max])=><label className="block text-sm" key={key}>{label}<input aria-label={label} type="number" step={1} min={min} max={max} className="block w-full border rounded p-2 mt-1" value={draft[key]} disabled={locked || saving || !ready} onChange={e=>setDraft(d=>({...d,[key]:e.target.value}))} /></label>)}</div>
    {locked && <p className="text-xs text-slate-500">请结束当前会话后修改。</p>}
    {message && <p className="text-sm" role="status">{message}</p>}
    <div className="flex flex-wrap gap-2">
      <Button variant="outline" size="sm" disabled={locked || saving || !ready} onClick={save}>{saving?'正在保存…':'保存回答请求设置'}</Button>
      <Button variant="outline" size="sm" disabled={locked || saving} onClick={()=>{setReload(n=>n+1);}}>重新读取请求设置</Button>
      <Button variant="outline" size="sm" disabled={locked || saving} onClick={()=>{revision.current++;setDraft(draftOf(defaults));setReady(true);setMessage('已恢复默认草稿，点击保存后生效。');}}>恢复默认请求设置</Button>
    </div>
  </div>;
}
