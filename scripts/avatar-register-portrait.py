"""Register a local photo, render a neutral preview, then publish a new profile.

Requires an explicitly selected photo. Never captures, plays, uploads or replaces
an existing profile. Failed attempts remain inspectable and are not loadable.
"""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import sys
import time
import uuid

os.environ["HF_HUB_OFFLINE"] = "1"
os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"
os.environ["TORCH_FORCE_WEIGHTS_ONLY_LOAD"] = "1"
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from avatar.portrait_profile import PortraitPreprocessor, write_profile_assets, load_profile, digest, PROFILE_FILES
from avatar.liveportrait_renderer import LivePortraitRenderer, HeadMotion, UPSTREAM_COMMIT


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--detector", type=Path, required=True)
    parser.add_argument("--photo", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--name", required=True)
    parser.add_argument("--source-kind", choices=("synthetic", "user_selected"), required=True)
    args = parser.parse_args()
    if not args.name.strip() or len(args.name) > 80 or any(ord(c) < 32 for c in args.name):
        parser.error("name must be 1..80 characters without control characters")
    root = Path(__file__).resolve().parents[1]
    destination = args.output.resolve()
    if destination == root or root in destination.parents:
        parser.error("profiles must be kept outside the source checkout")
    destination.mkdir(parents=True, exist_ok=False)
    started = time.perf_counter()
    report = {"schema_version": 1, "profile_id": str(uuid.uuid4()), "name": args.name.strip(),
              "created_at": datetime.now(timezone.utc).isoformat(), "source_kind": args.source_kind,
              "upstream_commit": UPSTREAM_COMMIT, "status": "preparing", "capture": False,
              "playback": False, "network_inference": False, "virtual_output": False,
              "identity_verified": False, "personal_likeness_accepted": False}
    try:
        from PIL import Image
        processor = PortraitPreprocessor(args.source, args.weights, args.detector)
        draft = processor.prepare(args.photo)
        write_profile_assets(destination, draft)
        report["preparation"] = draft.report
        report["preparation_ms"] = (time.perf_counter()-started)*1000
        renderer = LivePortraitRenderer(args.source, args.weights)
        renderer.prepare(draft.portrait, landmarks=draft.landmarks)
        preview = renderer.render(HeadMotion())
        with (destination / "preview.png").open("xb") as handle:
            Image.fromarray(preview).save(handle, format="PNG")
        report.update(status="preview_ready", source_eye_ratio=renderer.eye_open,
                      capabilities=renderer.capabilities, total_ms=(time.perf_counter()-started)*1000,
                      files={name: digest(destination / name) for name in PROFILE_FILES})
        with (destination / "profile.json").open("x", encoding="utf-8") as handle:
            json.dump(report, handle, ensure_ascii=False, indent=2)
        load_profile(destination)
        print(json.dumps({"status": report["status"], "profile_id": report["profile_id"],
                          "output": str(destination), "warnings": draft.report["warnings"]}, ensure_ascii=False))
    except Exception as error:
        report.update(status="failed", error=str(error), total_ms=(time.perf_counter()-started)*1000)
        with (destination / "failure.json").open("x", encoding="utf-8") as handle:
            json.dump(report, handle, ensure_ascii=False, indent=2)
        raise


if __name__ == "__main__":
    main()
