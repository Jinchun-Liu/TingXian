"""Offline portable smoke test; retains its synthetic output for inspection."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

skill=Path(__file__).resolve().parents[1]
work=Path(tempfile.mkdtemp(prefix="tingxian-background-example-"))
sources=work/"sources";sources.mkdir()
source=sources/"source.txt";shutil.copyfile(skill/"assets/example/source.txt",source)
before=hashlib.sha256(source.read_bytes()).hexdigest()
script=skill/"scripts/background.py"
def run(*args,ok=True):
    p=subprocess.run([sys.executable,"-B",str(script),*map(str,args)],capture_output=True,text=True,encoding="utf-8")
    if (p.returncode==0)!=ok: raise AssertionError(p.stderr or "Unexpected success")
    return json.loads(p.stdout) if ok else None
plan=run("plan","--root",sources)
assert len(plan)==1 and plan[0]["pending_parts"]==[1]
row=plan[0];data=run("read","--root",sources,"--id",row["id"])
cards=work/"cards.json"
records=[{"kind":"practice","location":r["location"],"quote":r["text"],"note":"虚构验收资料，保留冲突和练习限制"} for r in data["segments"]]
cards.write_text(json.dumps({row["id"]:{"hash":row["hash"],"parts":[{"records":records}]}},ensure_ascii=False),encoding="utf-8")
assert run("plan","--root",sources,"--cards",cards)[0]["pending_parts"]==[]
bundle=work/"example.meetily.json"
args=["pack","--root",sources,"--cards",cards,"--profile",skill/"assets/example/profile.json","--out",bundle]
assert run(*args)["documents"]==1
assert run("validate","--bundle",bundle)["format"]=="meetily-background-v2"
run(*args,ok=False)  # Never overwrite an existing output.
assert hashlib.sha256(source.read_bytes()).hexdigest()==before
source.write_text("变更后的虚构文件",encoding="utf-8")
assert run("plan","--root",sources,"--cards",cards)[0]["changed"]
run(*args[:-1],work/"stale.meetily.json",ok=False)
assert not (work/"stale.meetily.json").exists()
print(json.dumps({"passed":True,"work":str(work),"model_called":False},ensure_ascii=False))
