import { invoke } from '@tauri-apps/api/core';
import { TranscriptInput } from './interview-scheduler';

export interface MicrophoneState { recording: boolean; enabled: boolean; epoch: number | null; health?: import('./interview-health').AudioHealthSnapshot | null }

export const stopInterviewAudio = () => invoke<void>('stop_recording', { args: { save_path: '' } });

export const listeningPhase = (automatic: boolean, cloudAsr: boolean, addressed = false) => automatic
  ? addressed ? '监听中 · 等待点名' : '监听中 · 对方说完后回答'
  : cloudAsr ? '监听中 · 百炼实时转写' : '监听中 · 本地转写';

/** Mirrors the native stream token so late IPC deliveries cannot appear after
 * muting, or after reopening with a different microphone epoch. */
export class MicrophoneTranscriptGate {
  private epoch: number | null = null;
  apply(state: MicrophoneState) { this.epoch = state.recording && state.enabled ? state.epoch : null; }
  close() { this.epoch = null; }
  accepts(event: TranscriptInput) {
    return event.source !== 'Microphone' || (this.epoch !== null && event.microphone_epoch === this.epoch);
  }
}
