'use client';
import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {Button} from '@/components/ui/button';
import {BackgroundSettings} from '@/components/interview/BackgroundSettings';
type Profile={overview:string;role:string;aliases:string};
type Metrics={peak_prompt_tokens:number|null;sum_prompt_tokens?:number;calls:number};
type Draft={id:string;job?:string;profile:Profile;documents:{id:string;name:string;count:number}[];failures:[string,string][];metrics:Metrics|null};
type Report={documents:number;chunks:number;baseline:number;full_budget:number;rag_budget:number;fits_full:boolean;version:string;state:null|{profile:Profile;share:boolean;metrics:Metrics|null};measurement:null|{version:string;model:string;chunks:number;metrics:Metrics}};
type Job={id:string;model?:string;status:string;total:number;completed:number;failed:number;skipped:number;unknown_calls:number};
type JobFile={name:string;status:string;reason:string;done:number;total:number};
type View={library:Report;model:string;ready:boolean;jobs?:Job[];work:{busy:boolean;progress:string;draft:Draft|null}};
type Selection={id:string;label:string;count:number;folder:boolean};
const num=(n:number)=>n.toLocaleString('zh-CN');
export function BackgroundLibrary({locked,onBusy,onApplied,onModels,revision,active,names,applyNames}:{locked:boolean;onBusy:(v:boolean)=>void;onApplied:()=>Promise<void>;onModels:()=>void;revision?:string;active:boolean;names?:string;applyNames?:(expected:string,value:string)=>boolean}){
  const [v,setV]=useState<View|null>(null),[selected,select]=useState<Selection|null>(null),[msg,setMsg]=useState(''),[progress,setProgress]=useState(''),[working,setWorking]=useState(false);
  const [edit,setEdit]=useState<Profile|null>(null),[base,setBase]=useState(''),[share,setShare]=useState(false),[terms,setTerms]=useState(true),[include,setInclude]=useState(true);
  const [preview,setPreview]=useState<{id:string;offset:number;more:boolean;rows:{location:string;quote:string;note:string}[]}|null>(null);
  const [jobFiles,setJobFiles]=useState<{id:string;files:JobFile[]}|null>(null);
  const running=useRef(false), draftKey=useRef('');
  const sync=async()=>{const x=await invoke<View>('background_status');setV(x);
    const d=x.work.draft;if(d&&d.id!==draftKey.current){setPreview(null);draftKey.current=d.id;setBase(x.library.version);setEdit(d.profile);setShare(false);setTerms(true);}
    return x;
  };
  useEffect(()=>{if(active)void sync().catch(e=>setMsg(String(e)));},[revision,active]);
  useEffect(()=>{
    if(!v?.work.busy||working)return;
    setProgress(v.work.progress||'正在整理…');
    const timer=setInterval(()=>{void invoke<{busy:boolean;progress:string}>('background_progress').then(w=>{
      setProgress(w.progress);if(!w.busy)void sync().catch(e=>setMsg(String(e)));
    }).catch(e=>setMsg(String(e)));},800);
    return ()=>clearInterval(timer);
  },[v?.work.busy,working]);
  const op=async(fn:()=>Promise<void>)=>{if(running.current||locked||v?.work.busy)return;running.current=true;setWorking(true);onBusy(true);setMsg('');
    try{await fn();}catch(e){setMsg(String(e));}finally{running.current=false;setWorking(false);onBusy(false);}};
  const pick=(mode:'folder'|'files'|'bundle')=>op(async()=>{
    const s=await invoke<Selection|null>('background_choose',{mode});if(!s)return;
    if(mode==='bundle'){select(null);await sync();}else select(s);
  });
  const generate=(measure=false,job?:string)=>op(async()=>{
    setProgress(measure?'正在计量…':'正在读取资料…');
    const timer=setInterval(()=>{void invoke<{progress:string}>('background_progress').then(w=>{if(running.current&&w.progress)setProgress(w.progress);}).catch(()=>{});},800);
    try{await invoke(job?'background_resume':'background_generate',job?{job}:{selection:measure?'measure':selected?.id,includeLibrary:include});if(!measure)select(null);}
    finally{clearInterval(timer);setProgress('');await sync();}
  });
  const draft=v?.work.draft;
  const inspect=async(id:string,offset:number)=>{try{
    const result=await invoke<{more:boolean;rows:{location:string;quote:string;note:string}[]}>('background_preview',{draft:draft?.id,document:id,offset});setPreview({...result,id,offset});
  }catch(e){setMsg(String(e));}};
  const apply=()=>op(async()=>{if(!edit)return;
    await invoke(draft?'background_apply':'background_edit',draft?{id:draft.id,profile:edit,share,mergeTerms:terms}:{version:base,profile:edit,share,mergeTerms:terms});
    draftKey.current='';setEdit(null);await sync();await onApplied();setMsg('背景与检索索引已保存。');
  });
  const off=locked||working||!!v?.work.busy;
  return <section aria-label="背景资料整理" className="space-y-3 rounded-lg border p-4 text-sm">
    <div className="flex flex-wrap items-center justify-between gap-2"><h3 className="font-medium">已导入资料</h3><div className="flex flex-wrap gap-2">
      <Button size="sm" disabled={off} onClick={()=>pick('folder')}>整理文件夹</Button>
      <Button size="sm" variant="outline" disabled={off} onClick={()=>pick('files')}>添加重点文件</Button>
    </div></div>
    {selected&&<div className="space-y-2 border-l-2 pl-3">
      <p className="break-all">{selected.label}{!selected.folder&&selected.count>3?' 等 '+selected.count+' 份':''}</p>
      <p className="text-xs">{selected.folder?'此文件夹内支持的文档':'所选文件'}将逐份发送至背景模型 {v?.model}（建议用高智能模型）。JSON / YAML 按整份处理。</p>
      <label className="block"><input type="checkbox" checked={include} disabled={off} onChange={e=>setInclude(e.target.checked)}/> 一并整理库中允许外发的片段</label>
      {v?.ready?<Button size="sm" disabled={off} onClick={()=>generate()}>发送并整理</Button>:<Button size="sm" variant="outline" onClick={onModels}>配置背景整理模型</Button>}
    </div>}
    {progress&&<div role="status">{progress} <Button size="sm" variant="outline" onClick={()=>void invoke('background_cancel').catch(e=>setMsg(String(e)))}>取消</Button></div>}
    {!!v?.jobs?.length&&<details><summary>整理任务 · {v.jobs.length}</summary>
      <p className="py-2 text-xs text-slate-500">继续时重核来源；仅未完成或已变化的内容需要再次发送至背景模型。</p>
      <ul className="divide-y">{v.jobs.map(job=><li key={job.id} className="py-3 space-y-2">
        <div className="flex flex-wrap items-center justify-between gap-2"><span>{job.model||'任务不可读'} · {job.completed}/{job.total} 份完成{job.failed>0?` · ${job.failed} 份失败`:''}{job.skipped>0?` · ${job.skipped} 份跳过`:''}</span>
          <div className="flex flex-wrap gap-2"><Button size="sm" variant="outline" disabled={off||job.status==='unreadable'} onClick={()=>void invoke<JobFile[]>('background_job_details',{job:job.id}).then(files=>setJobFiles({id:job.id,files})).catch(e=>setMsg(String(e)))}>查看进度</Button>
            <Button size="sm" disabled={off||!v.ready||job.status==='unreadable'} onClick={()=>generate(false,job.id)}>继续并核对</Button>
            <Button size="sm" variant="outline" disabled={off} onClick={()=>op(async()=>{await invoke('background_job_trash',{job:job.id});if(draft?.job===job.id){setEdit(null);draftKey.current='';}setJobFiles(null);await sync();})}>移入废纸篓</Button></div></div>
        {job.unknown_calls>0&&<p className="text-xs">{job.unknown_calls} 次中断请求用量未知，继续可能产生新的调用。</p>}
        {jobFiles?.id===job.id&&<ul className="max-h-52 overflow-auto text-xs space-y-1">{jobFiles.files.map(f=><li key={f.name}>{f.name} · {({queued:'等待',processing:'未完成',completed:'完成',failed:'失败',skipped:'跳过'} as Record<string,string>)[f.status]||f.status}{f.total>0?` · ${f.done}/${f.total} 段`:''}{f.reason?` · ${f.reason}`:''}</li>)}</ul>}
      </li>)}</ul>
    </details>}
    {edit&&<div className="space-y-2 rounded border p-3">
      <h4>{draft?'核对整理结果':'编辑已保存背景'}</h4>
      {draft&&<details><summary>{draft.documents.length} 份资料 · {draft.failures.length} 份未处理</summary>
        <ul>{draft.documents.map(f=><li key={f.id}><button type="button" className="underline" onClick={()=>inspect(f.id,0)}>{f.name} · {f.count} 条引文</button></li>)}
        {draft.failures.map(([f,e])=><li key={f}>{f}：{e}</li>)}</ul>{preview&&<div className="max-h-64 overflow-auto space-y-2 border-l pl-3">{preview.rows.map((r,i)=><div key={i}><p>{r.note}</p><blockquote className="text-xs">{r.location} · {r.quote}</blockquote></div>)}{preview.offset>0&&<button onClick={()=>inspect(preview.id,preview.offset-20)}>上一页</button>} {preview.more&&<button onClick={()=>inspect(preview.id,preview.offset+20)}>下一页</button>}</div>}</details>}
      {(['overview','role','aliases'] as const).map((k,i)=><label className="block" key={k}>{['背景综述','面试岗位','术语别名'][i]}
        <textarea className="block w-full rounded border p-2" rows={[7,1,3][i]} maxLength={[8000,200,2000][i]} disabled={off} value={edit[k]} onChange={e=>setEdit({...edit,[k]:e.target.value})}/></label>)}
      <label className="block"><input type="checkbox" checked={share} disabled={off} onChange={e=>setShare(e.target.checked)}/> {draft?'允许回答接口使用本批引文、综述和岗位':'允许回答接口使用综述和岗位'}</label>
      <label className="block"><input type="checkbox" checked={terms} disabled={off} onChange={e=>setTerms(e.target.checked)}/> 将术语合并到检索设置</label>
      <Button size="sm" disabled={off||!edit.overview.trim()} onClick={apply}>{draft?'应用到资料库':'保存调整'}</Button>
    </div>}
    {v?.library.state&&!edit&&<details><summary>当前背景</summary><p className="whitespace-pre-wrap py-2">{v.library.state.profile.overview}</p>
      <Button size="sm" variant="outline" disabled={off} onClick={()=>{setEdit(v.library.state!.profile);setBase(v.library.version);setShare(v.library.state!.share);}}>微调背景与岗位</Button></details>}
    {active&&v?.library.state&&!edit&&<BackgroundSettings revision={v.library.version} locked={off} names={names} applyNames={applyNames} onBusy={onBusy} onApplied={onApplied}/>}
    {v&&<details><summary>上下文预算 · 256K</summary>
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 py-2">
        <dt>整库一次读入</dt><dd>≤ 约 {num(v.library.full_budget)} tokens</dd>
        <dt>单次文字问答</dt><dd>≤ 约 {num(v.library.rag_budget)} tokens</dd>
        <dt>调试窗口</dt><dd>{num(v.library.baseline)} tokens（含输出）</dd>
      </dl>
      <p>{v.library.fits_full?'整库估算在 256K 基线以内。':'整库估算超过 256K，使用分批整理和检索问答。'} 单次文字问答建议选择至少 64K 的窗口。</p>
      <p className="mt-1 text-xs text-slate-500">本机按 UTF-8 字节保守估算，含提示和输出预留。模型计量会分批发送允许外发的片段并读取 API token 用量。</p>
      {v.library.measurement&&<p>{v.library.measurement.version!==v.library.version?'上版资料库':'本版资料库'} · {v.library.measurement.model}：{num(v.library.measurement.metrics.sum_prompt_tokens??0)} 输入 tokens（{v.library.measurement.chunks} 段，含 {v.library.measurement.metrics.calls} 次提示开销）。</p>}
      {(draft?.metrics||v.library.state?.metrics)?.peak_prompt_tokens!=null&&<p>上次整理单请求输入峰值：{num((draft?.metrics||v.library.state?.metrics)!.peak_prompt_tokens!)} tokens</p>}
      <Button size="sm" variant="outline" disabled={off||!v.ready||!v.library.chunks} onClick={()=>generate(true)}>用背景模型计量可外发资料</Button>
    </details>}
    <details><summary>外部 Agent 工具</summary><div className="flex flex-wrap gap-2 py-2">
      <Button variant="outline" size="sm" disabled={off} onClick={()=>pick('bundle')}>导入 Agent 结果</Button>
      <Button variant="outline" size="sm" disabled={off} onClick={()=>op(async()=>{const path=await invoke<string|null>('background_export_skill');if(path)setMsg('技能已导出：'+path);})}>导出整理 Skill</Button></div>
      <p className="text-xs text-slate-500">将技能文件夹放入项目的 .agents/skills（Codex）或 .claude/skills（Claude Code），用高智能模型整理后导回 .meetily.json。</p></details>
    {msg&&<p role="status" className="break-words">{msg}</p>}
  </section>;
}
