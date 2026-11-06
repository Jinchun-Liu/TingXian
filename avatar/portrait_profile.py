"""Local single-person photo preparation and verified, immutable avatar profiles.

No capture, networking or identity recognition. YuNet locates a face; the pinned
LivePortrait landmark model supplies geometry, not evidence of identity/consent.
"""
from dataclasses import dataclass
import hashlib
import io
import json
import math
from pathlib import Path
import warnings

import numpy as np
from PIL import Image, ImageOps

YUNET_REVISION = "48b10fa505aeb51fbd127fc60a5c660040e823b7"
YUNET_FILE = "face_detection_yunet_2023mar.onnx"
YUNET_SHA256 = "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4"
LANDMARK_SHA256 = "31d22a5041326c31f19b78886939a634a5aedcaa5ab8b9b951a1167595d147db"
MAX_PHOTO_BYTES = 20 * 1024 * 1024
MAX_PHOTO_PIXELS = 16_000_000
PROFILE_FILES = ("normalized.png", "portrait.png", "landmarks.json", "preview.png")


def digest(path: Path) -> str:
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def read_photo(path: Path):
    """Decode once, honor orientation and strip metadata from the returned RGB."""
    with path.open("rb") as handle:
        data = handle.read(MAX_PHOTO_BYTES + 1)
    if not data or len(data) > MAX_PHOTO_BYTES:
        raise ValueError("请选择 20 MB 以内的照片。")
    with warnings.catch_warnings():
        warnings.simplefilter("error", Image.DecompressionBombWarning)
        with Image.open(io.BytesIO(data)) as image:
            if image.format not in {"JPEG", "PNG", "WEBP"} or getattr(image, "n_frames", 1) != 1:
                raise ValueError("请选择单张 JPEG、PNG 或 WebP 照片。")
            width, height = image.size
            if min(width, height) < 256 or width * height > MAX_PHOTO_PIXELS or max(width, height) > 8192:
                raise ValueError("照片短边至少 256 像素，长边不超过 8192 像素，总像素不超过 1600 万。")
            oriented = ImageOps.exif_transpose(image)
            rgba = oriented.convert("RGBA")
            background = Image.new("RGBA", rgba.size, "white")
            rgb = np.asarray(Image.alpha_composite(background, rgba).convert("RGB")).copy()
    return rgb, {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data),
                 "stored_size": [width, height], "oriented_size": [rgb.shape[1], rgb.shape[0]],
                 "metadata_retained": False, "transparent_background": "flattened_to_white"}


def single_face(rows, width: int, height: int):
    """Do not silently choose the largest person in a group picture."""
    if rows is None or len(rows) == 0:
        raise ValueError("未找到清晰的人脸，请选择正面、光线充足的单人照片。")
    if len(rows) != 1:
        raise ValueError("照片中有多张人脸，请先裁出要使用的人物。")
    row = np.asarray(rows[0], dtype=np.float64)
    if row.shape != (15,) or not np.isfinite(row).all() or not .9 <= row[14] <= 1:
        raise ValueError("人脸检测结果无效。")
    x, y, w, h = row[:4]
    if min(w, h) < 96 or x < 0 or y < 0 or x+w > width or y+h > height:
        raise ValueError("人脸太小或被画面边缘裁断，请换用更完整的近景照片。")
    points = row[4:14].reshape(5, 2)
    if (points < 0).any() or (points[:, 0] >= width).any() or (points[:, 1] >= height).any():
        raise ValueError("人脸关键点超出照片范围。")
    eyes = points[:2][np.argsort(points[:2, 0])]
    axis = eyes[1]-eyes[0]
    separation = float(np.linalg.norm(axis))
    if separation < 32 or abs(math.degrees(math.atan2(axis[1], axis[0]))) > 30:
        raise ValueError("请使用更清晰、头部较正的照片。")
    eye_mid, lip_mid = eyes.mean(axis=0), points[3:].mean(axis=0)
    if lip_mid[1] <= eye_mid[1] or abs(float((points[2]-eye_mid) @ (axis/separation))) > separation*.6:
        raise ValueError("请使用能看清双眼、鼻子和嘴部的正面照片。")
    return row, points.astype(np.float32)


