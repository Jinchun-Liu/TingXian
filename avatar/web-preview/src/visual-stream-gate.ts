import type {VoiceOutput,VoiceToken} from "./original-voice-stream";
const p95=(values:number[])=>values.length?[...values].sort((a,b)=>a-b)[Math.min(values.length-1,Math.floor(values.length*.95))]:null;

/** Silent preview sink. PCM is checked against the synthetic file, never played.
 * One latest mouth frame; stats refer to render requests, not GPU completion. */
export class VisualStreamGate {
  private nextSample=0;
  private latest?:VoiceOutput;
  private drawnSample=-1;
  private closed=false;
  private packets=0;
  private drawn=0;
  private receiptDelays:number[]=[];
  private requestDelays:number[]=[];
  private lastReceipt?:{receiptMs:number;scheduledMs:number;workerDeliveredMs:number;ipcMs:number;latenessMs:number};
  readonly maxOutputLateMs=80;
  constructor(readonly token:VoiceToken,private readonly expected:Int16Array,readonly sampleRate:number,readonly epochMs:number){
    if(!Number.isFinite(epochMs)||!Number.isInteger(sampleRate)||sampleRate<8000||sampleRate>48000||!expected.length)throw new Error("Invalid visual stream configuration");
  }
  get snapshot(){return {verifiedSamples:this.nextSample,totalSamples:this.expected.length,receivedPackets:this.packets,renderRequestedPackets:this.drawn,
    receiptDelayUpperEstimateP95Ms:p95(this.receiptDelays),renderRequestDelayUpperEstimateP95Ms:p95(this.requestDelays),ended:!!this.latest?.end,closed:this.closed,
    lastReceipt:this.lastReceipt,maxOutputLateMs:this.maxOutputLateMs,scope:"silent_stream_render_request",audioPlayback:false,capture:false,gpuCompletionMeasured:false,virtualOutput:false};}
  accept(packet:VoiceOutput,now:number):boolean {
    if(this.closed||this.latest?.end||packet.token.session!==this.token.session||packet.token.generation!==this.token.generation)return false;
    if(packet.startSample!==this.nextSample||packet.sampleRate!==this.sampleRate||!(packet.pcm instanceof Int16Array)||packet.pcm.length>Math.ceil(this.sampleRate*.02)||packet.startSample+packet.pcm.length>this.expected.length)throw new Error("推流样本位置不连续，已停止。");
    this.lastReceipt={receiptMs:now,scheduledMs:packet.scheduledMs,workerDeliveredMs:packet.deliveredMs,ipcMs:now-packet.deliveredMs,latenessMs:now-packet.scheduledMs};
    if(![now,packet.mediaMs,packet.scheduledMs,packet.deliveredMs].every(Number.isFinite)||Math.abs(packet.mediaMs-packet.startSample/this.sampleRate*1000)>.001||Math.abs(packet.scheduledMs-this.epochMs-320-packet.mediaMs)>.001||packet.deliveredMs<packet.scheduledMs||now<packet.deliveredMs||now-packet.scheduledMs>this.maxOutputLateMs)throw new Error("画面交付超时，已停止；可重新开始。");
    if(packet.end?packet.pcm.length!==0||this.nextSample!==this.expected.length||Object.keys(packet.visemes).length!==0:packet.pcm.length===0)throw new Error("推流结束标记不完整，已停止。");
    if(Object.values(packet.visemes).some(v=>!Number.isFinite(v)||v<0||v>1))throw new Error("口型参数无效，已停止。");
    for(let i=0;i<packet.pcm.length;i++)if(packet.pcm[i]!==this.expected[this.nextSample+i])throw new Error("原声样本校验失败，已停止。");
    this.nextSample+=packet.pcm.length;this.packets++;this.receiptDelays.push(now-this.epochMs-packet.mediaMs);
    // Do not retain audio in the visual frame queue.
    this.latest={...packet,pcm:new Int16Array(),visemes:{...packet.visemes}};
    return true;
  }
  checkFresh(now:number){
    if(this.closed||this.latest?.end)return;
    if(!Number.isFinite(now))throw new Error("画面时钟无效，已停止。");
    const deadline=(this.latest?.scheduledMs??this.epochMs+320)+this.maxOutputLateMs;
    if(now>deadline)throw new Error("画面更新中断，已停止；可重新开始。");
  }
  frame(now:number){
    this.checkFresh(now);
    if(this.closed||this.latest?.end)return {visemes:{},mode:"listening" as const};
    if(!this.latest)return {visemes:{},mode:"listening" as const};
    if(this.drawnSample!==this.latest.startSample){this.drawnSample=this.latest.startSample;this.drawn++;this.requestDelays.push(now-this.epochMs-this.latest.mediaMs);}
    return {visemes:this.latest.visemes,mode:Object.keys(this.latest.visemes).length?"speaking" as const:"listening" as const};
  }
  stop(){this.closed=true;this.latest=undefined;}
}
