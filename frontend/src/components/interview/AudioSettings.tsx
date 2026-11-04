'use client';
import {useEffect, useRef, useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {Button} from '@/components/ui/button';
import {ChoiceMenu, ChoiceOption} from '@/components/ui/choice-menu';

type Device = {name:string; device_type:'Input'|'Output'};
type Devices = {preferred_mic_device:string|null;preferred_system_device:string|null};
type Model = {name:string;size_mb:number;status:'Available'|'Missing'|{Downloading:{progress:number}}|{Corrupted:unknown}|{Error:string}};
type Catalog = {selected:string;models:Model[]};
const statusOf=(s:Model['status'])=>s==='Available'?'已就绪':s==='Missing'?'未下载':('Downloading' in s?'下载中':'需重新下载');
export function AudioSettings({locked,microphone,onMicrophone}:{locked:boolean;microphone:boolean;onMicrophone:(value:boolean)=>void}){
  const [devices,setDevices]=useState<Device[]>([]),[prefs,setPrefs]=useState<Devices|null>(null);
  const [catalog,setCatalog]=useState<Catalog|null>(null),[selected,setSelected]=useState('');
  const [pending,setPending]=useState(false),[message,setMessage]=useState(''),[download,setDownload]=useState<string|null>(null);
  const [progress,setProgress]=useState(0),[micCheck,setMicCheck]=useState('未测试'),[systemCheck,setSystemCheck]=useState('未测试');
  const mounted=useRef(false),revision=useRef(0),task=useRef(false);
  async function refresh(){
    const n=++revision.current;
    const [d,p,c]=await Promise.allSettled([invoke<Device[]>('get_audio_devices'),invoke<Devices>('get_recording_preferences'),invoke<Catalog>('tingxian_asr_models')]);
    if(!mounted.current||revision.current!==n)return;
    if(d.status==='fulfilled')setDevices(d.value);
    if(p.status==='fulfilled')setPrefs(p.value);
    if(c.status==='fulfilled'){setCatalog(c.value);setSelected(c.value.selected);}
    const failures=[d,p,c].filter(x=>x.status==='rejected');
    if(failures.length)setMessage('部分音频设置未能读取，请刷新重试：'+failures.map(x=>String((x as PromiseRejectedResult).reason)).join('；'));
  }
  useEffect(()=>{
    mounted.current=true;void refresh();
    const off=listen<{modelName:string;progress:number}>('model-download-progress',({payload})=>{
      if(mounted.current){setDownload(payload.modelName);setProgress(payload.progress);}
    });
    const finish=listen('model-download-complete',()=>{if(mounted.current){setDownload(null);void refresh();}});
    function focused(){if(!task.current)void refresh();}
    window.addEventListener('focus',focused);
    return()=>{mounted.current=false;revision.current++;window.removeEventListener('focus',focused);void off.then(fn=>fn()).catch(()=>{});void finish.then(fn=>fn()).catch(()=>{});};
  },[]);
  async function run(fn:()=>Promise<void>){
    if(task.current||locked)return;
    task.current=true;setPending(true);setMessage('');
    try{await fn();}catch(e){if(mounted.current)setMessage('操作未完成：'+String(e));}
    finally{task.current=false;if(mounted.current)setPending(false);}
  }
  const disabled=locked||pending;
  const field=(kind:Device['device_type'],label:string)=>{
    const key=kind==='Input'?'preferred_mic_device':'preferred_system_device',value=prefs?.[key]||'';
    const names=Array.from(new Set(devices.filter(d=>d.device_type===kind).map(d=>d.name)));
    const system=kind==='Output';
    const testLabel=system?'测试系统音频':'测试麦克风权限';
    return <div className="tx-audio-row"><div className="tx-audio-field"><span>{label}</span><ChoiceMenu aria-label={label} disabled={disabled||!prefs} value={value} onValueChange={v=>void run(async()=>{
      if(!prefs)return;const next={...prefs,[key]:v||null};
      await invoke('tingxian_select_devices',{microphone:next.preferred_mic_device,system:next.preferred_system_device});
      setPrefs(next);setMessage('设备选择已保存');
    })}><ChoiceOption value="">跟随系统默认</ChoiceOption>{value&&!names.some(n=>value===n+' ('+kind.toLowerCase()+')')&&<ChoiceOption value={value}>{value}（未连接）</ChoiceOption>}{names.map(n=><ChoiceOption key={n} value={n+' ('+kind.toLowerCase()+')'}>{n}</ChoiceOption>)}</ChoiceMenu></div>
      <div className="tx-audio-test"><Button size="sm" variant="outline" disabled={disabled} aria-label={testLabel} onClick={()=>void run(async()=>{
        if(system)setSystemCheck(await invoke<boolean>('trigger_system_audio_permission_command')?'通道可用，音量在采集中确认':'未能建立，请检查系统权限');
        else setMicCheck(await invoke<boolean>('trigger_microphone_permission')?'默认麦克风可打开':'默认麦克风不可用，请检查权限');
      })}>{testLabel}</Button><span role="status">{system?systemCheck:micCheck}</span></div></div>;
  };
  const model=catalog?.models.find(m=>m.name===selected);
  const activeDownload=download||(model && typeof model.status==='object' && 'Downloading' in model.status?model.name:null);
  return <section aria-label="音频设备与转写模型" className="tx-audio">
    <section><header className="tx-section-head"><h3>采集设备</h3><button className="tx-text-action" disabled={pending} onClick={()=>{setMessage('');void refresh();}}>刷新设备与模型</button></header>
      {field('Output','对方声音 · 系统输出')}{field('Input','我的声音 · 麦克风')}
      <div className="tx-audio-options"><label><input type="checkbox" checked={microphone} disabled={disabled} onChange={e=>onMicrophone(e.target.checked)}/>记录我的声音</label>
      <button className="tx-text-action" onClick={()=>void run(async()=>{await invoke('request_screen_recording_permission_command');})} disabled={disabled}>打开 macOS 隐私设置 ↗</button></div>
    </section>
    <section><header className="tx-section-head"><h3>本地 Whisper 模型</h3></header><div className="tx-model-row"><ChoiceMenu aria-label="本地 Whisper 模型" disabled={disabled||!catalog||!!activeDownload} value={selected} onValueChange={setSelected}>{catalog?.models.map(m=><ChoiceOption value={m.name} key={m.name}>{m.name} · {m.size_mb} MB · {statusOf(m.status)}</ChoiceOption>)}</ChoiceMenu>
    <div className="tx-model-action">{activeDownload?<><span role="status">{activeDownload} 下载中 · {progress}%</span><Button size="sm" variant="outline" onClick={async()=>{try{await invoke('whisper_cancel_download',{modelName:activeDownload});setDownload(null);await refresh();}catch(e){setMessage('取消失败：'+String(e));}}}>取消下载</Button></>:model?.status==='Available'&&selected===catalog?.selected?<span className="tx-current">使用中</span>:model?.status==='Available'?<Button size="sm" disabled={disabled||selected===catalog?.selected} onClick={()=>void run(async()=>{await invoke('tingxian_select_model',{model:selected});await refresh();setMessage('转写模型已保存');})}>使用此模型</Button>:<Button size="sm" disabled={disabled||!model} onClick={()=>void run(async()=>{
      setProgress(0);setDownload(selected);
      try{await invoke('whisper_download_model',{modelName:selected});const next=await invoke<Catalog>('tingxian_asr_models');setCatalog(next);setSelected(selected);}
      finally{setDownload(null);}
    })}>下载模型</Button>}
    </div></div></section>
    {message&&<p role="status">{message}</p>}
  </section>;
}
