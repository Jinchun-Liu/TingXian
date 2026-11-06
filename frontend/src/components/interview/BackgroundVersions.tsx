'use client';
import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {Button} from '@/components/ui/button';
type Version={revision:string;name:string;source_hash:string;recorded_at:string|null;count:number;current:boolean};
type RecordRow={location:string;quote:string;note:string;kind:string};
const kinds:Record<string,string>={personal:'个人资料',evidence:'佐证材料',practice:'练习参考',job:'岗位资料',technical:'技术资料'};
export function BackgroundVersions({source,expected,locked}:{source:string;expected:string;locked:boolean}){
 const [versions,setVersions]=useState<{rows:Version[];more:boolean;offset:number}|null>(null),[records,setRecords]=useState<{rows:RecordRow[];more:boolean;offset:number;revision:string;current:boolean}|null>(null),[error,setError]=useState(''),[busy,setBusy]=useState(false);
 const epoch=useRef(0),running=useRef(false);
 useEffect(()=>{epoch.current++;running.current=false;setBusy(false);setVersions(null);setRecords(null);setError('');return()=>{epoch.current++;};},[source,expected]);
 const read=async(revision:string|null,offset:number)=>{
  if(running.current||locked)return;running.current=true;setBusy(true);setError('');const request=epoch.current;
  try{
   if(revision){const r=await invoke<{rows:RecordRow[];more:boolean;current:boolean}>('background_version_records',{source,expected,revision,offset});if(request===epoch.current)setRecords({...r,revision,offset});}
   else{const v=await invoke<{rows:Version[];more:boolean}>('background_versions',{source,expected,offset});if(request===epoch.current){setVersions({...v,offset});setRecords(null);}}
  }catch(e){if(request===epoch.current){setRecords(null);setError(String(e));}}
  finally{if(request===epoch.current){running.current=false;setBusy(false);}}
 };
 return <div className="space-y-2"><Button variant="outline" size="sm" disabled={locked||busy} onClick={()=>void read(null,0)}>查看来源版本</Button>
  {versions&&<div className="rounded border p-2 space-y-2"><p className="text-slate-500">保存时间仅用于排序，不代表事实生效时间。旧版只供核对，不进入当前问答。</p>
   <ul>{versions.rows.map(v=><li key={v.revision}><button type="button" disabled={locked||busy} className="underline py-1 text-left break-all" onClick={()=>void read(v.revision,0)}>{v.current?'当前':'历史'} · {v.recorded_at?new Date(v.recorded_at).toLocaleString('zh-CN'):'早期导入，时间未知'} · {v.count} 条</button></li>)}</ul>
   {versions.offset>0&&<Button size="sm" variant="outline" disabled={locked||busy} onClick={()=>void read(null,versions.offset-20)}>上一页版本</Button>}{versions.more&&<Button size="sm" variant="outline" disabled={locked||busy} onClick={()=>void read(null,versions.offset+20)}>下一页版本</Button>}
  </div>}
  {records&&<div className="max-h-72 overflow-auto border-l pl-3 space-y-2" aria-label="来源版本引文"><strong>{records.current?'当前引文':'历史引文'}</strong>{records.rows.map((r,i)=><div key={i}><p>{r.location} · {kinds[r.kind]||'资料'}</p><blockquote className="whitespace-pre-wrap">{r.quote}</blockquote><p className="text-slate-500">{r.note}</p></div>)}
   {records.offset>0&&<Button size="sm" variant="outline" disabled={locked||busy} onClick={()=>void read(records.revision,records.offset-20)}>上一页引文</Button>}{records.more&&<Button size="sm" variant="outline" disabled={locked||busy} onClick={()=>void read(records.revision,records.offset+20)}>下一页引文</Button>}
  </div>}{error&&<p role="alert">{error}</p>}
 </div>;
}
