import {invoke} from '@tauri-apps/api/core';
import {indexedDBService, InterviewRecoverySnapshot} from '@/services/indexedDBService';

export type ExitRequest={request_id:number;phase:string;restart:boolean};
export type ExitView={requestId:number|null;busy:boolean;phase:string;error:string};
const IDLE:ExitView={requestId:null,busy:false,phase:'',error:''};
type Bridge={
  invoke:<T>(command:string,args?:Record<string,unknown>)=>Promise<T>;
  cache:Pick<typeof indexedDBService,'recordingWritesSuspended'|'suspendRecordingWrites'|'resumeRecordingWrites'|'snapshotInterviewHistory'|'removeArchivedInterviewHistory'>;
};

/** One controller across routes/React remounts. Repeated quit notifications
 * cannot run parallel cleanup or turn a failure into silent retries. */
export class InterviewExit {
  private view=IDLE;
  private listeners=new Set<()=>void>();
  private latestRequest=0;
  private checkpoints=new Set<()=>Promise<void>>();
  registerCheckpoint=(flush:()=>Promise<void>)=>{this.checkpoints.add(flush);return()=>{this.checkpoints.delete(flush);};};
  private originalSuspended=false;
  constructor(private bridge:Bridge){}
  getSnapshot=()=>this.view;
  getServerSnapshot=()=>IDLE;
  subscribe=(listener:()=>void)=>{this.listeners.add(listener);return()=>{this.listeners.delete(listener);};};
  pending=()=>this.view.requestId!==null;
  generation=()=>this.latestRequest;
  private update(view:ExitView){this.view=view;for(const listener of this.listeners)listener();}
  async request(request:ExitRequest):Promise<void> {
    if(request.request_id<=this.latestRequest)return;
    this.latestRequest=request.request_id;
    if(!this.pending())this.originalSuspended=this.bridge.cache.recordingWritesSuspended();
    if(request.phase==='failed'){
      this.bridge.cache.suspendRecordingWrites();
      this.update({requestId:request.request_id,busy:false,phase:'退出未完成',error:'上次退出清理未完成，可以重试或取消退出。'});
      return;
    }
    this.update({requestId:request.request_id,busy:false,phase:'准备退出',error:''});
    await this.retry();
  }
  async retry():Promise<void> {
    const id=this.view.requestId;
    if(id===null || this.view.busy)return;
    this.update({requestId:id,busy:true,phase:'正在停止采集和AI请求',error:''});
    this.bridge.cache.suspendRecordingWrites();
    let archivePath:string|null=null;
    try {
      await this.bridge.invoke('interview_exit_prepare',{requestId:id});
      if(this.view.requestId!==id)return;
      this.update({...this.view,phase:'正在保存恢复草稿'});
      for(const flush of this.checkpoints)await flush();
      if(this.view.requestId!==id)return;
      const recovery:InterviewRecoverySnapshot=await this.bridge.cache.snapshotInterviewHistory();
      if(this.view.requestId!==id)return;
      this.update({...this.view,phase:'正在将临时记录移入废纸篓'});
      archivePath=await this.bridge.invoke<string|null>('interview_exit_archive',{requestId:id,recovery});
      if(this.view.requestId!==id)return;
      await this.bridge.cache.removeArchivedInterviewHistory(recovery);
      if(this.view.requestId!==id)return;
      this.update({...this.view,phase:'临时记录已清理，正在退出'});
      await this.bridge.invoke('interview_exit_finish',{requestId:id});
    }catch(error){
      if(this.view.requestId!==id)return;
      this.update({requestId:id,busy:false,phase:'退出未完成',error:`${String(error)}${archivePath?`；完整备份保留在废纸篓：${archivePath}`:''}`});
    }
  }
  async cancel():Promise<void> {
    const id=this.view.requestId;if(id===null || this.view.busy)return;
    this.update({...this.view,busy:true});
    try {
      await this.bridge.invoke('interview_exit_cancel',{requestId:id});
      if(this.view.requestId!==id)return;
      if(!this.originalSuspended)this.bridge.cache.resumeRecordingWrites();
      this.update(IDLE);
    }catch(error){if(this.view.requestId===id)this.update({...this.view,busy:false,error:String(error)});}
  }
}
export const interviewExit=new InterviewExit({invoke,cache:indexedDBService});
