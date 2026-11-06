"""Render a silent, synthetic control sequence; never opens capture or playback.

E04: Can the pinned CUDA model produce controlled frames within a 40 ms frame
budget? Stop after ten seconds of media, an exception, or the wall-time ceiling.
This measures a file renderer, not delivery to a virtual device or meeting.
"""
import argparse
import csv
import hashlib
import json
import math
import os
from pathlib import Path
import sys
import time
from fractions import Fraction

os.environ["HF_HUB_OFFLINE"] = "1"
os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"
os.environ["TORCH_FORCE_WEIGHTS_ONLY_LOAD"] = "1"
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from avatar.liveportrait_renderer import HeadMotion, LivePortraitRenderer, UPSTREAM_COMMIT
from avatar.idle_motion import IdleMotion


class SilentVideoWriter:
    def __init__(self, path):
        import av
        self.av = av
        self.container = av.open(str(path), mode="w")
        self.stream = self.container.add_stream("libx264", rate=25)
        self.stream.width = self.stream.height = 512
        self.stream.pix_fmt = "yuv420p"
        self.stream.options = {"crf": "18", "preset": "fast"}
        self.index = 0
        self.closed = False

    def append_data(self, rgb):
        frame = self.av.VideoFrame.from_ndarray(rgb, format="rgb24")
        frame.pts = self.index
        frame.time_base = Fraction(1, 25)
        for packet in self.stream.encode(frame):
            self.container.mux(packet)
        self.index += 1

    def close(self):
        if self.closed:
            return
        self.closed = True
        try:
            for packet in self.stream.encode():
                self.container.mux(packet)
        finally:
            self.container.close()


