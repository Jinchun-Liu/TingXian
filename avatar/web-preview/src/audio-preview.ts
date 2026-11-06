import { MAX_WAV_BYTES } from "./pcm-wav";
import { clipFrame, type ClipAnalysis } from "./viseme-analysis";
import { SilentSampleClock } from "./silent-clock";

const element=<T extends HTMLElement>(id:string)=>document.getElementById(id) as T;
export class SilentAudioPreview {
  private readonly file=element<HTMLInputElement>("audio-file");
  private readonly toggle=element<HTMLButtonElement>("audio-toggle");
  private readonly clearButton=element<HTMLButtonElement>("audio-clear");
  private readonly seek=element<HTMLInputElement>("audio-seek");
  private readonly status=element("audio-status");
  private readonly position=element("audio-position");
  private readonly diagnostics=element("audio-diagnostics");
  private worker?:Worker;
  private timer?:ReturnType<typeof setTimeout>;
  private generation=0;
  private result?:ClipAnalysis;
  private clock?:SilentSampleClock;
  private enabled=false;
  private suspended=false;
  private updatedAt=-Infinity;
  get hasClip() { return !!this.result; }

  constructor(private readonly changed:()=>void, private readonly startRendering:()=>void) {
    this.file.addEventListener("change",()=>void this.importFile());
    this.clearButton.addEventListener("click",()=>this.clear());
    this.toggle.addEventListener("click",()=> {
      if (!this.enabled || !this.clock) return;
      if (this.clock.running) this.pause();
      else { this.clock.resume(performance.now()); this.suspended=false; this.startRendering(); }
      this.refresh(); this.changed();
    });
    this.seek.addEventListener("input",()=> {
      if (!this.clock) return;
      this.clock.pause(performance.now()); this.clock.seek(+this.seek.value,performance.now());
      this.suspended=false; this.refresh(); this.changed();
    });
  }
  setEnabled(enabled:boolean) {
    this.enabled=enabled;
    if (!enabled) this.pause();
    this.toggle.disabled=!enabled || !this.result;
  }
  setRendererPaused(paused:boolean) {
    if (!this.clock) return;
    if (paused) { this.suspended=this.clock.running; this.clock.pause(performance.now()); }
    else if (this.suspended) { this.clock.resume(performance.now()); this.suspended=false; }
    this.refresh();
  }
  pause() { this.suspended=false; this.clock?.pause(performance.now()); this.refresh(); }
  clear() {
    this.generation++; this.worker?.terminate(); this.worker=undefined; clearTimeout(this.timer);
    this.result=undefined; this.clock=undefined; this.suspended=false;
    this.toggle.disabled=this.clearButton.disabled=this.seek.disabled=true;
    this.seek.value="0"; this.toggle.textContent="查看口型"; this.position.textContent="0.0 秒";
    this.status.textContent="未导入语音"; this.diagnostics.textContent=""; this.changed();
  }
  dispose() { this.generation++; this.worker?.terminate(); clearTimeout(this.timer); }
  frame() {
    if (!this.result || !this.clock) return undefined;
    const now=performance.now(), sample=this.clock.position(now);
    if (now-this.updatedAt>100 || !this.clock.running) this.refresh(sample);
    return clipFrame(this.result,sample);
  }
  private refresh(sample?:number) {
    if (!this.clock || !this.result) return;
    const now=performance.now(); sample??=this.clock.position(now); this.updatedAt=now;
    this.seek.value=String(sample);
    this.position.textContent=`${(sample/this.result.sampleRate).toFixed(1)} / ${(this.result.durationMs/1000).toFixed(1)} 秒`;
    this.seek.setAttribute("aria-valuetext",this.position.textContent);
    this.toggle.textContent=this.clock.running?"暂停口型":sample===this.result.sampleCount?"重新查看":"查看口型";
    this.status.textContent=this.clock.running?"无声查看中":sample===this.result.sampleCount?"已结束":"口型已就绪";
  }
  private async importFile() {
    const file=this.file.files?.[0]; if (!file) return;
    this.clear(); const token=this.generation;
    this.clearButton.disabled=false; this.status.textContent="正在本地分析…";
    const fail=(message:string)=> {
      if (token!==this.generation) return;
      this.worker?.terminate(); this.worker=undefined; clearTimeout(this.timer);
      this.status.textContent=message; this.clearButton.disabled=false;
    };
    try {
      if (file.size>MAX_WAV_BYTES) throw new Error("请选择 6 MB 以内的 WAV 文件。");
      const buffer=await file.arrayBuffer(); if (token!==this.generation) return;
      const worker=new Worker(new URL("./viseme.worker.ts",import.meta.url),{type:"module"}); this.worker=worker;
      worker.onmessage=({data}:MessageEvent<{result?:ClipAnalysis;error?:string}>)=> {
        if (token!==this.generation) return;
        if (!data.result) { fail(data.error??"口型分析失败。"); return; }
        clearTimeout(this.timer); worker.terminate(); this.worker=undefined;
        this.result=data.result; this.clock=new SilentSampleClock(data.result.sampleCount,data.result.sampleRate);
        this.seek.max=String(data.result.sampleCount); this.seek.disabled=false; this.toggle.disabled=!this.enabled;
        const {events,segments,...timing}=data.result;
        this.diagnostics.textContent=JSON.stringify({...timing,eventCount:events.length,visemeIds:[...new Set(events.map(e=>e.id))],scope:"offline_pcm_to_viseme",audioPlayback:false,capture:false,virtualOutput:false},null,2);
        this.refresh(); this.changed();
      };
      worker.onerror=()=>fail("口型分析失败，请检查本地模型与浏览器日志。");
      this.timer=setTimeout(()=>fail("口型分析超时，请缩短测试语音。"),65000);
      worker.postMessage({buffer},[buffer]);
    } catch (reason) { fail(reason instanceof Error?reason.message:String(reason)); }
    finally { this.file.value=""; }
  }
}
