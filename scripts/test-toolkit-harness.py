#!/usr/bin/env python3
import copy
import importlib.util
import json
import unittest
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location("toolkit",ROOT/"interview/toolkit/harness.py")
h=importlib.util.module_from_spec(spec);spec.loader.exec_module(h)

class Fixture:
    fixture_only=True
    def __init__(self,improve=True):self.improve=improve;self.inputs=[]
    def lead(self,task,feedback,version):
        return {"id":f"v{version}","artifact":"仅模拟练习" if version>1 and self.improve else "已在生产独立实现",
                "claim_evidence_map":[{"claim":"仅模拟练习","evidence_refs":["source:1"]}],"gaps":[],"changelog":[f"revision {version}"],"policy_proposals":[]}
    def review(self,role,candidate,task):
        self.inputs.append((role,copy.deepcopy(candidate)))
        result={"schema_version":"review-1","reviewer":role,"mode":"coaching" if role=="consultant" else "artifact_review",
                "candidate_id":candidate["id"],"candidate_hash":h.digest(candidate),"findings":[],"missing_evidence":[],"prioritized_actions":[],"status":"reviewed"}
        if candidate["artifact"]!="仅模拟练习":
            result["findings"]=[{"id":"scope","severity":"major","category":"fact","location":"artifact","claim":"生产经历无依据","evidence_refs":["source:1"],"certainty":"supported","recommended_change":"仅说明模拟练习","verification":"核对source:1"}]
        candidate["artifact"]="reviewer mutation must not escape"
        task["evidence_ids"].clear()
        return result

