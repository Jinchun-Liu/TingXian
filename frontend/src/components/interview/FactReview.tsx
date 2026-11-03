'use client';
import {useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {Button} from '@/components/ui/button';
import {FactCandidate,FactCard,FactCorrection,FactHistory,FactSource} from '@/lib/interview-facts';

function Sources({sources}:{sources:FactSource[]}) {
  return <>{sources.map(source=><details key={source.source_id} className="rounded border p-2 mt-2"><summary className="cursor-pointer break-all">{source.path} · {source.location} · 保存时的原文快照</summary><p className="whitespace-pre-wrap mt-2">{source.text}</p></details>)}</>;
}
export function FactReview({card,candidates,locked,onSave,onRefresh,onBusyChange}:{card:FactCard;candidates:FactCandidate[];locked:boolean;onSave:(update:FactCorrection)=>Promise<FactCard>;onRefresh:()=>Promise<FactCard>;onBusyChange:(value:boolean)=>void}) {
  const [base,setBase]=useState(card.version),[text,setText]=useState(card.text),[note,setNote]=useState(card.confirmed_at ? card.source_note : ''),[reason,setReason]=useState('');
  const [chosen,setChosen]=useState<FactCandidate[]>([]),[confirmed,setConfirmed]=useState(false),[error,setError]=useState(''),[saving,setSaving]=useState(false);
  const [history,setHistory]=useState<FactCard[]|null>(null),[before,setBefore]=useState<number|null>(null),[historyBusy,setHistoryBusy]=useState(false);
  const savingRef=useRef(false),historyGeneration=useRef(0);
  const stale=base!==card.version;
  const reset=(next:FactCard)=>{setBase(next.version);setText(next.text);setNote(next.confirmed_at?next.source_note:'');setReason('');setChosen([]);setConfirmed(false);setError('');};
  const edit=(set:(value:string)=>void,value:string)=>{set(value);setConfirmed(false);};
  const reload=async()=>{
    if(savingRef.current || locked)return;
    savingRef.current=true;setSaving(true);onBusyChange(true);setError('');
    try {reset(await onRefresh());}
    catch(e){setError(String(e));}
    finally {savingRef.current=false;setSaving(false);onBusyChange(false);}
  };
  const save=async()=>{
    if(savingRef.current || locked || stale)return;
    if(!confirmed || !note.trim() || !reason.trim()) {setError('请填写具体来源、更正原因，并明确确认本次更正');return;}
    savingRef.current=true;setSaving(true);onBusyChange(true);setError('');
    try {
      const next=await onSave({expected_version:base,text,source_note:note,reason,confirmed,sources:chosen.map(s=>({source_id:s.source_id,path:s.path,hash:s.hash!,root:s.root!}))});
      reset(next);historyGeneration.current++;setHistory(null);setBefore(null);setHistoryBusy(false);
    } catch(e){setError(String(e));setConfirmed(false);}
    finally{savingRef.current=false;setSaving(false);onBusyChange(false);}
  };
  const loadHistory=async(older=false)=>{
    const generation=++historyGeneration.current;setHistoryBusy(true);setError('');
    try {
      const result=await invoke<FactHistory>('profile_facts_history',{before:older?before:null});
      if(generation===historyGeneration.current){setHistory(current=>older?[...(current||[]),...result.versions]:result.versions);setBefore(result.next_before);}
    }catch(e){if(generation===historyGeneration.current)setError(String(e));}
    finally{if(generation===historyGeneration.current)setHistoryBusy(false);}
  };
  return <section className="rounded border p-3 space-y-3 text-sm">
    <h3 className="font-medium">已确认事实更正 · 当前 v{card.version}</h3>
    <p className="text-xs text-slate-500">更正只写入独立版本，保留旧值、来源和确认时间，不修改原文件。语音、练习答案和 AI 建议不会自动写入事实卡。清空正文也会保存为空的新版本，旧版本仍可查看。</p>
    <div className="grid md:grid-cols-2 gap-3">
      <details className="rounded bg-slate-50 p-3" open><summary>当前事实卡 v{card.version} · {card.confirmed_at?'本人已确认':'沿用旧版，尚未重新确认'}</summary><p className="whitespace-pre-wrap mt-2">{card.text || '尚无已确认事实'}</p><p className="text-xs text-slate-500 mt-2">来源：{card.source_note}</p>{card.confirmed_at && <p className="text-xs text-slate-500">确认：{new Date(card.confirmed_at).toLocaleString('zh-CN')}</p>}<Sources sources={card.sources}/></details>
      <label>本人确认的事实卡（可留空）<textarea className="w-full border rounded p-2 mt-1 h-40 text-sm" maxLength={2200} value={text} disabled={locked || saving} onChange={e=>edit(setText,e.target.value)} placeholder="只填写本人核对过的事实与来源，不粘贴 AI 建议。" /></label>
    </div>
    {stale && <p role="alert" className="text-amber-700">事实卡已更新到 v{card.version}，当前编辑稿仍基于 v{base}，不能直接覆盖。请比较上方最新事实卡。</p>}
    <Button variant="outline" size="sm" disabled={locked || saving} onClick={reload}>载入最新事实卡并替换编辑稿</Button>
    <div className="grid md:grid-cols-2 gap-3">
      <label>具体来源与定位<textarea aria-label="事实更正来源" className="block w-full border rounded p-2 mt-1" maxLength={1200} value={note} disabled={locked || saving} onChange={e=>edit(setNote,e.target.value)} placeholder="例如资料名称、段落；或本人直接核对的事实及依据。仅记录在本地版本中。" /></label>
      <label>更正原因<textarea aria-label="事实更正原因" className="block w-full border rounded p-2 mt-1" maxLength={300} value={reason} disabled={locked || saving} onChange={e=>edit(setReason,e.target.value)} placeholder="说明旧值为何需要更正，或为何确认仍然有效。" /></label>
    </div>
    <details className="text-xs"><summary>附上当前检索的原始证据（可选，最多6段）</summary><p className="my-2 text-slate-500">先在主面板本地检索，再选择原始证据或个人汇总。练习、岗位和技术笔记不能作为个人经历证明。保存时会核对内容是否仍为同一版本。</p>{candidates.map(source=>{
      const valid=!!source.hash && !!source.root && ['personal','evidence'].includes(source.kind);
      const selected=chosen.some(s=>s.source_id===source.source_id);
      return <details className="border rounded p-2 mb-2" key={source.source_id}><summary>{source.path} · {source.location}</summary><p className="whitespace-pre-wrap mt-2">{source.text}</p><label className="flex items-center gap-2 mt-2"><input type="checkbox" aria-label={`事实来源 ${source.source_id}`} disabled={locked || saving || !valid || (!selected && chosen.length>=6)} checked={selected} onChange={e=>{setChosen(current=>e.target.checked?[...current,source]:current.filter(s=>s.source_id!==source.source_id));setConfirmed(false);}} />{valid?'附上这个来源快照':'仅可作参考，不能附为个人经历证明'}</label></details>;
    })}{!candidates.length && <p>目前没有检索片段；也可直接填写已亲自核对的具体来源。</p>}{chosen.length>0 && <p>已选择 {chosen.length} 段：{chosen.map(s=>s.path).join('、')}</p>}</details>
    <label className="flex items-center gap-2 text-xs"><input type="checkbox" disabled={locked || saving || stale} checked={confirmed} onChange={e=>setConfirmed(e.target.checked)} />我已核对新事实及来源，确认保存此更正</label>
    <div className="flex flex-wrap gap-2"><Button disabled={locked || saving || stale || !confirmed || !reason.trim() || !note.trim()} onClick={save}>{saving?'正在保存事实更正':'确认保存事实新版本'}</Button><Button variant="outline" disabled={historyBusy || saving} onClick={()=>loadHistory()}>{historyBusy?'正在读取事实版本':'查看事实版本历史'}</Button></div>
    {error && <p role="alert" className="text-amber-700">{error}</p>}
    {history && <div className="space-y-2 max-h-96 overflow-auto text-xs">{history.map(version=><details key={version.version} className="border rounded p-3"><summary className="cursor-pointer">v{version.version} · {version.version===card.version?'当前页面版本':version.version>card.version?'新于当前页面，请载入最新事实卡':'已被覆盖，保留记录'} · {version.confirmed_at?new Date(version.confirmed_at).toLocaleString('zh-CN'):'迁移基线，未重新确认'}</summary><p className="whitespace-pre-wrap mt-2">{version.text || '空事实卡'}</p><p className="mt-2">来源：{version.source_note}</p><p>更正原因：{version.reason}</p><Sources sources={version.sources}/></details>)}{before!==null && <Button variant="outline" disabled={historyBusy} onClick={()=>loadHistory(true)}>加载更早版本</Button>}</div>}
  </section>;
}
