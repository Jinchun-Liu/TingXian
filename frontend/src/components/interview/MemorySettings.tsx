'use client';
import {useEffect,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {Button} from '@/components/ui/button';
import {ChoiceMenu,ChoiceOption} from '@/components/ui/choice-menu';
import {BudgetFields,budgetValid,defaultBudget} from '@/components/interview/RequestSettings';
import {CapabilityFields,capabilitiesError,matchingCapabilities} from '@/components/interview/ModelCapabilities';
import type {RequestConfig} from './RequestSettings';
import type {SessionMemory,MemoryStatus} from '@/lib/interview-memory';
type Profile={id:string;name:string;enabled:boolean;endpoint:string;protocol:'deepseek'|'qwen'|'openai';request:RequestConfig};
type Config={enabled:boolean;profile:Profile;min_events:number;cooldown_ms:number;max_requests:number;session_units:number};
export function MemorySettings({locked,onSaved,onBusyChange=()=>{}}:{locked:boolean;onSaved:()=>Promise<void>;onBusyChange?:(busy:boolean)=>void}){
  const [config,setConfig]=useState<Config|null>(null),[key,setKey]=useState(''),[keyReady,setKeyReady]=useState(false),[working,setWorking]=useState(false),[message,setMessage]=useState('');
  const load=async()=>{const v=await invoke<{config:Config;key_configured:boolean}>('interview_memory_config_get');setConfig(v.config);setKeyReady(v.key_configured);};
  useEffect(()=>{void load().catch(e=>setMessage(String(e)));},[]);
  if(!config)return <p role="status">{message||'正在读取会话记忆设置…'}</p>;
  const p=config.profile,disabled=locked||working;
  const patch=(change:Partial<Profile>)=>setConfig(c=>{if(!c)return c;const p={...c.profile,...change};return {...c,profile:{...p,request:{...p.request,capabilities:matchingCapabilities(p.request.capabilities,p.request.model,p.request.budget||defaultBudget,p.endpoint)}}};});
  const save=async()=>{
    setWorking(true);onBusyChange(true);setMessage('');
    try{
      if(!budgetValid(p.request.budget||defaultBudget))throw Error('请检查压缩接口的上下文预算');
      const e=capabilitiesError(p.request.capabilities,p.request.model,p.request.budget||defaultBudget,p.endpoint);if(e)throw Error(e);
      await invoke('interview_memory_config_save',{config,key:key.trim()||null});setKey('');await load();await onSaved();setMessage('已保存，未发送模型请求。');
    }catch(e){setMessage(String(e));}finally{setWorking(false);onBusyChange(false);}
  };
  return <details className="border rounded-lg p-3" aria-label="会话记忆设置"><summary>会话记忆 · {config.enabled?'已启用':'关闭'}</summary><div className="mt-3 space-y-3 text-sm">
    <label className="flex items-center gap-2"><input type="checkbox" checked={config.enabled} disabled={disabled} onChange={e=>setConfig({...config,enabled:e.target.checked})}/>启用会话记忆压缩</label>
    <p className="text-xs text-slate-500">独立于三路回答。仅在本次会话允许发送 AI 时整理已结束的旧发言。</p>
    <div className="grid sm:grid-cols-2 gap-3"><label>接口类型<ChoiceMenu aria-label="压缩接口类型" value={p.protocol} disabled={disabled} onValueChange={protocol=>patch({protocol:protocol as Profile['protocol']})}><ChoiceOption value="deepseek">DeepSeek</ChoiceOption><ChoiceOption value="qwen">千问 / 百炼</ChoiceOption><ChoiceOption value="openai">OpenAI 兼容 / 本地</ChoiceOption></ChoiceMenu></label><label>模型 ID<input aria-label="压缩模型 ID" className="w-full border rounded p-2" value={p.request.model} disabled={disabled} maxLength={128} onChange={e=>patch({request:{...p.request,model:e.target.value}})}/></label></div>
    <label className="block">完整 API 地址<input aria-label="压缩 API 地址" className="w-full border rounded p-2" value={p.endpoint} disabled={disabled} onChange={e=>patch({endpoint:e.target.value.trim()})}/></label>
    <label className="block">API Key<input aria-label="压缩 API Key" className="w-full border rounded p-2" type="password" autoComplete="off" value={key} disabled={disabled} onChange={e=>setKey(e.target.value)} placeholder={keyReady?'已配置；留空保留本次运行凭证':'仅在本次运行有效'}/></label>
    <details><summary>调用限制</summary><div className="grid sm:grid-cols-2 gap-3 mt-2">{([['min_events','最少旧发言',8,64,1],['cooldown_ms','冷却间隔（毫秒）',30000,3600000,1000],['max_requests','每场最多请求',1,32,1],['session_units','每场预算预留单位',4096,2000000,1000]] as const).map(([field,label,min,max,step])=><label key={field}>{label}<input aria-label={`压缩 ${label}`} className="w-full border rounded p-2" type="number" min={min} max={max} step={step} disabled={disabled} value={config[field]} onChange={e=>setConfig({...config,[field]:Number(e.target.value)})}/></label>)}</div><p className="text-xs text-slate-500 mt-2">无实际用量时按 UTF-8 字节与输出上限预留；不是实际 tokens 或金额。</p></details>
    <BudgetFields prefix="压缩" value={p.request.budget||defaultBudget} disabled={disabled} onChange={budget=>patch({request:{...p.request,budget}})}/>
    <CapabilityFields prefix="压缩" value={p.request.capabilities} model={p.request.model} endpoint={p.endpoint} budget={p.request.budget||defaultBudget} disabled={disabled} onChange={capabilities=>patch({request:{...p.request,capabilities}})}/>
    <Button variant="outline" disabled={disabled} onClick={()=>void save()}>保存会话记忆设置</Button>
    <Button variant="outline" disabled={disabled} onClick={()=>{setWorking(true);onBusyChange(true);void invoke<{message:string;elapsed_ms:number;total_tokens:number|null}>('interview_memory_test').then(r=>setMessage(`${r.message} · ${r.elapsed_ms} ms · ${r.total_tokens===null?'用量未知':r.total_tokens+' tokens'}`)).catch(e=>setMessage(String(e))).finally(()=>{setWorking(false);onBusyChange(false);});}}>测试已保存压缩接口（可能计费）</Button>
    {message&&<p role="status">{message}</p>}
  </div></details>;
}
/** A main-window status only. Never a chat row or part of the scroll signature. */
export function MemoryStatusLine({controller,onError}:{controller:SessionMemory;onError:(e:unknown)=>void}){
  const [status,setStatus]=useState<MemoryStatus|null>(null),[retrying,setRetrying]=useState(false);
  useEffect(()=>{
    let disposed=false;const refresh=()=>{if(controller.handle)void invoke<MemoryStatus>('interview_memory_status').then(s=>{if(!disposed)setStatus(s);}).catch(()=>{});else if(!disposed)setStatus(null);};
    const unlisten=listen('interview-memory-status-changed',refresh);const timer=setInterval(refresh,5000);refresh();
    return()=>{disposed=true;clearInterval(timer);void unlisten.then(f=>f()).catch(()=>{});};
  },[controller]);
  if(!status?.active&&!controller.blocked)return null;
  const failed=controller.blocked||status?.persistence_failed;
  const label=failed?'原文保存未完成':status?.running?'后台整理中':status?.last_attempt?.status==='committed'?'已更新会话记忆':status?.last_attempt?'整理未完成，保留原上下文':'会话原文已保存';
  return <details className="text-xs"><summary>{label}</summary><div className="space-y-1 p-2">
    <p>上下文 v{status?.context_version??0} · 原文记录 {status?.records??0} · 已覆盖至 {status?.covered_through??0}</p>
    {status?.missing_older_history&&<p>旧快照只含部分原文，更早内容缺失。</p>}
    <p>后台请求 {status?.attempts??0} 次 · 计量预留 {status?.accounted_units??0} 单位（估算，非账单）</p>
    {!!status?.pressure?.hard_input_budget&&<p>最近发送前估算 {status.pressure.estimated_bytes} / {status.pressure.hard_input_budget}；增长预测 {status.pressure.projected_bytes}。{status.pressure.image_cost_unknown?'图片开销未知。':''}{status.pressure.active?'已接近预算，发送前将复核可选上下文。':''}</p>}
    {status?.error&&<p>{status.error}</p>}
    {failed&&<Button variant="outline" size="sm" disabled={retrying} onClick={()=>{setRetrying(true);void controller.retry().then(()=>controller.handle?invoke('interview_memory_checkpoint',{handle:controller.handle}):undefined).catch(onError).finally(()=>setRetrying(false));}}>重试保存原文</Button>}
  </div></details>;
}
