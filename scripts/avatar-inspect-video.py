"""Verify the actual encoded silent probe, not only the renderer's frame count."""
import argparse
import hashlib
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", type=Path, required=True)
    args = parser.parse_args()
    report = json.loads((args.run / "report.json").read_text(encoding="utf-8"))
    video = args.run / "silent-controls.mp4"
    output = args.run / "encoded-validation.json"
    if output.exists():
        raise FileExistsError(output)
    import av
    import numpy as np
    result = {"status": "failed", "file_sha256": hashlib.sha256(video.read_bytes()).hexdigest()}
    try:
        with av.open(str(video)) as container:
            if len(container.streams.video) != 1 or len(container.streams.audio) != 0:
                raise ValueError("expected one video stream and zero audio streams")
            stream = container.streams.video[0]
            if float(stream.average_rate) != report["target_fps"]:
                raise ValueError("encoded frame rate mismatch")
            times, changes = [], []
            previous = None
            for frame in container.decode(stream):
                if [frame.width, frame.height] != report["native_size"]:
                    raise ValueError("encoded frame size mismatch")
                time_s = float(frame.pts * frame.time_base)
                if times and abs(time_s - times[-1] - 1 / report["target_fps"]) > 0.0001:
                    raise ValueError("non-contiguous encoded timestamps")
                times.append(time_s)
                current = frame.to_ndarray(format="rgb24").astype(np.float32)
                if previous is not None:
                    changes.append(float(np.abs(current - previous).mean()))
                previous = current
            if len(times) != report["frame_count"] or not times or abs(times[0]) > 0.0001:
                raise ValueError("encoded frame count or starting timestamp mismatch")
            result.update({"status": "passed", "decoded_frames": len(times), "audio_streams": 0,
                           "video_size": report["native_size"], "encoded_fps": float(stream.average_rate),
                           "duration_seconds": times[-1] + 1 / report["target_fps"],
                           "mean_adjacent_pixel_change": float(np.mean(changes)) if changes else 0.0,
                           "visual_naturalness": "requires_separate_review",
                           "actual_device_output": "not_tested"})
    except Exception as exc:
        result["error"] = str(exc)
        raise
    finally:
        with output.open("x", encoding="utf-8") as handle:
            json.dump(result, handle, indent=2)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
