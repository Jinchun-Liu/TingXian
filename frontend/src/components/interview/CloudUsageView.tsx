'use client';
import {CloudUsageSnapshot,cloudUsageLabel,cloudUsageSummary} from '@/lib/interview-cloud-usage';
export function CloudUsageView({value,error}:{value:CloudUsageSnapshot;error:string}){
  if(!value.tasks.length&&!value.history_missing&&!error)return null;
  return <div className="text-xs"><details><summary>{cloudUsageLabel(value)||'云转写用量'}</summary><div className="mt-2 space-y-1">
    {(['System','Microphone'] as const).map(source=>{const s=cloudUsageSummary(value,source);return s.attempts?<p key={source}>{source==='System'?'对方声音':'我的声音'} · {s.known?`已报告 ${s.seconds} 秒`:'用量未知'} · {s.attempts} 次连接{s.unknown?` · ${s.unknown} 次未知`:''}{s.unfinished?` · ${s.unfinished} 次未正常结束`:''}</p>:null;})}
    <p>任务累计值按连接去重。未返回的尾段、取消和失败可能另有用量，以服务商账单为准。</p>
  </div></details>{error&&<p role="status">{error}</p>}</div>;
}
