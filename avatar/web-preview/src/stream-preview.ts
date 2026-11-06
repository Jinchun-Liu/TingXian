import {readPcmWav} from "./pcm-wav";
import {VisualStreamGate} from "./visual-stream-gate";
import type {VoiceToken,VoiceOutput} from "./original-voice-stream";
import {calibrateWorkerClock,type ClockProbe} from "./worker-clock";

const now=()=>performance.now();
type Message={type:"loaded"|"pong"|"ready"|"frame"|"error";token:VoiceToken;sent:number;workerMs:number;epochMs:number;sampleRate:number;sampleCount:number;packet:VoiceOutput;sequence:number;snapshot:unknown;maxInFlight:number;error:string};
export class StreamPreview {
  private worker?:Worker;
  private gate?:VisualStreamGate;
  private token?:VoiceToken;
  private generation=0;
  private watchdog?:ReturnType<typeof setInterval>;
  private report:Record<string,unknown>={};
  private running=false;
  private ready=false;
  private startedAt=0;
  private position=0;
  get active(){return this.running;}
  get sample(){return this.position;}
  get diagnostics(){return {...this.report,...this.gate?.snapshot};}
  constructor(private readonly update:(status:string)=>void,private readonly redraw:()=>void){}
  start(buffer:ArrayBuffer){
    this.stop();
    try{this.startWorker(buffer);}
    catch(reason){this.fail(reason instanceof Error?reason.message:String(reason));}
  }
  private startWorker(buffer:ArrayBuffer){
    const clip=readPcmWav(buffer),expected=Int16Array.from(clip.samples,v=>Math.round(v*32768));
    const token={session:crypto.randomUUID(),generation:++this.generation};this.token=token;
    this.running=true;this.ready=false;this.startedAt=now();this.position=0;this.gate=undefined;
    this.report={scope:"silent_stream_render_request",status:"loading",audioPlayback:false,capture:false,virtualOutput:false};
    const worker=new Worker(new URL("./stream-preview.worker.ts",import.meta.url),{type:"module"});this.worker=worker;
    const probes:ClockProbe[]=[];let clock:ReturnType<typeof calibrateWorkerClock>|undefined;
    const ping=()=>worker.postMessage({type:"ping",token,sent:now()});
    worker.onmessage=({data}:MessageEvent<Message>)=> {
      if(!this.running||this.token!==token||data.token.session!==token.session||data.token.generation!==token.generation)return;
      try {
        if(data.type==="loaded")ping();
        else if(data.type==="pong"){
          probes.push({sent:data.sent,worker:data.workerMs,received:now()});
          if(probes.length<3)ping();else{clock=calibrateWorkerClock(probes);this.report.clockCalibration=clock;worker.postMessage({type:"begin",token});}
        }else if(data.type==="ready"){
          if(!clock||data.sampleRate!==clip.sampleRate||data.sampleCount!==expected.length)throw new Error("推流格式或时钟不一致，已停止。");
          this.gate=new VisualStreamGate(token,expected,clip.sampleRate,data.epochMs+clock.offsetMs);this.ready=true;
          this.report.status="running";this.update("逐段推流中 · 无声");
        }else if(data.type==="frame"){
          this.report={...this.report,worker:data.snapshot,maxInFlight:data.maxInFlight};
          if(!this.gate||!clock)return;
          const mapped={...data.packet,scheduledMs:data.packet.scheduledMs+clock.offsetMs,deliveredMs:data.packet.deliveredMs+clock.offsetMs};
          if(!this.gate.accept(mapped,now()))return;
          worker.postMessage({type:"ack",token,sequence:data.sequence});
          this.position=data.packet.startSample+data.packet.pcm.length;
          if(data.packet.end){this.running=false;clearInterval(this.watchdog);worker.terminate();this.worker=undefined;this.report.status="ended";this.update("推流已结束");this.redraw();}
        }else if(data.type==="error"){this.report={...this.report,worker:data.snapshot,maxInFlight:data.maxInFlight};throw new Error(data.error);}
      }catch(reason){this.fail(reason instanceof Error?reason.message:String(reason));}
    };
    worker.onerror=()=>this.fail("推流线程异常，已停止。");
    this.watchdog=setInterval(()=>{
      if(!this.running)return;
      try{if(!this.ready&&now()-this.startedAt>10000)throw new Error("推流准备超时，请重试。");if(this.ready)this.gate?.checkFresh(now());}
      catch(reason){this.fail(reason instanceof Error?reason.message:String(reason));}
    },50);
    const transfer=buffer.slice(0);worker.postMessage({type:"start",token,buffer:transfer},[transfer]);
    this.update("正在准备推流…");
  }
  frame(){
    if(!this.running)return {visemes:{},mode:"listening" as const};
    try{return this.gate?.frame(now())??{visemes:{},mode:"listening" as const};}
    catch(reason){this.fail(reason instanceof Error?reason.message:String(reason));return {visemes:{},mode:"listening" as const};}
  }
  stop(message="推流已停止"){
    const wasActive=this.running;this.running=false;this.token=undefined;clearInterval(this.watchdog);this.worker?.terminate();this.worker=undefined;
    this.gate?.stop();if(wasActive){this.report.status="stopped";this.update(message);}
  }
  private fail(message:string){this.stop();this.report={...this.report,status:"failed",error:message};this.update(message);this.redraw();}
}
