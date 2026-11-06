import { VisemeDetector, type VisemeEvent } from "./vendor/visemeDetector";
import type { VisemeModel } from "./vendor/visemeModel";
import { streamMouthAt, MOUTH_FUTURE_MS, BORROW_MS, RELEASE_MS } from "./stream-mouth";

export type VoiceToken = Readonly<{session:string;generation:number}>;
export type VoiceInput = {token:VoiceToken;startSample:number;pcm:Int16Array};
export type VoiceOutput = {
  token:VoiceToken; startSample:number; sampleRate:number; pcm:Int16Array;
  mediaMs:number; scheduledMs:number; deliveredMs:number;
  visemes:Record<string,number>; end:boolean;
};
export type StreamStatus="priming"|"running"|"draining"|"ended"|"cancelled"|"failed";

/** One owner, one capture epoch, one stream. No device, worker, transport or playback.
 * pull() hands exact PCM + matching mouth PTS to a sink; this is NOT an output ack.
 * Caller must stop feeding on failure and explicitly create a new generation to restart. */
export class OriginalVoiceStream {
  readonly delayMs=320;
  readonly maxBufferedMs=400;
  readonly maxLateMs=40;
  readonly token:VoiceToken;
  private readonly detector:VisemeDetector;
  private readonly ring:Int16Array;
  private readonly blockSamples:number;
  private events:VisemeEvent[]=[{id:0,ms:0}];
  private received=0;
  private delivered=0;
  private endedInput=false;
  private lastNow:number;
  private state:StreamStatus="priming";
  private failure:string|undefined;
  private maxPending=0;
  private maxEvents=1;
  private inferredMs=0;
  private discarded=0;

  constructor(model:VisemeModel,readonly sampleRate:number,token:VoiceToken,readonly captureEpochMs:number) {
    if(!Number.isInteger(sampleRate)||sampleRate<8000||sampleRate>48000||!Number.isFinite(captureEpochMs)||!token.session.trim()||!Number.isSafeInteger(token.generation)||token.generation<0) throw new Error("Invalid voice stream configuration");
    this.token=Object.freeze({...token});this.lastNow=captureEpochMs;
    this.ring=new Int16Array(Math.ceil(sampleRate*this.maxBufferedMs/1000));
    this.blockSamples=Math.round(sampleRate*.02);
    this.detector=new VisemeDetector(model,sampleRate,0,event=> {
      if(event.ms>this.events.at(-1)!.ms && event.ms<this.received/sampleRate*1000) this.events.push(event);
    });
  }
  get status(){return this.state;}
  get snapshot(){return {status:this.state,failure:this.failure,receivedSamples:this.received,deliveredSamples:this.delivered,
    pendingSamples:this.received-this.delivered-this.discarded,discardedSamples:this.discarded,capacitySamples:this.ring.length,maxPendingSamples:this.maxPending,
    retainedEvents:this.events.length,maxRetainedEvents:this.maxEvents,inferenceMs:this.inferredMs,
    stableBeforeMs:this.detector.stableBeforeMs,delayMs:this.delayMs,maxBufferedMs:this.maxBufferedMs,maxLateMs:this.maxLateMs,
    mouthFutureMs:MOUTH_FUTURE_MS,deviceOutputAcknowledged:false};}
  private matches(token:VoiceToken){return token.session===this.token.session&&token.generation===this.token.generation;}
  private checkTime(now:number){if(!Number.isFinite(now)||now<this.lastNow)throw new Error("Voice clock must be monotonic");this.lastNow=now;}
  private open(){if(["ended","cancelled","failed"].includes(this.state))throw new Error(`Voice stream is ${this.state}`);}
  private fail(message:string):never {this.state="failed";this.failure=message;throw new Error(message);}

