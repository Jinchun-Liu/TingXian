"""Camera-free synthetic idle motion sampled on a monotonic media clock.

Independent random streams make the same seed/time invariant to render FPS.
Smooth holds and transitions replace a periodic sine-wave fixture. This does
not infer attention, agreement, emotion, speech, or the user's real actions.
"""
import math
import random
from .liveportrait_renderer import HeadMotion


def smoothstep(value):
    value = max(0.0, min(1.0, value))
    return value ** 3 * (value * (value * 6.0 - 15.0) + 10.0)


class _MotionTrack:
    def __init__(self, rng, limits, dwell, transition):
        self.rng, self.limits = rng, limits
        self.dwell, self.transition = dwell, transition
        self.target = (0.0,) * len(limits)
        self.end = 0.0
        self._advance()

    def _advance(self):
        self.origin = self.target
        self.target = tuple(max(-limit, min(limit, self.rng.gauss(0, limit / 2)))
                            for limit in self.limits)
        self.start = self.end + self.rng.uniform(*self.dwell)
        self.end = self.start + self.rng.uniform(*self.transition)

    def sample(self, time_s):
        while time_s >= self.end:
            self._advance()
        weight = smoothstep((time_s - self.start) / (self.end - self.start))
        return tuple(a + (b - a) * weight for a, b in zip(self.origin, self.target))


class IdleMotion:
    """Use a fresh instance for each session; elapsed seconds may not go backward."""
    def __init__(self, seed: int = 42):
        if not isinstance(seed, int):
            raise TypeError("seed must be an integer")
        self.head = _MotionTrack(random.Random(seed * 3), (0.7, 2.0, 0.3),
                                 (0.8, 2.8), (1.2, 2.4))
        self.gaze = _MotionTrack(random.Random(seed * 3 + 1), (0.28, 0.14),
                                 (1.4, 3.8), (0.18, 0.38))
        self.blinks = random.Random(seed * 3 + 2)
        self.last_time = 0.0
        self.blink_end = 0.0
        self._advance_blink()

    def _advance_blink(self):
        self.blink_start = self.blink_end + self.blinks.uniform(2.4, 5.8)
        self.blink_peak = self.blink_start + self.blinks.uniform(0.07, 0.10)
        self.blink_end = self.blink_peak + self.blinks.uniform(0.12, 0.18)

    def sample(self, time_s: float) -> HeadMotion:
        # Bound catch-up work as well as invalid/late timestamps. A daily session
        # reset is a prototype limit, not silent timestamp clipping.
        if not math.isfinite(time_s) or not self.last_time <= time_s <= 86400:
            raise ValueError("idle time must be finite, monotonic, and within a 24h session")
        while time_s >= self.blink_end:
            self._advance_blink()
        if time_s < self.blink_start:
            blink = 0.0
        elif time_s <= self.blink_peak:
            blink = smoothstep((time_s - self.blink_start) / (self.blink_peak - self.blink_start))
        else:
            blink = 1.0 - smoothstep((time_s - self.blink_peak) / (self.blink_end - self.blink_peak))
        pitch, yaw, roll = self.head.sample(time_s)
        gaze_x, gaze_y = self.gaze.sample(time_s)
        self.last_time = time_s
        return HeadMotion(pitch, yaw, roll, blink, gaze_x, gaze_y)
