"""Synthetic protocol tests: no sound, devices, credentials or network."""
import base64
import json
from pathlib import Path
import random
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from avatar.original_voice import OriginalVoicePackets


def pcm_from(packets):
    return b"".join(base64.b64decode(
        p.payload.get("audio", p.payload.get("Data", {}).get("Audio", ""))
    ) for p in packets)


class OriginalVoiceTests(unittest.TestCase):
    def test_fragmentation_conserves_samples_and_clock(self):
        # Arbitrary small transport/input boundaries must not lose speech samples.
        for provider in ("tencent", "liveavatar"):
            with self.subTest(provider=provider):
                stream = OriginalVoicePackets(provider, session_id="synthetic-session")
                raw = bytes(range(256)) * 251 + b"\x01\x02"
                rng = random.Random(17)
                packets, cursor = [], 0
                while cursor < len(raw):
                    end = min(len(raw), cursor + rng.randint(1, 1500) * 2)
                    packets.extend(stream.push(raw[cursor:end]))
                    self.assertLess(stream.pending_samples, stream.chunk_bytes // 2)
                    cursor = end
                packets.extend(stream.finish())
                self.assertEqual(pcm_from(packets), raw)
                cursor = 0
                for packet in packets:
                    self.assertEqual(packet.start_sample, cursor)
                    self.assertEqual(packet.sample_rate, stream.sample_rate)
                    cursor += packet.sample_count
                self.assertEqual(cursor, len(raw) // 2)

    def test_tencent_live_mic_packet_and_final_contract(self):
        stream = OriginalVoicePackets("tencent", session_id="synthetic-session")
        packets = stream.push(b"\0" * 5120) + stream.push(b"\0" * 100) + stream.finish()
        self.assertEqual([p.sample_count for p in packets], [2560, 50, 0])
        payloads = [p.payload for p in packets]
        self.assertEqual(len({p["ReqId"] for p in payloads}), 1)
        self.assertEqual(len(payloads[0]["ReqId"]), 32)
        self.assertEqual([p["Data"]["Seq"] for p in payloads], [1, 2, 3])
        self.assertEqual([p["Data"]["IsFinal"] for p in payloads], [False, False, True])
        self.assertEqual(payloads[-1]["Data"]["Audio"], "")

    def test_liveavatar_audio_format_frame_limit_and_seal(self):
        stream = OriginalVoicePackets("liveavatar")
        packets = stream.push(b"\0" * 48000) + stream.finish()
        self.assertEqual(packets[0].sample_count, 24000)
        self.assertEqual([p.payload["type"] for p in packets], ["agent.speak", "agent.speak_end"])
        self.assertNotIn("audio", packets[-1].payload)
        self.assertLess(len(json.dumps(packets[0].payload).encode()), 1_000_000)
        self.assertNotEqual(packets[0].payload["event_id"], packets[1].payload["event_id"])

    def test_interrupt_drops_pending_and_rejects_late_audio(self):
        for provider in ("tencent", "liveavatar"):
            with self.subTest(provider=provider):
                stream = OriginalVoicePackets(provider, session_id="synthetic-session")
                self.assertEqual(stream.push(b"\0" * 100), [])
                command = stream.interrupt()
                self.assertEqual(stream.pending_samples, 0)
                if provider == "tencent":
                    self.assertEqual(command["Data"], {"Interrupt": True})
                else:
                    self.assertEqual(command["type"], "agent.interrupt")
                for operation in (lambda: stream.push(b"\0\0"), stream.finish):
                    with self.assertRaises(RuntimeError):
                        operation()
                self.assertIsInstance(stream.interrupt(), dict)

    def test_input_finished_does_not_prevent_stopping_queued_playback(self):
        for provider in ("tencent", "liveavatar"):
            stream = OriginalVoicePackets(provider, session_id="synthetic-session")
            stream.push(b"\0" * stream.chunk_bytes)
            stream.finish()
            command = stream.interrupt()
            if provider == "tencent":
                self.assertEqual(command["Data"], {"Interrupt": True})
            else:
                self.assertEqual(command["type"], "agent.interrupt")
            with self.assertRaises(RuntimeError):
                stream.push(b"\0\0")

    def test_finish_rejects_reuse_and_new_utterance_gets_new_id(self):
        first = OriginalVoicePackets("tencent", session_id="synthetic-session")
        first_id = first.finish()[0].payload["ReqId"]
        with self.assertRaises(RuntimeError):
            first.push(b"\0\0")
        second = OriginalVoicePackets("tencent", session_id="synthetic-session")
        self.assertNotEqual(first_id, second.finish()[0].payload["ReqId"])

    def test_invalid_input_leaves_pending_audio_untouched(self):
        stream = OriginalVoicePackets("liveavatar")
        stream.push(b"\x01\x02")
        for invalid in (b"\0", b"\0" * 192002):
            with self.assertRaises(ValueError):
                stream.push(invalid)
        with self.assertRaises(TypeError):
            stream.push("text is not original voice")
        self.assertEqual(pcm_from(stream.finish()), b"\x01\x02")

    def test_provider_and_session_must_be_explicit(self):
        with self.assertRaises(ValueError):
            OriginalVoicePackets("tts")
        with self.assertRaises(ValueError):
            OriginalVoicePackets("tencent")


if __name__ == "__main__":
    unittest.main(verbosity=2)
