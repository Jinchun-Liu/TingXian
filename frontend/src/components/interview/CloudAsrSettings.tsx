'use client';
import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {Button} from '@/components/ui/button';
import {ChoiceMenu,ChoiceOption} from '@/components/ui/choice-menu';
type Config={mode:'local'|'aliyun';workspace_id:string;silence_ms:number};
export function CloudAsrSettings({locked,onSaved}:{locked:boolean;onSaved?:()=>void}){
  const [config,setConfig]=useState<Config|null>(null),[key,setKey]=useState(''),[keyEdited,setKeyEdited]=useState(false),[configured,setConfigured]=useState(false);
  const [busy,setBusy]=useState(false),[message,setMessage]=useState('');const alive=useRef(false),working=useRef(false);
  useEffect(()=>{alive.current=true;void invoke<{config:Config;key_configured:boolean}>('interview_cloud_asr_get').then(r=>{if(alive.current){setConfig(r.config);setConfigured(r.key_configured);}}).catch(e=>{if(alive.current)setMessage(String(e));});return()=>{alive.current=false;};},[]);
  async function save(){
    if(!config||locked||working.current)return;working.current=true;setBusy(true);setMessage('');
    try{await invoke('interview_cloud_asr_save',{config,key:keyEdited?key:null});if(alive.current){setConfigured(keyEdited?!!key.trim():configured);setKey('');setKeyEdited(false);setMessage('转写服务已保存');onSaved?.();}}
    catch(e){if(alive.current)setMessage(String(e));}finally{working.current=false;if(alive.current)setBusy(false);}
  }
  const disabled=locked||busy;
  return <section aria-label="转写服务设置"><header className="tx-section-head"><h3>转写服务</h3></header>
    <ChoiceMenu aria-label="转写服务" disabled={disabled||!config} value={config?.mode||'local'} onValueChange={mode=>setConfig(c=>c&&({...c,mode:mode as Config['mode']}))}><ChoiceOption value="local">本地 Whisper</ChoiceOption><ChoiceOption value="aliyun">百炼实时 · 北京</ChoiceOption></ChoiceMenu>
    {config?.mode==='aliyun'&&<div className="grid gap-3 mt-3">
      <label className="grid gap-1 text-sm">Workspace ID<input aria-label="百炼 Workspace ID" className="border rounded p-2" disabled={disabled} value={config.workspace_id} onChange={e=>setConfig({...config,workspace_id:e.target.value})}/></label>
      <label className="grid gap-1 text-sm">API Key<input aria-label="云转写 API Key" type="password" autoComplete="off" className="border rounded p-2" disabled={disabled} value={key} placeholder={configured?'已设置，留空保持':'填写北京区 API Key'} onChange={e=>{setKey(e.target.value);setKeyEdited(true);}}/><span className="text-xs text-slate-500">凭证仅在本次打开期间有效。</span></label>
      <label className="grid gap-1 text-sm">断句停顿<input aria-label="云转写断句毫秒" className="border rounded p-2" type="number" min={200} max={6000} step={100} disabled={disabled} value={config.silence_ms} onChange={e=>setConfig({...config,silence_ms:Number(e.target.value)})}/></label>
    </div>}
    <Button className="mt-3" size="sm" disabled={disabled||!config||(config.mode==='aliyun'&&!config.workspace_id.trim())||!Number.isInteger(config.silence_ms)||config.silence_ms<200||config.silence_ms>6000} onClick={()=>void save()}>保存转写服务</Button>
    {message&&<p role="status" className="mt-2 text-sm">{message}</p>}
  </section>;
}
