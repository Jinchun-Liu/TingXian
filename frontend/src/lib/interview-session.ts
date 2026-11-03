import type {DocumentMetadata} from './interview-documents';
import type {SourceConflict} from './interview-conflicts';
import type {LatencyRecord} from './interview-latency';
import type {TranscriptInput} from './interview-scheduler';
import {parseUsageSnapshot,UsageSnapshot} from '@/lib/interview-usage';
export type Source = { source_id: string; path: string; location: string; text: string; kind: string; allow_external: boolean; hash?: string; root?: string; metadata?: DocumentMetadata };
export type Turn = { restored_from?: string; role: 'remote' | 'self' | 'manual'; sequence_id?: number; utterance_id?: number | null; microphone_epoch?: number | null; capture_session?: number; id: string; question: string; confirmed_answer: string | null; suggestion: string; status: 'listening' | 'queued' | 'generating' | 'done' | 'error' | 'cancelled' | 'recorded'; sources: Source[]; conflicts?: SourceConflict[]; fact_version?: number; time: string; timing: string; latency?: LatencyRecord };
export type SessionSnapshot={version:1;history:Turn[];transcripts:TranscriptInput[];spoken_dialogue:TranscriptInput[];current:{question:string;answer:string};calls:number;usage:string;usage_ledger?:UsageSnapshot|null};
export type SavedSession={id:number;saved_at:string;turns:number;calls:number};
export type SessionPage={sessions:SavedSession[];next_before:number|null};
const object=(v:any)=>v!==null && typeof v==='object' && !Array.isArray(v);
const strings=(v:any,keys:string[])=>object(v) && keys.every(k=>typeof v[k]==='string');
const nullableString=(v:any)=>v===null || typeof v==='string';
const number=(v:any)=>typeof v==='number' && Number.isFinite(v);
function sourceValid(v:any):boolean {
  if(!strings(v,['source_id','path','location','text','kind']) || typeof v.allow_external!=='boolean')return false;
  const m=v.metadata;
  return m==null || (object(m) && ['unreviewed','confirmed','content_changed'].includes(m.confirmation)
    && ['unknown','current','expired','not_yet_valid','content_changed','invalid'].includes(m.validity)
    && ['confirmed_at','valid_from','valid_until','updated_at'].every(k=>nullableString(m[k])));
}
function conflictValid(v:any):boolean {
  return strings(v,['id','topic','category','reason']) && [v.left,v.right].every(s=>strings(s,['path','location','quote','value']) && nullableString(s.source_id) && (s.fact_version===null || number(s.fact_version)));
}
function transcriptValid(v:any):boolean {
  return strings(v,['text','source','timestamp']) && number(v.sequence_id) && Number.isSafeInteger(v.sequence_id) && v.sequence_id>=0
    && typeof v.is_partial==='boolean' && number(v.audio_start_time) && number(v.audio_end_time)
    && ['utterance_id','microphone_epoch','capture_session','native_session_id','confidence'].every(k=>v[k]==null || number(v[k]));
}
/** Validate the full payload before changing the live window. No best-effort
 * partial recovery that could silently mix or drop dialogue. */
export function parseSessionSnapshot(raw:unknown):SessionSnapshot {
  const v:any=raw;
  if(!object(v) || v.version!==1 || !Array.isArray(v.history) || !v.history.length || v.history.length>10000
    || !strings(v.current,['question','answer']) || !Number.isSafeInteger(v.calls) || v.calls<0 || typeof v.usage!=='string'
    || ![v.transcripts,v.spoken_dialogue].every(a=>Array.isArray(a) && a.length<=120 && a.every(transcriptValid)))throw Error('保存的会话格式无效，当前记录保留');
  const ids=new Set<string>();
  for(const t of v.history){
    if(!strings(t,['id','question','suggestion','time','timing']) || ids.has(t.id) || !nullableString(t.confirmed_answer)
      || !['remote','self','manual'].includes(t.role) || !['listening','queued','generating','done','error','cancelled','recorded'].includes(t.status)
      || !Array.isArray(t.sources) || !t.sources.every(sourceValid) || (t.conflicts!==undefined && (!Array.isArray(t.conflicts)||!t.conflicts.every(conflictValid)))
      || !['sequence_id','utterance_id','microphone_epoch','capture_session','fact_version'].every(k=>t[k]==null || number(t[k]))) throw Error('保存的发言或来源格式无效，当前记录保留');
    ids.add(t.id);
  }
  if(v.usage_ledger!=null)parseUsageSnapshot(v.usage_ledger,v.calls);
  return JSON.parse(JSON.stringify(v));
}
export function recoverSessionSnapshot(raw:unknown,prefix:string):SessionSnapshot {
  const s=parseSessionSnapshot(raw);
  s.history=s.history.map(t=>({...t,restored_from:prefix,id:`restored-${prefix}-${t.id}`,status:t.status==='generating'?'cancelled':['queued','listening'].includes(t.status)?'recorded':t.status}));
  if(s.usage_ledger)s.usage_ledger.requests=s.usage_ledger.requests.map(r=>({...r,question_id:`restored-${prefix}-${r.question_id}`,state:r.state==='pending'?'cancelled':r.state}));
  return parseSessionSnapshot(s);
}
