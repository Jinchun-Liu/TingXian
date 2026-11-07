'use client';
import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {Button} from '@/components/ui/button';
type View={aliases:string;blocked_lines:string[]};

export function AliasSettings({locked,sourceRevision}:{locked:boolean;sourceRevision?:string}) {
  const [draft,setDraft]=useState(''),[view,setView]=useState<View>({aliases:'',blocked_lines:[]});
  const [ready,setReady]=useState(false),[saving,setSaving]=useState(false),[reload,setReload]=useState(0),[message,setMessage]=useState('');
  const alive=useRef(false),sequence=useRef(0),working=useRef(false),baseline=useRef<string|null>(null),editing=useRef('');
  const change=(value:string)=>{editing.current=value;setDraft(value);};
  useEffect(()=>{alive.current=true;return()=>{alive.current=false;sequence.current++;};},[]);
  useEffect(()=>{
    if(working.current)return;
    let current=true;const request=++sequence.current;
    void invoke<View>('interview_aliases_state').then(next=>{
      if(!current||request!==sequence.current)return;
      if(baseline.current===null||editing.current===baseline.current){change(next.aliases);baseline.current=next.aliases;setReady(true);setMessage('');}
      else if(next.aliases!==baseline.current){setReady(false);setMessage('别名已变化，编辑草稿已保留；请重新读取后核对。');}
      setView(next);
    }).catch(e=>{if(current&&request===sequence.current){setReady(false);setMessage(`读取失败：${String(e)}`);}});
    return()=>{current=false;};
  },[sourceRevision,reload]);
  const disabled=locked||saving||!ready;
  const act=async(line?:string)=>{
    if(disabled||working.current||baseline.current===null)return;
    working.current=true;setSaving(true);setMessage('');const request=++sequence.current;let committed=false;
    try{
      if(line!==undefined)await invoke('interview_aliases_manual',{lines:[line],expected:baseline.current});
      else await invoke('interview_aliases_save',{aliases:draft,expected:baseline.current});
      committed=true;
      if(!alive.current||request!==sequence.current)return;
      baseline.current=draft;
      const next=await invoke<View>('interview_aliases_state');
      if(!alive.current||request!==sequence.current)return;
      baseline.current=next.aliases;change(next.aliases);setView(next);setMessage(line!==undefined?'已转为手动别名。':next.blocked_lines.length?'已保存；待核对别名仍暂停检索。':'别名已保存。');
    }catch(e){if(alive.current&&request===sequence.current){setMessage(`${committed?'已保存，但状态刷新失败':'保存失败'}：${String(e)}`);if(committed)setReady(false);}}
    finally{working.current=false;if(alive.current&&request===sequence.current)setSaving(false);}
  };
  return <div className="space-y-3 border-t pt-3">
    <label className="block text-sm">术语别名<textarea aria-label="术语别名" placeholder="每行一组，英文逗号分隔" className="block w-full border rounded p-2 mt-1 h-28 text-sm" maxLength={4000} disabled={disabled} value={draft} onChange={e=>change(e.target.value)}/></label>
    {view.blocked_lines.length>0&&<div className="space-y-2"><p className="text-sm text-amber-700">以下别名的来源待核对，已暂停检索。</p>
      <ul className="divide-y rounded border">{view.blocked_lines.map(line=><li key={line} className="flex flex-wrap items-center gap-2 p-2"><span className="min-w-0 flex-1 break-words text-sm">{line}</span><Button size="sm" variant="outline" aria-label={`将 ${line} 转为手动别名`} disabled={disabled||draft!==baseline.current} onClick={()=>void act(line)}>转为手动别名</Button></li>)}</ul>
    </div>}
    {message&&<p role="status" className="text-sm">{message}</p>}
    <div className="flex flex-wrap gap-2"><Button size="sm" variant="outline" disabled={disabled} onClick={()=>void act()}>保存别名</Button><Button size="sm" variant="outline" disabled={locked||saving} onClick={()=>{baseline.current=null;setReady(false);setReload(n=>n+1);}}>重新读取别名</Button></div>
  </div>;
}
