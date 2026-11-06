import {test} from "node:test";
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {resolve} from "node:path";
import {verifiedVisemeModel,analyzePcm} from "../src/viseme-analysis";
import {OriginalVoiceStream,type VoiceOutput} from "../src/original-voice-stream";
import {streamMouthAt,MOUTH_FUTURE_MS} from "../src/stream-mouth";
import {VisemeDetector,type VisemeEvent} from "../src/vendor/visemeDetector";

const root=process.env.TINGXIAN_AVATAR_SOURCE;
if(!root)throw new Error("Set TINGXIAN_AVATAR_SOURCE for pinned model regressions");
const bytes=(name:string)=>{const b=readFileSync(resolve(root!,name));return b.buffer.slice(b.byteOffset,b.byteOffset+b.byteLength);};
const model=await verifiedVisemeModel(bytes("model/viseme-model.bin"));
const raw=new Float32Array(bytes("test/fixtures/parity-audio.f32"));
const pcm=Int16Array.from(raw,v=>Math.max(-32768,Math.min(32767,Math.round(v*32768))));
const samples=Float32Array.from(pcm,v=>v/32768);
const token={session:"synthetic",generation:1};

test("detector watermark forbids future events from rewriting settled media time",()=> {
  let settled=0,count=0;
  const detector=new VisemeDetector(model,16000,0,e=>{assert.ok(e.ms>=settled,`${e.ms} < ${settled}`);count++;});
  for(let at=0;at<samples.length;at+=277){detector.push(samples.subarray(at,at+277));assert.ok(detector.stableBeforeMs>=settled);settled=detector.stableBeforeMs;}
  detector.flush();assert.ok(count>10);
});

test("mouth output uses only a bounded future, including long vowels and tongue borrowing",()=> {
  const events:VisemeEvent[]=[{id:0,ms:0},{id:1,ms:20},{id:14,ms:80},{id:7,ms:170},{id:21,ms:310},{id:6,ms:350},{id:0,ms:3000}];
  for(let t=0;t<=3100;t+=5){
    const prefix=events.filter(e=>e.ms<=t+MOUTH_FUTURE_MS);
    assert.deepEqual(streamMouthAt(prefix,t),streamMouthAt(events,t));
    assert.ok(Object.values(streamMouthAt(events,t)).every(v=>Number.isFinite(v)&&v>=0&&v<=1));
  }
  assert.deepEqual(streamMouthAt(events,3100),{});
});

test("paced stream preserves every PCM sample and matches bounded full-data mouths",()=> {
  const stream=new OriginalVoiceStream(model,16000,token,0),outputs:VoiceOutput[]=[];
  let at=0;
  // Non-aligned last packet tests actual EOF, not an exact multiple of 20ms.
  const input=pcm.subarray(0,pcm.length-73);
  for(let now=20;stream.status!=="ended";now+=20){
    assert.ok(now<5000,"stream failed to terminate");
    if(at<input.length){const part=input.subarray(at,Math.min(at+320,input.length));stream.push({token,startSample:at,pcm:part},now);at+=part.length;if(at===input.length)stream.finish(token,now);}
    for(;;){const out=stream.pull(now);if(!out)break;outputs.push(out);if(out.end)break;}
    assert.ok(stream.snapshot.pendingSamples<=stream.snapshot.capacitySamples);
  }
  const joined=outputs.flatMap(o=>[...o.pcm]);assert.deepEqual(joined,[...input]);
  let position=0;
  const truncated=analyzePcm(model,{samples:samples.subarray(0,input.length),sampleRate:16000});
  for(const out of outputs){
    assert.equal(out.startSample,position);position+=out.pcm.length;
    assert.equal(out.scheduledMs-out.mediaMs,320);assert.ok(out.deliveredMs-out.scheduledMs<=40);
    assert.deepEqual(out.visemes,out.end?{}:streamMouthAt(truncated.events,out.mediaMs));
  }
  assert.ok(truncated.events.length>10);assert.equal(outputs.at(-1)!.end,true);
  assert.ok(stream.snapshot.maxRetainedEvents<40);assert.equal(stream.snapshot.deviceOutputAcknowledged,false);
});

