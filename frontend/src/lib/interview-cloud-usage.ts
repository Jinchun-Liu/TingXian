/** Provider-reported cumulative seconds, not speech duration, tokens or a bill. */
export type CloudTaskUsage={task_id:string;source:'System'|'Microphone';state:'connecting'|'running'|'finished'|'failed'|'cancelled'|'interrupted';reported_seconds:number|null;inconsistent:boolean};
export type CloudUsageSnapshot={version:1;tasks:CloudTaskUsage[];history_missing?:boolean};
const id=(s:unknown)=>typeof s==='string'&&/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s);
export function parseCloudUsage(raw:unknown):CloudUsageSnapshot {
  const v=raw as CloudUsageSnapshot,seen=new Set<string>();
  if(!v||v.version!==1||!Array.isArray(v.tasks)||v.tasks.length>20000||(v.history_missing!==undefined&&typeof v.history_missing!=='boolean'))throw Error('云转写用量记录格式无效，原记录保留');
  for(const r of v.tasks){
    if(!r||!id(r.task_id)||seen.has(r.task_id.toLowerCase())||!['System','Microphone'].includes(r.source)||!['connecting','running','finished','failed','cancelled','interrupted'].includes(r.state)
      ||typeof r.inconsistent!=='boolean'||(r.reported_seconds!==null&&(typeof r.reported_seconds!=='number'||!Number.isFinite(r.reported_seconds)||r.reported_seconds<0||r.reported_seconds>Number.MAX_SAFE_INTEGER)))throw Error('云转写用量记录格式无效，原记录保留');
    seen.add(r.task_id.toLowerCase());
  }
  return {version:1,...(v.history_missing?{history_missing:true}:{}),tasks:v.tasks.map(r=>({task_id:r.task_id,source:r.source,state:r.state,reported_seconds:r.reported_seconds,inconsistent:r.inconsistent}))};
}
export function cloudUsageSummary(s:CloudUsageSnapshot,source?:CloudTaskUsage['source']) {
  const tasks=s.tasks.filter(t=>!source||t.source===source);
  // Integer milliseconds permit stable addition of fractional legacy values.
  let milliseconds=BigInt(0),known=0,unknown=0,unfinished=0;
  for(const t of tasks){if(t.reported_seconds===null||t.inconsistent)unknown++;else{milliseconds+=BigInt(Math.trunc(t.reported_seconds))*BigInt(1000)+BigInt(Math.round((t.reported_seconds%1)*1000));known++;}if(t.state!=='finished')unfinished++;}
  const seconds=`${milliseconds/BigInt(1000)}${milliseconds%BigInt(1000)?'.'+String(milliseconds%BigInt(1000)).padStart(3,'0').replace(/0+$/,''):''}`;
  return {attempts:tasks.length,known,unknown,unfinished,seconds};
}
export function cloudUsageLabel(s:CloudUsageSnapshot):string {
  const n=cloudUsageSummary(s);if(!n.attempts)return s.history_missing?'云转写：旧记录用量未知':'';
  return [`云转写：${n.known?'已报告 '+n.seconds+' 秒':'用量未知'}`,`${n.attempts} 次连接`,...(n.unknown?[`${n.unknown} 次用量未知`]:[]),...(s.history_missing?['旧记录用量未知']:[])].join(' · ');
}
export class CloudUsage {
  private scope='';private tracking=false;private epoch=0;private pending:Promise<CloudUsageSnapshot>|null=null;
  private restored:CloudUsageSnapshot={version:1,tasks:[]};private data:CloudUsageSnapshot={version:1,tasks:[]};
  constructor(private invoke:<T>(cmd:string,args:Record<string,unknown>)=>Promise<T>,private uuid:()=>string){}
  begin():string {if(this.restored.tasks.length>20000-2048)throw Error('云转写用量记录已满，请先保存并重新开始会话');if(!this.scope)this.scope=this.uuid();this.tracking=true;return this.scope;}
  snapshot():CloudUsageSnapshot{return parseCloudUsage(this.data);}
  async flush():Promise<CloudUsageSnapshot>{
    if(!this.tracking)return this.snapshot();if(this.pending)return this.pending;
    const epoch=this.epoch,scope=this.scope;
    const task=(async()=>{
      let raw:unknown;try{raw=await this.invoke('interview_cloud_asr_usage',{scope});}catch(e){if(epoch!==this.epoch)return this.snapshot();throw e;}
      if(epoch!==this.epoch)return this.snapshot();
      const fresh=parseCloudUsage(raw);
      const tasks=[...this.restored.tasks,...fresh.tasks];
      const next=parseCloudUsage({version:1,tasks,history_missing:!!this.restored.history_missing});this.data=next;return this.snapshot();
    })();
    this.pending=task;
    try{return await task;}finally{if(this.pending===task)this.pending=null;}
  }
  restore(raw?:CloudUsageSnapshot|null){
    const next:CloudUsageSnapshot=raw?parseCloudUsage(raw):{version:1,tasks:[],history_missing:true};
    next.tasks=next.tasks.map(t=>({...t,state:['connecting','running'].includes(t.state)?'interrupted':t.state}));
    this.epoch++;this.scope='';this.tracking=false;this.pending=null;this.restored=next;this.data=parseCloudUsage(next);
  }
  reset(){this.restore({version:1,tasks:[]});}
}
