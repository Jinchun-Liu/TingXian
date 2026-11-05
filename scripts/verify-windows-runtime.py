"""Verify installed resources against the manifest produced before bundling."""
import hashlib
import json
from pathlib import Path
import sys

root = Path(sys.argv[1]).resolve()
manifest = json.loads((root / "runtime-manifest.json").read_text(encoding="utf-8"))
for relative, expected in manifest.items():
    deployed = relative.removeprefix("media/")
    path = (root / deployed).resolve()
    if not path.is_relative_to(root):
        raise SystemExit("Manifest path escapes installation")
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for part in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(part)
    if h.hexdigest() != expected:
        raise SystemExit(f"Installed runtime hash mismatch: {deployed}")
print(f"PASS: {len(manifest)} installed runtime files match the build manifest")
