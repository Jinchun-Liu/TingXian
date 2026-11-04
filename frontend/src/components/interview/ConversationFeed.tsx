'use client';

import {ReactNode, useLayoutEffect, useRef, useState} from 'react';

type Props = {
  children: ReactNode;
  latest?: {id: string; text: string};
  className?: string;
  readingHistory?: boolean;
  onRead?: () => void;
  onReturn?: () => void;
};

export function ConversationFeed({children, latest, className = 'answer-scroll', readingHistory, onRead, onReturn}: Props) {
  const feed = useRef<HTMLDivElement>(null);
  const [paused, setPaused] = useState(false);
  const [jump, setJump] = useState(0);
  const following = !paused && !readingHistory;

  useLayoutEffect(() => {
    const region = feed.current;
    const body = region?.querySelector<HTMLElement>('[data-latest-answer]');
    if (!following || !latest?.text || !region || !body) return;
    const answer = body.getBoundingClientRect();
    const top = region.getBoundingClientRect().top + region.clientTop;
    const bottom = top + region.clientHeight;
    // Move only this window's feed. The corrected question can be much taller than its answer.
    if (answer.bottom > bottom || answer.bottom < top || (answer.top < top && answer.height <= region.clientHeight)) {
      region.scrollTop += answer.bottom - bottom;
    }
  }, [following, latest?.id, latest?.text, jump]);

  const pause = () => {
    if (!following) return;
    setPaused(true);
    onRead?.();
  };
  const resume = () => {
    onReturn?.();
    setPaused(false);
    setJump(n => n + 1);
  };

  return <>
    <div ref={feed} role="region" aria-label="面试对话记录" tabIndex={0} aria-live="off"
      className={className} style={{overflowAnchor:'none'}}
      onWheel={event => { if (event.deltaY !== 0) pause(); }}
      onTouchMove={pause} onPointerDownCapture={pause}
      onKeyDown={event => { if (['ArrowUp','ArrowDown','PageUp','PageDown','Home','End',' '].includes(event.key)) pause(); }}>
      {children}
    </div>
    {latest && <div className="answer-scroll-tools flex shrink-0 justify-end px-4 py-1">
      <button type="button" className="text-sm" onClick={resume}>返回最新答复</button>
    </div>}
  </>;
}
