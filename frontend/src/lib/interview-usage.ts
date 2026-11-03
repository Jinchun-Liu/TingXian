/** Observed API usage, never an estimate of the bill. One record per generation
 * revision; usage belongs to the streamed attempt, earlier retries stay unknown. */
export type UsageRequest = {question_id:string;revision:number;accepted:boolean;attempts:number;total_tokens:number|null;state:'pending'|'done'|'error'|'cancelled'};
export type UsageSnapshot = {version:1;legacy_calls:number;requests:UsageRequest[]};
const integer=(v:unknown,max=Number.MAX_SAFE_INTEGER):v is number=>typeof v==='number' && Number.isSafeInteger(v) && v>=0 && v<=max;
export function parseUsageSnapshot(value:unknown,calls:number):UsageSnapshot {
  const v=value as UsageSnapshot;
  if(!v || v.version!==1 || !integer(v.legacy_calls,0xffffffff) || !Array.isArray(v.requests) || v.requests.length>20000)throw Error('会话用量记录格式无效，当前记录保留');
  const seen=new Set<string>();let accepted=v.legacy_calls;
  for(const r of v.requests){
    if(!r || typeof r.question_id!=='string' || !r.question_id || r.question_id.length>1000 || !integer(r.revision,0xffffffff)
      || typeof r.accepted!=='boolean' || !integer(r.attempts,2) || (r.total_tokens!==null && !integer(r.total_tokens))
      || !['pending','done','error','cancelled'].includes(r.state) || (r.total_tokens!==null && r.attempts===0))throw Error('会话用量记录格式无效，当前记录保留');
    const key=JSON.stringify([r.question_id,r.revision]);
    if(seen.has(key))throw Error('会话用量记录重复，当前记录保留');seen.add(key);
    if(r.accepted)accepted++;
  }
  if(accepted!==calls)throw Error('会话调用数与用量记录不一致，当前记录保留');
  return JSON.parse(JSON.stringify(v));
}
export class InterviewUsage {
  private data:UsageSnapshot;
  constructor(value?:UsageSnapshot,calls=0){this.data=value?parseUsageSnapshot(value,calls):{version:1,legacy_calls:calls,requests:[]};}
  private find(id:string,revision:number){return this.data.requests.find(r=>r.question_id===id && r.revision===revision);}
  begin(id:string,revision:number){
    if(this.find(id,revision))throw Error('重复的用量请求标识');
    this.data.requests.push({question_id:id,revision,accepted:false,attempts:0,total_tokens:null,state:'pending'});
  }
  accept(id:string,revision:number):boolean {
    const r=this.find(id,revision);if(!r || r.accepted)return false;r.accepted=true;return true;
  }
  finish(id:string,revision:number,state:UsageRequest['state']):boolean {
    const r=this.find(id,revision);if(!r || r.state!=='pending')return false;r.state=state;return true;
  }
  event(id:string,revision:number,event:{type:string;attempt?:number;usage?:{total_tokens?:number}|null}):boolean {
    const r=this.find(id,revision);if(!r)return false;
    if(event.type==='request_sent' && integer(event.attempt,2) && event.attempt>0){r.attempts=Math.max(r.attempts,event.attempt);return true;}
    if(event.type==='usage' && integer(event.attempt,2) && event.attempt>0){
      r.attempts=Math.max(r.attempts,event.attempt);
      if(integer(event.usage?.total_tokens))r.total_tokens=event.usage!.total_tokens!;
      return true;
    }
    if(event.type==='done'){
      // A complete response proves at least one attempt even if its send event
      // was lost. Replace cumulative usage; never add repeated snapshots.
      r.attempts=Math.max(1,r.attempts);
      if(integer(event.usage?.total_tokens))r.total_tokens=event.usage!.total_tokens!;
      this.finish(id,revision,'done');return true;
    }
    if(event.type==='error')return this.finish(id,revision,'error');
    return false;
  }
  snapshot():UsageSnapshot{return JSON.parse(JSON.stringify(this.data));}
  acceptedCalls():number{return this.data.legacy_calls+this.data.requests.filter(r=>r.accepted).length;}
}
export function usageSummary(snapshot:UsageSnapshot){
  let knownTokens=BigInt(0),knownResponses=0,attempts=0,unknownAttempts=0,unobservedSubmissions=0;
  for(const r of snapshot.requests){
    attempts+=r.attempts;
    if(r.total_tokens!==null){knownTokens+=BigInt(r.total_tokens);knownResponses++;}
    unknownAttempts+=Math.max(0,r.attempts-(r.total_tokens!==null?1:0));
    if(r.accepted && !r.attempts)unobservedSubmissions++;
  }
  return {knownTokens:knownTokens.toString(),knownResponses,attempts,unknownAttempts,unobservedSubmissions,legacyCalls:snapshot.legacy_calls};
}
export function usageLabel(snapshot:UsageSnapshot):string {
  const s=usageSummary(snapshot);
  if(!snapshot.requests.length && !s.legacyCalls)return '累计用量：尚无调用记录';
  const incomplete=s.unknownAttempts || s.unobservedSubmissions || s.legacyCalls;
  const parts=[s.knownResponses?`${incomplete?'已知累计':'累计'} ${s.knownTokens} tokens`:'累计用量未知'];
  if(s.attempts)parts.push(`观测到 ${s.attempts} 次发送`);
  if(s.unknownAttempts)parts.push(`${s.unknownAttempts} 次发送用量未知`);
  if(s.unobservedSubmissions)parts.push(`${s.unobservedSubmissions} 次提交未观测到发送`);
  if(s.legacyCalls)parts.push(`旧记录 ${s.legacyCalls} 次用量未知`);
  return parts.join(' · ');
}
