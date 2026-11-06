'use client';
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import { BackgroundVersions } from '@/components/interview/BackgroundVersions';
import { DocumentRecord, DocumentUpdate, sourceStatusLabel } from '@/lib/interview-documents';

export function DocumentReview({document,locked,onSave}:{document:DocumentRecord;locked:boolean;onSave:(update:DocumentUpdate)=>Promise<unknown>}) {
  const [confirmed,setConfirmed]=useState(document.metadata.confirmation==='confirmed');
  const [from,setFrom]=useState(document.metadata.valid_from || '');
  const [until,setUntil]=useState(document.metadata.valid_until || '');
  const [error,setError]=useState('');
  const [loading,setLoading]=useState(false);
  const [segments,setSegments]=useState<{location:string;text:string}[] | null>(null);
  const preview=async()=>{
    setLoading(true);setError('');setSegments(null);
    try {setSegments(await invoke('profile_document_segments',{path:document.path,expectedRoot:document.root,expectedHash:document.hash}));}
    catch(e){setError(String(e));}finally{setLoading(false);}
  };
  const save=async()=>{
    if (from && until && from>until) {setError('有效期结束日期不能早于开始日期');return;}
    setError('');
    try {await onSave({path:document.path,expected_root:document.root,expected_hash:document.hash,confirmed,valid_from:from || null,valid_until:until || null});}
    catch(e){setError(String(e));}
  };
  return <details className="rounded border p-3 text-xs">
    <summary className="cursor-pointer break-all">{document.path.replace(/^\.背景\/(?:[a-f0-9]{64}\/)?/,'')} · {sourceStatusLabel(document.metadata)}</summary>
    <div className="mt-3 space-y-3">
      <p className="text-slate-500">核对针对本次导入的内容；源文件修改后请重新导入。日期可留空，保存不会修改原文件。练习参考仍只用于组织表达。</p>
      <Button variant="outline" size="sm" disabled={locked || loading} onClick={preview}>{loading ? '正在读取片段' : '查看已导入片段'}</Button>
      {/^\.背景\/[a-f0-9]{64}\//.test(document.path)&&<BackgroundVersions source={document.path.split('/')[1]} expected={document.hash} locked={locked}/>}
      {segments && <div className="max-h-72 overflow-auto space-y-2 bg-slate-50 p-2">{segments.map((segment,i)=><div key={i}><p className="text-slate-500">{segment.location}</p><p className="whitespace-pre-wrap">{segment.text}</p></div>)}</div>}
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
