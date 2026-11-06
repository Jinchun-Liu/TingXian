'use client';
import {Button} from '@/components/ui/button';
import type {RecoverySummary} from '@/lib/interview-recovery';
export function RecoveryDrafts({drafts,disabled,onRestore}:{drafts:RecoverySummary[];disabled:boolean;onRestore:(id:string)=>void}){
  if(!drafts.length)return null;
  return <section aria-label="恢复草稿"><header className="tx-section-head"><h3>恢复草稿</h3></header>
    <p className="text-xs text-slate-500">未替代正式保存。恢复不会开启采集或发送请求。</p>
    {drafts.map(d=><div className="tx-history-row" key={d.id}><span>{new Date(d.updated_at).toLocaleString('zh-CN')} · {d.turns} 条发言 · {d.calls} 次调用</span><Button size="sm" variant="outline" disabled={disabled} onClick={()=>onRestore(d.id)}>恢复草稿</Button></div>)}
  </section>;
}