  push(packet:VoiceInput,now:number):boolean {
    if(!this.matches(packet.token))return false;
    this.open();if(this.endedInput)throw new Error("Voice input is finished");
    if(!(packet.pcm instanceof Int16Array)||!packet.pcm.length||packet.pcm.length>Math.ceil(this.sampleRate*.04)||packet.startSample!==this.received)throw new Error("Expected contiguous PCM16 packets of at most 40ms");
    if(!Number.isFinite(now)||now<this.lastNow)throw new Error("Voice clock must be monotonic");
    if((this.received+packet.pcm.length)/this.sampleRate*1000>now-this.captureEpochMs+.001)throw new Error("PCM has arrived before its capture time");
    this.checkTime(now);
    if(this.received-this.delivered+packet.pcm.length>this.ring.length)this.fail("Voice buffer exceeded 400ms; output stopped without dropping queued speech");
    const samples=new Float32Array(packet.pcm.length);
    for(let i=0;i<packet.pcm.length;i++) {const sample=packet.pcm[i];this.ring[(this.received+i)%this.ring.length]=sample;samples[i]=sample/32768;}
    this.received+=packet.pcm.length;
    const started=performance.now();
    try {this.detector.push(samples);}catch(error){this.fail(`Viseme inference failed: ${String(error)}`);}
    this.inferredMs+=performance.now()-started;
    this.maxPending=Math.max(this.maxPending,this.received-this.delivered);this.maxEvents=Math.max(this.maxEvents,this.events.length);
    return true;
  }
  finish(token:VoiceToken,now:number):boolean {
    if(!this.matches(token))return false;
    this.open();if(this.endedInput)throw new Error("Voice input is finished");this.checkTime(now);
    this.endedInput=true;
    const started=performance.now();
    try {this.detector.flush();}catch(error){this.fail(`Viseme flush failed: ${String(error)}`);}
    this.inferredMs+=performance.now()-started;
    const duration=this.received/this.sampleRate*1000;
    if(duration>this.events.at(-1)!.ms)this.events.push({id:0,ms:duration});
    this.maxEvents=Math.max(this.maxEvents,this.events.length);
    this.state="draining";
    return true;
  }
  pull(now:number):VoiceOutput|undefined {
    this.open();this.checkTime(now);
    const mediaMs=this.delivered/this.sampleRate*1000,scheduledMs=this.captureEpochMs+this.delayMs+mediaMs;
    if(now<scheduledMs)return;
    const available=this.received-this.delivered;
    if(now-scheduledMs>this.maxLateMs)this.fail("Voice delivery deadline missed; output stopped instead of bursting stale speech");
    if(this.endedInput&&!available){
      this.state="ended";
      return {token:this.token,startSample:this.delivered,sampleRate:this.sampleRate,pcm:new Int16Array(),mediaMs,scheduledMs,deliveredMs:now,visemes:{},end:true};
    }
    const count=this.endedInput?Math.min(this.blockSamples,available):this.blockSamples;
    if(available<count)this.fail("Voice input underflow; output stopped instead of inventing silence");
    if(!this.endedInput&&this.detector.stableBeforeMs<=mediaMs+MOUTH_FUTURE_MS)this.fail("Viseme future window unavailable at output deadline");
    const pcm=new Int16Array(count),visemes=streamMouthAt(this.events,mediaMs);
    for(let i=0;i<count;i++){const at=(this.delivered+i)%this.ring.length;pcm[i]=this.ring[at];this.ring[at]=0;}
    const output:VoiceOutput={token:this.token,startSample:this.delivered,sampleRate:this.sampleRate,pcm,mediaMs,scheduledMs,deliveredMs:now,visemes,end:false};
    this.delivered+=count;if(!this.endedInput)this.state="running";
    // Keep the active event plus its bounded backwards borrowing context. A long
    // held vowel remains one event; old transitions do not accumulate over a meeting.
    let anchor=0;
    while(anchor+1<this.events.length&&this.events[anchor+1].ms<=mediaMs-RELEASE_MS)anchor++;
    const cutoff=this.events[anchor].ms-BORROW_MS;
    let drop=0;while(drop<anchor&&this.events[drop].ms<cutoff)drop++;
    if(drop)this.events.splice(0,drop);
    return output;
  }
  cancel(token:VoiceToken):boolean {
    if(!this.matches(token))return false;
    this.state="cancelled";this.ring.fill(0);this.events=[];this.discarded=this.received-this.delivered;
    return true;
  }
}
