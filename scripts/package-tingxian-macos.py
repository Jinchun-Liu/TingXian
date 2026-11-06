"""Package the Apple Silicon release with its offline runtime. Requires macOS."""
import argparse
import ctypes
import hashlib
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tarfile
import tempfile
from xml.sax.saxutils import escape

ROOT = Path(__file__).resolve().parents[1]
MODEL_SHA = "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b"
FFMPEG_SOURCE_SHA = "05ee0b03119b45c0bdb4df654b96802e909e0a752f72e4fe3794f487229e5a41"
MACH_HEADERS = {bytes.fromhex(x) for x in ["feedfacf","cffaedfe","feedface","cefaedfe","cafebabe","bebafeca"]}

def copy_bundle_file(source, destination):
    # Independent APFS copy-on-write files avoid duplicating the model/runtime
    # before pkg compression. Never hard-link: codesign changes the new bundle.
    if os.uname().sysname == "Darwin":
        clone = ctypes.CDLL("/usr/lib/libSystem.B.dylib", use_errno=True).clonefile
        clone.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint32]
        clone.restype = ctypes.c_int
        if clone(os.fsencode(source), os.fsencode(destination), 0) == 0:
            return str(destination)
    # Cross-volume/non-APFS inputs remain supported by a regular independent copy.
    return shutil.copy2(source, destination)

def run(*args):
    subprocess.run([str(a) for a in args], check=True)

def sha(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda:stream.read(1024*1024),b""): h.update(chunk)
    return h.hexdigest()

def clean_finder_metadata(app):
    # Only the newly generated bundle: copied Finder/resource-fork metadata
    # prevents codesign. Preserve quarantine and all other extended attributes.
    app=app.resolve()
    attributes=subprocess.check_output(["/usr/bin/xattr","-rs",str(app)],text=True)
    for line in attributes.splitlines():
        name,separator,attr=line.rpartition(": ")
        if not separator or attr not in ("com.apple.FinderInfo","com.apple.ResourceFork"): continue
        path=Path(name)
        if not path.resolve().is_relative_to(app): raise SystemExit("Metadata path outside generated bundle")
        if not path.is_symlink(): run("/usr/bin/xattr","-ds",attr,path)

