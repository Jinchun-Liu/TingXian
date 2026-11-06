export interface TranscriptInput {
  sequence_id: number; text: string; is_partial: boolean; source: string;
  confidence?: number | null; // Not a correctness or completion signal.
  audio_start_time: number; audio_end_time: number; timestamp: string;
  utterance_id?: number | null;
  microphone_epoch?: number | null;
  replaces_utterance?: boolean;
  capture_session?: number;
  native_session_id?: number | null;
  interrupted?: boolean;
}

export function sameCapturedUtterance(a: Pick<TranscriptInput, 'source' | 'utterance_id' | 'microphone_epoch' | 'capture_session'>, b: TranscriptInput) {
  return a.utterance_id !== undefined && a.utterance_id !== null && a.capture_session !== undefined
    && a.source === b.source && a.utterance_id === b.utterance_id && a.capture_session === b.capture_session
    && (a.source !== 'Microphone' || a.microphone_epoch === b.microphone_epoch);
}
export function mergeSpokenTranscript(previous: TranscriptInput[], event: TranscriptInput) {
  return [...previous.filter(t => (t.sequence_id !== event.sequence_id || t.capture_session !== event.capture_session) && !(event.replaces_utterance && sameCapturedUtterance(t, event))), event].slice(-120);
}
export interface UtteranceEnd { source: string; utterance_id: number; success: boolean; native_session_id?: number | null; timing?: import('./interview-latency').NativeSpeechTiming | null }
export interface RemoteUtterance { id: string; text: string; time: string }

/** Capture prefixes update the conversation, but only an explicit System
 * speech-end event can enqueue an answer. Microphone never enters this queue. */
export class RemoteUtteranceQueue {
  private generation = 0;
  private seen = new Set<number>();
  private ended = new Set<number>();
  private drafts = new Map<number, RemoteUtterance & { suppressed: boolean }>();
  private pending: RemoteUtterance[] = [];
  constructor(public capacity = 20, public maxLength = 1500) {}
  receive(event: TranscriptInput): RemoteUtterance | null {
    const id = event.utterance_id;
    if (event.source !== 'System' || event.is_partial || !event.text.trim()
      || id == null || !Number.isSafeInteger(id) || id < 0
      || this.seen.has(event.sequence_id) || this.ended.has(id)) return null;
    this.seen.add(event.sequence_id);
    if (this.seen.size > 2000) this.seen.delete(this.seen.values().next().value!);
    const draft = this.drafts.get(id) || { id: `remote-${this.generation}-${id}`, text: '', time: event.timestamp, suppressed: false };
    draft.text = event.replaces_utterance ? event.text.trim() : [draft.text, event.text.trim()].filter(Boolean).join(' ');
    this.drafts.set(id, draft);
    return { id: draft.id, text: draft.text, time: draft.time };
  }
  finish(event: UtteranceEnd, allowAnswer = true, route?: (text:string)=>boolean): { status: 'ignored' | 'recorded' | 'failed' | 'full' | 'queued'; turn?: RemoteUtterance } {
    if (event.source !== 'System' || this.ended.has(event.utterance_id)) return { status: 'ignored' };
    this.ended.add(event.utterance_id);
    if (this.ended.size > 2000) this.ended.delete(this.ended.values().next().value!);
    const draft = this.drafts.get(event.utterance_id);
    this.drafts.delete(event.utterance_id);
    const turn = draft && { id: draft.id, text: draft.text, time: draft.time };
    if (!event.success) return { status: 'failed', turn };
    if (!turn || !draft) return { status: 'ignored' };
    if (draft.suppressed || (route && !route(turn.text)) || !allowAnswer) return { status: 'recorded', turn };
    if (this.pending.length >= this.capacity || turn.text.length > this.maxLength) return { status: 'full', turn };
    this.pending.push(turn);
    return { status: 'queued', turn };
  }
  get size() { return this.pending.length; }
  list(): RemoteUtterance[] { return this.pending.map(t => ({ ...t })); }
  take(): RemoteUtterance | undefined { return this.pending.shift(); }
  clear() { this.pending = []; this.drafts.forEach(d => { d.suppressed = true; }); }
  reset() { this.clear(); this.drafts.clear(); this.seen.clear(); this.ended.clear(); this.generation++; }
}
export interface CandidateQuestion { text: string; eventIds: number[]; due: number }

/** Completed ASR segments only; punctuation separates sentences, while a
 * pause-finalized segment without punctuation is still a complete input. */
export class SentenceQueue {
  private seen = new Set<number>();
  private pending: string[] = [];
  constructor(public capacity = 20) {}
  receive(event: TranscriptInput): 'queued' | 'ignored' | 'full' {
    if (event.is_partial || !event.text.trim() || this.seen.has(event.sequence_id)) return 'ignored';
    const sentences = event.text.trim().match(/[^。！？!?]+(?:[。！？!?]+[”’」』"]*|$)/gu)
      ?.map(text => text.trim()).filter(Boolean) || [];
    if (this.pending.length + sentences.length > this.capacity || sentences.some(text => text.length > 1500)) return 'full';
    this.seen.add(event.sequence_id);
    if (this.seen.size > 2000) this.seen.delete(this.seen.values().next().value!);
    this.pending.push(...sentences);
    return 'queued';
  }
  get size() { return this.pending.length; }
  list(): string[] { return [...this.pending]; }
  take(): string | undefined { return this.pending.shift(); }
  clear() { this.pending = []; }
  reset() { this.clear(); this.seen.clear(); }
}

const QUESTION = /为什么|如何|怎么|什么|是否|能否|请介绍|介绍一下|谈谈|说说|举个例子|[?？]|\b(why|how|what|describe|explain)\b/i;

/** Pure bounded trigger state, independent from audio capture and network requests. */
export class QuestionScheduler {
  private seen = new Map<number, string>();
  private submitted = new Set<number>();
  candidate: CandidateQuestion | null = null;
  constructor(public debounceMs = 900) {}
  receive(event: TranscriptInput, now: number): CandidateQuestion | null {
    if (event.is_partial || !event.text.trim() || this.submitted.has(event.sequence_id)) return this.candidate;
    if (this.seen.get(event.sequence_id) === event.text) return this.candidate;
    this.seen.set(event.sequence_id, event.text);
    if (this.seen.size > 2000) this.seen.delete(this.seen.keys().next().value!);
    if (this.candidate) {
      const ids = this.candidate.eventIds.includes(event.sequence_id) ? this.candidate.eventIds : [...this.candidate.eventIds, event.sequence_id];
      this.candidate = { text: ids.map(id => this.seen.get(id) || '').join(' '), eventIds: ids, due: now + this.debounceMs };
    } else if (QUESTION.test(event.text)) {
      this.candidate = { text: event.text.trim(), eventIds: [event.sequence_id], due: now + this.debounceMs };
    }
    if ((this.candidate?.text.length || 0) > 1500) this.candidate = null;
    return this.candidate;
  }
  take(now: number): string | null {
    if (!this.candidate || now < this.candidate.due) return null;
    const result = this.candidate;
    result.eventIds.forEach(id => this.submitted.add(id));
    if (this.submitted.size > 2000) this.submitted.delete(this.submitted.values().next().value!);
    this.candidate = null;
    return result.text;
  }
  discard() { this.candidate?.eventIds.forEach(id => this.submitted.add(id)); this.candidate = null; }
  reset() { this.candidate = null; this.seen.clear(); this.submitted.clear(); }
}
