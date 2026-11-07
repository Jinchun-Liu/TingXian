import type {TranscriptInput,UtteranceEnd} from './interview-scheduler';
export type MemoryHandle={id:string;epoch:number;generation:string};
export type MemoryCheckpoint={session_id:string;epoch:number;revision:number;last_record:number;sha256:string};
export type MemoryBoundary={session_id:string;epoch:number;last_record:number;last_sha256:string;checkpoint:MemoryCheckpoint|null};
export type MemoryStatus={active:boolean;running?:boolean;context_version?:number;records?:number;covered_through?:number;missing_older_history?:boolean;persistence_failed?:boolean;accounted_units?:number;attempts?:number;error?:string|null;pressure?:{active:boolean;estimated_bytes:number;hard_input_budget:number;projected_bytes:number;image_cost_unknown:boolean;compaction_p95_ms:number};last_attempt?:{status:string;request_sent:boolean;usage:unknown}};
type Invoke=<T>(command:string,args?:Record<string,unknown>)=>Promise<T>;
type Raw={id:string;expected_revision:number;source:string;text:string;capture_session:string|null;utterance_id:string|null;closed:boolean;topic_closed:boolean};
type Entry={raw:Raw;pieces:Map<number,string>;revision:number};
export function memoryBoundaryValid(v:any):v is MemoryBoundary {
  const integer=(n:any)=>Number.isSafeInteger(n)&&n>=0,hash=(s:any)=>typeof s==='string'&&/^[a-f0-9]{64}$/i.test(s);
  return !!v&&typeof v.session_id==='string'&&/^[\w:./-]{1,160}$/.test(v.session_id)&&integer(v.epoch)&&v.epoch>0&&integer(v.last_record)&&v.last_record<=20000&&hash(v.last_sha256)
    &&(v.checkpoint===null||!!v.checkpoint&&v.checkpoint.session_id===v.session_id&&v.checkpoint.epoch===v.epoch&&integer(v.checkpoint.revision)&&v.checkpoint.revision>0&&v.checkpoint.revision<=128&&integer(v.checkpoint.last_record)&&v.checkpoint.last_record<=v.last_record&&hash(v.checkpoint.sha256));
}
/** Serial local-write acknowledgements precede every 120-entry ring update.
 * A failed operation stays queued; it is not skipped on the next transcript.
 * No provider credentials, prompts, timers or network model calls live here. */
