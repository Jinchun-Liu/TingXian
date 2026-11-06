"""Synthetic photo/profile boundary tests. Test output is retained in --output."""
import argparse
import json
from pathlib import Path
import sys
import unittest
import uuid

import numpy as np
from PIL import Image, ImageOps, UnidentifiedImageError

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from avatar.portrait_profile import (read_photo, single_face, fit_crop_to_source,
    write_profile_assets, load_profile, PortraitDraft, digest, PROFILE_FILES, load_yunet)
from avatar.liveportrait_renderer import UPSTREAM_COMMIT

TEST_ROOT = None


class PortraitTests(unittest.TestCase):
    def setUp(self):
        self.directory = TEST_ROOT / str(uuid.uuid4())
        self.directory.mkdir()

    def test_orientation_and_metadata_are_separated_from_rgb(self):
        path = self.directory / "带方向的照片.jpg"
        image = Image.new("RGB", (320, 256), (30, 90, 180))
        exif = Image.Exif(); exif[274] = 6; exif[315] = "synthetic author"
        image.save(path, exif=exif)
        before = path.read_bytes()
        rgb, info = read_photo(path)
        self.assertEqual(rgb.shape, (320, 256, 3))
        with Image.open(path) as original:
            np.testing.assert_array_equal(rgb, np.array(ImageOps.exif_transpose(original).convert("RGB")))
        self.assertFalse(info["metadata_retained"])
        self.assertEqual(path.read_bytes(), before)

    def test_invalid_animated_and_oversized_photos_fail_before_registration(self):
        invalid = self.directory / "broken.png"; invalid.write_bytes(b"nope")
        with self.assertRaises(UnidentifiedImageError): read_photo(invalid)
        huge = self.directory / "too-wide.png"; Image.new("RGB", (8193, 256)).save(huge)
        with self.assertRaisesRegex(ValueError, "像素"): read_photo(huge)
        gif = self.directory / "animated.gif"
        Image.new("RGB", (256, 256)).save(gif, save_all=True, append_images=[Image.new("RGB", (256, 256), "white")], duration=100)
        with self.assertRaisesRegex(ValueError, "单张"): read_photo(gif)

    def test_group_photos_missing_faces_and_bad_geometry_are_not_auto_selected(self):
        row = np.array([80, 60, 180, 220, 120, 130, 220, 132, 170, 180, 140, 230, 200, 230, .95])
        single_face([row], 512, 512)
        for rows, message in ((None, "未找到"), ([row, row], "多张"), ([row.copy()*np.nan], "无效")):
            with self.assertRaisesRegex(ValueError, message): single_face(rows, 512, 512)
        tiny = row.copy(); tiny[2] = 50
        with self.assertRaisesRegex(ValueError, "太小"): single_face([tiny], 512, 512)

    def test_crop_fit_preserves_scale_rotation_and_contains_every_output_corner(self):
        matrix = np.array([[1.1, -.02, -10], [.02, 1.1, 22], [0, 0, 1]])
        fitted, shift = fit_crop_to_source(matrix, 512, 512)
        np.testing.assert_array_equal(fitted[:2, :2], matrix[:2, :2])
        corners = np.array([[0, 0, 1], [511, 0, 1], [511, 511, 1], [0, 511, 1]]) @ np.linalg.inv(fitted).T
        self.assertTrue((corners[:, :2] >= .999).all())
        self.assertTrue((corners[:, :2] <= 510.001).all())
        self.assertGreater(np.linalg.norm(shift), 0)
        with self.assertRaisesRegex(ValueError, "留白"): fit_crop_to_source(np.eye(3), 256, 256)

    def make_profile(self):
        rgb = np.full((512, 512, 3), 128, np.uint8)
        points = np.full((203, 2), 256, np.float32)
        write_profile_assets(self.directory, PortraitDraft(rgb, rgb, points, {}))
        Image.fromarray(rgb).save(self.directory / "preview.png")
        manifest = {"schema_version": 1, "status": "preview_ready", "upstream_commit": UPSTREAM_COMMIT,
                    "files": {name: digest(self.directory / name) for name in PROFILE_FILES}}
        (self.directory / "profile.json").write_text(json.dumps(manifest), encoding="utf-8")
        return rgb, points, manifest

    def test_profile_reload_preserves_pixels_and_geometry_without_metadata(self):
        rgb, points, _ = self.make_profile()
        actual, geometry, _ = load_profile(self.directory)
        np.testing.assert_array_equal(actual, rgb); np.testing.assert_array_equal(geometry, points)
        with Image.open(self.directory / "portrait.png") as photo: self.assertFalse(photo.getexif())
        with self.assertRaises(FileExistsError): write_profile_assets(self.directory, PortraitDraft(rgb, rgb, points, {}))

    def test_incomplete_tampered_and_external_profile_files_are_rejected(self):
        _, _, manifest = self.make_profile()
        path = self.directory / "profile.json"
        manifest["files"]["../elsewhere.png"] = manifest["files"].pop("preview.png")
        path.write_text(json.dumps(manifest), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "清单"): load_profile(self.directory)
        manifest["files"]["preview.png"] = manifest["files"].pop("../elsewhere.png")
        path.write_text(json.dumps(manifest), encoding="utf-8")
        (self.directory / "preview.png").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "校验"): load_profile(self.directory)
        (self.directory / "failure.json").write_text("{}", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "未完成"): load_profile(self.directory)

    def test_corrupt_detector_is_rejected_before_native_onnx_loading(self):
        path = self.directory / "坏模型.onnx"; path.write_bytes(b"invalid")
        with self.assertRaisesRegex(ValueError, "模型校验"): load_yunet(path)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    TEST_ROOT = args.output
    TEST_ROOT.mkdir(parents=True, exist_ok=False)
    unittest.main(argv=[sys.argv[0]], verbosity=2)
