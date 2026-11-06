import {test} from "node:test";
import assert from "node:assert/strict";
import {VisualStreamGate} from "../src/visual-stream-gate";
import type {VoiceOutput} from "../src/original-voice-stream";
import {calibrateWorkerClock} from "../src/worker-clock";
const token={session:"synthetic",generation:3},epoch=1793950000000;
const input=new Int16Array(640).fill(42);
const packet=(at=0,end=false):VoiceOutput=>({token,startSample:at,sampleRate:16000,pcm:end?new Int16Array():input.slice(at,at+320),mediaMs:at/16,scheduledMs:epoch+320+at/16,deliveredMs:epoch+321+at/16,visemes:end?{}:{aa:.5},end});

test("visual sink verifies original samples and counts actual render requests separately",()=>{
  const gate=new VisualStreamGate(token,input,16000,epoch);
  gate.accept(packet(),epoch+322);gate.checkFresh(epoch+325);
  assert.equal(gate.snapshot.renderRequestedPackets,0);
  assert.deepEqual(gate.frame(epoch+330).visemes,{aa:.5});gate.frame(epoch+331);
  assert.equal(gate.snapshot.renderRequestedPackets,1);assert.equal(gate.snapshot.renderRequestDelayUpperEstimateP95Ms,330);
  gate.accept(packet(320),epoch+342);gate.frame(epoch+350);
  gate.accept(packet(640,true),epoch+362);
  assert.deepEqual(gate.frame(epoch+1000).visemes,{});
  assert.equal(gate.snapshot.verifiedSamples,640);assert.equal(gate.snapshot.ended,true);
  assert.equal(gate.accept(packet(640,true),epoch+363),false);
});

test("stale generations, stop and missing frames cannot revive mouth movement",()=>{
  const gate=new VisualStreamGate(token,input,16000,epoch);
  assert.equal(gate.accept({...packet(),token:{...token,generation:2}},epoch+1000),false);
  assert.throws(()=>gate.checkFresh(epoch+401),/中断/);
  gate.accept(packet(),epoch+322);gate.stop();
  assert.equal(gate.accept(packet(320),epoch+342),false);
  assert.deepEqual(gate.frame(epoch+1000).visemes,{});
});

test("late, future, corrupt, skipped and duplicate PCM are rejected without advancing",()=>{
  const gate=new VisualStreamGate(token,input,16000,epoch);
  assert.throws(()=>gate.accept(packet(),epoch+401),/超时/);
  assert.throws(()=>gate.accept(packet(),epoch+319),/超时/);
  assert.throws(()=>gate.accept({...packet(),pcm:new Int16Array(320)},epoch+322),/校验/);
  assert.throws(()=>gate.accept(packet(320),epoch+342),/不连续/);
  assert.throws(()=>gate.accept({...packet(),mediaMs:1},epoch+322),/超时/);
  assert.throws(()=>gate.accept({...packet(),mediaMs:NaN},epoch+322),/超时/);
  assert.throws(()=>gate.accept({...packet(),deliveredMs:NaN},epoch+322),/超时/);
  assert.throws(()=>gate.accept({...packet(),deliveredMs:epoch+319},epoch+322),/超时/);
  assert.throws(()=>gate.accept({...packet(),deliveredMs:epoch+323},epoch+322),/超时/);
  assert.throws(()=>gate.accept({...packet(),visemes:{aa:NaN}},epoch+322),/参数/);
  assert.equal(gate.snapshot.verifiedSamples,0);
  gate.accept(packet(),epoch+322);assert.throws(()=>gate.accept(packet(),epoch+323),/不连续/);
  assert.throws(()=>gate.accept(packet(320,true),epoch+342),/不完整/);
});

test("a slow renderer uses one latest visual frame without inflating rendered count",()=>{
  const gate=new VisualStreamGate(token,input,16000,epoch);
  gate.accept(packet(),epoch+322);gate.accept(packet(320),epoch+342);
  gate.frame(epoch+350);
  assert.equal(gate.snapshot.receivedPackets,2);assert.equal(gate.snapshot.renderRequestedPackets,1);
  assert.equal(gate.snapshot.renderRequestDelayUpperEstimateP95Ms,330);
  assert.throws(()=>gate.frame(epoch+421),/中断/);
});

test("worker clocks with arbitrary origins use bounded minimum RTT calibration",()=>{
  const calibrated=calibrateWorkerClock([{sent:1000,worker:10,received:1004},{sent:1010,worker:21,received:1012},{sent:1020,worker:30,received:1026}]);
  assert.equal(calibrated.offsetMs,987);assert.equal(calibrated.uncertaintyMs,6);
  // True main-worker offset is 990; mapping is conservative by 3ms after rounding.
  assert.ok(1033-(42+calibrated.offsetMs)>=0);
  assert.throws(()=>calibrateWorkerClock(Array(3).fill({sent:0,worker:0,received:11})),/延迟/);
  assert.throws(()=>calibrateWorkerClock(Array(3).fill({sent:NaN,worker:0,received:1})),/无效/);
  assert.throws(()=>calibrateWorkerClock([{sent:2,worker:1,received:1}]),/无效/);
  assert.equal(calibrateWorkerClock(Array(3).fill({sent:5,worker:2,received:5})).uncertaintyMs,4);
});
