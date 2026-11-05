'use client';
import {useRef,useState} from 'react';
import {Button} from '@/components/ui/button';
import {Send} from 'lucide-react';

export function QuestionComposer({value,editingSelf,busy,cloudAllowed,os,onChange,onSubmit,onLocalSearch,onCancelEdit}:{
  os?:string;
  value:string;editingSelf:boolean;busy:boolean;cloudAllowed:boolean;onChange:(text:string)=>void;
  onSubmit:()=>void;onLocalSearch:()=>void;onCancelEdit:()=>void;
}) {
  const composing=useRef(false);
  const [compositionActive,setCompositionActive]=useState(false);
  const disabled=busy || !value.trim() || compositionActive || (!editingSelf && !cloudAllowed);
  const submit=()=>{if(!disabled && !composing.current)onSubmit();};
  return <div className="desk-composer">
    <label htmlFor="question" className={editingSelf?'composer-editing':'sr-only'}>{editingSelf?'纠正本人回答':'手动提问'}</label>
      <textarea id="question" value={value} maxLength={1500} onChange={e=>onChange(e.target.value)}
        onCompositionStart={()=>{composing.current=true;setCompositionActive(true);}}
        onCompositionEnd={()=>{composing.current=false;setCompositionActive(false);}}
        onKeyDown={e=>{
          if(!(e.metaKey||e.ctrlKey) || e.key!=='Enter' || e.altKey || e.shiftKey)return;
          // Keep the IME's Enter event intact; keyCode 229 also covers events
          // where the native composition flag is unavailable or already ended.
          if(composing.current || e.nativeEvent.isComposing || e.nativeEvent.keyCode===229)return;
          e.preventDefault();if(!e.repeat)submit();
        }}
        rows={2} placeholder={editingSelf?"修正我的发言":"输入问题…"} />
    <div className="composer-actions">
      <kbd title="发送快捷键">{os==='macos'?'⌘':'Ctrl'} ↵</kbd>
      <Button disabled={disabled} onClick={submit}><Send className="h-4 w-4 mr-1" />{editingSelf?'保存修正':'发送'}</Button>
      {editingSelf && <Button variant="outline" disabled={busy || compositionActive} onClick={onCancelEdit}>取消修改</Button>}
      <Button variant="outline" disabled={!value.trim() || busy || compositionActive} onClick={onLocalSearch}>本地检索</Button>
    </div>
  </div>;
}
