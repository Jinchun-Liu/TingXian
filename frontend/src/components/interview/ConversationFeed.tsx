'use client';

import {ReactNode,useCallback,useEffect,useRef,useState} from 'react';
import {Button} from '@/components/ui/button';

/** Reading and generation are independent: interaction pauses scrolling only. */
export function ConversationFeed({children,revision,fill=false}:{children:ReactNode;revision:unknown;fill?:boolean}) {
  const feed=useRef<HTMLDivElement>(null);
  const follow=useRef(false);
  const previous=useRef(revision);
  const [following,setFollowing]=useState(false);
  const [updated,setUpdated]=useState(false);
  const pause=()=>{follow.current=false;setFollowing(false);};
  const latest=useCallback(()=>{
    if(feed.current)feed.current.scrollTop=feed.current.scrollHeight;
    setUpdated(false);
  },[]);
  useEffect(()=>{
    if(previous.current===revision)return;
    previous.current=revision;
    if(follow.current)latest();else setUpdated(true);
  },[revision,latest]);
  return <>
    <div ref={feed} role="region" aria-label="面试对话记录" tabIndex={0} aria-live="off"
      className={`${fill?'min-h-0 flex-1':'h-[58vh] min-h-80'} overflow-y-auto p-5 space-y-6 bg-slate-50/60 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500`}
      onPointerDownCapture={pause} onFocusCapture={pause}
      onWheel={event=>{if(event.deltaY<0)pause();}}
      onTouchMove={pause}
      onKeyDown={event=>{if(['ArrowUp','PageUp','Home'].includes(event.key))pause();}}
      onScroll={()=>{
        const el=feed.current;
        // Reaching the bottom never opts into following. Explicit user action
        // is required, so scrolling/selection cannot silently re-enable it.
        if(follow.current && el && el.scrollHeight-el.scrollTop-el.clientHeight>2)pause();
      }}>
      {children}
    </div>
    <div className="shrink-0 flex flex-wrap items-center justify-between gap-2 border-t px-4 py-2 text-xs text-slate-500">
      <span>{following?'正在跟随最新对话':'阅读模式 · 保留当前位置'}{!following && updated?' · 有新内容':''}</span>
      <div className="flex flex-wrap gap-2">
        <Button variant="outline" size="sm" onClick={latest}>回到最新对话</Button>
        <Button variant={following?'default':'outline'} size="sm" aria-pressed={following} onClick={()=>{
          if(follow.current)pause();else {follow.current=true;setFollowing(true);latest();}
        }}>{following?'停止跟随':'跟随最新对话'}</Button>
      </div>
    </div>
  </>;
}
