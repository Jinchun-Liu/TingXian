'use client';
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import {CapabilityFields,capabilitiesError,matchingCapabilities,type ModelCapabilities} from '@/components/interview/ModelCapabilities';

export type ContextBudget={context_tokens:number;max_output_tokens:number;safety_tokens:number};
export const defaultBudget:ContextBudget={context_tokens:262144,max_output_tokens:8192,safety_tokens:2048};
export const budgetValid=(v:ContextBudget)=>Object.values(v).every(Number.isSafeInteger)&&v.context_tokens>=4096&&v.context_tokens<=2097152&&v.max_output_tokens>=16&&v.max_output_tokens<=65536&&v.safety_tokens>=256&&v.safety_tokens<=16384&&v.max_output_tokens+v.safety_tokens<Math.min(262144,v.context_tokens);
export type RequestConfig = {model:string;connect_timeout_secs:number;first_answer_warning_secs:number;total_timeout_secs:number;budget?:ContextBudget;capabilities?:ModelCapabilities};
const defaults:RequestConfig={model:'deepseek-flash',connect_timeout_secs:5,first_answer_warning_secs:8,total_timeout_secs:20};
type Draft={model:string;connect_timeout_secs:string;first_answer_warning_secs:string;total_timeout_secs:string;budget:ContextBudget;capabilities?:ModelCapabilities};
const endpoint='https://api.deepseek.com/chat/completions';
const draftOf=(c:RequestConfig):Draft=>({model:c.model,connect_timeout_secs:String(c.connect_timeout_secs),first_answer_warning_secs:String(c.first_answer_warning_secs),total_timeout_secs:String(c.total_timeout_secs),budget:c.budget||{...defaultBudget},capabilities:c.capabilities});
const fields=[['connect_timeout_secs','连接超时（秒）',1,30],['first_answer_warning_secs','未收到建议的提示时间（秒）',1,119],['total_timeout_secs','单次总截止（秒）',5,120]] as const;

export function BudgetFields({value,disabled,onChange,prefix='回答'}:{value:ContextBudget;disabled:boolean;onChange:(value:ContextBudget)=>void;prefix?:string}){
  return <details><summary>上下文预算</summary><div className="grid sm:grid-cols-3 gap-2 mt-2">{([['context_tokens','模型窗口',4096,2097152],['max_output_tokens','模型输出上限',16,65536],['safety_tokens','安全余量',256,16384]] as const).map(([key,label,min,max])=><label className="text-sm" key={key}>{label}<input aria-label={`${prefix} ${label}`} className="w-full rounded border p-2" type="number" min={min} max={max} step={1} disabled={disabled} value={value[key]} onChange={e=>onChange({...value,[key]:Number(e.target.value)})}/></label>)}</div><p className="mt-2 text-xs text-slate-500">按服务商填写 tokens。单请求最多使用 262,144，含输出和余量；图片用量另计。</p></details>;
}

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
    const config:RequestConfig={model:draft.model.trim(),connect_timeout_secs:numbers[0],first_answer_warning_secs:numbers[1],total_timeout_secs:numbers[2],budget:draft.budget,capabilities:draft.capabilities};
    if(config.connect_timeout_secs<1 || config.connect_timeout_secs>30 || config.total_timeout_secs<5 || config.total_timeout_secs>120 || config.connect_timeout_secs>config.total_timeout_secs || config.first_answer_warning_secs<1 || config.first_answer_warning_secs>=config.total_timeout_secs){setMessage('连接超时须为1–30秒且不超过总截止；总截止须为5–120秒；等待提示须早于总截止。');return;}
    if(!budgetValid(draft.budget)){setMessage('请检查模型预算：输出上限和安全余量须小于模型窗口及 256K 应用上限。');return;}
    const capabilityError=capabilitiesError(draft.capabilities,config.model,draft.budget,endpoint);if(capabilityError){setMessage(capabilityError);return;}
    setSaving(true);
    try{const saved=await invoke<RequestConfig>('interview_request_config_save',{config});setDraft(draftOf(saved));onSaved?.(saved);setMessage('已保存，下次回答使用新设置。');}
    catch(e){setMessage(`保存失败：${String(e)}`);}
    finally{setSaving(false);}
  };
  return <div className="border-t pt-3 space-y-3">
    <h3 className="text-sm font-medium">回答请求设置</h3>
    <p className="text-xs text-slate-500">请求固定发送到 DeepSeek 官方服务，默认模型为 deepseek-flash。请填写自己账号可用的模型 ID；不同模型可能有不同能力和费用。超时保留部分答案；保存设置不会发起请求。</p>
    <label className="block text-sm">模型 ID<input aria-label="回答模型 ID" className="block w-full border rounded p-2 mt-1" value={draft.model} maxLength={64} disabled={locked || saving || !ready} onChange={e=>setDraft(d=>({...d,model:e.target.value,capabilities:matchingCapabilities(d.capabilities,e.target.value,d.budget,endpoint)}))} /></label>
    <div className="grid sm:grid-cols-3 gap-3">{fields.map(([key,label,min,max])=><label className="block text-sm" key={key}>{label}<input aria-label={label} type="number" step={1} min={min} max={max} className="block w-full border rounded p-2 mt-1" value={draft[key]} disabled={locked || saving || !ready} onChange={e=>setDraft(d=>({...d,[key]:e.target.value}))} /></label>)}</div>
    <BudgetFields value={draft.budget} disabled={locked||saving||!ready} onChange={budget=>setDraft(d=>({...d,budget,capabilities:matchingCapabilities(d.capabilities,d.model,budget,endpoint)}))}/>
    <CapabilityFields value={draft.capabilities} model={draft.model} endpoint={endpoint} budget={draft.budget} prefix="回答" disabled={locked||saving||!ready} onChange={capabilities=>setDraft(d=>({...d,capabilities}))}/>
    {locked && <p className="text-xs text-slate-500">请结束当前会话后修改。</p>}
    {message && <p className="text-sm" role="status">{message}</p>}
    <div className="flex flex-wrap gap-2">
      <Button variant="outline" size="sm" disabled={locked || saving || !ready} onClick={save}>{saving?'正在保存…':'保存回答请求设置'}</Button>
      <Button variant="outline" size="sm" disabled={locked || saving} onClick={()=>{setReload(n=>n+1);}}>重新读取请求设置</Button>
      <Button variant="outline" size="sm" disabled={locked || saving} onClick={()=>{revision.current++;setDraft(draftOf(defaults));setReady(true);setMessage('已恢复默认草稿，点击保存后生效。');}}>恢复默认请求设置</Button>
    </div>
  </div>;
}
