import {test} from "node:test";
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {resolve} from "node:path";
import {execFileSync} from "node:child_process";
import {readPcmWav} from "../src/pcm-wav";
import {SilentSampleClock} from "../src/silent-clock";
import {verifiedVisemeModel,analyzePcm,clipFrame} from "../src/viseme-analysis";
import {VisemeStream} from "../src/vendor/visemeModel";

function wav(samples:number[],rate=16000,junk=false) {
  const extra=junk?12:0, buffer=new ArrayBuffer(44+samples.length*2+extra), view=new DataView(buffer);
  const tag=(at:number,text:string)=>[...text].forEach((c,i)=>view.setUint8(at+i,c.charCodeAt(0)));
  tag(0,"RIFF");view.setUint32(4,buffer.byteLength-8,true);tag(8,"WAVE");
  tag(12,"fmt ");view.setUint32(16,16,true);view.setUint16(20,1,true);view.setUint16(22,1,true);
  view.setUint32(24,rate,true);view.setUint32(28,rate*2,true);view.setUint16(32,2,true);view.setUint16(34,16,true);
  if(junk){tag(36,"JUNK");view.setUint32(40,3,true);}
  tag(36+extra,"data");view.setUint32(40+extra,samples.length*2,true);
  samples.forEach((value,i)=>view.setInt16(44+extra+i*2,value,true));return buffer;
}
test("WAV preserves PCM values/rate and skips odd padded metadata",()=> {
  const clip=readPcmWav(wav([-32768,0,32767],24000,true));
  assert.equal(clip.sampleRate,24000);assert.deepEqual([...clip.samples],[-1,0,32767/32768]);
});
test("WAV rejects inconsistent lengths, truncated chunks and non-PCM mono",()=> {
  const a=wav([1,2]);new DataView(a).setUint32(4,a.byteLength,true);assert.throws(()=>readPcmWav(a));
  const b=wav([1,2]);new DataView(b).setUint32(40,9999,true);assert.throws(()=>readPcmWav(b));
  for(const [at,value] of [[20,3],[22,2],[34,8],[32,4]]) {const c=wav([1,2]);new DataView(c).setUint16(at,value,true);assert.throws(()=>readPcmWav(c));}
});
test("WAV rejects durations above 60 seconds and missing sample data",()=> {
  assert.throws(()=>readPcmWav(wav(Array(480001).fill(0),8000)),/60/);
  assert.throws(()=>readPcmWav(wav([])));
});
test("silent sample clock uses elapsed time without accumulating frame jitter",()=> {
  const clock=new SilentSampleClock(160000,16000);clock.resume(100);
  for(const now of [100,106,117,360,987,1100]) assert.equal(clock.position(now),Math.floor((now-100)*16));
  clock.pause(1125);assert.equal(clock.position(9125),16400);clock.resume(9125);assert.equal(clock.position(10125),32400);
});
test("clock seek/end/invalid values cannot restart stale movement",()=> {
  const clock=new SilentSampleClock(16000,16000);clock.resume(0);assert.equal(clock.position(2000),16000);assert.equal(clock.running,false);
  assert.throws(()=>clock.seek(NaN,2001));assert.throws(()=>clock.position(100));assert.equal(clock.position(2001),16000);
  clock.seek(8000,2002);assert.equal(clock.position(2003),8000);clock.resume(2004);assert.equal(clock.position(2254),12000);
});

const root=process.env.TINGXIAN_AVATAR_SOURCE;
if(!root) throw new Error("Set TINGXIAN_AVATAR_SOURCE for pinned model regressions.");
assert.equal(execFileSync("git",["-C",root,"rev-parse","HEAD"],{encoding:"utf8"}).trim(),"d3e4dcf6609d733097466eb17b3fa63fc720140a");
assert.equal(execFileSync("git",["-C",root,"status","--porcelain","--","model","test/fixtures"],{encoding:"utf8"}).trim(),"");
const bytes=(name:string)=>{const b=readFileSync(resolve(root!,name));return b.buffer.slice(b.byteOffset,b.byteOffset+b.byteLength);};
const model=await verifiedVisemeModel(bytes("model/viseme-model.bin"));
const parity=new Float32Array(bytes("test/fixtures/parity-audio.f32"));
test("ported inference reproduces every upstream PyTorch golden frame",()=> {
  const observed:number[]=[];const stream=new VisemeStream(model);
  for(let at=0;at<parity.length;at+=997) stream.push(parity.subarray(at,at+997),id=>observed.push(id));
  assert.deepEqual(observed,[...new Uint8Array(bytes("test/fixtures/parity-visemes.u8"))]);
});
test("model inference is independent of PCM packet boundaries and ends closed",()=> {
  const a=analyzePcm(model,{samples:parity,sampleRate:16000},320);
  const b=analyzePcm(model,{samples:parity,sampleRate:16000},997);
  assert.deepEqual(a.events,b.events);assert.ok(a.events.some(e=>e.id!==0));
  assert.deepEqual(clipFrame(a,a.sampleCount),{visemes:{},mode:"listening"});
  assert.ok(a.events.every((e,i)=>e.ms>=0&&e.ms<=a.durationMs&&(!i||e.ms>a.events[i-1].ms)));
});

test("44.1 kHz resampling keeps the same timeline across uneven packet boundaries",()=> {
  const samples=Float32Array.from({length:44100},(_,i)=>parity[Math.floor(i*16000/44100)]);
  const a=analyzePcm(model,{samples,sampleRate:44100},882);
  const b=analyzePcm(model,{samples,sampleRate:44100},997);
  assert.deepEqual(a.events,b.events);assert.ok(a.events.some(e=>e.id!==0));
  assert.equal(a.durationMs,1000);assert.equal(a.events.at(-1)!.ms,1000);
});
test("silence stays closed and corrupt weights fail before parsing",async()=> {
  const silent=analyzePcm(model,{samples:new Float32Array(16000),sampleRate:16000});
  assert.ok(silent.events.every(e=>e.id===0));assert.deepEqual(clipFrame(silent,8000).visemes,{});
  const damaged=bytes("model/viseme-model.bin");new Uint8Array(damaged)[9000]^=1;
  await assert.rejects(()=>verifiedVisemeModel(damaged),/校验失败/);
});
