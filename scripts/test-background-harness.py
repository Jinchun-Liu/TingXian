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
