export interface AudioFault { session_id: number; sequence: number; code: string; at_ms?: number; detail?: string }
export interface AudioHealthSnapshot {
  session_id: number; recording: boolean; active: boolean; paused: boolean;
  system_started: boolean; microphone_enabled: boolean; active_for_ms: number;
  system_frame_age_ms: number | null; microphone_frame_age_ms: number | null;
  system_signal_age_ms: number | null; asr_pending: number; asr_no_progress_ms: number | null;
  fault: AudioFault | null;
  restart_required?: boolean;
  restart_reason?: string | null;
  elapsed_ms?: number;
  asr_pending_bytes?: number;
  asr_peak_bytes?: number;
  asr_streaming?: boolean;
}
export interface AudioHealthView { paused: boolean; message: string; canResume: boolean; advisory: string }
const messages: Record<string, string> = {
  capture_error: '音频采集报错', system_disconnected: '系统音频设备已断开',
  microphone_disconnected: '麦克风设备已断开，双方上下文可能不完整',
  asr_init_failed: '转写模型未能启动，请停止采集后重新开始',
  asr_worker_failed: '转写任务已异常退出，请停止采集后重新开始',
  cloud_asr_failed: '云转写已中断，请停止后重试或在音频设置切回本地模型',
  asr_failed: '这段语音转写失败', inactive: '音频采集已异常停止，请停止采集后重新开始',
  asr_overflow: '转写积压达到容量上限，已停止接收新语音，文字可能不完整。请点击停止采集，等待处理完成后重新开始',
  system_missing: '系统音频没有启动，请停止采集后重新开始',
  backlog: '转写积压，请等待待处理音频完成', stalled: '转写已超过 30 秒没有进展',
  disconnected: '暂时无法读取采集状态',
};

/** The pause is latched: a successful poll never silently re-arms AI requests.
 * Native session IDs also isolate delayed faults/transcripts after a restart. */
export class InterviewAudioHealth {
  private session: number | null = null;
  private lastSession = 0;
  private awaitingStart = false;
  private closed = false;
  private sequence = 0;
  private reason = '';
  private detail = '';
  private restartRequired = false;
  private snapshot: AudioHealthSnapshot | null = null;
  private lastPoll = 0;
  private faultAt = 0;
  private lastTranscript = 0;
  constructor(private now: () => number = () => performance.now()) {}
  prepareStart() {
    this.session = null; this.awaitingStart = true; this.closed = false;
    this.sequence = 0; this.reason = ''; this.detail = ''; this.restartRequired = false; this.snapshot = null;
    this.lastPoll = this.now(); this.lastTranscript = this.now(); this.faultAt = this.now();
  }
  close() { this.closed = true; this.reason = ''; this.detail = ''; this.snapshot = null; }
  private bind(id: number) {
    if (this.closed || !Number.isSafeInteger(id) || id < 1) return false;
    if (this.session !== null) return id === this.session;
    if (this.awaitingStart && id <= this.lastSession) return false;
    this.session = id; this.lastSession = Math.max(id, this.lastSession); this.awaitingStart = false;
    this.lastPoll = this.now(); this.lastTranscript = this.now();
    return true;
  }
  accepts(id?: number | null) {
    // Compatibility for old fixtures/native versions. Once a session is
    // identified, unscoped legacy events cannot affect it.
    return id == null ? !this.closed && this.session === null : !this.closed && id === this.session;
  }
  fault(event: AudioFault) {
    // Only a native status response may establish the current recording.
    // A delayed event cannot claim an unbound page while start is in flight.
    if (!this.accepts(event.session_id) || event.sequence <= this.sequence) return false;
    this.sequence = event.sequence; this.reason = event.code; this.detail = event.detail || ''; this.faultAt = this.now();
    if (['asr_init_failed', 'asr_worker_failed', 'asr_overflow','cloud_asr_failed'].includes(event.code)) this.restartRequired = true;
    return true;
  }
  observe(snapshot?: AudioHealthSnapshot | null) {
    if (!snapshot || !this.bind(snapshot.session_id)) return false;
    if ((snapshot.fault?.sequence ?? 0) < this.sequence) return false;
    this.snapshot = snapshot; this.lastPoll = this.now();
    if (snapshot.fault) this.fault(snapshot.fault);
    if (snapshot.restart_required) { this.restartRequired = true; this.reason = snapshot.restart_reason || 'asr_init_failed'; }
    if (snapshot.recording) {
      if (!snapshot.active) { if (!this.reason || (!snapshot.fault && !snapshot.restart_reason && this.reason !== 'asr_overflow')) { this.reason = 'inactive'; this.detail = ''; } this.restartRequired = true; }
      else if (!snapshot.system_started) { this.reason = 'system_missing'; this.restartRequired = true; }
      else if (!snapshot.paused && !this.reason) {
        if (snapshot.asr_streaming ? (snapshot.asr_pending_bytes ?? 0) >= 384000 : snapshot.asr_pending >= 4) { this.reason = 'backlog'; this.faultAt = this.now(); }
        else if (snapshot.asr_pending > 0 && (snapshot.asr_no_progress_ms ?? 0) >= 30000) { this.reason = 'stalled'; this.faultAt = this.now(); }
      }
    }
    return true;
  }
  transcript() { this.lastTranscript = this.now(); }
  checkPoll() {
    if (!this.closed && this.snapshot?.recording && this.now() - this.lastPoll > 10000 && !this.reason) this.reason = 'disconnected';
  }
  get paused() { return !!this.reason; }
  view(): AudioHealthView {
    const s = this.snapshot;
    // A pre-fault snapshot/frame cannot prove that audio has recovered.
    const nativeAfterFault = s?.elapsed_ms !== undefined && s.fault?.at_ms !== undefined ? s.elapsed_ms - s.fault.at_ms : Infinity;
    const frameAgeLimit = Math.min(3000, this.lastPoll - this.faultAt, nativeAfterFault);
    const fresh = (age: number | null) => age !== null && age < frameAgeLimit;
    const canResume = !!this.reason && !!s?.recording && s.active && !s.paused && s.system_started
      && this.now() - this.lastPoll < 3000 && s.asr_pending === 0
      && fresh(s.system_frame_age_ms) && (!s.microphone_enabled || fresh(s.microphone_frame_age_ms))
      && !this.restartRequired;
    let advisory = '';
    if (!this.reason && s?.recording && s.active && !s.paused && s.active_for_ms > 10000) {
      if (s.system_frame_age_ms === null || s.system_frame_age_ms > 5000) advisory = '暂未收到系统音频数据。若对方正在说话，请检查系统音频权限和输出设备。';
      else if (s.microphone_enabled && (s.microphone_frame_age_ms === null || s.microphone_frame_age_ms > 5000)) advisory = '暂未收到麦克风数据；请检查输入设备，或关闭“记录我的声音”。';
      else if (s.system_signal_age_ms === null || s.system_signal_age_ms > 15000) advisory = '尚未检测到对方声音；等待开场或静音时属于正常情况。若对方正在说话，请检查飞书输出和系统音频权限。';
      else if (s.active_for_ms > 20000 && this.now() - this.lastTranscript > 20000) advisory = '检测到系统声音，但近期没有字幕；请确认声音包含人声，并检查转写模型。';
    }
    return { paused: !!this.reason, message: this.reason ? `${this.detail || messages[this.reason] || '采集或转写异常'}，已暂停自动回答。转写保留，可手动输入或本地检索。` : '', canResume, advisory };
  }
  resume() { if (!this.view().canResume) return false; this.reason = ''; return true; }
}
