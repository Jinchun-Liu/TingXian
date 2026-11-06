'use client';
import {useMemo} from 'react';
import {ConversationFeed} from './WorkspaceFeed';
import {answerPresentation, DisplayTurn, fontPixels, PresentationPreferences, visibleTurns} from '@/lib/interview-presentation';

type Props = {
  history: DisplayTurn[];
  preferences: PresentationPreferences;
  onEdit?: (id: string) => void;
};

export function AnswerConversation({history, preferences, onEdit}: Props) {
  // Older installations can still send focus:true. Every turn remains in the
  // timeline; the legacy preference must never collapse restored history.
  const turns = useMemo(() => visibleTurns(history, preferences.showSelf), [history, preferences.showSelf]);
  const rows = useMemo(() => turns.map((turn, index) => {
    const result = answerPresentation(turn.suggestion, ['done','recorded'].includes(turn.status), turn.question);
    return {turn, index, result};
  }), [turns]);
  // Prefer the newest question with visible answer text. Parallel updates to
  // earlier questions never steal the viewport from a newer answer.
  const hasAnswer = ({turn, result}: typeof rows[number]) => turn.role !== 'self' && !turn.passive && !!result.answer.trim();
  const latest = rows.findLast(hasAnswer);
  const answerRevision = JSON.stringify(rows.filter(hasAnswer).map(({turn,result})=>[turn.id,result.answer]));
  return <section className="answer-workspace" aria-label="转写与对话">
    <header className="answer-nav"><h2>对话</h2></header>
    <ConversationFeed latest={latest && {id:latest.turn.id, text:latest.result.answer}} answerRevision={answerRevision}>
      {!rows.length && <div className="answer-empty">
        <svg viewBox="0 0 60 26" width="60" height="26" aria-hidden="true"><path d="M2 13h10m5-5v10m6-15v20m7-17v14m7-11v8m6-6v4m6-2h9" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"/></svg>
        <p>等待提问</p>
      </div>}
      {rows.map(({turn,index,result}) => <article key={turn.id} id={'turn-'+turn.id} data-turn-id={turn.id}
        className="answer-sheet" style={{fontSize:fontPixels[preferences.fontSize]}}>
        {turn.passive ? <p className="answer-source">{turn.question}</p> : turn.role==='self' ? <p className="answer-self"><span>我</span>{turn.question}</p> : <>
          <div className="answer-body" aria-label="回答正文" data-latest-answer={turn.id === latest?.turn.id || undefined}>{result.answer || <span className="answer-pending">{
            turn.status==='listening' ? '对方仍在说话…' : turn.status==='queued' ? '等待回答通道…' : turn.status==='generating' ? '正在回答…' : '尚无回答'
          }</span>}</div>
          {result.question && <blockquote className="answer-source" aria-label={result.corrected?'纠正后的问题':'原始识别问题'}>{result.question}</blockquote>}
        </>}
        {['error','cancelled'].includes(turn.status) && <p role="status" className="answer-status">{turn.status==='error'?'回答中断':'已取消'}</p>}
        <footer className="answer-meta">
          <span>{String(index+1).padStart(2,'0')} / {String(rows.length).padStart(2,'0')}</span>
          {onEdit && ['done','error','cancelled','recorded'].includes(turn.status) && <button type="button" onClick={()=>onEdit(turn.id)}>{turn.role==='self'?'纠正我的发言':'编辑后重问'}</button>}
        </footer>
      </article>)}
    </ConversationFeed>
  </section>;
}
