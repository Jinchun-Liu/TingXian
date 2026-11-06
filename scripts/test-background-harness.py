"""End-to-end with synthetic documents; fake transport records actual request shape."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
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