test("44.1 kHz voice remains unchanged while inference uses its own resampled timeline",()=> {
  const rate=44100,input=Int16Array.from({length:44113},(_,i)=>pcm[Math.floor(i*16000/rate)]);
  const normalized=Float32Array.from(input,v=>v/32768),full=analyzePcm(model,{samples:normalized,sampleRate:rate});
  const stream=new OriginalVoiceStream(model,rate,token,0),joined:number[]=[];
  let at=0;
  for(let now=20;stream.status!=="ended";now+=20){
    assert.ok(now<2000);
    if(at<input.length){const part=input.subarray(at,Math.min(at+882,input.length));stream.push({token,startSample:at,pcm:part},now);at+=part.length;if(at===input.length)stream.finish(token,now);}
    for(;;){const out=stream.pull(now);if(!out)break;joined.push(...out.pcm);assert.deepEqual(out.visemes,out.end?{}:streamMouthAt(full.events,out.mediaMs));if(out.end)break;}
  }
  assert.deepEqual(joined,[...input]);
});

test("overflow and missed deadlines stop explicitly while preserving undelivered counts",()=> {
  const overflow=new OriginalVoiceStream(model,16000,token,0);
  for(let i=0;i<20;i++)overflow.push({token,startSample:i*320,pcm:new Int16Array(320)},(i+1)*20);
  assert.throws(()=>overflow.push({token,startSample:6400,pcm:new Int16Array(320)},420),/buffer/);
  assert.equal(overflow.status,"failed");assert.equal(overflow.snapshot.pendingSamples,6400);assert.equal(overflow.snapshot.deliveredSamples,0);
  const late=new OriginalVoiceStream(model,16000,token,0);
  for(let i=0;i<16;i++)late.push({token,startSample:i*320,pcm:new Int16Array(320)},(i+1)*20);
  assert.throws(()=>late.pull(361),/deadline/);assert.equal(late.snapshot.deliveredSamples,0);assert.throws(()=>late.pull(362),/failed/);
  const empty=new OriginalVoiceStream(model,16000,token,0);assert.throws(()=>empty.pull(320),/underflow/);
});

test("cancel isolates generations and never counts queued samples as delivered",()=> {
  const stream=new OriginalVoiceStream(model,16000,token,0);
  const packet={token,startSample:0,pcm:new Int16Array(320).fill(42)};stream.push(packet,20);packet.pcm.fill(99);
  assert.equal(stream.cancel({...token,generation:0}),false);assert.equal(stream.snapshot.pendingSamples,320);
  stream.cancel(token);stream.cancel(token);
  assert.equal(stream.snapshot.pendingSamples,0);assert.equal(stream.snapshot.discardedSamples,320);assert.equal(stream.snapshot.deliveredSamples,0);
  assert.throws(()=>stream.pull(400),/cancelled/);assert.throws(()=>stream.push(packet,30),/cancelled/);
  const next=new OriginalVoiceStream(model,16000,{...token,generation:2},500);
  assert.equal(next.push(packet,20),false);assert.equal(next.finish(token,20),false);assert.equal(next.cancel(token),false);
  assert.equal(next.snapshot.receivedSamples,0);assert.equal(next.status,"priming");
});

test("invalid input leaves state intact and PCM ownership is copied at ingestion",()=> {
  const stream=new OriginalVoiceStream(model,16000,token,0),input=new Int16Array(320).fill(42);
  assert.throws(()=>stream.push({token,startSample:0,pcm:input},10),/capture time/);
  assert.throws(()=>stream.push({token,startSample:1,pcm:input},20),/contiguous/);
  assert.equal(stream.snapshot.receivedSamples,0);
  stream.push({token,startSample:0,pcm:input},20);input.fill(99);stream.finish(token,20);
  assert.throws(()=>stream.pull(19),/monotonic/);
  const output=stream.pull(320)!;assert.deepEqual([...output.pcm],Array(320).fill(42));output.pcm.fill(11);
  const end=stream.pull(340)!;assert.ok(end.end);assert.deepEqual(end.visemes,{});
});
