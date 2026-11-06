/// <reference lib="webworker" />
import {readPcmWav} from "./pcm-wav";
import {verifiedVisemeModel} from "./viseme-analysis";
import {OriginalVoiceStream,type VoiceToken} from "./original-voice-stream";

type Start={type:"start";token:VoiceToken;buffer:ArrayBuffer};
type Ack={type:"ack";token:VoiceToken;sequence:number};
type Ping={type:"ping";token:VoiceToken;sent:number};
type Begin={type:"begin";token:VoiceToken};
declare const self:DedicatedWorkerGlobalScope;
const now=()=>performance.now();
const wait=(ms:number)=>new Promise<void>(resolve=>setTimeout(resolve,ms));
let active:VoiceToken|undefined;
const inFlight=new Set<number>();
let begin:undefined|(()=>void);
self.onmessage=({data}:MessageEvent<Start|Ack|Ping|Begin>)=> {
  if(data.type==="start"){if(!active){active=data.token;void run(data);}return;}
  if(active?.session!==data.token.session||active.generation!==data.token.generation)return;
  if(data.type==="ack")inFlight.delete(data.sequence);
  else if(data.type==="ping")self.postMessage({type:"pong",token:active,sent:data.sent,workerMs:now()});
  else if(data.type==="begin"){begin?.();begin=undefined;}
};
async function run({token,buffer}:Start){
  let stream:OriginalVoiceStream|undefined;
  let maxInFlight=0;
  try {
    const clip=readPcmWav(buffer),pcm=Int16Array.from(clip.samples,v=>Math.round(v*32768));
    const response=await fetch("/fixtures/viseme-model.bin",{credentials:"omit",cache:"force-cache"});
    if(!response.ok)throw new Error("无法读取本地口型模型。");
    const model=await verifiedVisemeModel(await response.arrayBuffer());
    await new Promise<void>(resolve=>{begin=resolve;self.postMessage({type:"loaded",token});});
    const epochMs=now();
    stream=new OriginalVoiceStream(model,clip.sampleRate,token,epochMs);
    self.postMessage({type:"ready",token,epochMs,sampleRate:clip.sampleRate,sampleCount:pcm.length});
    const block=Math.round(clip.sampleRate*.02);let received=0,delivered=0,sequence=0;
    while(stream.status!=="ended") {
      const nextInput=received<pcm.length?epochMs+Math.min(received+block,pcm.length)/clip.sampleRate*1000:Infinity;
      const nextOutput=epochMs+stream.delayMs+delivered/clip.sampleRate*1000;
      const due=Math.min(nextInput,inFlight.size<2?nextOutput:nextOutput+stream.maxLateMs+1);
      while(now()<due)await wait(Math.max(1,Math.ceil(due-now())));
      let time=now();
      if(received<pcm.length&&time>=nextInput){
        const part=pcm.subarray(received,Math.min(received+block,pcm.length));
        stream.push({token,startSample:received,pcm:part},time);received+=part.length;
        if(received===pcm.length)stream.finish(token,now());
      }
      time=now();
      if(inFlight.size>=2){if(time-nextOutput>stream.maxLateMs)throw new Error("预览窗口未及时接收数据，推流已停止。");continue;}
      const packet=stream.pull(time);
      if(packet){
        delivered+=packet.pcm.length;inFlight.add(++sequence);maxInFlight=Math.max(maxInFlight,inFlight.size);
        self.postMessage({type:"frame",token,sequence,packet,snapshot:stream.snapshot,maxInFlight},[packet.pcm.buffer as ArrayBuffer]);
      }
      if(time-epochMs>pcm.length/clip.sampleRate*1000+2000)throw new Error("推流超过预期时长，已停止。");
    }
  }catch(error){self.postMessage({type:"error",token,error:error instanceof Error?error.message:String(error),snapshot:stream?.snapshot,maxInFlight});}
}
