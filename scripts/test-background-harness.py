"""End-to-end with synthetic documents; fake transport records actual request shape."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import subprocess
import unittest
from unittest.mock import patch
HERE=Path(__file__).resolve().parents[1]/"interview/skills/interview-background/scripts/background.py"
spec=importlib.util.spec_from_file_location("harness",HERE); m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
class Response:
    def __init__(self,body): self.body=body
    def __enter__(self): return self
    def __exit__(self,*a): pass
    def read(self,n): return self.body[:n]
class Test(unittest.TestCase):
    def test_portable_bundle_runs_outside_repository_and_rejects_invalid_input(self):
        import shutil
        root=Path(tempfile.mkdtemp());exported=root/"interview-background"
        shutil.copytree(HERE.parents[1],exported,ignore=shutil.ignore_patterns("__pycache__"))
        p=subprocess.run([sys.executable,"-B",str(exported/"scripts/self_test.py")],cwd=root,capture_output=True,text=True,encoding="utf-8")
        self.assertEqual(p.returncode,0,p.stderr);result=json.loads(p.stdout);self.assertTrue(result["passed"]);self.assertFalse(result["model_called"])
        bundle=json.loads((Path(result["work"])/"example.meetily.json").read_text(encoding="utf-8"))
        m.validate_bundle(bundle)
        for modify in [lambda b:b["documents"].append(b["documents"][0]),lambda b:b["profile"].update({"unknown":"field"}),lambda b:b["documents"][0].update({"name":"../source.txt"}),lambda b:b["settings"].update({"context":"bad\x01text"}),lambda b:b.update({"format":"meetily-background-v1"})]:
            bad=json.loads(json.dumps(bundle));modify(bad)
            with self.assertRaises(ValueError):m.validate_bundle(bad)
    def test_settings_bundle_version_and_legacy_compatibility(self):
        doc={"id":"a"*64,"name":"synthetic.txt","hash":"b"*64,"records":[{"kind":"practice","location":"行 1","quote":"模拟项目","note":"只证明练习"}]}
        profile={"overview":"模拟项目并非生产经验","role":"示例岗位","aliases":"幂等,idempotency"}
        legacy=m.pack([doc],profile);self.assertEqual(legacy["format"],"meetily-background-v1");self.assertNotIn("settings",legacy)
        settings={"role":"示例岗位","organization":"目标示例组织","context":"只证明练习","terminology":"幂等","call_names":"小林"}
        current=m.pack([doc],{**profile,"settings":settings});self.assertEqual(current["format"],"meetily-background-v2");self.assertEqual(current["profile"],profile);self.assertEqual(current["settings"],settings)
        for forbidden in ["key","microphone","meeting_mode","allow_external"]:
            with self.assertRaisesRegex(ValueError,"不支持的字段"):m.pack([doc],{**profile,"settings":{**settings,forbidden:"anything"}})
    def test_small_model_budget_repartitions_and_never_exceeds_effective_window(self):
        root=Path(tempfile.mkdtemp());(root/"small.txt").write_text("虚构练习材料\n"*600)
        cfg={"model":"small-test","key":"","endpoint":"http://localhost/chat/completions","timeout":1,"context_tokens":4096,"max_output_tokens":512,"safety_tokens":256}
        observed=[]
        def transport(request,timeout):
            body=json.loads(request.data);system=body["messages"][0]["content"];raw=body["messages"][1]["content"];data=json.loads(raw)
            bound=len(system.encode())+len(raw.encode())+128+body["max_tokens"]+256
            self.assertLessEqual(bound,4096);self.assertEqual(body["max_tokens"],512);observed.append(bound)
            if system==m.MAP:
                row=data["segments"][0];reply={"records":[{"location":row["location"],"quote":row["text"],"note":"虚构练习","kind":"practice"}]}
            else: reply={"overview":"虚构练习材料","role":"","aliases":""}
            return Response(m.dumps({"choices":[{"finish_reason":"stop","message":{"content":m.dumps(reply)}}]}).encode())
        with patch.object(m.urllib.request.OpenerDirector,"open",side_effect=transport):
            result=m.process({"folder":str(root),"api":cfg},lambda _:None)
            self.assertGreater(len(observed),3);self.assertIsNone(result["metrics"]["peak_prompt_tokens"])
        with patch.object(m.urllib.request.OpenerDirector,"open",side_effect=AssertionError("oversize request reached network")):
            with self.assertRaisesRegex(ValueError,"未发送"): m.complete(cfg,"规则","不裁掉的限制"*3000,{})
        self.assertEqual(m.model_limits({**cfg,"context_tokens":1048576})[0],262144)

    def test_durable_resume_source_change_model_fence_and_unknown_usage(self):
        root=Path(tempfile.mkdtemp());sources=root/"sources";sources.mkdir()
        text="虚构项目仅用于练习，不代表真实生产经历。\n"*1400
        source=sources/"a.txt";source.write_text(text)
        (sources/"duplicate.txt").write_text(text)
        checkpoint=root/"job.sqlite"
        req={"folder":str(sources),"checkpoint":str(checkpoint),"api":{"key":"synthetic-only-key","model":"test-model","endpoint":"http://localhost/chat/completions","timeout":1}}
        mapped=[];fail=True
        def transport(request,timeout):
            nonlocal fail
            body=json.loads(request.data);system=body["messages"][0]["content"];data=json.loads(body["messages"][1]["content"])
            if system==m.MAP:
                mapped.append(data["segments"][0]["location"])
                if fail and len(mapped)==2: raise m.urllib.error.URLError("synthetic disconnect")
                row=data["segments"][0]
                reply={"records":[{"location":row["location"],"quote":row["text"],"note":"练习资料","kind":"practice"}]}
            else: reply={"overview":"虚构练习项目","role":"","aliases":""}
            return Response(m.dumps({"usage":{"prompt_tokens":100},"choices":[{"finish_reason":"stop","message":{"content":m.dumps(reply)}}]}).encode())
        with patch.object(m.urllib.request.OpenerDirector,"open",side_effect=transport):
            with self.assertRaises(m.urllib.error.URLError): m.process(req,lambda _:None)
            state=m.checkpoint_status(checkpoint)
            self.assertEqual(state["status"],"interrupted");self.assertEqual(state["files"][0]["done"],1)
            self.assertNotIn("synthetic-only-key",checkpoint.read_bytes().decode(errors="ignore"))
            self.assertNotIn("key",state["request"]["api"])
            fail=False;out=m.process(req,lambda _:None)
            self.assertEqual(mapped.count("行 1"),1,"completed map was sent again")
            self.assertEqual(len(out["bundle"]["documents"]),1,"duplicate content counted twice")
            self.assertEqual(out["metrics"]["unknown_calls"],1)
            statuses={r["status"] for r in m.checkpoint_status(checkpoint)["files"]}
            self.assertEqual(statuses,{"completed","skipped"})
            before=len(mapped);self.assertEqual(m.process(req,lambda _:None),out);self.assertEqual(len(mapped),before)
            source.write_text("虚构来源已改动")
            changed=m.process(req,lambda _:None)
            self.assertEqual(len(changed["bundle"]["documents"]),2)
            self.assertNotEqual(changed["bundle"]["documents"][0]["hash"],out["bundle"]["documents"][0]["hash"])
            with self.assertRaisesRegex(ValueError,"模型、预算"):
                m.process({**req,"api":{**req["api"],"model":"different"}},lambda _:None)
        with m.Checkpoint(checkpoint,req):
            with self.assertRaisesRegex(ValueError,"仍在运行"): m.Checkpoint(checkpoint,req)
        with patch.object(m.urllib.request.OpenerDirector,"open",side_effect=AssertionError("verification sent data")):
            self.assertTrue(m.process({**req,"verify_documents":changed["bundle"]["documents"]},lambda _:None)["verified"])
            source.write_text("核对后再次变化")
            with self.assertRaisesRegex(ValueError,"来源已变化"):
                m.process({**req,"verify_documents":changed["bundle"]["documents"]},lambda _:None)

    def test_agent_resume_plan_checks_each_partial_against_current_source(self):
        root=Path(tempfile.mkdtemp());path=root/"sample.txt";path.write_text("虚构练习材料\n"*2000)
        key=next(iter(m.inventory(root)));h,parts=m.source(path,path.name)
        first=parts[0][0];card={"hash":h,"parts":[{"records":[{"kind":"practice","quote":first["text"],"location":first["location"],"note":"练习"}]},None]}
        result=m.agent_plan(root,{key:card})[0];self.assertEqual(result["reusable_parts"],[1]);self.assertIn(2,result["pending_parts"])
        card["parts"][0]["records"][0]["quote"]="不存在的原文"
        self.assertIn("error",m.agent_plan(root,{key:card})[0])
        path.write_text("修改后的资料")
        result=m.agent_plan(root,{key:card})[0];self.assertTrue(result["changed"]);self.assertEqual(result["reusable_parts"],[])

    def test_budget_and_docx(self):
        from docx import Document
        root=Path(tempfile.mkdtemp()); path=root/"resume.docx";doc=Document();doc.add_paragraph("虚构背景")
        table=doc.add_table(rows=1,cols=2);table.cell(0,0).text="岗位";table.cell(0,1).text="测试";doc.save(path)
        _,parts=m.source(path,path.name);self.assertEqual([x["text"] for x in parts[0]],["虚构背景","岗位 | 测试"])
        cfg={"model":"none","key":"","endpoint":"http://127.0.0.1/chat/completions","timeout":1}
        with patch.object(m.urllib.request.OpenerDirector,"open",side_effect=AssertionError("over-budget request reached transport")):
            with self.assertRaisesRegex(ValueError,"256K"):
                m.complete(cfg,"读取","词"*90000,{})
    def test_large_corpus_parts_quotes_usage_and_pack(self):
        root=Path(tempfile.mkdtemp()); files=root/"sources";files.mkdir()
        (files/"project.md").write_text(("模拟候选人练习 JSON schema，不能作为生产经历。\n"*4000))
        (files/"岗位.txt").write_text("招聘接口测试实习生")
        (files/"local-secret.md").write_text("not sent")
        (files/"broken.pdf").write_text("fixture")
        (files/"link.md").symlink_to(files/"project.md")
        profile={"overview":"项目说明为练习；岗位要求接口测试","role":"接口测试","aliases":"契约测试,contract test"}
        shape=[];progress=[];output={}
        def transport(req,timeout):
            body=json.loads(req.data);s,d=[x["content"] for x in body["messages"]];d=json.loads(d)
            self.assertLess(len(req.data)+body["max_tokens"],262144)
            self.assertNotIn("unused-fixture-key",d);self.assertNotIn("thinking",body)
            if s==m.MAP:
                row=d["segments"][0];reply={"records":[{"location":row["location"],"quote":row["text"],"note":"练习要求","kind":"practice"}]}
                shape.append(d["file"]);output.setdefault(d["file"],[]).append(reply)
            elif s==m.REDUCE:reply=profile
            else:reply={"ok":True}
            return Response(m.dumps({"usage":{"prompt_tokens":777},"choices":[{"finish_reason":"stop","message":{"content":m.dumps(reply)}}]}).encode())
        with patch.object(m.urllib.request.OpenerDirector,"open",side_effect=transport):
            req={"folder":str(files),"api":{"key":"unused-fixture-key","model":"synthetic","endpoint":"http://localhost/chat/completions","timeout":5}}
            res=m.process(req,progress.append);bundle=res["bundle"]
            self.assertGreater(shape.count("project.md"),2);self.assertEqual(res["metrics"]["peak_prompt_tokens"],777)
            self.assertEqual({d["name"] for d in bundle["documents"]},{"project.md","岗位.txt"})
            self.assertEqual(res["failures"][0][0],"broken.pdf")
            meter=m.process({**req,"measure":True,"previous":[{"quote":"测量"*300}]*200},progress.append)["metrics"]
            self.assertGreater(meter["calls"],1);self.assertEqual(meter["sum_prompt_tokens"],meter["calls"]*777)
        inv=m.inventory(files);cards={d["id"]:{"hash":d["hash"],"parts":output[d["name"]]} for d in bundle["documents"]}
        cards.update({k:{"skip":"损坏"} for k,(_,n) in inv.items() if n=="broken.pdf"})
        for n,data in [("cards",cards),("profile",profile)]: (root/n).write_text(m.dumps(data))
        def pack(out):
            args=["background.py","pack","--root",str(files),"--cards",str(root/"cards"),"--profile",str(root/"profile"),"--out",str(root/out)]
            with patch.object(sys,"argv",args): m.main()
        pack("external.meetily.json")
        self.assertEqual(json.loads((root/"external.meetily.json").read_text()),bundle)
        (files/"project.md").write_text("changed")
        with self.assertRaises(ValueError):pack("stale.json")
        self.assertFalse((root/"stale.json").exists())
        with self.assertRaises(ValueError):m.records({"records":[{"kind":"personal","location":"行 1","quote":"编造","note":"test"}]},[{"location":"行 1","text":"原文"}])
if __name__=="__main__": unittest.main()
