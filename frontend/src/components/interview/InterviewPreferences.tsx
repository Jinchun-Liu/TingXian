'use client';
import {ChoiceMenu, ChoiceOption} from '@/components/ui/choice-menu';
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ConversationScenario, scenarioNames } from '@/lib/conversation-scenarios';
import { Button } from '@/components/ui/button';

export type InterviewContext = {scenario?:ConversationScenario;role:string;organization:string;context:string;terminology:string;answer_language:string;transcription_language:string};
export const defaultInterviewContext:InterviewContext = {scenario:'general',role:'',organization:'',context:'',terminology:'',answer_language:'auto',transcription_language:'auto'};
const textFields=[['role','角色 / 目标岗位',120,'例如：学习者、项目协调人、应聘岗位'],['organization','组织 / 团队（可选）',120,'例如：企业、学校或团队'],['context','对话背景与关注点',1000,'描述讨论主题、学习基础或会议目标；需要确认的事实请放入事实卡'],['terminology','领域术语',1000,'填写专有名词及正确拼写，供 AI 理解转写；不会自动覆盖原始听写']] as const;
const speechLanguages=[['auto','自动识别'],['zh','中文'],['en','英文'],['ja','日文'],['ko','韩文'],['fr','法文'],['de','德文'],['es','西班牙文']];

export function InterviewPreferences({locked,onSaved}:{locked:boolean;onSaved:(value:InterviewContext)=>void}) {
  const [draft,setDraft]=useState<InterviewContext>(defaultInterviewContext);
  const [ready,setReady]=useState(false),[saving,setSaving]=useState(false),[reload,setReload]=useState(0);
  const [message,setMessage]=useState('正在读取对话场景…');
  const revision=useRef(0);
  useEffect(()=>{
    let current=true;const request=++revision.current;setReady(false);setMessage('正在读取对话场景…');
    invoke<InterviewContext>('interview_preferences_get').then(value=>{if(current && request===revision.current){setDraft(value);setReady(true);setMessage('');}}).catch(e=>{if(current && request===revision.current)setMessage(`读取失败：${String(e)}`);});
    return ()=>{current=false;};
  },[reload]);
  const disabled=locked || saving || !ready;
  const save=async()=>{
    setSaving(true);setMessage('');
    try{const saved=await invoke<InterviewContext>('interview_preferences_save',{preferences:draft});setDraft(saved);onSaved(saved);setMessage('已保存；下次回答和采集使用此场景。');}
    catch(e){setMessage(`保存失败：${String(e)}`);}finally{setSaving(false);}
  };
  return <div className="border-t pt-3 space-y-3">
    <h3 className="text-sm font-medium">对话场景</h3>
    <p className="text-xs text-slate-500">可选择面试、学习问答、会议或通用对话。背景可全部留空。这里的目标与术语不代表个人经历；回答时会发送到 AI。界面目前为中文。</p>
    <label className="block text-sm">使用场景<ChoiceMenu aria-label="使用场景" className="w-full mt-1" disabled={disabled} value={draft.scenario || 'interview'} onValueChange={nextValue =>setDraft(value=>({...value,scenario:nextValue as ConversationScenario}))}>{Object.entries(scenarioNames).map(([value,label])=><ChoiceOption key={value} value={value}>{label}</ChoiceOption>)}</ChoiceMenu></label>
    <p className="text-xs text-slate-500">面试模式在获得外发许可后默认自动回答对方发言；其他模式默认只转写、按需提问。本人声音始终不自动触发 AI。切换场景不会清空历史。</p>
    {textFields.map(([field,label,maxLength,placeholder])=><label key={field} className="block text-sm">{label}<textarea aria-label={label} className="block w-full border rounded p-2 mt-1" rows={field==='context' || field==='terminology'?3:1} value={draft[field]} maxLength={maxLength} placeholder={placeholder} disabled={disabled} onChange={e=>setDraft(value=>({...value,[field]:e.target.value}))} /></label>)}
    <div className="grid sm:grid-cols-2 gap-3">
      <label className="block text-sm">回答语言<ChoiceMenu aria-label="回答语言" className="w-full mt-1" disabled={disabled} value={draft.answer_language} onValueChange={nextValue =>setDraft(value=>({...value,answer_language:nextValue}))}><ChoiceOption value="auto">跟随当前发言</ChoiceOption><ChoiceOption value="zh">中文</ChoiceOption><ChoiceOption value="en">英文</ChoiceOption></ChoiceMenu></label>
      <label className="block text-sm">转写语言<ChoiceMenu aria-label="转写语言" className="w-full mt-1" disabled={disabled} value={draft.transcription_language} onValueChange={nextValue =>setDraft(value=>({...value,transcription_language:nextValue}))}>{speechLanguages.map(([value,label])=><ChoiceOption key={value} value={value}>{label}</ChoiceOption>)}</ChoiceMenu></label>
    </div>
    <p className="text-xs text-slate-500">较短发言可指定转写语言；自动识别和非英语转写需要多语言 Whisper 模型。领域术语供 AI 参考，检索同义词请在“术语别名”中配置。</p>
    {locked && <p className="text-xs text-slate-500">请结束当前会话后修改场景。</p>}
    {message && <p className="text-sm" role="status">{message}</p>}
    <div className="flex flex-wrap gap-2"><Button size="sm" variant="outline" disabled={disabled} onClick={save}>{saving?'正在保存…':'保存对话场景'}</Button><Button size="sm" variant="outline" disabled={locked || saving} onClick={()=>setReload(value=>value+1)}>重新读取场景</Button><Button size="sm" variant="outline" disabled={locked || saving} onClick={()=>{revision.current++;setDraft({...defaultInterviewContext});setReady(true);setMessage('已恢复空白场景草稿，保存后生效。');}}>恢复空白场景</Button></div>
  </div>;
}
