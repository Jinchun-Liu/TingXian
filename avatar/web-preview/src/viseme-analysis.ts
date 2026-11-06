import { parseVisemeModel, type VisemeModel } from "./vendor/visemeModel";
import { VisemeDetector, type VisemeEvent } from "./vendor/visemeDetector";
import { buildSegments, mouthAt, isSpeakingAt, type Segment } from "./vendor/coarticulation";
import type { PcmClip } from "./pcm-wav";

export const VISEME_MODEL_SHA256 = "f1cdf2b76b9f2da8730b1d3e47d7e05a2b79c9b92d00dd00c595d64e471f419c";
export async function verifiedVisemeModel(bytes: ArrayBuffer): Promise<VisemeModel> {
  if (bytes.byteLength !== 1040276) throw new Error("口型模型文件大小不正确。");
  const digest = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), b => b.toString(16).padStart(2,"0")).join("");
  if (digest !== VISEME_MODEL_SHA256) throw new Error("口型模型校验失败。");
  return parseVisemeModel(bytes);
}
export type ClipAnalysis = {
  sampleRate: number; sampleCount: number; durationMs: number;
  events: VisemeEvent[]; segments: Segment[];
  inferenceMs: number; chunkP95Ms: number; chunks: number;
  modelLookaheadMs: number; modelWindowMs: number;
};

/** File/CPU experiment, not a live capture or audio output implementation. */
export function analyzePcm(model: VisemeModel, clip: PcmClip, chunkSamples = Math.round(clip.sampleRate*.02)): ClipAnalysis {
  const {samples,sampleRate} = clip;
  if (!Number.isInteger(sampleRate) || sampleRate < 8000 || sampleRate > 48000 || !samples.length || samples.length > sampleRate*60 || !Number.isInteger(chunkSamples) || chunkSamples < 1 || chunkSamples > sampleRate) throw new Error("无效的音频输入或分片长度。");
  if (samples.some(v=>!Number.isFinite(v) || Math.abs(v)>1)) throw new Error("PCM 样本超出有效范围。");
  const durationMs = samples.length/sampleRate*1000;
  const events: VisemeEvent[] = [{id:0,ms:0}];
  const detector = new VisemeDetector(model,sampleRate,0,event=> {
    if (event.ms >= 0 && event.ms < durationMs && event.ms > events.at(-1)!.ms) events.push(event);
  });
  const started = performance.now(), timings: number[] = [];
  for (let at=0;at<samples.length;at+=chunkSamples) {
    if (performance.now()-started>60000) throw new Error("口型推断超时，请缩短测试语音。");
    const before=performance.now(); detector.push(samples.subarray(at,at+chunkSamples)); timings.push(performance.now()-before);
  }
  detector.flush();
  events.push({id:0,ms:durationMs});
  const inferenceMs=performance.now()-started;
  const segments=buildSegments(events);
  for (const segment of segments) segment.end=Math.min(durationMs,segment.end);
  return { sampleRate,sampleCount:samples.length,durationMs,events,segments,inferenceMs,
    chunkP95Ms:[...timings].sort((a,b)=>a-b)[Math.min(timings.length-1,Math.floor(timings.length*.95))],chunks:timings.length,
    modelLookaheadMs:model.lookahead*model.HOP/model.SR*1000,modelWindowMs:model.WIN/model.SR*1000 };
}

export function clipFrame(clip: ClipAnalysis, sample: number) {
  if (!Number.isFinite(sample) || sample < 0 || sample >= clip.sampleCount) return {visemes:{},mode:"listening" as const};
  const ms = Math.floor(sample)/clip.sampleRate*1000;
  return {visemes:mouthAt(clip.segments,ms),mode:isSpeakingAt(clip.events,ms)?"speaking" as const:"listening" as const};
}
