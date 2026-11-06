'use client';

import {ReactNode, useLayoutEffect, useRef, useState} from 'react';

type Props = {
  children: ReactNode;
  latest?: {id: string; text: string};
  className?: string;
  answerRevision?: string;
};

export function ConversationFeed({children, latest, className = 'answer-scroll', answerRevision}: Props) {
  const feed = useRef<HTMLDivElement>(null);
  const [paused, setPaused] = useState(false);
  const [jump, setJump] = useState(0);
  const following = !paused;

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
  }, [following, latest?.id, latest?.text, answerRevision, jump]);

  const pause = () => {
    if (!following) return;
    setPaused(true);
  };
  const resume = () => {
    setPaused(false);
    setJump(n => n + 1);
  };

  return <div className="answer-feed">
    <div ref={feed} role="region" aria-label="面试对话记录" tabIndex={0} aria-live="off"
      className={className} style={{overflowAnchor:'none'}}
      onWheel={event => { if (event.deltaY !== 0) pause(); }}
      onTouchMove={pause} onPointerDownCapture={pause}
      onKeyDown={event => { if (['ArrowUp','ArrowDown','PageUp','PageDown','Home','End',' '].includes(event.key)) pause(); }}>
      {children}
    </div>
    {latest && paused && <div className="answer-scroll-tools">
      <button type="button" onClick={resume}><svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M12 4v16m-6-6 6 6 6-6"/></svg>回到最新</button>
    </div>}
  </div>;
}
