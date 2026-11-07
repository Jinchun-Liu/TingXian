import {parseSessionSnapshot,SessionSnapshot} from '@/lib/interview-session';
export type RecoverySnapshot={version:1;snapshot:SessionSnapshot;composer:{question:string;confirmed_answer:string}};
export type RecoverySummary={id:string;revision:number;updated_at:string;turns:number;calls:number};
type Listing={epoch:number;drafts:RecoverySummary[]};
type Invoke=<T>(name:string,args?:Record<string,unknown>)=>Promise<T>;
export function parseRecoverySnapshot(value:unknown):RecoverySnapshot {
  const v=value as RecoverySnapshot;
  if(!v || v.version!==1 || !v.composer || typeof v.composer.question!=='string' || typeof v.composer.confirmed_answer!=='string')throw Error('恢复草稿格式无效，当前内容保留');
  return {version:1,snapshot:parseSessionSnapshot(v.snapshot,true),composer:{question:v.composer.question,confirmed_answer:v.composer.confirmed_answer}};
}
export type RecoveryView={drafts:RecoverySummary[];error:string;savedAt:string|null};
const INITIAL:RecoveryView={drafts:[],error:'',savedAt:null};
/** One writer per mounted owner. Other mounts use distinct IDs, so merely
 * opening the app cannot replace an unread recovery draft. */
export class RecoveryWriter {
  private id:string;private epoch:number|null=null;private revision=0;private signature='';
  private running:Promise<void>|null=null;private initializing:Promise<void>|null=null;
  private paused=false;private fault=false;private generation=0;
  private view=INITIAL;private listeners=new Set<()=>void>();
  constructor(private invoke:Invoke,private read:()=>RecoverySnapshot|Promise<RecoverySnapshot>,private uuid:()=>string){this.id=uuid();}
  subscribe=(fn:()=>void)=>{this.listeners.add(fn);return()=>{this.listeners.delete(fn);};};
  getSnapshot=()=>this.view;getServerSnapshot=()=>INITIAL;
  private update(next:Partial<RecoveryView>){this.view={...this.view,...next};this.listeners.forEach(fn=>fn());}
  async initialize():Promise<void>{
    if(this.epoch!==null)return;
    if(this.initializing)return this.initializing;
    const generation=this.generation;
    const work=this.invoke<Listing>('interview_recovery_list').then(list=>{
      if(generation!==this.generation)return;
      if(!Number.isSafeInteger(list?.epoch)||list.epoch<0||!Array.isArray(list.drafts))throw Error('恢复草稿目录无效');
      this.epoch=list.epoch;this.update({drafts:list.drafts.filter(d=>d.id!==this.id),error:''});
    });
    this.initializing=work;
    try{await work;}catch(e){if(generation===this.generation){this.fault=true;this.update({error:String(e)});}throw e;}finally{if(this.initializing===work)this.initializing=null;}
  }
  async flush():Promise<void>{
    if(this.paused)return;
    if(this.running){await this.running;return this.flush();}
    const generation=this.generation;
    const work=(async()=>{
      await this.initialize();if(this.paused||generation!==this.generation)return;
      const draft=parseRecoverySnapshot(await this.read()),signature=JSON.stringify(draft);
      if(this.paused||generation!==this.generation)return;
      const meaningful=draft.snapshot.cloud_asr_usage?.tasks.length||draft.snapshot.history.length||draft.snapshot.transcripts.length||draft.composer.question||draft.composer.confirmed_answer;
      if(signature===this.signature || (!meaningful&&!this.signature))return;
      await this.invoke('interview_recovery_write',{id:this.id,epoch:this.epoch,revision:++this.revision,draft});
      if(generation!==this.generation)return;
      this.signature=signature;this.fault=false;this.update({error:'',savedAt:new Date().toISOString()});
    })();
    this.running=work;
    try{await work;}catch(e){if(generation===this.generation){this.fault=true;this.update({error:String(e)});}throw e;}
    finally{if(this.running===work)this.running=null;}
  }
  // After a storage error wait for an explicit retry, avoiding a write loop on a full disk.
  tick(){if(!this.fault&&!this.paused&&!this.running)void this.flush().catch(()=>{});}
  retry=async()=>{this.fault=false;await this.flush();};
  async pause(){this.paused=true;if(this.running)await this.running.catch(()=>{});}
  resume(){this.paused=false;}
  async resetAfterClear(){
    this.generation++;this.id=this.uuid();this.epoch=null;this.initializing=null;this.revision=0;this.signature='';this.fault=false;
    this.update({drafts:[],error:'',savedAt:null});
    await this.initialize();
  }
}
