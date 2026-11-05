"""Stage the Windows x64 offline runtime before Tauri builds. No user data touched."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
LOCK = ROOT / "packaging/windows/runtime-lock.json"


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for part in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(part)
    return h.hexdigest()


def download(name, entry, cache):
    path = cache / (name + "-" + entry["sha256"])
    if path.is_file() and digest(path) == entry["sha256"]:
        return path
    if path.exists():
        raise RuntimeError(f"Existing download has wrong hash, preserved for inspection: {path}")
    part = path.with_suffix(".partial")
    request = urllib.request.Request(entry["url"], headers={"User-Agent": "TingXian-build"})
    with urllib.request.urlopen(request, timeout=120) as response, part.open("wb") as out:
        shutil.copyfileobj(response, out, 1024 * 1024)
    if digest(part) != entry["sha256"]:
        raise RuntimeError(f"Download hash mismatch, preserved: {part}")
    part.rename(path)
    return path


def extract(archive, directory):
    with zipfile.ZipFile(archive) as z:
        for info in z.infolist():
            target = (directory / info.filename).resolve()
            if not target.is_relative_to(directory.resolve()):
                raise RuntimeError("Archive path escapes the runtime directory")
        z.extractall(directory)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cache", type=Path, default=ROOT / ".windows-downloads")
    args = parser.parse_args()
    if sys.platform != "win32":
        raise SystemExit("Run this staging command on Windows x64 (or the Windows CI runner).")
    stage = ROOT / "frontend/src-tauri/binaries/windows-runtime"
    stage.mkdir(parents=True, exist_ok=True)
    args.cache.mkdir(parents=True, exist_ok=True)
    lock = json.loads(LOCK.read_text())
    downloaded = {name: download(name, entry, args.cache) for name, entry in lock.items()}
    python = stage / "python"
    extract(downloaded["python"], python)
    # Isolated embedding includes only bundled packages. It never depends on PATH Python.
    (python / "python312._pth").write_text("python312.zip\n.\nLib/site-packages\nimport site\n")
    subprocess.run([sys.executable, "-m", "pip", "install", "--disable-pip-version-check",
                    "--only-binary=:all:", "--platform", "win_amd64", "--python-version", "312",
                    "--implementation", "cp", "--abi", "cp312", "--upgrade",
                    "--target", str(python / "Lib/site-packages"),
                    "-r", str(ROOT / "interview/requirements.txt")], check=True)
    subprocess.run([str(python / "python.exe"), "-I", "-B", "-c",
                    "import docx,pypdf,yaml,lxml; print('Bundled document runtime OK')"], check=True)
    whisper = stage / "whisper"
    whisper.mkdir(exist_ok=True)
    shutil.copy2(downloaded["whisper-small"], whisper / "ggml-small.bin")
    media_source = args.cache / ("ffmpeg-expanded-" + lock["ffmpeg"]["sha256"][:12])
    extract(downloaded["ffmpeg"], media_source)
    ffmpeg = next(media_source.glob("*/bin/ffmpeg.exe"))
    media = stage / "media"
    media.mkdir(exist_ok=True)
    for binary in ffmpeg.parent.iterdir():
        if binary.suffix.lower() == ".dll" or binary.name == "ffmpeg.exe":
            shutil.copy2(binary, media / binary.name)
    config = subprocess.check_output([str(media / "ffmpeg.exe"), "-version"], text=True)
    if "--enable-gpl" in config or "--enable-nonfree" in config or "--enable-shared" not in config:
        raise RuntimeError("Expected replaceable LGPL shared FFmpeg libraries")
    # build.rs verifies this sidecar. It needs its adjacent shared DLLs on the build host.
    binaries = stage.parent
    shutil.copy2(media / "ffmpeg.exe", binaries / "ffmpeg-x86_64-pc-windows-msvc.exe")
    for dll in media.glob("*.dll"):
        shutil.copy2(dll, binaries / dll.name)
    notices = stage / "licenses"
    notices.mkdir(exist_ok=True)
    shutil.copy2(ROOT / "LICENSE.md", notices / "TingXian-MIT.txt")
    shutil.copy2(ROOT / "NOTICE.md", notices / "NOTICE.md")
    shutil.copy2(ROOT / "packaging/macos/Whisper-LICENSE.txt", notices / "Whisper-LICENSE.txt")
    shutil.copy2(ROOT / "packaging/windows/THIRD-PARTY.md", notices / "THIRD-PARTY.md")
    shutil.copy2(LOCK, notices / "runtime-lock.json")
    (notices / "ffmpeg-build.txt").write_text(config, encoding="utf-8")
    for child in ffmpeg.parent.parent.iterdir():
        if child.name == "bin":
            continue
        dest = notices / "ffmpeg" / child.name
        if child.is_dir():
            shutil.copytree(child, dest, dirs_exist_ok=True)
        else:
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(child, dest)
    manifest = {str(p.relative_to(stage)).replace("\\", "/"): digest(p)
                for p in stage.rglob("*") if p.is_file() and p.name != "runtime-manifest.json"}
    (stage / "runtime-manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"Staged {len(manifest)} runtime files at {stage}")


if __name__ == "__main__":
    main()
