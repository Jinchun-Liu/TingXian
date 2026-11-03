'use client';
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';

export type InterviewContext = {role:string;organization:string;context:string;terminology:string;answer_language:string;transcription_language:string};
export const defaultInterviewContext:InterviewContext = {role:'',organization:'',context:'',terminology:'',answer_language:'auto',transcription_language:'auto'};
const textFields=[['role','目标岗位',120,'例如：品牌策划、产品运营、工程师'],['organization','目标组织（可选）',120,'例如：目标企业或机构'],['context','面试背景与关注点',1000,'描述行业、面试阶段、希望关注的能力；个人经历请在事实卡中确认'],['terminology','领域术语',1000,'填写专有名词及正确拼写，供 AI 理解转写；不会自动覆盖原始听写']] as const;
const speechLanguages=[['auto','自动识别'],['zh','中文'],['en','英文'],['ja','日文'],['ko','韩文'],['fr','法文'],['de','德文'],['es','西班牙文']];

export function InterviewPreferences({locked,onSaved}:{locked:boolean;onSaved:(value:InterviewContext)=>void}) {
  const [draft,setDraft]=useState<InterviewContext>(defaultInterviewContext);
  const [ready,setReady]=useState(false),[saving,setSaving]=useState(false),[reload,setReload]=useState(0);
  const [message,setMessage]=useState('正在读取面试场景…');
  const revision=useRef(0);
  useEffect(()=>{
    let current=true;const request=++revision.current;setReady(false);setMessage('正在读取面试场景…');
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
    <h3 className="text-sm font-medium">面试场景</h3>
    <p className="text-xs text-slate-500">适用于不同行业和岗位，可全部留空。这里的目标与术语不代表个人经历；回答时会发送到 AI。界面目前为中文。</p>
    {textFields.map(([field,label,maxLength,placeholder])=><label key={field} className="block text-sm">{label}<textarea aria-label={label} className="block w-full border rounded p-2 mt-1" rows={field==='context' || field==='terminology'?3:1} value={draft[field]} maxLength={maxLength} placeholder={placeholder} disabled={disabled} onChange={e=>setDraft(value=>({...value,[field]:e.target.value}))} /></label>)}
    <div className="grid sm:grid-cols-2 gap-3">
      <label className="block text-sm">回答语言<select aria-label="回答语言" className="block w-full border rounded p-2 mt-1" disabled={disabled} value={draft.answer_language} onChange={e=>setDraft(value=>({...value,answer_language:e.target.value}))}><option value="auto">跟随当前发言</option><option value="zh">中文</option><option value="en">英文</option></select></label>
      <label className="block text-sm">转写语言<select aria-label="转写语言" className="block w-full border rounded p-2 mt-1" disabled={disabled} value={draft.transcription_language} onChange={e=>setDraft(value=>({...value,transcription_language:e.target.value}))}>{speechLanguages.map(([value,label])=><option key={value} value={value}>{label}</option>)}</select></label>
    </div>
    <p className="text-xs text-slate-500">较短发言可指定转写语言；自动识别和非英语转写需要多语言 Whisper 模型。领域术语供 AI 参考，检索同义词请在“术语别名”中配置。</p>
    {locked && <p className="text-xs text-slate-500">请结束当前会话后修改场景。</p>}
    {message && <p className="text-sm" role="status">{message}</p>}
    <div className="flex flex-wrap gap-2"><Button size="sm" variant="outline" disabled={disabled} onClick={save}>{saving?'正在保存…':'保存面试场景'}</Button><Button size="sm" variant="outline" disabled={locked || saving} onClick={()=>setReload(value=>value+1)}>重新读取场景</Button><Button size="sm" variant="outline" disabled={locked || saving} onClick={()=>{revision.current++;setDraft({...defaultInterviewContext});setReady(true);setMessage('已恢复空白场景草稿，保存后生效。');}}>恢复空白场景</Button></div>
  </div>;
}
