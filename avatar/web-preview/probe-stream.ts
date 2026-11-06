import {readFileSync,writeFileSync,existsSync} from "node:fs";
import {resolve} from "node:path";
import {createHash} from "node:crypto";
import {setTimeout as wait} from "node:timers/promises";
import {readPcmWav} from "./src/pcm-wav";
import {verifiedVisemeModel,VISEME_MODEL_SHA256} from "./src/viseme-analysis";
import {OriginalVoiceStream} from "./src/original-voice-stream";

const [input,output]=process.argv.slice(2),source=process.env.TINGXIAN_AVATAR_SOURCE;
if(!input||!output||!source)throw new Error("Usage: TINGXIAN_AVATAR_SOURCE=... tsx probe-stream.ts input.wav new-report.json");
if(existsSync(output))throw new Error("Preserve existing evidence; choose a new report path");
const bytes=(path:string)=>{const b=readFileSync(path);return b.buffer.slice(b.byteOffset,b.byteOffset+b.byteLength);};
const model=await verifiedVisemeModel(bytes(resolve(source,"model/viseme-model.bin")));
const clip=readPcmWav(bytes(input));
const pcm=Int16Array.from(clip.samples,v=>Math.round(v*32768));
const leBytes=(pcm:Int16Array)=>{const buffer=Buffer.alloc(pcm.length*2);pcm.forEach((value,i)=>buffer.writeInt16LE(value,i*2));return buffer;};
const inputHash=createHash("sha256").update(leBytes(pcm)).digest("hex"),sinkHash=createHash("sha256");
const token={session:"synthetic-file-probe",generation:1},epoch=performance.now();
const stream=new OriginalVoiceStream(model,clip.sampleRate,token,epoch);
const block=Math.round(clip.sampleRate*.02),rows:object[]=[],delays:number[]=[],late:number[]=[];
let received=0,delivered=0,ended=false,error:string|undefined;
console.log(JSON.stringify({state:"started",scope:"wall_clock_paced_pcm_to_silent_sink",seconds:clip.samples.length/clip.sampleRate,delayMs:stream.delayMs}));
try {
  while(stream.status!=="ended") {
    const nextInput=received<pcm.length?epoch+Math.min(received+block,pcm.length)/clip.sampleRate*1000:Infinity;
    const nextOutput=epoch+stream.delayMs+delivered/clip.sampleRate*1000;
    const due=Math.min(nextInput,nextOutput);
    while(performance.now()<due)await wait(Math.max(1,Math.ceil(due-performance.now())));
    let now=performance.now();
    if(received<pcm.length&&now>=nextInput) {
      const part=pcm.subarray(received,Math.min(received+block,pcm.length));
      stream.push({token,startSample:received,pcm:part},now);received+=part.length;
      if(received===pcm.length)stream.finish(token,performance.now());
    }
    now=performance.now();
    const packet=stream.pull(now);
    if(packet) {
      if(packet.startSample!==delivered)throw new Error("Sink received non-contiguous samples");
      sinkHash.update(leBytes(packet.pcm));delivered+=packet.pcm.length;ended=packet.end;
      const deliveryDelay=packet.deliveredMs-epoch-packet.mediaMs,lateness=packet.deliveredMs-packet.scheduledMs;
      delays.push(deliveryDelay);late.push(lateness);
      rows.push({startSample:packet.startSample,count:packet.pcm.length,mediaMs:packet.mediaMs,deliveryMs:packet.deliveredMs-epoch,delayMs:deliveryDelay,latenessMs:lateness,visemes:packet.visemes,end:packet.end,pendingSamples:stream.snapshot.pendingSamples});
    }
    if(performance.now()-epoch>clip.samples.length/clip.sampleRate*1000+2000)throw new Error("Probe deadline exceeded");
  }
}catch(reason){error=reason instanceof Error?reason.message:String(reason);}
const outputHash=sinkHash.digest("hex"),p95=(xs:number[])=>xs.length?[...xs].sort((a,b)=>a-b)[Math.min(xs.length-1,Math.floor(xs.length*.95))]:null;
const passed=!error&&ended&&delivered===pcm.length&&outputHash===inputHash;
const report={scope:"wall_clock_paced_pcm_to_silent_sink",passed,error,modelSha256:VISEME_MODEL_SHA256,sampleRate:clip.sampleRate,sampleCount:pcm.length,
  durationMs:pcm.length/clip.sampleRate*1000,wallMs:performance.now()-epoch,inputPcmSha256:inputHash,outputPcmSha256:outputHash,
  delayP95Ms:p95(delays),latenessP95Ms:p95(late),snapshot:stream.snapshot,rows,capture:false,audioPlayback:false,videoRendered:false,virtualOutput:false,
  limitations:["file_input_clock_not_microphone","sink_delivery_not_audio_device_or_remote_ack","no_parallel_asr_or_render_load","no_lip_accuracy_or_av_skew_measurement"]};
writeFileSync(output,JSON.stringify(report,null,2),{flag:"wx"});
console.log(JSON.stringify({passed,error,samples:delivered,wallMs:report.wallMs,delayP95Ms:report.delayP95Ms,latenessP95Ms:report.latenessP95Ms,maxPendingMs:stream.snapshot.maxPendingSamples/clip.sampleRate*1000,maxEvents:stream.snapshot.maxRetainedEvents}));
if(!passed)process.exitCode=1;
