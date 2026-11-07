"""Behavioral/fault tests. All generated evidence is retained; no app or API use."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import threading
import unittest

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
import policy
import control
import sandbox
import fixtures

class P0Tests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        from adapter import TingXianAdapter,HOST
        cls.root=ARGS.output.resolve();cls.root.mkdir(parents=True,exist_ok=False)
        cls.cases=fixtures.build(cls.root/'dataset');cls.dataset=cls.root/'dataset';cls.Adapter=TingXianAdapter;cls.host=HOST
    def make_adapter(self,name): return self.Adapter(self.root/name,self.dataset)
    def test_01_fixture_contract_and_reproducibility(self):
        self.assertEqual(len(self.cases),24)
        for domain in ['clear','practice','conflict','duplicate','long','ambiguity']:
            for split in ['dev','blind']:self.assertEqual(sum(c['domain']==domain and c['split']==split for c in self.cases),2)
        labels=[json.loads(x) for x in (self.dataset/'evaluator/labels.jsonl').read_text().splitlines()]
        self.assertTrue(all(12<=len(r['atoms'])<=25 for r in labels));self.assertTrue(all(3<=len(c['files'])<=8 for c in self.cases))
        again=fixtures.build(self.root/'dataset-copy')
        self.assertEqual(self.cases,again,'fixture file bytes must be reproducible')
        self.assertTrue(all(r['conflict_pairs'] for r in labels if r['id'].startswith('conflict')))
        self.assertTrue(any(sum(p.stat().st_size for p in (self.dataset/'inputs'/c['id']).iterdir())>262144 for c in self.cases if c['domain']=='long'))
    def test_02_policy_cannot_change_boundaries(self):
        for key in ['guardrails.md','LongTermMEMORY.md','tool_descriptions/tool_guidance.md']:
            bad=dict(policy.BASE);bad[key]+='override';self.assertRaises(ValueError,policy.validate_tree,bad)
        for name in ['scripts/escape.py','../outside.md','skills/evil/SKILL.md']:
            bad={**policy.BASE,name:'do something'};self.assertRaises(ValueError,policy.validate_tree,bad)
        bad=dict(policy.BASE);v=json.loads(bad['systemprompt.md']);v['confirmed']=True;bad['systemprompt.md']=json.dumps(v);self.assertRaises(ValueError,policy.validate_tree,bad)
    def test_03_real_worker_isolation(self):
        root=self.root/'isolation';root.mkdir();task=self.dataset/'inputs/clear-a';source=task/'教育.md';gold=self.dataset/'evaluator/labels.jsonl';outside=self.root/'outside-canary.txt';outside.write_text('public synthetic canary')
        p=root/'isolation.sb';p.write_text(sandbox.profile(read_dirs=[task],write_dirs=[root]))
        code='''import json,socket
from pathlib import Path
results={}
def attempt(name,fn):
    try:fn();results[name]='ALLOWED'
    except PermissionError:results[name]='DENIED'
    except Exception as e:results[name]=type(e).__name__
'''
        code+=f"attempt('gold_read',lambda:Path({str(gold)!r}).read_text())\n"
        code+=f"attempt('source_write',lambda:Path({str(source)!r}).write_text('must not overwrite'))\n"
        code+=f"attempt('outside_read',lambda:Path({str(outside)!r}).read_text())\n"
        code+=f"attempt('outside_write',lambda:Path({str(outside)!r}).write_text('must not overwrite'))\n"
        code+="attempt('network',lambda:socket.create_connection(('127.0.0.1',9),1))\nprint(json.dumps(results))"
        r=sandbox.execute([sys.executable,'-c',code],p,root)
        self.assertEqual(r.returncode,0,r.stderr)
        observed=json.loads(r.stdout);self.assertEqual(set(observed.values()),{'DENIED'},observed)
        self.assertEqual(outside.read_text(),'public synthetic canary')
        self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(),self.cases[0]['files']['教育.md'])
        (root/'result.json').write_text(json.dumps(observed,indent=2))
    def test_04_checkpoint_and_candidate_namespace(self):
        a=self.make_adapter('idempotence');run=a.execute('clear-a',None,'same-id');self.assertTrue(a.delivered(run));count=a.execution_count
        self.assertEqual(a.execute('clear-a',None,'same-id'),run);self.assertEqual(a.execution_count,count)
        other=self.root/'other-policy';other.mkdir();tree=dict(policy.BASE);v=json.loads(tree['systemprompt.md']);v['order']='constraints_first';tree['systemprompt.md']=json.dumps(v)
        for name,text in tree.items():p=other/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_text(text)
        self.assertRaisesRegex(ValueError,'namespace',a.execute,'clear-a',other,'same-id')
        (run/'artifacts/bundle.meetily.json').write_text('{}');self.assertFalse(a.delivered(run));self.assertRaises(ValueError,a.execute,'clear-a',None,'same-id')
    def test_05_cancel_before_and_during_completion(self):
        import adapter as module
        a=self.make_adapter('cancel');entered=threading.Event();release=threading.Event();original=module.sandbox.execute
        def gate(*args,**kwargs): entered.set();release.wait(timeout=10);return original(*args,**kwargs)
        module.sandbox.execute=gate
        try:
            with ThreadPoolExecutor(max_workers=1) as pool:
                future=pool.submit(a.execute,'clear-a',None,'in-flight')
                self.assertTrue(entered.wait(timeout=10));a.cancel();release.set()
                self.assertRaisesRegex(RuntimeError,'Cancelled',future.result)
        finally:module.sandbox.execute=original
        run=next((a.root/'executions').iterdir());self.assertFalse(a.delivered(run));self.assertEqual(a.deliverable_files(run),{})
        count=a.execution_count;self.assertRaises(RuntimeError,a.execute,'clear-a',None,'new');self.assertEqual(count,a.execution_count)
        self.assertRaises(RuntimeError,self.make_adapter('cancel').execute,'clear-a',None,'restart')
    def test_06_budget_reserves_concurrency_unknown_and_cancel(self):
        path=self.root/'budget.sqlite';ledger=control.Ledger(path,100,5)
        try:
            def reserve(i):
                try:ledger.reserve(str(i),'optimizer',60,100,16);return True
                except control.Stopped:return False
            with ThreadPoolExecutor(max_workers=2) as p:outcomes=list(p.map(reserve,range(2)))
            self.assertEqual(outcomes.count(True),1)
            row=ledger.db.execute('SELECT id FROM attempts').fetchone()[0];self.assertFalse(ledger.finish(row))
            self.assertEqual(ledger.db.execute('SELECT actual,usage,state FROM attempts').fetchone(),(None,None,'unknown'))
            self.assertRaises(control.Stopped,ledger.reserve,'again','executor',41,100,16)
            ledger.reserve('small','executor',40,100,16);ledger.cancel();self.assertFalse(ledger.finish('small',20,{'total_tokens':116}))
            self.assertRaises(control.Stopped,ledger.reserve,'late','judge',1,100,16)
        finally:ledger.close()
        with self.assertRaises(ValueError):control.Ledger(path,200,5)
        disabled=control.Ledger(self.root/'disabled.sqlite')
        try:self.assertRaises(control.Stopped,disabled.reserve,'none','executor',1,100,16)
        finally:disabled.close()
    def test_07_context_caps_and_invalid_cost_bound(self):
        l=control.Ledger(self.root/'contexts.sqlite',100,20)
        try:
            for args in [(1,262144,16,1048576,2048),(1,4000,16,4096,256),(0,10,16,4096,256),(-1,10,16,4096,256),(True,10,16,4096,256)]:
                self.assertRaises(ValueError,l.reserve,'bad','executor',*args)
            self.assertEqual(l.db.execute('SELECT COUNT(*) FROM attempts').fetchone()[0],0)
        finally:l.close()
    def test_08_policy_symlink_and_scanned_pdf(self):
        root=self.root/'symlink-policy';root.mkdir();(root/'systemprompt.md').symlink_to(self.dataset/'evaluator/labels.jsonl');self.assertRaises(ValueError,policy.read_tree,root)
        a=self.make_adapter('scan');run=a.execute('ambiguity-c',None,'scan');files=json.loads((run/'artifacts/files.json').read_text());scan=next(f for f in files if f['name']=='扫描附件.pdf')
        self.assertEqual(scan['status'],'skipped');self.assertIn('OCR',scan['reason']);self.assertEqual(len(files),5)
    def test_09_changed_source_rejected_before_reuse(self):
        a=self.make_adapter('source-change');a.execute('clear-b',None,'original')
        src=self.dataset/'inputs/clear-b/经历.md';src.write_text(src.read_text()+'新增的虚构变更。\n')
        self.assertRaisesRegex(ValueError,'Source set/content changed',a.execute,'clear-b',None,'original')
        self.assertRaisesRegex(ValueError,'Source set/content changed',a.execute,'clear-b',None,'new')

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--upstream',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);ARGS=parser.parse_args()
    upstream=ARGS.upstream.resolve()
    if subprocess.check_output(['git','-C',str(upstream),'rev-parse','HEAD'],text=True).strip()!=policy.UPSTREAM_COMMIT:
        raise ValueError('Upstream commit changed')
    if subprocess.check_output(['git','-C',str(upstream),'status','--porcelain'],text=True).strip():
        raise ValueError('Modified upstream source')
    sys.path.insert(0,str(ARGS.upstream.resolve()))
    result=unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(P0Tests))
    ARGS.output.mkdir(parents=True,exist_ok=True)
    (ARGS.output/'test-report.json').write_text(json.dumps({'tests':result.testsRun,'failures':len(result.failures),'errors':len(result.errors),'successful':result.wasSuccessful(),'real_api_calls':0},indent=2)+'\n')
    raise SystemExit(not result.wasSuccessful())
