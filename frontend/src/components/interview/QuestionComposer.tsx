'use client';
import {useRef,useState} from 'react';
import {Button} from '@/components/ui/button';
import {Send} from 'lucide-react';

export function QuestionComposer({value,editingSelf,busy,cloudAllowed,onChange,onSubmit,onLocalSearch,onCancelEdit}:{
  value:string;editingSelf:boolean;busy:boolean;cloudAllowed:boolean;onChange:(text:string)=>void;
  onSubmit:()=>void;onLocalSearch:()=>void;onCancelEdit:()=>void;
}) {
  const composing=useRef(false);
  const [compositionActive,setCompositionActive]=useState(false);
  const disabled=busy || !value.trim() || compositionActive || (!editingSelf && !cloudAllowed);
  const submit=()=>{if(!disabled && !composing.current)onSubmit();};
  return <>
    <label htmlFor="question" className="text-xs text-slate-500">{editingSelf?'纠正本人回答 · 不调用 AI':'手动纠正或补充问题 · ⌘ Enter 发送'}</label>
    <div className="flex flex-wrap gap-2 items-end">
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
        className="flex-1 min-w-40 h-20 border rounded-lg p-3 text-sm resize-y" placeholder="可选：编辑识别内容，或手动补充一句话" />
      <Button disabled={disabled} onClick={submit}><Send className="h-4 w-4 mr-1" />{editingSelf?'保存修正':'发送'}</Button>
      {editingSelf && <Button variant="outline" disabled={busy || compositionActive} onClick={onCancelEdit}>取消修改</Button>}
      <Button variant="outline" disabled={!value.trim() || busy || compositionActive} onClick={onLocalSearch}>本地检索</Button>
    </div>
  </>;
}
