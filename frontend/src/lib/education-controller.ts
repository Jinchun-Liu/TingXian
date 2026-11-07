import {Study,Request,Result,Task,Draft,clone,newStudy,restoreStudy,editDraft,invalidate,commitAttempt,prepareRequest,applyResult,packs,studyLimits} from './education';
export type Bridge={invoke:<T>(name:string,args?:Record<string,unknown>)=>Promise<T>};
export type Preview={request:Request;budget:number;model:string;endpoint:string;profile_stamp:string;scope_epoch:number;output_reserve:number;context_limit:number;estimate:string};

/** One owner; serialized saves and generation-bound results. No restore dispatch. */
export class EducationController {
  study=newStudy();dirty=false;saving=false;error='';preview:Preview|null=null;preparing=false;assistActive=false;
  private serial=0;private savePromise:Promise<void>|null=null;private timer:ReturnType<typeof setTimeout>|null=null;
  private listeners=new Set<()=>void>();private pending=new Map<string,Request>();private cancelled:Promise<unknown>=Promise.resolve();
  constructor(private bridge:Bridge){}
  subscribe=(fn:()=>void)=>{this.listeners.add(fn);return()=>{this.listeners.delete(fn);};};
  private notify(){for(const fn of this.listeners)fn();}
  changed(){this.serial++;this.dirty=true;this.error='';this.notify();if(this.timer)clearTimeout(this.timer);this.timer=setTimeout(()=>{void this.persist().catch(()=>{});},600);}
  private cancelNative(){const sessionId=this.study.id,generation=this.study.generation;this.preview=null;this.cancelled=this.bridge.invoke('education_cancel',{sessionId,generation}).catch(e=>{this.error='取消尚未确认：'+String(e);this.notify();throw e;});void this.cancelled.catch(()=>{});}
  edit(change:Partial<Draft>){editDraft(this.study,change);this.cancelNative();this.changed();}
  configure(change:Partial<Study['preferences']>){invalidate(this.study);this.study.preferences={...this.study.preferences,...change};this.cancelNative();this.changed();}
  updateScope(update:(study:Study)=>void){invalidate(this.study);update(this.study);this.study.feedback.forEach(f=>f.status='stale');this.cancelNative();this.changed();}
  async commit(reviseId?:string){await commitAttempt(this.study,reviseId);this.cancelNative();this.changed();await this.persist();}
  async persist():Promise<void>{
    if(this.timer){clearTimeout(this.timer);this.timer=null;}if(this.savePromise)return this.savePromise;
    if(!this.dirty)return;
    this.saving=true;this.notify();
    this.savePromise=(async()=>{
      while(this.dirty){const serial=this.serial,document=clone(this.study),expected=this.study.store_revision;
        try{const revision=await this.bridge.invoke<number>('education_save',{document,expected});if(!Number.isSafeInteger(revision)||revision<expected)throw Error('保存回执无效');this.study.store_revision=revision;if(serial===this.serial)this.dirty=false;}
        catch(e){this.error='保存未完成，当前输入仍保留：'+String(e);throw e;}
      }
    })();
    try{await this.savePromise;}finally{this.savePromise=null;this.saving=false;this.notify();}
  }
  async load(id:string){await this.cancel();await this.persist();const serial=this.serial;const raw=await this.bridge.invoke('education_load',{id});const recovered=await restoreStudy(raw);if(serial!==this.serial)throw Error('读取期间输入已变化，当前输入保留；请重新选择记录');this.study=recovered;this.preview=null;this.changed();await this.persist();}
  async fresh(packId=this.study.pack_id){await this.cancel();await this.persist();const preferences=clone(this.study.preferences);this.study=newStudy();this.study.preferences=preferences;const pack=packs.find(p=>p.id===packId)!;this.study.pack_id=pack.id;this.study.pack_version=pack.version;this.study.goal=pack.questions[0].goal;this.preview=null;this.changed();}
  async archive(){this.study.status='archived';this.changed();await this.fresh();}
  async cancel(){invalidate(this.study);this.cancelNative();this.changed();await this.cancelled;}
  async prepare(attemptId:string,lane:Task['lane']='interactive'){
    this.preparing=true;this.error='';this.notify();
    try{
      await this.cancelled;await this.persist();const request=await prepareRequest(this.study,attemptId,lane);
      const info=await this.bridge.invoke<Omit<Preview,'request'>>('education_preview',{request});
      if(request.generation!==this.study.generation||request.session_id!==this.study.id)throw Error('输入已变化，请重新预览');
      this.preview={request:{...request,profile_stamp:info.profile_stamp,scope_epoch:info.scope_epoch},...info};
    }catch(e){this.error=String(e);throw e;}finally{this.preparing=false;this.notify();}
  }
  async send(){
    const p=this.preview;if(!p||p.request.generation!==this.study.generation)throw Error('请先预览当前使用范围');
    if([...this.pending.values()].some(r=>r.lane===p.request.lane))throw Error('当前教育通道正在收尾，请稍后继续');
    const request={...p.request,confirmed:true},input=JSON.parse(request.input_json);
    const task:Task={task_id:request.task_id,session_id:request.session_id,generation:request.generation,input_hash:request.input_hash,attempt_id:input.attempt_id,answer_revision:input.answer_revision,lane:request.lane,state:'queued',budget:p.budget,cancel_requested:false,output_revision:null,error:'',usage:null,request_attempts:0,raw_output:'',reported:false};
    if(this.study.tasks.some(t=>t.task_id===request.task_id))return;
    if(this.study.tasks.length>=studyLimits.tasks)throw Error('本份复盘任务数已达上限，请保存草稿后新建练习；当前输入保留');
    if(request.lane==='background'&&this.assistActive){this.error='实时辅助活跃，后台评估等待；结束后重新确认发送';this.notify();return;}
    this.study.tasks.push(task);this.pending.set(request.task_id,request);this.preview=null;
    this.study.consent_snapshot={task_id:request.task_id,input_hash:request.input_hash,profile_id:request.profile_id};this.changed();
    try{
      await this.persist();await this.cancelled;
      if(this.study.id!==request.session_id||this.study.generation!==request.generation)return;
      task.state='running';this.changed();await this.persist();
      if(this.study.generation!==request.generation)return;
      const result=await this.bridge.invoke<Result>('education_generate',{request});
      if(this.study.id===request.session_id){applyResult(this.study,request,result);this.changed();await this.persist();}
    }catch(e){if(this.study.id===request.session_id&&['queued','running'].includes(task.state)){task.state='failed';task.error=String(e);this.changed();void this.persist().catch(()=>{});}throw e;}
    finally{this.pending.delete(request.task_id);this.notify();}
  }
  async protect(){await this.cancel();await this.persist();}
  dispose(){if(this.timer)clearTimeout(this.timer);invalidate(this.study);this.cancelNative();this.serial++;this.dirty=true;this.listeners.clear();void this.persist().catch(()=>{});}
}
