"""Synthetic cross-language protocol test. No model, network, or user files."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[1]
harness = root / "interview/toolkit/harness.py"
scope = Path(sys.argv[1])
with tempfile.TemporaryDirectory(prefix="tingxian-preparation-cli-") as folder:
    folder = Path(folder)
    task = folder / "task.json"
    task.write_text(json.dumps({"role":"测试开发", "organization":"合成组织", "requirements":["解释契约测试"]}, ensure_ascii=False))
    base = ["--scope", str(scope), "--task", str(task)]
    def run(action, *args):
        result = subprocess.run([sys.executable, "-B", str(harness), action, *base, *args], capture_output=True, text=True, check=True)
        return json.loads(result.stdout)
    plan = run("preparation-plan")
    proposals = []
    for number in range(1, len(plan["parts"])+1):
        packet = run("preparation-input", "--part", str(number))
        records = json.loads(packet["messages"][1]["content"])["records"]
        personal = next(r for r in records if r["kind"] == "personal")
        rid = personal["record_id"]
        proposals.append({"format":"preparation-part-v1", "input_hash":packet["input_hash"],
          "requirements":[{"id":"req-1", "matches":[{"record_id":rid,"relevance":"supporting","reason":"合成匹配，仍需核对贡献范围"}],"references":[],"gaps":[]}],
          "stories":[{"title":"合成契约测试", "activity":personal["activity_status"],
             "sections":{"context":[],"task":[],"action":[{"text":personal["statement"],"record_ids":[rid]}],"result":[],"learning":[]},
             "missing":{key:"待补合成证据" for key in ("context","task","result","learning")}}]})
    responses = folder / "responses.json"
    responses.write_text(json.dumps(proposals,ensure_ascii=False))
    candidate = run("preparation-collect", "--proposals", str(responses), "--execution-mode", "fixture")
    print(json.dumps(candidate,ensure_ascii=False))
