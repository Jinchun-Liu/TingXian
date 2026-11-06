/** PCM16 mono only. Parsing bytes must never open a browser audio device. */
export const MAX_WAV_BYTES = 6 * 1024 * 1024;
export type PcmClip = { samples: Float32Array; sampleRate: number };
export function readPcmWav(buffer: ArrayBuffer): PcmClip {
  if (buffer.byteLength < 44 || buffer.byteLength > MAX_WAV_BYTES) throw new Error("请选择 6 MB 以内的完整 WAV 文件。");
  const view = new DataView(buffer);
  const tag = (at: number) => String.fromCharCode(...new Uint8Array(buffer, at, 4));
  if (tag(0) !== "RIFF" || tag(8) !== "WAVE" || view.getUint32(4,true) + 8 !== buffer.byteLength) throw new Error("WAV 文件长度或标识不正确。");
  let sampleRate = 0, offset = -1, count = 0, hasFormat = false;
  for (let at = 12; at < buffer.byteLength;) {
    if (buffer.byteLength - at < 8) throw new Error("WAV 数据块不完整。");
    const kind = tag(at), length = view.getUint32(at+4,true), start = at+8;
    if (length > buffer.byteLength-start || start+length+(length%2) > buffer.byteLength) throw new Error("WAV 数据块不完整。");
    if (kind === "fmt ") {
      if (hasFormat || length < 16) throw new Error("WAV 格式块不正确。");
      hasFormat = true;
      sampleRate = view.getUint32(start+4,true);
      if (view.getUint16(start,true) !== 1 || view.getUint16(start+2,true) !== 1 || view.getUint16(start+14,true) !== 16) throw new Error("请使用 PCM16 单声道 WAV。");
      if (sampleRate < 8000 || sampleRate > 48000 || view.getUint16(start+12,true) !== 2 || view.getUint32(start+8,true) !== sampleRate*2) throw new Error("WAV 采样率或样本格式不正确。");
    } else if (kind === "data") {
      if (offset !== -1 || length % 2 || !length) throw new Error("WAV 音频数据不完整。");
      offset = start; count = length/2;
    }
    at = start+length+(length%2);
  }
  if (!hasFormat || offset < 0 || count > sampleRate*60) throw new Error("请选择时长不超过 60 秒的 PCM16 单声道 WAV。");
  const samples = new Float32Array(count);
  for (let i=0;i<count;i++) samples[i] = view.getInt16(offset+i*2,true)/32768;
  return {samples,sampleRate};
}