@dataclass
class PortraitDraft:
    normalized: np.ndarray
    portrait: np.ndarray
    landmarks: np.ndarray
    report: dict


def fit_crop_to_source(matrix, width: int, height: int):
    """Translate the crop within available source pixels; never invent border skin.

    Preserve scale/rotation. If the rotated square cannot fit, request a wider
    photo rather than stretching pixels or hiding a large missing region.
    """
    matrix = np.asarray(matrix, dtype=np.float64)
    if matrix.shape != (3, 3) or not np.isfinite(matrix).all():
        raise ValueError("裁切变换无效。")
    inverse = np.linalg.inv(matrix)
    corners = np.array([[0, 0, 1], [511, 0, 1], [511, 511, 1], [0, 511, 1]]) @ inverse.T
    low, high = corners[:, :2].min(axis=0), corners[:, :2].max(axis=0)
    lower = 1-low
    upper = np.array([width-2, height-2])-high
    if (lower > upper).any():
        raise ValueError("照片周围留白不足，请换用包含完整头部和肩部的照片。")
    shift = np.clip(np.zeros(2), lower, upper)
    if np.linalg.norm(shift) > max(high-low)*.2:
        raise ValueError("头部太靠近照片边缘，请使用四周留有空间的照片。")
    result = matrix.copy()
    result[:2, 2] -= result[:2, :2] @ shift
    return result, shift


def load_yunet(detector_path: Path):
    import cv2
    # OpenCV's native filename loader can fail on Windows Unicode paths. Read with
    # Python's Unicode-safe file API, verify those exact bytes, then use buffer IO.
    with detector_path.open("rb") as handle:
        model_bytes = handle.read(232590)
    if len(model_bytes) != 232589 or hashlib.sha256(model_bytes).hexdigest() != YUNET_SHA256:
        raise ValueError("人脸检测模型校验失败。")
    return cv2.FaceDetectorYN.create("onnx", np.frombuffer(model_bytes, dtype=np.uint8),
                                    np.empty(0, dtype=np.uint8), (512, 512), .9, .3, 5000,
                                    cv2.dnn.DNN_BACKEND_OPENCV, cv2.dnn.DNN_TARGET_CPU)


class PortraitPreprocessor:
    def __init__(self, source_root: Path, weights_root: Path, detector_path: Path):
        import cv2
        from .liveportrait_renderer import load_upstream
        import importlib
        self.detector = load_yunet(detector_path)
        landmark_path = weights_root / "liveportrait/landmark.onnx"
        if digest(landmark_path) != LANDMARK_SHA256:
            raise ValueError("人脸关键点模型校验失败。")
        _, _, landmark_module = load_upstream(source_root)
        self.landmark = landmark_module.LandmarkRunner(ckpt_path=str(landmark_path), onnx_provider="cpu")
        self.crop_image = importlib.import_module("_tingxian_liveportrait.utils.crop").crop_image

    def prepare(self, path: Path) -> PortraitDraft:
        import cv2
        rgb, source = read_photo(path)
        height, width = rgb.shape[:2]
        ratio = min(1., 512 / max(width, height))
        size = (max(1, round(width*ratio)), max(1, round(height*ratio)))
        bgr = cv2.cvtColor(cv2.resize(rgb, size, interpolation=cv2.INTER_AREA), cv2.COLOR_RGB2BGR)
        self.detector.setInputSize(size)
        _, rows = self.detector.detect(bgr)
        if rows is not None:
            # Rounding can give distinct x/y resize factors; preserve source coordinates.
            rows = rows.copy()
            rows[:, [0, 2, 4, 6, 8, 10, 12]] *= width/size[0]
            rows[:, [1, 3, 5, 7, 9, 11, 13]] *= height/size[1]
        row, points = single_face(rows, width, height)
        landmarks = self.landmark.run(rgb, points)
        if landmarks.shape != (203, 2) or not np.isfinite(landmarks).all():
            raise ValueError("人脸关键点注册失败，请换用正面照片。")
        eye_mid = landmarks[[0, 6, 12, 18, 24, 30, 36, 42]].mean(axis=0)
        if np.linalg.norm(eye_mid-points[:2].mean(axis=0)) > np.linalg.norm(points[1]-points[0])*.3:
            raise ValueError("人脸关键点与检测位置不一致，已停止注册。")
        cropped = self.crop_image(rgb, landmarks, dsize=512, scale=2.3, vy_ratio=-.125, flag_do_rot=True)
        matrix, shift = fit_crop_to_source(cropped["M_o2c"], width, height)
        portrait = cv2.warpAffine(rgb, matrix[:2], (512, 512), flags=cv2.INTER_LINEAR)
        crop_landmarks = landmarks @ matrix[:2, :2].T + matrix[:2, 2]
        if (crop_landmarks < 0).any() or (crop_landmarks > 512).any():
            raise ValueError("裁切后五官不完整，请换用更完整的正面照片。")
        notes = []
        if np.linalg.norm(shift) > 1:
            notes.append("已调整取景位置以保留完整画面，请检查预览。")
        x, y, w, h = row[:4].astype(int)
        sharpness = float(cv2.Laplacian(cv2.cvtColor(rgb[y:y+h, x:x+w], cv2.COLOR_RGB2GRAY), cv2.CV_64F).var())
        if sharpness < 50:
            notes.append("照片可能较模糊，请在预览中检查五官。")
        return PortraitDraft(rgb, portrait, crop_landmarks, {
            "source": source, "detector": {"name": "YuNet", "revision": YUNET_REVISION,
            "sha256": YUNET_SHA256, "input_size": list(size), "score": float(row[14]),
            "face_count": 1, "box": row[:4].tolist()}, "landmark_sha256": LANDMARK_SHA256,
            "source_to_crop": matrix.tolist(), "crop_size": [512, 512],
            "framing_translation_px": shift.tolist(), "border_fill": False, "sharpness_variance": sharpness,
            "quality_scope": "heuristics_only_not_identity_or_naturalness_acceptance", "warnings": notes})


