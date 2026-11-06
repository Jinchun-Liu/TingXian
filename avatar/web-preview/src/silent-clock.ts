/** Silent visual playhead. This is not an audio-device or meeting clock. */
export class SilentSampleClock {
  private anchor = 0;
  private base = 0;
  private previous = -Infinity;
  running = false;
  constructor(readonly samples: number, readonly rate: number) {
    if (!Number.isSafeInteger(samples) || samples<1 || !Number.isInteger(rate) || rate<8000 || rate>48000) throw new Error("Invalid sample clock");
  }
  private check(now: number) {
    if (!Number.isFinite(now) || now<this.previous) throw new Error("Clock must be monotonic");
    this.previous=now;
  }
  position(now: number): number {
    this.check(now);
    const sample=Math.min(this.samples,Math.floor(this.base+(this.running?(now-this.anchor)*this.rate/1000:0)));
    if (sample===this.samples) { this.base=sample; this.running=false; }
    return sample;
  }
  resume(now: number) {
    this.check(now);
    if (this.running) return;
    if (this.base===this.samples) this.base=0;
    this.anchor=now; this.running=true;
  }
  pause(now: number) { this.base=this.position(now); this.running=false; }
  seek(sample: number, now: number) {
    if (!Number.isSafeInteger(sample) || sample<0 || sample>this.samples) throw new Error("Invalid seek sample");
    this.check(now); this.base=sample; this.anchor=now;
    if (sample===this.samples) this.running=false;
  }
}
