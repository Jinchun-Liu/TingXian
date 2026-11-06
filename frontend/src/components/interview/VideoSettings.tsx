'use client';
import {useCallback, useEffect, useRef, useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {Button} from '@/components/ui/button';

type Reason = 'not_bundled'|'unsupported_platform'|'runtime_not_pinned'|'integrity'|'launch'|'protocol'|'worker_exit'|'timeout'|'forced_stop'|'cleanup'|'abandoned';
type Snapshot = {
  generation:number;
  phase:'off'|'preparing'|'unavailable'|'stopping'|'fault';
  reason:Reason|null;
  inspection:{api_exports_present:boolean;machine_registration_present:boolean;user_registration_conflict:boolean}|null;
};
const reasons:Record<Reason,string> = {
  not_bundled:'当前版本未包含视频检查组件', unsupported_platform:'当前系统不支持此视频组件',
  runtime_not_pinned:'眼部校正组件尚未提供', integrity:'视频组件校验失败，请重新安装听弦',
  launch:'无法启动视频检查，请重试', protocol:'视频组件返回异常，请重新安装听弦',
  worker_exit:'视频检查意外退出，请重试', timeout:'视频检查超时，工作进程已关闭',
  forced_stop:'检查进程未响应，已强制关闭', cleanup:'尚未确认视频进程退出，请保留诊断并重启应用',
  abandoned:'视频检查中断，请重试',
};
type Action = 'read'|'inspect'|'stop';

// The controller belongs to the main native window. Opening or restoring this
// panel reads state only; neither mounting nor polling can launch a worker.
export function VideoSettings(){
  const [status,setStatus]=useState<Snapshot|null>(null);
  const [action,setAction]=useState<Action|null>('read');
  const [message,setMessage]=useState('');
  const mounted=useRef(false),revision=useRef(0),current=useRef<Snapshot|null>(null),operation=useRef<Action|null>(null);
  const request=useCallback(async(next:Action)=>{
    if(next==='inspect' && (operation.current || !current.current || ['preparing','stopping'].includes(current.current.phase) || current.current.reason==='cleanup'))return;
    if(next==='read' && operation.current)return;
    if(next==='stop' && operation.current==='stop')return;
    const ticket=++revision.current;
    operation.current=next;setAction(next);setMessage('');
    try {
      const result=await invoke<Snapshot>(next==='read'?'video_status':next==='inspect'?'video_inspect':'video_stop',
        next==='inspect'?{expectedGeneration:current.current!.generation}:undefined);
      if(!mounted.current || revision.current!==ticket)return;
      if(!current.current || result.generation>=current.current.generation){current.current=result;setStatus(result);}
    } catch {
      if(!mounted.current || revision.current!==ticket)return;
      // Unknown native state must be read again before another launch. Stop
      // remains available if an inspection/stop failed to report completion.
      current.current=null;setStatus(null);
      setMessage(next==='read'?'无法读取视频状态，请刷新重试':next==='stop'?'停止尚未确认，请刷新状态或再次停止':'检查未完成，请刷新状态后重试');
    } finally {
      if(mounted.current && revision.current===ticket){operation.current=null;setAction(null);}
    }
  },[]);
  useEffect(()=>{
    mounted.current=true;operation.current=null;void request('read');
    return()=>{mounted.current=false;++revision.current;};
  },[request]);
  useEffect(()=>{
    if(action || !status || !['preparing','stopping'].includes(status.phase))return;
    const timer=setTimeout(()=>void request('read'),500);
    return()=>clearTimeout(timer);
  },[action,status,request]);
  const checking=action==='inspect'||status?.phase==='preparing';
  const stopping=action==='stop'||status?.phase==='stopping';
  const active=checking||stopping;
  const uncertain=!status && !!message && !message.startsWith('无法读取');
  const description=stopping?'正在停止检查…':checking?'正在检查组件…':status?.reason?reasons[status.reason]:status?'尚未检查组件':'正在读取状态…';
  const inspection=status?.inspection;
  return <div className="tx-video">
    <section aria-labelledby="video-heading">
      <header className="tx-section-head"><h3 id="video-heading">视频已关闭</h3>
        {active||uncertain?<Button variant="outline" size="sm" disabled={stopping} onClick={()=>void request('stop')}>{stopping?'正在停止…':'停止检查'}</Button>:
          <Button variant="outline" size="sm" disabled={!!action || status?.reason==='cleanup'} onClick={()=>void request(status?'inspect':'read')}>{status?'检查组件':'刷新状态'}</Button>}
      </header>
      <p className="tx-video-status" role="status" aria-live="polite">{message||description}</p>
      {uncertain&&<button className="tx-text-action" type="button" disabled={!!action} onClick={()=>void request('read')}>刷新状态</button>}
      <div className="tx-video-controls" aria-describedby="video-unavailable">
        <label className="tx-audio-field"><span>摄像头</span><select aria-label="摄像头" disabled value="off"><option value="off">未开启</option></select></label>
        <Button variant="outline" size="sm" disabled>本地预览</Button>
        <label className="tx-video-toggle"><input type="checkbox" disabled checked={false} readOnly/>视线校正</label>
        <Button variant="outline" size="sm" disabled>开启视频输出</Button>
      </div>
      <p id="video-unavailable" className="tx-empty-line">眼部校正尚未开放。检查组件不会开启摄像头或音频。</p>
    </section>
    <details className="tx-video-details"><summary>组件状态</summary>
      <dl>
        <div><dt>系统视频接口</dt><dd>{inspection?(inspection.api_exports_present?'检测到':'未检测到'):'尚未检查'}</dd></div>
        <div><dt>视频输出组件</dt><dd>{inspection?(inspection.user_registration_conflict?'存在注册冲突':inspection.machine_registration_present?'检测到注册':'未安装'):'尚未检查'}</dd></div>
        <div><dt>眼部校正</dt><dd>尚未提供</dd></div>
      </dl>
    </details>
    <details className="tx-video-details"><summary>脸部替换<span>暂未开放</span></summary>
      <p>此模式尚未实现。</p><Button variant="outline" size="sm" disabled>开启脸部替换</Button>
    </details>
  </div>;
}
