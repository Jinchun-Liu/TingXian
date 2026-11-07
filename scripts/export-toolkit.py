#!/usr/bin/env python3
"""Export portable project-local roles and their shared offline runtime.

Does not install, overwrite existing packages, or include private holdouts.
"""
import argparse
import hashlib
import json
import subprocess
import zipfile
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
SKILLS=("tingxian-career-lead","tingxian-hr-reviewer","tingxian-mentor-reviewer","tingxian-consultant","interview-background")


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out",type=Path,required=True)
    args=parser.parse_args()
    files={}
    for folder in [ROOT/"interview/toolkit",*(ROOT/"interview/skills"/s for s in SKILLS)]:
        for path in sorted(folder.rglob("*")):
            if not path.is_file() or "__pycache__" in path.parts or path.suffix==".pyc":continue
            if path.is_symlink():raise ValueError("Export refuses symlinks")
            relative=path.relative_to(ROOT/"interview").as_posix()
            files[relative]=path.read_bytes()
    for name in SKILLS:
        assert f"skills/{name}/SKILL.md" in files
    assert "toolkit/harness.py" in files and "toolkit/registry.json" in files
    manifest={"schema_version":"tingxian-toolkit-export-1","version":"0.2-candidate",
              "source_commit":subprocess.check_output(["git","rev-parse","HEAD"],cwd=ROOT,text=True).strip(),
              "worktree_dirty":bool(subprocess.check_output(["git","status","--porcelain"],cwd=ROOT)),
              "harness2_enabled":False,"active_policy":"legacy","skills_installed":False,
              "model_quality_validated":False,"cross_host_roundtrip_validated":False,
              "files":{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}}
    files["manifest.json"]=json.dumps(manifest,ensure_ascii=False,indent=2).encode()
    files["README.md"]="# 听弦双工具候选包\n\n入口：[使用与安装说明](toolkit/README.md)。四角色与背景执行组件共用toolkit，保持目录关系。未执行安装、真实模型或跨宿主回导。\n".encode()
    args.out.parent.mkdir(parents=True,exist_ok=True)
    with zipfile.ZipFile(args.out,"x",zipfile.ZIP_DEFLATED) as z:
        for name,data in sorted(files.items()):
            info=zipfile.ZipInfo("TingXian-Toolkit/"+name,(2026,11,7,0,0,0));info.compress_type=zipfile.ZIP_DEFLATED
            z.writestr(info,data)
    with zipfile.ZipFile(args.out) as z:
        assert z.testzip() is None
        for name,expected in manifest["files"].items():
            assert hashlib.sha256(z.read("TingXian-Toolkit/"+name)).hexdigest()==expected
    checksum=hashlib.sha256(args.out.read_bytes()).hexdigest()
    print(json.dumps({"path":str(args.out),"sha256":checksum,"bytes":args.out.stat().st_size,"files":len(files),"source_commit":manifest["source_commit"],"worktree_dirty":manifest["worktree_dirty"]},ensure_ascii=False))


if __name__=="__main__":main()
