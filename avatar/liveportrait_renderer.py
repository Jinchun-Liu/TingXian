"""Optional, file-only LivePortrait CUDA renderer. No device capture or network.

The initial adapter accepts a prepared square portrait crop, not arbitrary photo
registration. The upstream detector-free landmark pass is only suitable for this
controlled input. Camera tracking, audio lip sync and virtual devices are separate.
"""
from dataclasses import dataclass
import importlib
import math
from pathlib import Path
import subprocess
import sys
import types
from .gaze import gaze_offsets

UPSTREAM_COMMIT = "9b294b3d0536135442ea73cb01e6cb3ca7029dd3"


@dataclass(frozen=True)
class HeadMotion:
    pitch_deg: float = 0.0
    yaw_deg: float = 0.0
    roll_deg: float = 0.0
    blink: float = 0.0  # 0 = source eye opening; 1 = closed, both eyes
    gaze_x: float = 0.0  # relative artistic adjustment, NOT eye angle in degrees
    gaze_y: float = 0.0

    def __post_init__(self):
        values = (self.pitch_deg, self.yaw_deg, self.roll_deg, self.blink,
                  self.gaze_x, self.gaze_y)
        if not all(math.isfinite(v) for v in values):
            raise ValueError("motion controls must be finite")
        if max(abs(v) for v in values[:3]) > 15 or not 0 <= self.blink <= 1:
            raise ValueError("prototype head angles must be within 15 degrees; blink within [0,1]")
        if abs(self.gaze_x) > 1 or abs(self.gaze_y) > 1:
            raise ValueError("relative gaze controls must be within [-1,1]")


def load_upstream(source_root: Path):
    source_root = source_root.resolve(strict=True)
    actual = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=source_root, text=True
    ).strip()
    if actual != UPSTREAM_COMMIT:
        raise ValueError("unverified LivePortrait revision; review and pin it first")
    changed = subprocess.check_output(
        ["git", "status", "--porcelain", "--", "src"], cwd=source_root, text=True
    ).strip()
    if changed:
        raise ValueError("LivePortrait source has local modifications; review before loading")
    # Keep upstream's generic 'src' namespace out of the application's imports.
    name = "_tingxian_liveportrait"
    if name in sys.modules:
        if sys.modules[name].__path__ != [str(source_root / "src")]:
            raise RuntimeError("a different LivePortrait source is already loaded")
    else:
        package = types.ModuleType(name)
        package.__path__ = [str(source_root / "src")]
        package.__package__ = name
        sys.modules[name] = package
    wrapper = importlib.import_module(name + ".live_portrait_wrapper")
    config = importlib.import_module(name + ".config.inference_config")
    landmark = importlib.import_module(name + ".utils.human_landmark_runner")
    return wrapper, config, landmark


class LivePortraitRenderer:
    capabilities = {
        "head_pose": True,
        "blink": True,
        "gaze": True,
        "gaze_tracking": False,
        "gaze_units": "relative_uncalibrated",
        "body_tracking": False,
        "audio_lip_sync": False,
        "camera_capture": False,
        "virtual_output": False,
        "native_output_size": [512, 512],
    }

    def __init__(self, source_root: Path, weights_root: Path, *, device_id: int = 0):
        import torch
        if not torch.cuda.is_available():
            raise RuntimeError("CUDA unavailable; this probe does not silently fall back to CPU")
        if not 0 <= device_id < torch.cuda.device_count():
            raise ValueError("invalid CUDA device")
        self.torch = torch
        wrapper, config, landmark = load_upstream(source_root)
        models = weights_root.resolve(strict=True) / "liveportrait"
        files = {
            "checkpoint_F": models / "base_models/appearance_feature_extractor.pth",
            "checkpoint_M": models / "base_models/motion_extractor.pth",
            "checkpoint_W": models / "base_models/warping_module.pth",
            "checkpoint_G": models / "base_models/spade_generator.pth",
            "checkpoint_S": models / "retargeting_models/stitching_retargeting_module.pth",
        }
        for path in [*files.values(), models / "landmark.onnx"]:
            if not path.is_file():
                raise FileNotFoundError(path)
        cfg = config.InferenceConfig(
            **{key: str(path) for key, path in files.items()},
            device_id=device_id, flag_use_half_precision=True,
            flag_do_torch_compile=False, mask_crop=None, lip_array=None,
        )
        self.wrapper = wrapper.LivePortraitWrapper(cfg)
        # One-time landmark registration on the provided crop, explicitly CPU.
        self.landmark = landmark.LandmarkRunner(
            ckpt_path=str(models / "landmark.onnx"), onnx_provider="cpu"
        )
        self._prepared = False

    def prepare(self, rgb):
        import numpy as np
        if rgb.dtype != np.uint8 or rgb.ndim != 3 or rgb.shape[2] != 3:
            raise ValueError("portrait must be an RGB uint8 array")
        if rgb.shape[0] != rgb.shape[1] or not 256 <= rgb.shape[0] <= 2048:
            raise ValueError("provide a prepared square face crop, 256..2048 pixels")
        self._prepared = False
        image = self.wrapper.prepare_source(rgb)
        self.info = self.wrapper.get_kp_info(image)
        self.source_kp = self.wrapper.transform_keypoint(self.info)
        self.feature = self.wrapper.extract_feature_3d(image)
        landmarks = self.landmark.run(rgb)
        self.eye_ratios = self.wrapper.calc_combined_eye_ratio([[0.0]], landmarks)
        self.eye_open = float(self.eye_ratios[0, :2].mean().item())
        if not math.isfinite(self.eye_open) or not 0.03 < self.eye_open < 0.8:
            raise ValueError("source eye registration outside prototype range; inspect portrait crop")
        self._prepared = True

    def render(self, motion: HeadMotion):
        if not self._prepared:
            raise RuntimeError("prepare a portrait first")
        info = dict(self.info)
        info["pitch"] = info["pitch"] + motion.pitch_deg
        info["yaw"] = info["yaw"] + motion.yaw_deg
        info["roll"] = info["roll"] + motion.roll_deg
        if motion.gaze_x or motion.gaze_y:
            # Never edit the cached source expression: otherwise idle motions drift.
            info["exp"] = info["exp"].clone()
            for point, axis, offset in gaze_offsets(motion.gaze_x, motion.gaze_y, motion.blink):
                info["exp"][0, point, axis] += offset
        driving = self.wrapper.transform_keypoint(info)
        if motion.blink > 0:
            ratios = self.eye_ratios.clone()
            ratios[:, 2] = self.eye_open * (1.0 - motion.blink)
            driving = driving + self.wrapper.retarget_eye(self.source_kp, ratios)
        driving = self.wrapper.stitching(self.source_kp, driving)
        result = self.wrapper.warp_decode(self.feature, self.source_kp, driving)
        return self.wrapper.parse_output(result["out"])[0]
