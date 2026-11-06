"""Synthetic clock/control regressions; no torch, models, camera, or playback."""
from dataclasses import asdict
import math
from pathlib import Path
import sys
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from avatar.gaze import gaze_offsets
from avatar.idle_motion import IdleMotion
from avatar.liveportrait_renderer import HeadMotion


class MotionTests(unittest.TestCase):
    def test_same_media_time_survives_frame_rate_changes(self):
        slow, fast = IdleMotion(17), IdleMotion(17)
        for frame in range(1, 751):
            fast.sample((frame * 2 - 1) / 50)
            self.assertEqual(slow.sample(frame / 25), fast.sample(frame * 2 / 50))

    def test_skipped_frames_do_not_change_future_motion(self):
        dense, sparse = IdleMotion(19), IdleMotion(19)
        for frame in range(7501):
            current = dense.sample(frame / 25)
        self.assertEqual(current, sparse.sample(300))

    def test_invalid_or_late_time_does_not_advance_state(self):
        controller, reference = IdleMotion(23), IdleMotion(23)
        controller.sample(10)
        reference.sample(10)
        for invalid in (9.99, -1, float("nan"), float("inf"), 86401):
            with self.assertRaises(ValueError):
                controller.sample(invalid)
        self.assertEqual(controller.sample(11), reference.sample(11))
        self.assertEqual(controller.sample(11), reference.sample(11))

    def test_long_session_stays_bounded_without_motion_jumps(self):
        controller = IdleMotion(42)
        last = HeadMotion()
        blink_peaks, peak = [], 0.0
        for frame in range(25 * 60 * 30):
            current = controller.sample(frame / 25)
            self.assertTrue(all(math.isfinite(v) for v in asdict(current).values()))
            self.assertLessEqual(abs(current.pitch_deg), 0.7)
            self.assertLessEqual(abs(current.yaw_deg), 2.0)
            self.assertLessEqual(abs(current.roll_deg), 0.3)
            self.assertLessEqual(abs(current.gaze_x), 0.28)
            self.assertLessEqual(abs(current.gaze_y), 0.14)
            self.assertLess(abs(current.yaw_deg - last.yaw_deg), 0.26)
            self.assertLess(abs(current.gaze_x - last.gaze_x), 0.24)
            if current.blink:
                peak = max(peak, current.blink)
            elif last.blink:
                blink_peaks.append(peak)
                peak = 0.0
            last = current
        self.assertGreater(len(blink_peaks), 250)
        self.assertTrue(all(v > 0.9 for v in blink_peaks))

    def test_gaze_is_neutral_during_full_blink(self):
        self.assertTrue(all(delta == 0 for _, _, delta in gaze_offsets(1, -1, 1)))
        self.assertTrue(all(delta == 0 for _, _, delta in gaze_offsets(0, 0)))
        for invalid in ((2, 0, 0), (0, -2, 0), (0, 0, 1.1), (math.nan, 0, 0)):
            with self.assertRaises(ValueError):
                gaze_offsets(*invalid)

    def test_new_session_resets_motion_and_import_does_not_load_gpu(self):
        self.assertEqual(IdleMotion(42).sample(0), HeadMotion())
        self.assertNotEqual(IdleMotion(42).sample(30), IdleMotion(43).sample(30))
        self.assertNotIn("torch", sys.modules)
        for invalid in (dict(gaze_x=1.1), dict(gaze_y=math.inf), dict(blink=-0.1)):
            with self.assertRaises(ValueError):
                HeadMotion(**invalid)


if __name__ == "__main__":
    unittest.main(verbosity=2)