class Tests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):cls.registry=h.load_registry(ROOT/"interview/toolkit/registry.json")
    def test_registry_namespaces_and_no_auto_activation(self):
        self.assertEqual(len(self.registry["rules"]),102)
        self.assertEqual(self.registry["active_policy"],"legacy")
        self.assertFalse(self.registry["harness2_enabled"])
        self.assertEqual(h.compile_policy(self.registry,h.route("契约测试","interview","测试开发"))["rule_ids"],[])
    def test_on_demand_role_and_negative_scope(self):
        p=h.compile_policy(self.registry,h.route("pytest 参数化怎么做？","interview","测试开发"),True)
        self.assertIn("INT:T04",p["rule_ids"]);self.assertIn("INT:R08",p["rule_ids"])
        self.assertNotIn("INT:R01",p["rule_ids"])
        self.assertFalse(any(i.startswith("CV:") or i.startswith("LIVE:") for i in p["rule_ids"]))
        meeting=h.compile_policy(self.registry,h.route("项目复盘","meeting","测试开发"),True)
        self.assertFalse(any(i.startswith("INT:") for i in meeting["rule_ids"]))
        self.assertNotIn("STAR",meeting["text"])
    def test_language_and_out_of_pilot_not_domain_validated(self):
        c=h.route("why?","interview","CUDA工程师");c["language"]="en"
        p=h.compile_policy(self.registry,c,True)
        self.assertFalse(any(i.startswith("SP:") for i in p["rule_ids"]))
        self.assertNotIn("INT:R20",p["rule_ids"])
    def test_compaction_preserves_every_selected_action_exception_and_source(self):
        c=h.route("分别设计幂等和null验证以及SQL边界","interview","后端")
        p=h.compile_policy(self.registry,c,True)
        for rule in self.registry["rules"]:
            if rule["id"] in p["rule_ids"]:
                self.assertIn(rule["action"],p["text"])
                for exception in rule["exceptions"]:self.assertIn(exception,p["text"])
                self.assertEqual(p["source_map"][rule["id"]]["source_refs"],rule["source_refs"])
        self.assertLess(len(p["text"].encode()),len(p["uncompacted_same_rules"].encode()))
        self.assertEqual(p["extra_model_calls"],0)
    def test_independent_rounds_no_unreviewed_final_no_false_quality(self):
        adapter=Fixture();result=h.execute_fixture_team({"evidence_ids":["source:1"]},adapter)
        run,audit=result["run"],result["audit"]
        self.assertEqual(len(run["versions"]),2);self.assertEqual(audit["top_level_role_turns"],8)
        self.assertEqual(audit["bottom_level_api_requests"],0);self.assertFalse(audit["quality_pass"])
        for start in (0,3):self.assertEqual(adapter.inputs[start][1],adapter.inputs[start+1][1]);self.assertEqual(adapter.inputs[start][1],adapter.inputs[start+2][1])
        run["versions"][-1]["candidate"]["artifact"]+="hidden edit"
        with self.assertRaisesRegex(ValueError,"Stale candidate"):h.audit_team(run)
    def test_no_improvement_stops_and_keeps_findings(self):
        r=h.execute_fixture_team({"evidence_ids":["source:1"]},Fixture(False))
        self.assertEqual(r["run"]["stop_reason"],"no_improvement")
        self.assertEqual(len(r["run"]["versions"]),2)
        self.assertEqual(r["audit"]["versions"][-1]["blocking_findings"],3)
    def test_missing_role_dangling_evidence_and_unknown_usage(self):
        run=h.execute_fixture_team({"evidence_ids":["source:1"]},Fixture())["run"]
        broken=copy.deepcopy(run);del broken["versions"][0]["reviews"]["hr"]
        with self.assertRaises(ValueError):h.audit_team(broken)
        broken=copy.deepcopy(run);broken["versions"][0]["reviews"]["mentor"]["findings"][0]["evidence_refs"]=["missing"]
        with self.assertRaises(ValueError):h.audit_team(broken)
        run["execution_mode"]="serial_isolated";run["attempts_complete"]=False
        self.assertIsNone(h.audit_team(run)["bottom_level_api_requests"])
        run["attempts"]=[{"id":"failed-1","role":"lead","candidate_id":"v1","input_tokens":100,"output_tokens":None,"reasoning_tokens":None,"cost_cny":None}]
        audit=h.audit_team(run);self.assertIsNone(audit["known_cost_cny"]);self.assertEqual(audit["unknown_cost_requests"],1)
    def test_spoken_guidance_can_be_inspected_without_duplicating_it_in_method_text(self):
        registry=h.load_registry(ROOT/"interview/toolkit/registry.json")
        pack=h.compile_policy(registry,h.route("幂等键",role="测试开发"),preview=True)
        self.assertTrue(pack["style_rule_ids"])
        self.assertTrue(all(not r.startswith("SP:") for r in pack["method_rule_ids"]))
        self.assertNotIn("不编造笑声",pack["method_text"])
        self.assertIn("不编造笑声",pack["style_text"])
        self.assertEqual(set(pack["rule_ids"]),set(pack["method_rule_ids"]+pack["style_rule_ids"]))
    def test_real_adapter_cannot_invoke_in_fixture_runner(self):
        a=Fixture();a.fixture_only=False
        with self.assertRaises(ValueError):h.execute_fixture_team({"evidence_ids":[]},a)
    def test_derived_archive_keeps_full_quotes_and_manual_assertions_separate(self):
        d={"id":"a"*64,"name":"synthetic.txt","hash":"b"*64,"records":[{"kind":"personal","location":"行 1","quote":"只参与模拟测试","note":"课程练习"},{"kind":"practice","location":"行 2","quote":"未来计划进行压测","note":"未来计划不是成果"}]}
        b={"format":"meetily-background-v1","documents":[d],"profile":{"overview":"仅练习","role":"测试","aliases":""}}
        view=h.archive_bundle(b,"library-a","测试开发")
        self.assertEqual(view["records"][1]["namespace"],"task_context")
        rev={"record_id":view["records"][0]["record_id"],"claim":"本人补充：只审阅测试","basis":"本人核对","revision_id":"edit1"}
        revised=h.archive_bundle(b,"library-a","测试开发",[rev])
        self.assertEqual(revised["records"][-1]["claim_status"],"user_asserted")
        self.assertEqual(revised["records"][-1]["source_refs"][0]["quote"],"只参与模拟测试")
        excluded=h.archive_bundle(b,"library-a","测试开发",[rev],[d["id"]])
        self.assertEqual(excluded["role_view"]["record_ids"],[]);self.assertEqual(len(excluded["records"]),3)
        other=h.archive_bundle(b,"library-b","测试开发")
        self.assertNotEqual(view["records"][0]["record_id"],other["records"][0]["record_id"])
        self.assertFalse(view["source_verified"]);self.assertIsNone(view["actual_model_usage"])

if __name__=="__main__":unittest.main()
