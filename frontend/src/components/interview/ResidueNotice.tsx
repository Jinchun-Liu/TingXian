'use client';
import {useEffect,useRef,useState} from 'react';
import {invoke,isTauri} from '@tauri-apps/api/core';
import {indexedDBService} from '@/services/indexedDBService';
import {Button} from '@/components/ui/button';
type Residue={recording_folders:number;unfinished_folders:number;staged_archives:number;database_meetings:number;saved_sessions:number};
export function ResidueNotice({blocked,revision,onClear}:{blocked:boolean;revision:number;onClear:()=>void}) {
  const [data,setData]=useState<(Residue & {cache_meetings:number;cache_transcripts:number})|null>(null);
  const [failure,setFailure]=useState('');const [dismissed,setDismissed]=useState(false);const [retry,setRetry]=useState(0);
  const epoch=useRef(0);
  useEffect(()=>{
    const version=++epoch.current;setData(null);setFailure('');
    if(blocked || !isTauri())return;
    let disposed=false;
    void (async()=>{
      try {
        const native=await invoke<{active:boolean;residue?:Residue}>('interview_residue_status');
        if(native.active || disposed || version!==epoch.current)return;
        if(!native.residue)throw Error('残留检查没有返回完整结果');
        const cache=await indexedDBService.inspectInterviewHistory();
        if(disposed || version!==epoch.current)return;
        setData({...native.residue,cache_meetings:cache.meetings,cache_transcripts:cache.transcripts});
      }catch(e){if(!disposed && version===epoch.current)setFailure(String(e));}
    })();
    return()=>{disposed=true;};
  },[blocked,revision,retry]);
  if(blocked)return null;
  if(dismissed)return <Button variant="outline" size="sm" onClick={()=>{setDismissed(false);setRetry(n=>n+1);}}>重新检查上次残留</Button>;
  if(failure)return <section className="rounded border border-amber-200 p-3 text-sm"><p>未能完成残留检查，记录保持原样：{failure}</p><Button variant="outline" size="sm" onClick={()=>setRetry(n=>n+1)}>重新检查残留</Button></section>;
  if(!data || !(data.recording_folders+data.staged_archives+data.database_meetings+data.cache_meetings+data.cache_transcripts))return null;
  return <section className="rounded border border-amber-200 bg-amber-50 p-4 text-sm space-y-2" aria-label="上次面试残留">
    <p>检测到上次采集留下的记录：{data.recording_folders} 个记录目录（其中 {data.unfinished_folders} 个未完成）、{data.database_meetings} 条会议记录、{data.cache_meetings} 个缓存会话（{data.cache_transcripts} 条转写）、{data.staged_archives} 个未完成清理的备份目录。</p>
    <p>可将这些临时记录移入废纸篓。{data.saved_sessions} 份明确保存的会话、个人资料和模型保留。未保存的 AI 答案不能从这些缓存补回。</p>
    <div className="flex gap-2"><Button size="sm" variant="outline" onClick={onClear}>清理上次残留，保留已保存会话</Button><Button size="sm" variant="outline" onClick={()=>setDismissed(true)}>暂时保留这些记录</Button></div>
  </section>;
}