export class SessionMemory {
  handle:MemoryHandle|null=null;private opened=false;private opening:Promise<void>|null=null;private seed:TranscriptInput[]=[];
  private capture='';private entries=new Map<string,Entry>();private pending:{work:()=>Promise<void>;fail:(e:unknown)=>void}[]=[];
  private running:Promise<void>|null=null;private failure:unknown=null;private tickRunning=false;
  constructor(private invoke:Invoke,private uuid:()=>string){}
  get blocked(){return this.failure!==null;}
  get pendingCount(){return this.pending.length;}
  async ensure(existing:TranscriptInput[]=[],missing=false){
    if(this.opening)return this.opening;if(this.opened)return;
    this.opening=(async()=>{
      if(!this.handle){this.handle=await this.invoke<MemoryHandle|null>('interview_memory_open',{resumeId:null,missingOlderHistory:missing||existing.length>0});this.beginCapture();this.seed=this.handle?existing.filter(e=>!e.is_partial):[];}
      while(this.seed.length){await this.transcript(this.seed[0],true);this.seed.shift();}this.opened=true;
    })();try{await this.opening;}finally{this.opening=null;}
  }
  /** A new recording gets a fresh namespace even when native sequence IDs reset. */
  beginCapture(){this.capture=this.uuid();}
  dispatch(work:()=>Promise<void>,fail:(e:unknown)=>void){
    if(this.pending.length>=128){const e=Error('未保存缓冲超过128段；后续输入未接收，请停止采集并保留窗口');this.failure=e;fail(e);return;}
    this.pending.push({work,fail});this.pump();
  }
  private pump(){
    if(this.running||this.failure!==null)return;
    this.running=(async()=>{while(this.pending.length&&this.failure===null){const job=this.pending[0];try{await job.work();this.pending.shift();}catch(e){this.failure=e;job.fail(e);}}})().finally(()=>{this.running=null;if(this.pending.length&&this.failure===null)this.pump();});
  }
  async flush(){while(this.running)await this.running;if(this.failure!==null)throw this.failure;}
  async retry(){this.failure=null;this.pump();await this.flush();}
  private key(e:TranscriptInput){return `${this.capture}-${e.capture_session??0}-${e.native_session_id??0}-${e.source}-${e.microphone_epoch??0}-${e.utterance_id??`segment-${e.sequence_id}`}`;}
  private async write(entry:Entry,raw:Raw){
    if(!this.handle)return;
    const ack=await this.invoke<{persisted:boolean;event:{revision:number}}>('interview_memory_append',{handle:this.handle,input:raw});
    if(!ack.persisted||ack.event.revision!==raw.expected_revision+1)throw Error('会话原文未得到保存确认');
    entry.raw=raw;entry.revision=ack.event.revision;
  }
  async transcript(e:TranscriptInput,legacy=false){
    if(!this.handle||e.is_partial||!['System','Microphone','Confirmed'].includes(e.source))return;
    const key=this.key(e);let entry=this.entries.get(key);
    if(!entry){entry={raw:{id:`raw-${key}`,source:e.source,expected_revision:0,text:'',capture_session:`${this.capture}:${e.capture_session??0}:${e.native_session_id??0}`,utterance_id:String(e.utterance_id??e.sequence_id),closed:false,topic_closed:false},pieces:new Map(),revision:0};this.entries.set(key,entry);}
    const pieces=new Map(e.replaces_utterance?[]:entry.pieces);pieces.set(e.sequence_id,e.text);
    const text=[...pieces.values()].join(' '),closed=legacy||e.utterance_id==null||entry.raw.closed;
    if(entry.revision===0||text!==entry.raw.text||closed!==entry.raw.closed)await this.write(entry,{...entry.raw,expected_revision:entry.revision,text,closed,topic_closed:closed});
    entry.pieces=pieces;e.memory_event_id=entry.raw.id;
    // The UI ring is smaller than this revision working set. Older originals
    // remain in SQLite; this map is not the persistent history.
    while(this.entries.size>256){const first=this.entries.keys().next().value!;this.entries.delete(first);}
  }
  async end(e:UtteranceEnd){
    if(!this.handle)return;
    for(const entry of this.entries.values())if(entry.raw.capture_session?.startsWith(this.capture+':')&&entry.raw.source===e.source&&entry.raw.utterance_id===String(e.utterance_id)&&!entry.raw.closed){
      if(e.success)await this.write(entry,{...entry.raw,expected_revision:entry.revision,closed:true,topic_closed:true});
    }
  }
  async correct(event:TranscriptInput,text:string){
    await this.flush();if(!this.handle)return;
    if(!event.memory_event_id)throw Error('这条历史原文没有记忆引用，请重新载入会话后修正');
    const raw=await this.invoke<Raw>('interview_memory_raw_get',{handle:this.handle,id:event.memory_event_id});
    if(!raw.text.includes(event.text)||raw.text.indexOf(event.text)!==raw.text.lastIndexOf(event.text))throw Error('原文片段已变化或重复，请重新选择完整发言');
    const updated={...raw,text:raw.text.replace(event.text,text)};
    const entry=[...this.entries.values()].find(e=>e.raw.id===raw.id)||{raw,pieces:new Map(),revision:raw.expected_revision};
    await this.write(entry,updated);
    if(entry.pieces.has(event.sequence_id))entry.pieces.set(event.sequence_id,text);
  }
  async confirmed(text:string){await this.flush();if(!this.handle)return;const raw:Raw={id:`confirmed-${this.uuid()}`,expected_revision:0,source:'Confirmed',text,capture_session:null,utterance_id:null,closed:true,topic_closed:true};await this.invoke('interview_memory_append',{handle:this.handle,input:raw});}
  async boundary():Promise<MemoryBoundary|null>{await this.flush();return this.handle?this.invoke('interview_memory_boundary',{handle:this.handle}):null;}
  async restore(boundary:MemoryBoundary|null,existing:TranscriptInput[]){
    await this.flush();
    if(boundary){const next=await this.invoke<MemoryHandle>('interview_memory_restore',{reference:boundary});this.handle=next;this.opened=true;this.entries.clear();this.beginCapture();}
    else {this.handle=null;this.opened=false;this.seed=[];this.entries.clear();await this.ensure(existing,true);}
  }
  async tick(allowed:boolean){if(!allowed||!this.handle||this.tickRunning||this.blocked||this.pending.length)return;this.tickRunning=true;try{await this.invoke('interview_memory_tick',{handle:this.handle,sendAllowed:true});}finally{this.tickRunning=false;}}
  settingsChanged(){if(!this.handle)this.opened=false;}
  reset(){this.handle=null;this.opened=false;this.seed=[];this.entries.clear();this.pending=[];this.failure=null;this.beginCapture();}
}
