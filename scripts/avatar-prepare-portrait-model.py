"""Download only the pinned public YuNet model and license; never install packages."""
import argparse
import hashlib
from pathlib import Path
import sys
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
# Keep acquisition independent of numpy, Pillow, OpenCV, torch or a GPU.
REVISION = "48b10fa505aeb51fbd127fc60a5c660040e823b7"
FILES = [
    ("face_detection_yunet_2023mar.onnx", 232589,
     "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4", "media.githubusercontent.com/media"),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    destination = args.output.resolve()
    checkout = Path(__file__).resolve().parents[1]
    if destination == checkout or checkout in destination.parents:
        parser.error("model output must be outside the source checkout")
    destination.mkdir(parents=True, exist_ok=True)
    for name, size, sha256, host in FILES:
        target = destination / name
        if target.exists():
            with target.open("rb") as handle:
                data = handle.read(size+1)
        else:
            url = f"https://{host}/opencv/opencv_zoo/{REVISION}/models/face_detection_yunet/{name}"
            with urllib.request.urlopen(url, timeout=30) as response:
                data = response.read(size+1)
        if len(data) != size or hashlib.sha256(data).hexdigest() != sha256:
            raise ValueError(f"Integrity check failed for {name}; existing files were preserved")
        if not target.exists():
            with target.open("xb") as handle:
                handle.write(data)
        print(f"VERIFIED {name}: {size} bytes {sha256}")
    license_bytes = (checkout / "avatar/licenses/YuNet-MIT.txt").read_text(encoding="utf-8").encode("utf-8")
    license_path = destination / "LICENSE"
    if license_path.exists():
        if license_path.read_text(encoding="utf-8").encode("utf-8") != license_bytes:
            raise ValueError("Existing license differs; preserved for review")
    else:
        with license_path.open("xb") as handle:
            handle.write(license_bytes)


if __name__ == "__main__":
    main()