def controls(t):
    # A repeatable control fixture, not a claimed natural gaze/idle algorithm.
    blink = max((max(0.0, 1.0 - abs(t - center) / 0.16)
                 for center in (1.6, 4.4, 7.8)), default=0.0)
    return HeadMotion(1.5 * math.sin(t * 0.7), 4 * math.sin(t * 0.55),
                      0.7 * math.sin(t * 0.9), blink)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    portrait_source = parser.add_mutually_exclusive_group(required=True)
    portrait_source.add_argument("--portrait", type=Path)
    portrait_source.add_argument("--profile", type=Path, help="load a complete, verified local portrait profile")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seconds", type=int, default=10, choices=range(1, 11))
    parser.add_argument("--cudnn-benchmark", action="store_true",
                        help="test cuDNN kernel selection for this fixed portrait size")
    parser.add_argument("--sequence", choices=("controls", "idle"), default="controls")
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args()
    # Fail rather than overwrite a previous result or user file.
    args.output.mkdir(parents=True, exist_ok=False)
    import numpy as np
    from PIL import Image
    import torch
    torch.backends.cudnn.benchmark = args.cudnn_benchmark

    report = {"scope": "offline_frame_render_with_synthetic_motion", "upstream_commit": UPSTREAM_COMMIT,
              "capture": False, "playback": False, "network_inference": False,
              "audio": False, "virtual_device": False, "meeting": False,
              "target_fps": 25, "native_size": [512, 512], "status": "started",
              "cudnn_benchmark": args.cudnn_benchmark,
              "motion_source": "synthetic_" + args.sequence, "motion_seed": args.seed,
              "timing_boundary": "render, CPU transfer, encode and flush; PNG writes excluded"}
    writer = None
    rows = []
    snapshots = {}
    try:
        registered_landmarks = None
        if args.profile:
            from avatar.portrait_profile import load_profile
            rgb, registered_landmarks, profile = load_profile(args.profile)
            report["profile_id"] = profile["profile_id"]
            report["portrait_source_kind"] = profile.get("source_kind", "unclassified_local_file")
            report["portrait_sha256"] = profile["files"]["portrait.png"]
        else:
            report["portrait_source_kind"] = "unclassified_local_file"
            report["portrait_sha256"] = hashlib.sha256(args.portrait.read_bytes()).hexdigest()
            rgb = np.array(Image.open(args.portrait).convert("RGB"))
        t0 = time.perf_counter()
        renderer = LivePortraitRenderer(args.source, args.weights)
        renderer.prepare(rgb, landmarks=registered_landmarks)
        torch.cuda.synchronize()
        report["load_and_prepare_ms"] = (time.perf_counter() - t0) * 1000
        report["source_eye_ratio"] = renderer.eye_open
        report["capabilities"] = renderer.capabilities
        report["device"] = torch.cuda.get_device_name(0)
        report["torch"] = torch.__version__
        report["cuda_runtime"] = torch.version.cuda
        # Record cold first frame separately, then warm up without counting FPS.
        t0 = time.perf_counter()
        first = renderer.render(HeadMotion())
        torch.cuda.synchronize()
        report["cold_first_frame_ms"] = (time.perf_counter() - t0) * 1000
        Image.fromarray(first).save(args.output / "neutral.png")
        for _ in range(3):
            renderer.render(HeadMotion())
        torch.cuda.synchronize()
        torch.cuda.reset_peak_memory_stats()
        writer = SilentVideoWriter(args.output / "silent-controls.mp4")
        motion_at = IdleMotion(args.seed).sample if args.sequence == "idle" else controls
        peak_blink = -1.0
        wall_start = time.perf_counter()
        for index in range(args.seconds * 25):
            if time.perf_counter() - wall_start > 120:
                raise TimeoutError("120 second render ceiling exceeded")
            t0 = time.perf_counter()
            motion = motion_at(index / 25)
            frame = renderer.render(motion)
            torch.cuda.synchronize()
            render_ms = (time.perf_counter() - t0) * 1000
            if frame.shape != (512, 512, 3) or frame.dtype != np.uint8:
                raise ValueError("unexpected renderer output")
            writer.append_data(frame)
            full_ms = (time.perf_counter() - t0) * 1000
            rows.append({"frame": index, "pts_ms": index * 40,
                         "render_ms": render_ms, "render_and_encode_submit_ms": full_ms,
                         "blink": motion.blink, "yaw_deg": motion.yaw_deg,
                         "pitch_deg": motion.pitch_deg, "roll_deg": motion.roll_deg,
                         "gaze_x": motion.gaze_x, "gaze_y": motion.gaze_y})
            if motion.blink > peak_blink:
                peak_blink = motion.blink
                snapshots["peak-blink"] = frame.copy()
            if index in (0, 40, 80, 110, 160, 195, 249):
                snapshots[index] = frame.copy()
        writer.close()
        writer = None
        elapsed = time.perf_counter() - wall_start
        report.update({"status": "rendered", "frame_count": len(rows),
                       "render_encode_wall_seconds": elapsed,
                       "offline_throughput_fps": len(rows) / elapsed,
                       "render_p50_ms": float(np.percentile([r["render_ms"] for r in rows], 50)),
                       "render_p95_ms": float(np.percentile([r["render_ms"] for r in rows], 95)),
                       "peak_cuda_allocated_bytes": torch.cuda.max_memory_allocated(),
                       "peak_cuda_reserved_bytes": torch.cuda.max_memory_reserved(),
                       "visual_review": "pending", "real_time_output_acceptance": "not_tested"})
        # Disk-heavy evidence capture must not distort renderer/encoder throughput.
        for index, frame in snapshots.items():
            name = f"frame-{index:04d}.png" if isinstance(index, int) else f"{index}.png"
            Image.fromarray(frame).save(args.output / name)
    except Exception as exc:
        report.update({"status": "failed", "error_type": type(exc).__name__, "error": str(exc)})
        raise
    finally:
        try:
            if writer is not None:
                writer.close()
        except Exception as cleanup_exc:
            report["cleanup_error"] = {
                "type": type(cleanup_exc).__name__, "error": str(cleanup_exc)
            }
            # A failing encoder flush must not replace the original render error.
            if report["status"] != "failed":
                report["status"] = "failed"
                raise
        finally:
            (args.output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
            if rows:
                with (args.output / "frames.csv").open("w", newline="", encoding="utf-8") as handle:
                    output = csv.DictWriter(handle, fieldnames=rows[0].keys())
                    output.writeheader()
                    output.writerows(rows)
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
