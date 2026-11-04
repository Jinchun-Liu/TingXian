export interface NativeSpeechTiming {
  stable_text_unix_ms: number;
  vad_to_stable_ms: number;
  audio_end_to_stable_ms: number | null;
}
type Stage = 'stable_text' | 'submitted' | 'retrieval_done' | 'request_sent' | 'first_model_text' | 'first_useful_text' | 'first_sentence' | 'answer_done' | 'ended';
export interface LatencyRecord {
  version: 1;
  audio_end_basis: 'vad_estimate' | 'unavailable';
  // Milliseconds relative to receipt of stable text (or manual submission).
  stages_ms: Partial<Record<Stage | 'question_audio_end', number>>;
  vad_to_stable_ms?: number;
  delivery_ms?: number;
  retrieval_ms?: number;
  provider_first_text_ms?: number;
  provider_total_ms?: number;
  request_attempts: number;
  terminal?: 'done' | 'error' | 'cancelled';
}

/** Browser intervals use one monotonic clock. Only native -> browser delivery
 * uses wall time; an invalid bridge leaves audio-end latency unavailable. */
export class TurnLatency {
  private record: LatencyRecord = { version: 1, audio_end_basis: 'unavailable', stages_ms: {}, request_attempts: 0 };
  constructor(private origin = performance.now(), native?: NativeSpeechTiming | null, wall = Date.now(), stable = false) {
    if (stable) this.record.stages_ms.stable_text = 0;
    if (!native) return;
    const delivery = wall - native.stable_text_unix_ms;
    if (Number.isFinite(native.vad_to_stable_ms) && native.vad_to_stable_ms >= 0) this.record.vad_to_stable_ms = native.vad_to_stable_ms;
    if (Number.isFinite(delivery) && delivery >= 0 && delivery <= 60_000) {
      this.record.delivery_ms = delivery;
      if (native.audio_end_to_stable_ms !== null && Number.isFinite(native.audio_end_to_stable_ms) && native.audio_end_to_stable_ms >= 0) {
        this.record.audio_end_basis = 'vad_estimate';
        this.record.stages_ms.question_audio_end = -(native.audio_end_to_stable_ms + delivery);
      }
    }
  }
  mark(stage: Stage, now = performance.now()) {
    if (this.record.stages_ms[stage] === undefined) this.record.stages_ms[stage] = Math.max(0, now - this.origin);
  }
  observe(text: string, now = performance.now()) {
    if (text.trim()) this.mark('first_model_text', now);
    // Headings/correction text are not useful answer characters. If a provider
    // ignores the format, keep the useful metric unknown instead of guessing.
    const answer = text.match(/(?:^|\n)\s*(?:#{1,6}\s*)?(?:\*\*)?建议回答(?:\*\*)?\s*[：:]\s*([\s\S]*)/u)?.[1]
      ?.split(/\n\s*(?:<<<QUESTION>>>|(?:听写纠正|纠正|关键点|可能追问|事实提醒)\s*[：:])/u)[0];
    if (!answer || !/[\p{L}\p{N}]/u.test(answer)) return;
    this.mark('first_useful_text', now);
    // A decimal point or punctuation-only prefix cannot count as a sentence.
    if (/[\p{L}\p{N}][\s\S]*[。！？!?]/u.test(answer)) this.mark('first_sentence', now);
  }
  provider(values: { retrieval_ms?: number; first_text_ms?: number; elapsed_ms?: number; attempt?: number }) {
    const put = (key: 'retrieval_ms' | 'provider_first_text_ms' | 'provider_total_ms', value?: number) => {
      if (value !== undefined && Number.isFinite(value) && value >= 0) this.record[key] = value;
    };
    put('retrieval_ms', values.retrieval_ms); put('provider_first_text_ms', values.first_text_ms); put('provider_total_ms', values.elapsed_ms);
    if (values.attempt !== undefined) this.record.request_attempts = Math.max(this.record.request_attempts, values.attempt);
  }
  end(status: 'done' | 'error' | 'cancelled', now = performance.now()) {
    if (this.record.terminal) return;
    this.record.terminal = status; this.mark(status === 'done' ? 'answer_done' : 'ended', now);
  }
  snapshot(): LatencyRecord { return { ...this.record, stages_ms: { ...this.record.stages_ms } }; }
}

export function latencyLabel(record: LatencyRecord) {
  const s = record.stages_ms; const result: string[] = [];
  if (s.question_audio_end !== undefined && s.first_sentence !== undefined) result.push(`说完→首句约 ${((s.first_sentence - s.question_audio_end) / 1000).toFixed(1)} 秒（VAD估计）`);
  if (s.first_useful_text !== undefined && s.submitted !== undefined) result.push(`提交→建议 ${Math.round(s.first_useful_text - s.submitted)} ms`);
  if (s.stable_text !== undefined && s.submitted !== undefined) result.push(`排队 ${Math.round(s.submitted - s.stable_text)} ms`);
  if (record.retrieval_ms !== undefined) result.push(`检索 ${record.retrieval_ms} ms`);
  if (record.provider_total_ms !== undefined) result.push(`AI ${record.provider_total_ms} ms`);
  if (record.terminal === 'done' && s.first_useful_text === undefined) result.push('建议首字未测得');
  return result.join(' · ');
}
