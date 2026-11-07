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
    def test_unusable_response_keeps_usage_and_does_not_cache_a_result(self):
        for label,response in [
            ("truncated",{"choices":[{"finish_reason":"length","message":{"content":"{unfinished"}}]}),
            ("missing_choice",{"choices":[]}),
            ("bad_json",{"choices":[{"message":{"content":"{unfinished"}}]}),
        ]:
            with self.subTest(label=label):
                root=Path(tempfile.mkdtemp());checkpoint=root/"job.sqlite"
                cfg={"model":"fixture","key":"","endpoint":"http://localhost/chat/completions","timeout":1}
                response["usage"]={"prompt_tokens":123,"completion_tokens":8192}
                metrics={}
                with m.Checkpoint(checkpoint,{"api":cfg}) as ledger:
                    with patch.object(m.urllib.request.OpenerDirector,"open",return_value=Response(m.dumps(response).encode())):
                        with self.assertRaises(ValueError):m.complete(cfg,m.MAP,{"segments":[]},metrics,ledger=ledger)
                    saved=ledger.metrics()
                    self.assertEqual(saved["calls"],1);self.assertEqual(saved["unknown_calls"],0)
                    self.assertEqual(saved["sum_prompt_tokens"],123);self.assertEqual(saved["sum_completion_tokens"],8192)
                    self.assertEqual(metrics["sum_completion_tokens"],8192)
                    self.assertIsNone(ledger.get("result"))
                with m.Checkpoint(checkpoint,{"api":cfg}) as ledger:
                    self.assertEqual(ledger.metrics(),saved,"reopening lost already reported usage")
                self.assertEqual(m.checkpoint_status(checkpoint)["metrics"],saved)

    def test_partial_missing_and_invalid_usage_stays_unknown(self):
        cfg={"model":"fixture","key":"","endpoint":"http://localhost/chat/completions","timeout":1}
        samples=[None,[],{"prompt_tokens":True,"completion_tokens":-1},{"prompt_tokens":"12","completion_tokens":3.5},{"prompt_tokens":12},{"completion_tokens":0},{"prompt_tokens":0,"completion_tokens":0}]
        root=Path(tempfile.mkdtemp());metrics={}
        with m.Checkpoint(root/"job.sqlite",{"api":cfg}) as ledger:
            for usage in samples:
                response={"usage":usage,"choices":[{"message":{"content":"{}"}}]}
                with patch.object(m.urllib.request.OpenerDirector,"open",return_value=Response(m.dumps(response).encode())):
                    self.assertEqual(m.complete(cfg,"规则",[],metrics,ledger=ledger),{})
            saved=ledger.metrics()
            self.assertEqual(saved["calls"],7);self.assertEqual(saved["unknown_calls"],6)
            self.assertEqual(saved["sum_prompt_tokens"],12);self.assertEqual(saved["sum_completion_tokens"],0)
            self.assertEqual(saved["measured_calls"],2);self.assertEqual(saved["completion_measured_calls"],2)
            self.assertEqual(metrics,saved)
        with patch.object(m.urllib.request.OpenerDirector,"open",side_effect=m.urllib.error.URLError("fixture disconnect")):
            with self.assertRaises(m.urllib.error.URLError):m.complete(cfg,"规则",[],metrics)
        self.assertEqual(metrics["calls"],8);self.assertEqual(metrics["unknown_calls"],7)

    def test_failed_file_keeps_completed_part_progress_and_all_call_usage(self):
        root=Path(tempfile.mkdtemp());sources=root/"sources";sources.mkdir();checkpoint=root/"job.sqlite"
        (sources/"a-long.txt").write_text("虚构练习材料\n"*1400)
        (sources/"b-valid.txt").write_text("虚构目标岗位")
        total=len(m.source(sources/"a-long.txt","a-long.txt")[1]);self.assertGreater(total,1)
        seen=0
        def transport(req,timeout):
            nonlocal seen
            body=json.loads(req.data);data=json.loads(body["messages"][1]["content"]);reason="stop"
            if body["messages"][0]["content"]==m.MAP:
                if data["file"]=="a-long.txt":seen+=1
                if data["file"]=="a-long.txt" and seen==2:reply={};reason="length"
                else:
                    row=data["segments"][0];reply={"records":[{"kind":"practice","location":row["location"],"quote":row["text"],"note":"虚构资料"}]}
            else:reply={"overview":"虚构岗位资料","role":"","aliases":""}
            return Response(m.dumps({"usage":{"prompt_tokens":100,"completion_tokens":30},"choices":[{"finish_reason":reason,"message":{"content":m.dumps(reply)}}]}).encode())
        req={"folder":str(sources),"checkpoint":str(checkpoint),"api":{"model":"fixture","key":"","endpoint":"http://localhost/chat/completions","timeout":1}}
        with patch.object(m.urllib.request.OpenerDirector,"open",side_effect=transport):
            result=m.process(req,lambda _:None)
        self.assertEqual(len(result["bundle"]["documents"]),1);self.assertIn("截断",result["failures"][0][1])
        file=m.checkpoint_status(checkpoint)["files"][0]
        self.assertEqual((file["status"],file["done"],file["total"]),("failed",1,total))
        self.assertEqual(result["metrics"]["calls"],4);self.assertEqual(result["metrics"]["sum_completion_tokens"],120)
        self.assertEqual(result["metrics"]["unknown_calls"],0)

    def test_incremental_synthesis_deduplicates_quotes_without_erasing_sources_or_conflicts(self):
        old={"id":"a","file":"old.md","source_hash":"before","kind":"personal","location":"行 1","quote":"Orchid 生产经验：无","source_status":{"confirmation":"confirmed","validity":"expired"}}
        same={**old,"id":"copy","file":"copy.md"}
        new={"id":"a","name":"new.md","hash":"after","records":[{"kind":"personal","location":"行 1","quote":"Orchid 生产经验：有","note":"待本人核实"}]}
        rows=m.merge_inputs([old,same], [new],18000)
        self.assertEqual(len(rows),2);self.assertEqual(len(rows[0]["sources"]),2)
        self.assertEqual([s["version_state"] for s in rows[0]["sources"]],["superseded","indexed"])
        self.assertEqual(rows[0]["sources"][0]["source_status"]["validity"],"expired")
        self.assertEqual(rows[1]["sources"][0]["version_state"],"current")
        repeated={**old,"source_hash":"after","quote":new["records"][0]["quote"]}
        self.assertEqual(len(m.merge_inputs([repeated],[new],18000)[0]["sources"]),1)
        rows=m.merge_inputs([old,{**old,"kind":"practice"}],[],18000);self.assertEqual(len(rows),2,"practice became personal evidence")
        many=[{**old,"id":str(i),"file":str(i)+".md"} for i in range(50)]
        rows=m.merge_inputs(many,[],1000);self.assertGreater(len(rows),1);self.assertEqual(sum(len(r["sources"]) for r in rows),50)
        self.assertTrue(all(len(m.dumps(r).encode())+2<=1000 for r in rows))
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
            return Response(m.dumps({"usage":{"prompt_tokens":100,"completion_tokens":30},"choices":[{"finish_reason":"stop","message":{"content":m.dumps(reply)}}]}).encode())
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
