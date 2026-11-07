"""Synthetic cross-language protocol test. No model, network, or user files."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import importlib.util
import copy

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
        people = [r for r in records if r["kind"] == "personal"]
        personal = people[0]
        rid = personal["record_id"]
        proposals.append({"format":"preparation-part-v1", "input_hash":packet["input_hash"],
          "requirements":[{"id":"req-1", "matches":[{"record_id":rid,"relevance":"supporting","reason":"合成匹配，仍需核对贡献范围"}],"references":[],"gaps":[]}],
          "stories":[{"title":"合成契约测试", "activity":personal["activity_status"],
             "sections":{"context":[],"task":[],"action":[{"text":personal["statement"],"record_ids":[rid]}],"result":[],"learning":[]},
             "missing":{key:"待补合成证据" for key in ("context","task","result","learning")}}]})
    responses = folder / "responses.json"
    responses.write_text(json.dumps(proposals,ensure_ascii=False))
    if '--consolidate' in sys.argv:
        # Different source records become independent original stories. The
        # consolidation fixture supplies synthetic proposals, not model output.
        for number,proposal in enumerate(proposals,1):
            packet=run('preparation-input','--part',str(number))
            people=[r for r in json.loads(packet['messages'][1]['content'])['records'] if r['kind']=='personal']
            template=proposal['stories'][0];proposal['stories']=[]
            for person in people:
                story=copy.deepcopy(template);story['sections']['action']=[{'text':person['statement'],'record_ids':[person['record_id']]}]
                proposal['stories'].append(story)
            proposal['requirements'][0]['gaps']=['缺少独立的结果验证']
        responses.write_text(json.dumps(proposals,ensure_ascii=False))
    candidate = run("preparation-collect", "--proposals", str(responses), "--execution-mode", "fixture")
    if '--consolidate' in sys.argv:
        def load(name,path):
            spec=importlib.util.spec_from_file_location(name,path);module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module
        c=load('consolidation',root/'interview/toolkit/consolidation.py');f=load('fixture',root/'scripts/consolidation-fixture.py')
        grouping=f.groups(c,candidate);plan=c.plan(candidate,grouping)
        (folder/'groups.json').write_text(json.dumps(grouping,ensure_ascii=False))
        (folder/'consolidation-responses.json').write_text(json.dumps(f.responses(plan),ensure_ascii=False))
        candidate=run('consolidation-collect','--proposals',str(responses),'--groups',str(folder/'groups.json'),'--responses',str(folder/'consolidation-responses.json'),'--execution-mode','fixture')
    print(json.dumps(candidate,ensure_ascii=False))