def main():
    args=argparse.ArgumentParser()
    for key in ("app","python","model","media","media-source","out"):
        args.add_argument("--"+key,type=Path,required=True)
    a=args.parse_args()
    if sha(a.model)!=MODEL_SHA or sha(a.media_source)!=FFMPEG_SOURCE_SHA:
        raise SystemExit("Model or FFmpeg source checksum mismatch")
    config=subprocess.check_output([a.media,"-version"],text=True)
    if "ffmpeg version 8.0.1" not in config or "--enable-gpl" in config or "--enable-nonfree" in config:
        raise SystemExit("Use the documented FFmpeg build")
    info=plistlib.loads((a.app/"Contents/Info.plist").read_bytes())
    version=info["CFBundleShortVersionString"];identifier=info["CFBundleIdentifier"]+".pkg"
    a.out.mkdir(parents=True,exist_ok=True)
    dest=a.out/f"TingXian-{version}-macOS-arm64.pkg"
    if dest.exists(): raise SystemExit("Choose a fresh output directory")
    # Keep signing work outside Desktop/Finder-synced output folders: FinderInfo
    # may be reintroduced there after signing. Retain this stage for inspection.
    stage=Path(tempfile.mkdtemp(prefix="tingxian-package-"))
    payload=stage/"payload";app=payload/"听弦.app"
    app.parent.mkdir();shutil.copytree(a.app,app,symlinks=True,copy_function=copy_bundle_file)
    resources=app/"Contents/Resources";macos=app/"Contents/MacOS";runtime=resources/"python"
    (runtime/"bin").mkdir(parents=True)
    copy_bundle_file(a.python/"bin/python3",runtime/"bin/python3")
    shutil.copytree(a.python/"lib",runtime/"lib",symlinks=True,copy_function=copy_bundle_file,ignore=shutil.ignore_patterns("__pycache__","*.pyc","libpython*.a","pkgconfig","config-3.*","idlelib","ensurepip","pip","pip-*.dist-info"))
    for link in runtime.rglob("*"):
        if link.is_symlink() and not link.resolve().is_relative_to(runtime.resolve()):
            raise SystemExit("External runtime link: "+str(link))
    (resources/"whisper").mkdir();copy_bundle_file(a.model,resources/"whisper/ggml-small.bin")
    copy_bundle_file(a.media,macos/"ffmpeg")
    notices=resources/"Licenses";notices.mkdir(exist_ok=True)
    shutil.copy2(ROOT/"LICENSE.md",notices/"Meetily-MIT.md")
    shutil.copy2(ROOT/"NOTICE.md",notices)
    shutil.copy2(ROOT/"vendor/cidre/LICENSE.txt",notices/"cidre-LICENSE.txt")
    shutil.copy2(ROOT/"packaging/macos/THIRD-PARTY.zh-CN.md",notices)
    python_notices = sorted(str(f.relative_to(runtime)) for f in runtime.rglob("*") if f.is_file() and f.name.lower().startswith(("license","copying","copyright")))
    if not any("python3.12/LICENSE" in f for f in python_notices): raise SystemExit("Python license is missing")
    (notices/"python-licenses.txt").write_text("\n".join(python_notices)+"\n")

    shutil.copy2(ROOT/"packaging/macos/Whisper-LICENSE.txt",notices)
    copy_bundle_file(a.media_source,notices/"ffmpeg-8.0.1.tar.xz")
    with tarfile.open(a.media_source) as src:
        (notices/"FFmpeg-LGPL-2.1.txt").write_bytes(src.extractfile("ffmpeg-8.0.1/COPYING.LGPLv2.1").read())
    (notices/"ffmpeg-build.txt").write_text(config+"\nSource and build instructions: https://github.com/Jinchun-Liu/TingXian\n")
    clean_finder_metadata(app)
    app_sign=os.getenv("TINGXIAN_APP_SIGN","-")
    def sign(path,outer=False):
        flags=["--force","--sign",app_sign]
        if app_sign!="-": flags+=["--timestamp","--options","runtime"]
        if outer: flags+=["--entitlements",ROOT/"frontend/src-tauri/entitlements.plist"]
        run("/usr/bin/codesign",*flags,path)
    for f in runtime.rglob("*"):
        if f.is_file() and not f.is_symlink():
            with f.open("rb") as fp: header=fp.read(4)
            if header in MACH_HEADERS: sign(f)
    sign(macos/"ffmpeg");sign(app,True)
    run("/usr/bin/codesign","--verify","--strict","--deep",app)
    run(macos/"meetily","--installation-check")
    components=stage/"components.plist"
    run("pkgbuild","--analyze","--root",payload,components)
    items=plistlib.loads(components.read_bytes())
    for item in items: item["BundleIsRelocatable"]=False
    components.write_bytes(plistlib.dumps(items))
    run("pkgbuild","--root",payload,"--component-plist",components,"--identifier",identifier,"--version",version,"--install-location","/Applications","--ownership","recommended",stage/"Application.pkg")
    dist=stage/"distribution.xml"
    dist.write_text(f'''<installer-gui-script minSpecVersion="2">
      <title>听弦</title><options customize="never" hostArchitectures="arm64"/>
      <volume-check><allowed-os-versions><os-version min="14.4"/></allowed-os-versions></volume-check>
      <domains enable_anywhere="false" enable_currentUserHome="false" enable_localSystem="true"/>
      <choices-outline><line choice="app"/></choices-outline>
      <choice id="app" visible="false"><pkg-ref id="{escape(identifier)}"/></choice>
      <pkg-ref id="{escape(identifier)}" version="{escape(version)}" onConclusion="none">Application.pkg</pkg-ref>
      </installer-gui-script>''')
    sign_pkg=["--sign",os.environ["TINGXIAN_PKG_SIGN"]] if os.getenv("TINGXIAN_PKG_SIGN") else []
    run("productbuild","--distribution",dist,"--package-path",stage,*sign_pkg,dest)
    (a.out/"SHA256SUMS.txt").write_text(sha(dest)+"  "+dest.name+"\n")
    print(dest)

if __name__=="__main__": main()
