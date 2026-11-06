"""Relative gaze expression offsets for the pinned LivePortrait model.

Adapted from GradioPipeline.update_delta_new_eyeball_direction in LivePortrait
9b294b3d, Copyright (c) 2024 Kuaishou Visual Generation and Interaction Center.
MIT license: licenses/LivePortrait-MIT.txt. Values are artistic controls, NOT
calibrated eye angles or measured user attention. No device access.
"""
import math


def gaze_offsets(x: float, y: float, blink: float = 0.0):
    """Return (keypoint, coordinate, delta) without mutating source identity.

    x/y are normalized relative controls in [-1,1]. Positive x follows the
    upstream image-horizontal direction; positive y raises gaze. Deliberately
    use a smaller range than upstream's editor. Attenuate while eyes close so
    gaze deformation does not fight the separate eye-closing retargeter.
    """
    if not all(math.isfinite(v) for v in (x, y, blink)):
        raise ValueError("gaze controls must be finite")
    if abs(x) > 1 or abs(y) > 1 or not 0 <= blink <= 1:
        raise ValueError("gaze must be in [-1,1]; blink in [0,1]")
    visibility = (1.0 - blink) ** 2
    horizontal, vertical = x * 8.0 * visibility, y * 6.0 * visibility
    # Upstream vertical gaze includes a coordinated eyelid offset.
    eyelid = -vertical / 2.0
    return (
        (11, 0, horizontal * (0.0007 if horizontal > 0 else 0.001)),
        (15, 0, horizontal * (0.001 if horizontal > 0 else 0.0007)),
        (11, 1, -0.001 * (vertical + eyelid)),
        (15, 1, -0.001 * (vertical + eyelid)),
        (13, 1, eyelid * 0.0003),
        (16, 1, eyelid * 0.0003),
    )
