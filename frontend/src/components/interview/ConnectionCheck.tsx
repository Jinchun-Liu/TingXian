'use client';
import {useCallback,useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {Button} from '@/components/ui/button';

type CheckStatus='untested'|'checking'|'cancelling'|'passed'|'failed'|'cancelled';
export type ConnectionReport={check_id:string|null;status:CheckStatus;model:string|null;started_at:string|null;finished_at:string|null;elapsed_ms:number|null;attempts:number;total_tokens:number|null;message:string};
const empty:ConnectionReport={check_id:null,status:'untested',model:null,started_at:null,finished_at:null,elapsed_ms:null,attempts:0,total_tokens:null,message:'尚未测试连接'};
const active=(report:ConnectionReport)=>report.status==='checking'||report.status==='cancelling';
const labels:Record<CheckStatus,string>={untested:'尚未测试',checking:'测试中',cancelling:'正在取消',passed:'上次测试成功',failed:'上次测试失败',cancelled:'上次测试已取消'};

export function ConnectionCheck({disabled,hasKey,revision,onCheckingChange}:{disabled:boolean;hasKey:boolean;revision:number;onCheckingChange:(checking:boolean)=>void}) {
  const [report,setReport]=useState<ConnectionReport>(empty),[ready,setReady]=useState(false),[error,setError]=useState('');
  const [pending,setPending]=useState(false),[cancelling,setCancelling]=useState(false);
  const current=useRef(empty),alive=useRef(true),version=useRef(0),ownPending=useRef(false),reading=useRef(false),queuedRead=useRef(false);
  const onChecking=useRef(onCheckingChange);onChecking.current=onCheckingChange;
  const apply=useCallback((next:ConnectionReport)=>{
    if(!alive.current)return;
    // A late cancellation acknowledgement must not regress a finished check.
    if(next.check_id===current.current.check_id && current.current.finished_at && active(next))return;
    current.current=next;setReport(next);setReady(true);onChecking.current(active(next));
  },[]);
  const refresh:()=>Promise<void>=useCallback(async()=>{
    if(ownPending.current)return;const request=++version.current;
    if(reading.current){queuedRead.current=true;return;}reading.current=true;
    try{const next=await invoke<ConnectionReport>('interview_connection_status');if(alive.current && request===version.current){apply(next);setError('');}}
    catch(e){if(alive.current && request===version.current)setError(`无法读取连接状态：${String(e)}`);}
    finally{reading.current=false;if(queuedRead.current){queuedRead.current=false;if(alive.current&&!ownPending.current)void refresh();}}
  },[apply]);
  useEffect(()=>{void refresh();},[revision,refresh]);
  useEffect(()=>{
    if(!active(report)||pending)return;
    const timer=setInterval(()=>{void refresh();},1000);return ()=>clearInterval(timer);
  },[report.status,pending,refresh]);
  useEffect(()=>{
    alive.current=true;
    return ()=>{alive.current=false;version.current++;const value=current.current;if(active(value)&&value.check_id)void invoke('interview_connection_cancel',{checkId:value.check_id}).catch(()=>{});};
  },[]);
  const test=async()=>{
    if(ownPending.current||active(current.current))return;
    const checkId=crypto.randomUUID(),request=++version.current;
    ownPending.current=true;queuedRead.current=false;setPending(true);setError('');
    apply({...empty,check_id:checkId,status:'checking',message:'正在发送固定测试文字…'});
    try{const next=await invoke<ConnectionReport>('interview_connection_test',{checkId,confirmed:true});if(alive.current&&request===version.current)apply(next);}
    catch(e){if(alive.current&&request===version.current){ownPending.current=false;await refresh();if(alive.current)setError(String(e));}}
    finally{ownPending.current=false;if(alive.current)setPending(false);}
  };
  const cancel=async()=>{
    const checkId=current.current.check_id;if(!checkId)return;setCancelling(true);setError('');
    try{const next=await invoke<ConnectionReport>('interview_connection_cancel',{checkId});if(alive.current&&next.check_id===checkId&&current.current.check_id===checkId)apply(next);}
    catch(e){if(alive.current)setError(`取消未确认：${String(e)}`);}
    finally{if(alive.current)setCancelling(false);}
  };
  return <section className="rounded-lg border px-3 py-2 space-y-1 text-xs" aria-label="API连接自检">
    <div className="flex flex-wrap gap-2 items-center justify-between">
      <div role="status">API 连接：{labels[report.status]}{report.model?` · ${report.model}`:''}{report.finished_at?` · ${new Date(report.finished_at).toLocaleString()}`:''}</div>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" variant="outline" disabled={disabled||!hasKey||!ready||active(report)||pending||cancelling} onClick={test}>测试 DeepSeek 连接</Button>
        {active(report)&&<Button size="sm" variant="outline" disabled={cancelling||report.status==='cancelling'} onClick={cancel}>取消连接测试</Button>}
        <Button size="sm" variant="outline" disabled={pending||cancelling} onClick={()=>{void refresh();}}>刷新连接状态</Button>
      </div>
    </div>
    <p>{report.message}{report.elapsed_ms!==null?` · ${report.elapsed_ms} ms · 请求尝试 ${report.attempts} 次 · 用量 ${report.total_tokens===null?'未知':`${report.total_tokens} tokens`}`:''}</p>
    {error&&<p role="alert" className="text-red-700">{error}</p>}
    <p className="text-slate-500">点击测试仅发送固定测试文字，会产生 API 用量，不进入面试记录。成功只代表测试时可用；更改 Key 或请求设置后需重新测试。</p>
  </section>;
}
