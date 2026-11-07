'use client';
import {useEffect,useRef,useState} from 'react';
import type {ReactNode} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {Button} from '@/components/ui/button';
type Source={record_id:string;claim:string;claim_status:string;activity_status:string;manual_revision:null|{basis:string};source_refs:{source_id:string;locator:string;quote:string}[]};
type Mapping={id:string;text:string;matches:{record_id:string;relevance:string;reason:string}[];references:{record_id:string;purpose:string}[];gaps:{text:string}[]};
type Story={id:string;title:string;activity:string;sections:Record<string,{text:string;record_ids:string[]}[]>;missing:Record<string,string>};
type Consolidation={conflicts:{text:string;record_ids:string[]}[];parent_candidate:{stories:Story[]};parts:{input_hash:string;requirement_id:string|null;from_story_ids:string[];input_gaps:{id:string;text:string}[];omitted:{record_id:string;reason:string}[];gap_review:{id:string;status:string;reason:string;evidence_ids:string[]}[]}[]};
type Candidate={candidate_hash:string;task:{role:string;organization:string};role_view:Mapping[];stories:Story[];source_records:Source[];execution_mode:string;consolidation?:Consolidation};
type Preview={candidate:Candidate;source_names?:Record<string,string>;index_version:string;previous_hash:string|null;source_binding_valid:boolean};
type Saved={candidate:Candidate;source_names?:Record<string,string>;source_binding_valid:boolean;local_only:true};
const sections=[['context','情境'],['task','任务'],['action','本人行动'],['result','结果'],['learning','复盘']] as const;
const activity:Record<string,string>={unknown:'完成状态待核对',planned:'计划中',in_progress:'进行中',completed:'已完成'};
function RequirementView({row,citations,locked}:{row:Mapping;citations:(ids:string[])=>ReactNode;locked:boolean}){
  const [limit,setLimit]=useState(10);
  return <section className="space-y-2 rounded border p-3"><h5 className="font-medium">{row.text}</h5>
    {row.matches.slice(0,limit).map(m=><div key={m.record_id}><p>{m.reason}</p>{citations([m.record_id])}</div>)}
    {!!row.references.length&&<details className="text-sm"><summary>参考资料 · {row.references.length}</summary>{row.references.slice(0,limit).map((r,i)=><div key={i} className="mt-2">{r.purpose}{citations([r.record_id])}</div>)}</details>}
    {!!row.gaps.length&&<details><summary>待补证据 · {row.gaps.length}</summary><ul className="list-disc space-y-1 pl-5 text-sm">{row.gaps.slice(0,limit).map((g,i)=><li key={i}>{g.text}</li>)}</ul></details>}
    {Math.max(row.matches.length,row.references.length,row.gaps.length)>limit&&<Button variant="outline" size="sm" disabled={locked} onClick={()=>setLimit(n=>n+10)}>查看更多依据与缺口</Button>}
  </section>;
}
function ConsolidationView({candidate,citations,locked}:{candidate:Candidate;citations:(ids:string[])=>ReactNode;locked:boolean}){
  const [limit,setLimit]=useState(10),c=candidate.consolidation;
  if(!c)return null;
  const titles=new Map(candidate.role_view.map(r=>[r.id,r.text]));
  const originals=new Map(c.parent_candidate.stories.map(s=>[s.id,s]));
  const more=Math.max(c.parts.length,c.conflicts.length,...c.parts.flatMap(p=>[p.from_story_ids.length,p.omitted.length,p.gap_review.length]))>limit;
  return <section className="space-y-3">
    {!!c.conflicts.length&&<div className="space-y-2 rounded border border-amber-300 p-3"><h5 className="font-medium">待核对冲突 · {c.conflicts.length}</h5>
      {c.conflicts.slice(0,limit).map((conflict,i)=><div key={i}><p>{conflict.text}</p>{citations(conflict.record_ids)}</div>)}
    </div>}
    <details className="rounded border p-3"><summary>整合依据</summary><div className="mt-3 space-y-4">
      {c.parts.slice(0,limit).map(part=><div key={part.input_hash} className="space-y-2 border-t pt-3">
        <h6 className="font-medium">{part.requirement_id?titles.get(part.requirement_id):'项目故事'}</h6>
        {part.from_story_ids.slice(0,limit).map(id=>{const story=originals.get(id);return story?<details key={id}><summary>原故事：{story.title}</summary>{sections.map(([key,label])=><div key={key} className="mt-2 text-sm"><span>{label}：</span>{story.sections[key].map((f,i)=><div key={i}>{f.text}{citations(f.record_ids)}</div>)}{story.missing[key]&&<p>待补：{story.missing[key]}</p>}</div>)}</details>:null;})}
        {part.omitted.slice(0,limit).map((r,i)=><div key={'o'+i} className="text-sm"><p>未选入：{r.reason}</p>{citations([r.record_id])}</div>)}
        {part.gap_review.slice(0,limit).map(r=><div key={r.id} className="text-sm"><p>{r.status==='retained'?'保留缺口':'候选认为已补齐，待核对'}：{part.input_gaps.find(g=>g.id===r.id)?.text}</p><p>{r.reason}</p>{!!r.evidence_ids.length&&citations(r.evidence_ids)}</div>)}
      </div>)}
    </div></details>
    {more&&<Button variant="outline" size="sm" disabled={locked} onClick={()=>setLimit(n=>n+10)}>查看更多整合依据</Button>}
  </section>;
}
export function PreparationLibrary({locked,revision,onBusy}:{locked:boolean;revision?:string;onBusy:(busy:boolean)=>void}){
  const [preview,setPreview]=useState<Preview|null>(null),[saved,setSaved]=useState<Saved|null>(null),[message,setMessage]=useState(''),[busy,setBusy]=useState(false),[page,setPage]=useState(0);
  const epoch=useRef(0),running=useRef(false),alive=useRef(false);
  useEffect(()=>{alive.current=true;epoch.current++;setPreview(null);setSaved(null);setPage(0);setMessage('');return()=>{alive.current=false;epoch.current++;};},[revision]);
  const op=async(fn:(current:()=>boolean)=>Promise<void>)=>{
    if(locked||running.current)return;running.current=true;setBusy(true);setMessage('');onBusy(true);
    const request=epoch.current,current=()=>alive.current&&request===epoch.current;
    try{await fn(current);}catch(e){if(current())setMessage(String(e));}
    finally{running.current=false;if(alive.current)setBusy(false);onBusy(false);}
  };
  const candidate=preview?.candidate||saved?.candidate;
  const names=preview?.source_names||saved?.source_names||{};
  const sources=new Map(candidate?.source_records.map(s=>[s.record_id,s])||[]);
  const citations=(ids:string[])=><details className="mt-1 text-xs"><summary>查看依据</summary>{ids.map(id=>{const source=sources.get(id);return source?<div key={id} className="my-2 space-y-1 border-l-2 pl-3">
    {source.claim_status==='user_asserted'&&<p>本人修订：{source.claim}<br/>依据：{source.manual_revision?.basis}</p>}
    {source.source_refs.map((ref,i)=><blockquote key={i} className="whitespace-pre-wrap break-words">{names[ref.source_id]||`来源 ${ref.source_id.slice(0,8)}`} · {ref.locator}<p>{ref.quote}</p></blockquote>)}
  </div>:null;})}</details>;
  const off=locked||busy;
  return <details className="border-t pt-3"><summary className="font-medium">岗位准备</summary>
    <div className="flex flex-wrap gap-2 py-3">
      <Button size="sm" variant="outline" disabled={off} onClick={()=>void op(async current=>{const path=await invoke<string|null>('background_preparation_export');if(current()&&path)setMessage('已导出：'+path);})}>导出整理资料</Button>
      <Button size="sm" variant="outline" disabled={off} onClick={()=>void op(async current=>{const value=await invoke<Preview|null>('background_preparation_import');if(current()&&value){setPreview(value);setSaved(null);setPage(0);}})}>导入准备候选</Button>
      <Button size="sm" variant="outline" disabled={off||!!preview} onClick={()=>void op(async current=>{const value=await invoke<Saved|null>('background_preparation_view');if(current()){setSaved(value);setPage(0);if(!value)setMessage('还没有保存的岗位准备。');}})}>查看已保存</Button>
    </div>
    <p className="text-xs text-slate-500">导出已整理且当前有效的资料，配合四角色工具生成候选后导回。候选仅保存在本机，不写入事实或自动用于答题。</p>
    {candidate&&<div className="space-y-3 pt-3">
      <div className="flex flex-wrap items-center justify-between gap-2"><h4 className="font-medium">{candidate.task.role}{candidate.task.organization?` · ${candidate.task.organization}`:''}</h4>{candidate.execution_mode==='fixture'&&<span className="text-xs">合成示例</span>}</div>
      {saved?.source_binding_valid===false&&<p role="alert" className="text-amber-700">来源已变化。以下是旧候选，请重新导出整理。</p>}
      <div className="max-h-[70vh] space-y-4 overflow-auto pr-2">
        <ConsolidationView key={candidate.candidate_hash} candidate={candidate} citations={citations} locked={off}/>
        {candidate.role_view.map(row=><RequirementView key={candidate.candidate_hash+row.id} row={row} citations={citations} locked={off}/>)}
        {!!candidate.stories.length&&<h4 className="font-medium">项目故事 · {candidate.stories.length}</h4>}
        {candidate.stories.slice(page*10,(page+1)*10).map(story=><section key={story.id} className="space-y-2 rounded border p-3"><h5 className="font-medium">{story.title}</h5><p className="text-xs text-slate-500">{activity[story.activity]}</p>
          {sections.map(([key,label])=><div key={key}><h6 className="text-sm font-medium">{label}</h6>{story.sections[key].map((fragment,i)=><div key={i}><p className="whitespace-pre-wrap break-words">{fragment.text}</p>{citations(fragment.record_ids)}</div>)}{story.missing[key]&&<p className="text-sm text-slate-500">待补：{story.missing[key]}</p>}</div>)}
        </section>)}
      </div>
      {candidate.stories.length>10&&<div className="flex items-center gap-3"><Button size="sm" variant="outline" disabled={off||!page} onClick={()=>setPage(p=>p-1)}>上一页故事</Button><span>{page+1} / {Math.ceil(candidate.stories.length/10)}</span><Button size="sm" variant="outline" disabled={off||(page+1)*10>=candidate.stories.length} onClick={()=>setPage(p=>p+1)}>下一页故事</Button></div>}
      {preview&&<div className="flex flex-wrap gap-2"><Button size="sm" disabled={off} onClick={()=>void op(async current=>{
        await invoke('background_preparation_save',{candidate:preview.candidate,version:preview.index_version,previousHash:preview.previous_hash});
        if(current()){setSaved({candidate:preview.candidate,source_names:preview.source_names,source_binding_valid:true,local_only:true});setPreview(null);setMessage('已保存岗位准备，个人事实未改变。');}
      })}>保存岗位准备</Button><Button size="sm" variant="outline" disabled={off} onClick={()=>{setPreview(null);setPage(0);}}>取消导入</Button></div>}
    </div>}
    {message&&<p role="status" className="pt-2 break-words text-sm">{message}</p>}
  </details>;
}
