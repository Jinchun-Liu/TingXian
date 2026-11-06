'use client';
import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {Button} from '@/components/ui/button';
type Change={id:string;name:string;state:'new'|'source_changed'|'extraction_changed'|'unchanged';identical_count:number;identical_sources:string[]};
const labels={new:'新增',source_changed:'原文变化',extraction_changed:'提取结果变化',unchanged:'内容未变'};
export function BackgroundChanges({draft}:{draft:string}){
 const [rows,setRows]=useState<Change[]|null>(null),[error,setError]=useState(''),[page,setPage]=useState(0),[reload,setReload]=useState(0);const epoch=useRef(0);
 useEffect(()=>{const id=++epoch.current;setRows(null);setError('');setPage(0);
  void invoke<Change[]>('background_changes',{draft}).then(r=>{if(id===epoch.current)setRows(r);}).catch(e=>{if(id===epoch.current)setError(String(e));});
  return()=>{epoch.current++;};
 },[draft,reload]);
 if(error)return <p role="status">{error}<Button size="sm" variant="outline" onClick={()=>setReload(n=>n+1)}>重读变更</Button></p>;
 if(!rows)return <p role="status">核对来源变更…</p>;
 const counts=Object.entries(labels).map(([key,label])=>{const count=rows.filter(r=>r.state===key).length;return count?`${label} ${count}`:'';}).filter(Boolean).join(' · ');
 return <details className="rounded border p-3"><summary>本批变更 · {counts}</summary>
  <ul className="divide-y mt-2">{rows.slice(page,page+50).map(r=><li className="py-2 break-words" key={r.id}>{r.name} · {labels[r.state]}{r.identical_count>0&&<p className="text-xs text-slate-500">与已入库的 {r.identical_sources.join('、')} 等 {r.identical_count} 份原文相同；不是额外的独立事实证明。</p>}</li>)}</ul>
  {page>0&&<Button size="sm" variant="outline" onClick={()=>setPage(n=>Math.max(0,n-50))}>上一批来源</Button>}{page+50<rows.length&&<Button size="sm" variant="outline" onClick={()=>setPage(n=>n+50)}>下一批来源</Button>}
 </details>;
}
