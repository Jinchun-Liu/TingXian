import { MAX_WAV_BYTES, readPcmWav } from "./pcm-wav";
import { clipFrame, type ClipAnalysis } from "./viseme-analysis";
import { SilentSampleClock } from "./silent-clock";
import { StreamPreview } from "./stream-preview";

const element=<T extends HTMLElement>(id:string)=>document.getElementById(id) as T;
export class SilentAudioPreview {
  private readonly file=element<HTMLInputElement>("audio-file");
  private readonly toggle=element<HTMLButtonElement>("audio-toggle");
  private readonly clearButton=element<HTMLButtonElement>("audio-clear");
  private readonly seek=element<HTMLInputElement>("audio-seek");
  private readonly status=element("audio-status");
  private readonly position=element("audio-position");
  private readonly diagnostics=element("audio-diagnostics");
  private readonly mode=element<HTMLSelectElement>("audio-mode");
  private buffer?:ArrayBuffer;
  private stream?:StreamPreview;
  private streamInfo?:{sampleRate:number;sampleCount:number};
  private worker?:Worker;
  private timer?:ReturnType<typeof setTimeout>;
  private generation=0;
  private result?:ClipAnalysis;
  private clock?:SilentSampleClock;
  private enabled=false;
  private suspended=false;
  private updatedAt=-Infinity;
  get hasClip() { return !!this.result||!!this.streamInfo; }

  constructor(private readonly changed:()=>void, private readonly startRendering:()=>void) {
    this.file.addEventListener("change",()=>void this.importFile());
    this.clearButton.addEventListener("click",()=>this.clear());
    this.mode.addEventListener("change",()=>{const buffer=this.buffer;this.clear();if(buffer){this.buffer=buffer;this.prepare(buffer);}});
    this.toggle.addEventListener("click",()=> {
      if(this.mode.value==="stream"){
        if(!this.enabled||!this.buffer||!this.streamInfo)return;
        if(this.stream?.active)this.stream.stop();
        else {this.startRendering();this.stream?.start(this.buffer);}
        this.refreshStream();this.changed();return;
      }
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
    this.toggle.disabled=!enabled || !this.hasClip;
  }
  setRendererPaused(paused:boolean) {
    if(this.streamInfo){if(paused)this.pause();return;}
    if (!this.clock) return;
    if (paused) { this.suspended=this.clock.running; this.clock.pause(performance.now()); }
    else if (this.suspended) { this.clock.resume(performance.now()); this.suspended=false; }
    this.refresh();
  }
  pause() { this.suspended=false; this.clock?.pause(performance.now());this.stream?.stop();this.refresh();this.refreshStream(); }
  clear() {
    this.generation++; this.worker?.terminate(); this.worker=undefined; clearTimeout(this.timer);
    this.stream?.stop();this.stream=undefined;this.streamInfo=undefined;this.buffer=undefined;
    this.result=undefined; this.clock=undefined; this.suspended=false;
    this.toggle.disabled=this.clearButton.disabled=this.seek.disabled=true;
    this.seek.value="0"; this.toggle.textContent=this.mode.value==="stream"?"开始推流":"查看口型"; this.position.textContent="0.0 秒";
    this.status.textContent="未导入语音"; this.diagnostics.textContent=""; this.changed();
  }
  dispose() { this.generation++; this.worker?.terminate();this.stream?.stop();clearTimeout(this.timer); }
  frame() {
    if(this.streamInfo){if(performance.now()-this.updatedAt>100)this.refreshStream();return this.stream?.frame();}
    if (!this.result || !this.clock) return undefined;
    const now=performance.now(), sample=this.clock.position(now);
    if (now-this.updatedAt>100 || !this.clock.running) this.refresh(sample);
    return clipFrame(this.result,sample);
  }
  private refreshStream(){
    if(!this.streamInfo||!this.stream)return;
    this.updatedAt=performance.now();const sample=this.stream.sample;
    this.seek.value=String(sample);this.seek.disabled=true;
    this.position.textContent=`${(sample/this.streamInfo.sampleRate).toFixed(1)} / ${(this.streamInfo.sampleCount/this.streamInfo.sampleRate).toFixed(1)} 秒`;
    this.toggle.textContent=this.stream.active?"停止推流":"开始推流";
    this.toggle.disabled=!this.enabled;
    this.diagnostics.textContent=JSON.stringify(this.stream.diagnostics,null,2);
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
      this.buffer=buffer;this.prepare(buffer);
    } catch (reason) { fail(reason instanceof Error?reason.message:String(reason)); }
    finally { this.file.value=""; }
  }
  private prepare(buffer:ArrayBuffer){
    const token=this.generation;this.clearButton.disabled=false;
    const fail=(message:string)=>{
      if(token!==this.generation)return;
      this.worker?.terminate();this.worker=undefined;clearTimeout(this.timer);
      this.status.textContent=message;this.toggle.disabled=true;
    };
    try{
      if(this.mode.value==="stream"){
        const clip=readPcmWav(buffer);this.streamInfo={sampleRate:clip.sampleRate,sampleCount:clip.samples.length};
        this.stream=new StreamPreview(message=>{this.status.textContent=message;this.refreshStream();},()=>this.changed());
        this.seek.max=String(clip.samples.length);this.status.textContent="语音已就绪";this.refreshStream();this.changed();return;
      }
      this.status.textContent="正在本地分析…";
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
      const transfer=buffer.slice(0);worker.postMessage({buffer:transfer},[transfer]);
    } catch (reason) { fail(reason instanceof Error?reason.message:String(reason)); }
  }
}
