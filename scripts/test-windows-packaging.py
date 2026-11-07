"""Contract checks for the Windows distribution and installed-path verifier."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[1]
cfg = json.loads((root / "frontend/src-tauri/tauri.windows.conf.json").read_text())
assert cfg["bundle"]["windows"]["webviewInstallMode"]["type"] == "offlineInstaller"
assert cfg["bundle"]["windows"]["nsis"]["installMode"] == "currentUser"
assert cfg["bundle"]["windows"]["signCommand"] is None
assert "python/" in cfg["bundle"]["resources"].values()
assert "whisper/" in cfg["bundle"]["resources"].values()
for filename in ("tauri.conf.json", "tauri.windows.conf.json"):
    resources = json.loads((root / "frontend/src-tauri" / filename).read_text(encoding="utf-8"))["bundle"]["resources"]
    assert resources["../../education/NOTICE.md"] == "licenses/education/NOTICE.md"
    assert resources["../../education/licenses/"] == "licenses/education/"
for name in ("OpenMAIC-MIT.txt", "ai-teaching-skills-MIT.txt"):
    license_text = (root / "education/licenses" / name).read_text(encoding="utf-8")
    assert "Copyright" in license_text and "Permission is hereby granted" in license_text
with tempfile.TemporaryDirectory(prefix="tingxian-runtime-test-") as folder:
    stage = Path(folder)
    content = b"synthetic media runtime"
    (stage / "ffmpeg.exe").write_bytes(content)
    manifest = {"media/ffmpeg.exe": hashlib.sha256(content).hexdigest()}
    (stage / "runtime-manifest.json").write_text(json.dumps(manifest))
    command = [sys.executable, str(root / "scripts/verify-windows-runtime.py"), str(stage)]
    assert subprocess.run(command).returncode == 0
    (stage / "ffmpeg.exe").write_bytes(b"tampered")
    assert subprocess.run(command, capture_output=True).returncode != 0
    (stage / "runtime-manifest.json").write_text(json.dumps({"../outside": "no"}))
    assert subprocess.run(command, capture_output=True).returncode != 0
print("PASS Windows packaging contracts, installed hashes and traversal rejection")