def write_profile_assets(directory: Path, draft: PortraitDraft):
    """Caller reserves a new directory. No replacing old profiles or source images."""
    for name, array in (("normalized.png", draft.normalized), ("portrait.png", draft.portrait)):
        with (directory / name).open("xb") as handle:
            Image.fromarray(array).save(handle, format="PNG")
    with (directory / "landmarks.json").open("x", encoding="utf-8") as handle:
        json.dump({"schema_version": 1, "points": draft.landmarks.tolist()}, handle)


def load_profile(directory: Path):
    """Only complete, digest-matching local profiles can become renderer inputs."""
    manifest_path = directory / "profile.json"
    if (directory / "failure.json").exists() or manifest_path.is_symlink():
        raise ValueError("形象注册未完成或记录路径无效。")
    if manifest_path.stat().st_size > 64 * 1024:
        raise ValueError("形象记录过大。")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    from .liveportrait_renderer import UPSTREAM_COMMIT
    if manifest.get("schema_version") != 1 or manifest.get("status") != "preview_ready" or manifest.get("upstream_commit") != UPSTREAM_COMMIT:
        raise ValueError("形象注册未完成。")
    if set(manifest.get("files", {})) != set(PROFILE_FILES):
        raise ValueError("形象文件清单无效。")
    root = directory.resolve(strict=True)
    for name in PROFILE_FILES:
        path = directory / name
        if path.is_symlink() or path.resolve(strict=True).parent != root or path.stat().st_size > 64*1024*1024:
            raise ValueError("形象文件路径或大小无效。")
        if digest(path) != manifest["files"][name]:
            raise ValueError("形象文件校验失败，请重新导入。")
    rgb, _ = read_photo(directory / "portrait.png")
    if rgb.shape != (512, 512, 3):
        raise ValueError("形象裁切尺寸不一致。")
    if (directory / "landmarks.json").stat().st_size > 32*1024:
        raise ValueError("形象关键点记录过大。")
    geometry = json.loads((directory / "landmarks.json").read_text(encoding="utf-8"))
    points = np.asarray(geometry.get("points"), dtype=np.float32)
    if geometry.get("schema_version") != 1 or points.shape != (203, 2) or not np.isfinite(points).all():
        raise ValueError("形象关键点无效。")
    if (points < 0).any() or (points > 512).any():
        raise ValueError("形象关键点超出裁切范围。")
    return rgb, points, manifest
