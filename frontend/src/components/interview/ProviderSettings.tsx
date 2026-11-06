'use client';
import {ChoiceMenu, ChoiceOption} from '@/components/ui/choice-menu';
import {useEffect,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {Button} from '@/components/ui/button';
import type {RequestConfig} from './RequestSettings';
import {BudgetFields,defaultBudget,budgetValid} from './RequestSettings';
import {CapabilityFields,capabilitiesError,matchingCapabilities} from '@/components/interview/ModelCapabilities';
type Profile={id:string;name:string;enabled:boolean;endpoint:string;protocol:'deepseek'|'openai'|'qwen';request:RequestConfig};
type Config={answers:Profile[];vision:Profile;background:Profile};
type View={config:Config;key_configured:Record<string,boolean>;capacity:number};
export function ProviderSettings({locked,onSaved,onBusyChange,openVision=false,openBackground=false}:{openVision?:boolean;openBackground?:boolean;locked:boolean;onSaved:()=>Promise<void>;onBusyChange:(busy:boolean)=>void}) {
  const [view,setView]=useState<View|null>(null),[keys,setKeys]=useState<Record<string,string>>({});
  const [message,setMessage]=useState(''),[working,setWorking]=useState(false);
  const load=async()=>setView(await invoke<View>('interview_providers_get'));
  useEffect(()=>{void load().catch(e=>setMessage(String(e)));},[]);
  const patch=(id:string,values:Partial<Profile>)=>setView(v=>{
    const update=(p:Profile)=>{if(p.id!==id)return p;const next={...p,...values};return {...next,request:{...next.request,capabilities:matchingCapabilities(next.request.capabilities,next.request.model,next.request.budget||defaultBudget,next.endpoint)}};};
    return v?{...v,config:{answers:v.config.answers.map(update),vision:update(v.config.vision),background:v.config.background?update(v.config.background):v.config.background}}:v;
  });
  const work=async(action:()=>Promise<void>)=>{setWorking(true);onBusyChange(true);setMessage('');try{await action();}catch(e){setMessage(String(e));}finally{setWorking(false);onBusyChange(false);}};
  if(!view)return <div role="status">{message||'读取多接口配置…'}<Button variant="outline" size="sm" onClick={()=>void load().catch(e=>setMessage(String(e)))}>重新读取</Button></div>;
  const disabled=locked||working;
  return <section className="border-t pt-3 space-y-3" aria-label="多 AI 接口设置">
    <h3 className="font-medium">回答接口</h3>
    <p className="text-xs text-slate-500">{view.capacity} / 3 路可用。手动输入的 Key 仅在本次运行有效。</p>
    {[...view.config.answers,view.config.vision,...(view.config.background?[view.config.background]:[])].map(p=><details key={p.id} className="border rounded-lg p-3" open={p.id===(openBackground?'background':openVision?'vision':'answer-1')}>
      <summary className="cursor-pointer text-sm font-medium">{p.id==='background'?'背景整理模型':p.id==='vision'?'截屏视觉模型':`回答位置 ${p.id.slice(-1)}`} · {p.name} · {view.key_configured[p.id]?'Key 已配置':'Key 未配置'}</summary>
      <div className="mt-3 space-y-2 text-sm">
        <label className="block"><input type="checkbox" disabled={disabled} checked={p.enabled} onChange={e=>patch(p.id,{enabled:e.target.checked})} /> {p.id==='background'?'启用背景整理':p.id==='vision'?'启用截屏问答':'启用此回答接口'}</label>
        {p.id==='background'&&<p className="text-xs text-slate-500">建议使用高智能推理模型。整理独立于实时回答，使用模型默认推理模式。</p>}
        {p.id==='vision'&&<p className="text-xs text-slate-500">填写支持图片输入的模型。可复用已有多模态接口的地址、模型和 Key，也可使用单独视觉服务或本机兼容服务。点击“截屏并回答”才会发送所选整屏图片与近期上下文；图片不写入会话历史。文字连接测试不能验证视觉能力。</p>}
        <div className="grid sm:grid-cols-2 gap-2">
          <label>名称<input aria-label={`${p.id} 名称`} className="w-full rounded border p-2" disabled={disabled} value={p.name} onChange={e=>patch(p.id,{name:e.target.value})} /></label>
          {p.id!=='background'&&<label>接口类型<ChoiceMenu aria-label={`${p.id} 接口类型`} className="w-full" disabled={disabled} value={p.protocol} onValueChange={nextValue =>patch(p.id,{protocol:nextValue as Profile['protocol']})}><ChoiceOption value="deepseek">DeepSeek（关闭思考）</ChoiceOption><ChoiceOption value="qwen">千问 / 百炼（关闭思考）</ChoiceOption><ChoiceOption value="openai">OpenAI 兼容 / 本地模型</ChoiceOption></ChoiceMenu></label>}
        </div>
        <label className="block">完整 API 地址<input aria-label={`${p.id} API 地址`} className="w-full rounded border p-2" disabled={disabled} value={p.endpoint} placeholder="https://服务地址/v1/chat/completions" onChange={e=>patch(p.id,{endpoint:e.target.value.trim()})} /></label>
        <label className="block">模型 ID<input aria-label={`${p.id} 模型 ID`} className="w-full rounded border p-2" disabled={disabled} value={p.request.model} maxLength={128} onChange={e=>patch(p.id,{request:{...p.request,model:e.target.value}})} /></label>
        <label className="block">API Key<input aria-label={`${p.id} API Key`} className="w-full rounded border p-2" type="password" autoComplete="off" disabled={disabled} value={keys[p.id]||''} placeholder="留空保留当前进程凭证；地址改变需重新填写" onChange={e=>setKeys(k=>({...k,[p.id]:e.target.value}))} /></label>
        <details><summary>超时设置</summary><div className="grid sm:grid-cols-3 gap-2">{([['connect_timeout_secs','连接超时',1,30],['first_answer_warning_secs','等待提示',1,119],['total_timeout_secs','总截止',5,120]] as const).map(([field,label,min,max])=><label key={field}>{label}（秒）<input type="number" className="w-full rounded border p-2" min={min} max={max} disabled={disabled} value={p.request[field]} onChange={e=>patch(p.id,{request:{...p.request,[field]:Number(e.target.value)}})} /></label>)}</div></details>
        <BudgetFields prefix={p.id} value={p.request.budget||defaultBudget} disabled={disabled} onChange={budget=>patch(p.id,{request:{...p.request,budget}})}/>
        <CapabilityFields prefix={p.id} value={p.request.capabilities} model={p.request.model} endpoint={p.endpoint} budget={p.request.budget||defaultBudget} disabled={disabled} onChange={capabilities=>patch(p.id,{request:{...p.request,capabilities}})}/>
        {p.id==='background'&&p.request.capabilities?.streaming==='unsupported'&&<p className="text-xs text-slate-500">背景整理使用非流式请求，仍可使用此模型。</p>}
        <Button size="sm" variant="outline" disabled={disabled||p.request.capabilities?.streaming==='unsupported'} onClick={()=>void work(async()=>{
          const r=await invoke<{success:boolean;message:string;elapsed_ms:number}>('interview_provider_test',{profileId:p.id});
          setMessage(`${p.name}：${r.message}（${r.elapsed_ms} ms）。测试使用已保存配置。`);
        })}>测试已保存接口（流式文字）</Button>
      </div>
    </details>)}
    <Button disabled={disabled} onClick={()=>void work(async()=>{
      if([...view.config.answers,view.config.vision,...(view.config.background?[view.config.background]:[])].some(p=>!budgetValid(p.request.budget||defaultBudget)))throw Error('请检查模型预算：输出上限和安全余量须小于模型窗口及 256K 应用上限。');
      for(const p of [...view.config.answers,view.config.vision,...(view.config.background?[view.config.background]:[])]){const error=capabilitiesError(p.request.capabilities,p.request.model,p.request.budget||defaultBudget,p.endpoint);if(error)throw Error(`${p.name}：${error}`);}
      await invoke('interview_providers_save',{config:view.config,keys:Object.fromEntries(Object.entries(keys).filter(([,v])=>v.trim()))});
      setKeys({});await load();await onSaved();setMessage('接口配置已保存，未发送模型请求。');
    })}>保存多接口配置</Button>
    {locked&&<p className="text-xs text-slate-500">请结束会话后修改接口。</p>}
    {message&&<p role="status" className="text-sm break-words">{message}</p>}
  </section>;
}
