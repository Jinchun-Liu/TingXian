'use client';
import {Button} from '@/components/ui/button';
import {ConflictSide,SourceConflict} from '@/lib/interview-conflicts';
function Side({side}:{side:ConflictSide}) {
  return <div className="rounded bg-white p-2 space-y-1"><p className="font-medium break-all">{side.fact_version!==null?`本人确认事实卡 v${side.fact_version}`:`${side.path} · ${side.location}`}</p><p className="whitespace-pre-wrap break-words">{side.quote}</p><p className="text-slate-500">识别到的表述：{side.value}</p></div>;
}
export function ConflictNotice({conflicts,onReview}:{conflicts:SourceConflict[];onReview:()=>void}) {
  if(!conflicts.length)return null;
  return <details className="rounded border border-amber-200 bg-amber-50 p-3 text-xs">
    <summary className="cursor-pointer text-amber-800">来源中有 {conflicts.length} 组表述差异，待核对</summary>
    <p className="text-slate-600 my-2">以下是本次检索时的原文，可能涉及计划变更或历史状态，不代表已判定哪一方错误。修改资料或事实卡后请重新检索。</p>
    <div className="space-y-3 max-h-96 overflow-auto">{conflicts.map(conflict=><section key={conflict.id} className="space-y-2"><p className="font-medium">{conflict.topic}</p><p>{conflict.reason}</p><div className="grid md:grid-cols-2 gap-2"><Side side={conflict.left}/><Side side={conflict.right}/></div></section>)}</div>
    <p className="text-slate-500 mt-3">需要更正时，在会前或结束会话后核对事实及来源；更正会保留旧版本。</p>
    <Button variant="outline" size="sm" className="mt-2" onClick={onReview}>打开资料与事实核对</Button>
  </details>;
}
