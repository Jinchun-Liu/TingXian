"""Offline packet construction for R10 original-voice avatar rendering.

No capture, resampling, TTS, HTTP or playback. Call from one owning thread with
PCM16 LE mono already converted to ``sample_rate``. One instance is one utterance;
finish seals input, while interrupt remains available for queued playback.
The transport must also isolate late events
by its session/generation and mute local output before sending an interrupt.
"""

from dataclasses import dataclass
import base64
import uuid


@dataclass(frozen=True)
class VoicePacket:
    # Sample clock covers input PCM only, not wall-clock or device playback.
    start_sample: int
    sample_count: int
    sample_rate: int
    payload: dict


class OriginalVoicePackets:
    """Documented Tencent SEND_AUDIO / LiveAvatar LITE wire formats.

    The defaults follow the provider docs checked on 2026-11-06, not TingXian
    latency acceptance. In particular, LiveAvatar's recommended 1 s chunk is
    unsuitable for promising sub-800 ms live-microphone latency.
    """

    def __init__(self, provider: str, *, session_id: str = ""):
        if provider not in {"tencent", "liveavatar"}:
            raise ValueError("unsupported original-voice provider")
        if provider == "tencent" and not session_id.strip():
            raise ValueError("Tencent requires a session_id")
        self.provider = provider
        self.sample_rate = 16000 if provider == "tencent" else 24000
        self.chunk_ms = 160 if provider == "tencent" else 1000
        self.chunk_bytes = self.sample_rate * self.chunk_ms // 1000 * 2
        self._session_id = session_id
        self._request_id = uuid.uuid4().hex
        self._seq = 0
        self._cursor = 0
        self._buffer = bytearray()
        self._closed = False

    @property
    def pending_samples(self) -> int:
        return len(self._buffer) // 2

    def _check_open(self):
        if self._closed:
            raise RuntimeError("utterance already finished or interrupted")

    def _packet(self, pcm: bytes, *, final: bool = False) -> VoicePacket:
        encoded = base64.b64encode(pcm).decode("ascii")
        self._seq += 1
        if self.provider == "tencent":
            payload = {
                "ReqId": self._request_id,
                "SessionId": self._session_id,
                "Command": "SEND_AUDIO",
                "Data": {"Audio": encoded, "Seq": self._seq, "IsFinal": final},
            }
        else:
            payload = {
                "type": "agent.speak_end" if final else "agent.speak",
                "event_id": str(uuid.uuid4()),
            }
            if pcm:
                payload["audio"] = encoded
        packet = VoicePacket(self._cursor, len(pcm) // 2, self.sample_rate, payload)
        self._cursor += len(pcm) // 2
        return packet

    def push(self, pcm: bytes) -> list[VoicePacket]:
        self._check_open()
        if not isinstance(pcm, bytes):
            raise TypeError("pcm must be bytes")
        if len(pcm) % 2:
            raise ValueError("PCM16 input must contain complete samples")
        # Bound each call and retained memory; do not queue entire recordings.
        if len(pcm) > self.sample_rate * 2 * 4:
            raise ValueError("push accepts at most four seconds of PCM")
        self._buffer.extend(pcm)
        packets = []
        while len(self._buffer) >= self.chunk_bytes:
            part = bytes(self._buffer[:self.chunk_bytes])
            del self._buffer[:self.chunk_bytes]
            packets.append(self._packet(part))
        return packets

    def finish(self) -> list[VoicePacket]:
        self._check_open()
        packets = []
        if self._buffer:
            packets.append(self._packet(bytes(self._buffer)))
        self._buffer.clear()
        # Tencent explicitly requires an empty final packet, with the SAME ReqId.
        # LiveAvatar permits speak_end without a trailing audio field.
        packets.append(self._packet(b"", final=True))
        self._closed = True
        return packets

    def interrupt(self) -> dict:
        # Input can have ended while the remote renderer still holds audio.
        # Repeated stop requests are harmless; they never reopen the utterance.
        self._buffer.clear()
        self._closed = True
        if self.provider == "tencent":
            return {
                "ReqId": self._request_id,
                "SessionId": self._session_id,
                "Command": "SEND_AUDIO",
                "Data": {"Interrupt": True},
            }
        return {"type": "agent.interrupt", "event_id": str(uuid.uuid4())}
