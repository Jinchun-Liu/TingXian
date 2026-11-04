'use client';
import {ChoiceMenu, ChoiceOption} from '@/components/ui/choice-menu';
import {useMemo, useState} from 'react';
import {ConversationFeed} from './WorkspaceFeed';
import {answerPresentation, DisplayTurn, fontPixels, PresentationPreferences, visibleTurns} from '@/lib/interview-presentation';

type Props = {
  history: DisplayTurn[];
  preferences: PresentationPreferences;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onEdit?: (id: string) => void;
};

export function AnswerConversation({history, preferences, selectedId, onSelect, onEdit}: Props) {
  const [search, setSearch] = useState('');
  const turns = useMemo(() => visibleTurns(history, preferences.showSelf).filter(t=>!preferences.focus || !t.passive), [history, preferences.showSelf, preferences.focus]);
  const rows = useMemo(() => turns.map((turn, index) => {
    const result = answerPresentation(turn.suggestion, ['done','recorded'].includes(turn.status), turn.question);
    return {turn, index, result, title: result.question || turn.question};
  }), [turns]);
  const query = search.trim().toLocaleLowerCase();
  const matches = query ? rows.filter(({turn, title, result}) =>
    [title, turn.question, result.answer].some(text => text.toLocaleLowerCase().includes(query))
  ) : rows;
  const selected = matches.find(({turn}) => turn.id === selectedId) || matches.at(-1);
  const shown = preferences.focus ? selected ? [selected] : [] : matches;
  const jumpTo = (id: string) => {
    onSelect(id);
    if (!preferences.focus) document.getElementById('turn-' + id)?.scrollIntoView({block:'start'});
  };
  return <section className="answer-workspace" aria-label="转写与对话">
    <header className="answer-nav">
      <h2>对话</h2>
      {rows.length > 0 && <div className="answer-history-controls">
        <input type="search" aria-label="搜索历史问题" placeholder="搜索历史" value={search} onChange={e=>setSearch(e.target.value)}/>
        <ChoiceMenu aria-label="选择历史问题" value={selected?.turn.id || ''} onValueChange={nextValue =>jumpTo(nextValue)}>
          {!matches.length && <ChoiceOption value="" disabled>没有匹配问题</ChoiceOption>}
          {matches.map(({turn, index, title}) => <ChoiceOption key={turn.id} value={turn.id}>{index+1} · {title.slice(0,90)}</ChoiceOption>)}
        </ChoiceMenu>
        <button type="button" onClick={()=>{
          setSearch('');onSelect(null);
          if (!preferences.focus) requestAnimationFrame(()=>document.getElementById('turn-'+rows.at(-1)?.turn.id)?.scrollIntoView({block:'start'}));
        }}>最新问题</button>
      </div>}
    </header>
    <ConversationFeed key={preferences.focus ? selected?.turn.id : 'all'}
      revision={shown.map(({turn}) => turn.id + turn.question + turn.suggestion + turn.status).join('|')} fill showNavigation={rows.length>0}>
      {!shown.length && <div className="answer-empty">
        <svg viewBox="0 0 60 26" width="60" height="26" aria-hidden="true"><path d="M2 13h10m5-5v10m6-15v20m7-17v14m7-11v8m6-6v4m6-2h9" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"/></svg>
        <p>{query ? '没有匹配的发言' : '等待提问'}</p>
      </div>}
      {shown.map(({turn,index,result}) => <article key={turn.id} id={'turn-'+turn.id} data-turn-id={turn.id}
        className="answer-sheet" style={{fontSize:fontPixels[preferences.fontSize]}}
        onPointerDown={()=>{if(preferences.focus)onSelect(turn.id);}}>
        {turn.passive ? <p className="answer-source">{turn.question}</p> : turn.role==='self' ? <p className="answer-self"><span>我</span>{turn.question}</p> : <>
          <div className="answer-body" aria-label="回答正文">{result.answer || <span className="answer-pending">{
            turn.status==='listening' ? '对方仍在说话…' : turn.status==='queued' ? '等待回答通道…' : turn.status==='generating' ? '正在回答…' : '尚无回答'
          }</span>}</div>
          {result.question && <blockquote className="answer-source" aria-label={result.corrected?'纠正后的问题':'原始识别问题'}>{result.question}</blockquote>}
        </>}
        {['error','cancelled'].includes(turn.status) && <p role="status" className="answer-status">{turn.status==='error'?'回答中断':'已取消'}</p>}
        <footer className="answer-meta">
          <span>{String(index+1).padStart(2,'0')} / {String(rows.length).padStart(2,'0')}</span>
          {preferences.focus && selectedId===turn.id && <span>已固定</span>}
          {onEdit && ['done','error','cancelled','recorded'].includes(turn.status) && <button type="button" onClick={()=>onEdit(turn.id)}>{turn.role==='self'?'纠正我的发言':'编辑后重问'}</button>}
        </footer>
      </article>)}
    </ConversationFeed>
  </section>;
}
