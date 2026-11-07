'use client';
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import { BackgroundVersions } from '@/components/interview/BackgroundVersions';
import { DocumentRecord, DocumentUpdate, RecordSegment, UserAssertion, sourceStatusLabel } from '@/lib/interview-documents';

import { RecordRevision } from '@/components/interview/RecordRevision';

export function DocumentReview({document,locked,onSave,onExclude,exclusionLocked}:{document:DocumentRecord;locked:boolean;exclusionLocked?:boolean;onExclude?:(excluded:boolean)=>Promise<unknown>;onSave:(update:DocumentUpdate)=>Promise<unknown>}) {
  const [confirmed,setConfirmed]=useState(document.metadata.confirmation==='confirmed');
  const [from,setFrom]=useState(document.metadata.valid_from || '');
  const [until,setUntil]=useState(document.metadata.valid_until || '');
  const [error,setError]=useState('');
  const [loading,setLoading]=useState(false);
  const [history,setHistory]=useState<{versions:(UserAssertion & {source:{hash:string;location:string;text:string}})[];next_offset:number|null}|null>(null);
  const readHistory=async(offset=0)=>{setLoading(true);setError('');try{const next=await invoke<NonNullable<typeof history>>('profile_document_revision_history',{path:document.path,root:document.root,offset});setHistory(old=>({versions:offset?[...(old?.versions || []),...next.versions]:next.versions,next_offset:next.next_offset}));}catch(e){setError(String(e));}finally{setLoading(false);}};
  const [segments,setSegments]=useState<RecordSegment[] | null>(null);
  const preview=async()=>{
    setLoading(true);setError('');setSegments(null);
    try {setSegments(await invoke('profile_document_records',{path:document.path,expectedRoot:document.root,expectedHash:document.hash}));}
    catch(e){setError(String(e));}finally{setLoading(false);}
  };
  const save=async()=>{
    if (from && until && from>until) {setError('有效期结束日期不能早于开始日期');return;}
    setError('');
    try {await onSave({path:document.path,expected_root:document.root,expected_hash:document.hash,confirmed,valid_from:from || null,valid_until:until || null});}
    catch(e){setError(String(e));}
  };
  return <details className="rounded border p-3 text-xs">
    <summary className="cursor-pointer break-all">{document.path.replace(/^\.背景\/(?:[a-f0-9]{64}\/)?/,'')} · {document.excluded?'已排除检索':sourceStatusLabel(document.metadata)}</summary>
    <div className="mt-3 space-y-3">
      {onExclude&&<Button variant="outline" size="sm" disabled={exclusionLocked||loading} onClick={async()=>{setLoading(true);setError('');try{await onExclude(!document.excluded);}catch(e){setError(String(e));}finally{setLoading(false);}}}>{document.excluded?'恢复参与检索':'排除检索'}</Button>}
      <p className="text-slate-500">核对针对本次导入的内容；源文件修改后请重新导入。日期可留空，保存不会修改原文件。练习参考仍只用于组织表达。</p>
      <Button variant="outline" size="sm" disabled={locked || loading} onClick={preview}>{loading ? '正在读取片段' : '查看已导入片段'}</Button>
      {/^\.背景\/[a-f0-9]{64}\//.test(document.path)&&<BackgroundVersions source={document.path.split('/')[1]} expected={document.hash} locked={locked}/>}
      <Button variant="ghost" size="sm" disabled={loading} onClick={()=>readHistory()}>查看全部修订</Button>
      {history&&<div className="max-h-60 overflow-auto space-y-2">{history.versions.length===0&&<p>暂无本人修订</p>}{history.versions.map(v=><details key={`${v.record_id}:${v.revision}`}><summary className="cursor-pointer">{v.source.location} · v{v.revision}{v.source.hash!==document.hash?' · 旧来源':''}</summary><p className="whitespace-pre-wrap">{v.claim}</p><p>{v.reason} · {new Date(v.recorded_at).toLocaleString('zh-CN')}</p><details><summary className="cursor-pointer">当时的原文</summary><p className="whitespace-pre-wrap">{v.source.text}</p></details></details>)}{history.next_offset!==null&&<Button variant="ghost" size="sm" disabled={loading} onClick={()=>readHistory(history.next_offset!)}>更早修订</Button>}</div>}
      {segments && <div className="max-h-[65vh] overflow-auto space-y-2 bg-slate-50 p-2">{segments.map((segment,i)=><RecordRevision key={segment.source_id || i} document={document} segment={segment} locked={locked} />)}</div>}
      <label className="flex items-center gap-2"><input type="checkbox" disabled={locked} checked={confirmed} onChange={e=>setConfirmed(e.target.checked)} />我已核对这份导入资料</label>
      <div className="flex flex-wrap items-end gap-3">
        <label>有效期开始<input type="date" aria-label={`${document.path} 有效期开始`} disabled={locked} className="block rounded border p-1 mt-1" value={from} onChange={e=>setFrom(e.target.value)} /></label>
        <label>有效期结束<input type="date" aria-label={`${document.path} 有效期结束`} disabled={locked} className="block rounded border p-1 mt-1" value={until} onChange={e=>setUntil(e.target.value)} /></label>
        <Button variant="outline" size="sm" disabled={locked} onClick={save}>保存核对与日期</Button>
      </div>
      {document.metadata.confirmed_at && <p className="text-slate-500">上次确认：{new Date(document.metadata.confirmed_at).toLocaleString('zh-CN')}</p>}
      {error && <p role="alert" className="text-amber-700">{error}</p>}
    </div>
  </details>;
}
