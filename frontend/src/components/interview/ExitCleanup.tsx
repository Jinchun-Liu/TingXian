'use client';
import {useEffect,useSyncExternalStore} from 'react';
import {invoke,isTauri} from '@tauri-apps/api/core';
import {listen,UnlistenFn} from '@tauri-apps/api/event';
import {Button} from '@/components/ui/button';
import {Dialog,DialogContent,DialogDescription,DialogHeader,DialogTitle} from '@/components/ui/dialog';
import {interviewExit,ExitRequest} from '@/lib/interview-exit';

export function ExitCleanup(){
  const view=useSyncExternalStore(interviewExit.subscribe,interviewExit.getSnapshot,interviewExit.getServerSnapshot);
  useEffect(()=>{
    if(!isTauri())return;
    let disposed=false;let unlisten:UnlistenFn|undefined;
    void (async()=>{
      const stop=await listen<ExitRequest>('interview-exit-requested',event=>{if(!disposed)void interviewExit.request(event.payload);});
      if(disposed){stop();return;}unlisten=stop;
      // Early Cmd-Q can precede React's listener. Native retains that request.
      const pending=await invoke<ExitRequest|null>('interview_exit_status');
      if(!disposed && pending)void interviewExit.request(pending);
    })().catch(error=>console.error('退出监听未完成',error));
    return()=>{disposed=true;unlisten?.();};
  },[]);
  return <Dialog open={view.requestId!==null} onOpenChange={()=>{}}>
    <DialogContent className="[&>button]:hidden" onEscapeKeyDown={event=>event.preventDefault()} onPointerDownOutside={event=>event.preventDefault()}>
      <DialogHeader><DialogTitle>退出听弦</DialogTitle><DialogDescription>退出前停止采集，将临时记录移入废纸篓。已保存会话、恢复草稿、资料和模型保留。恢复草稿不会自动发送未完成问题。</DialogDescription></DialogHeader>
      <p role="status">{view.phase}</p>
      {view.error && <p role="alert" className="text-sm text-amber-800 break-words">{view.error}</p>}
      {!view.busy && <div className="flex gap-2"><Button onClick={()=>void interviewExit.retry()}>重试清理并退出</Button><Button variant="outline" onClick={()=>void interviewExit.cancel()}>取消退出，返回应用</Button></div>}
    </DialogContent>
  </Dialog>;
}
