"""Coordinator fault injection. No provider calls, credentials or real app data."""
import json
import io
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

import prepare as p
import execute as e
import test_prepare as fixtures


def result(candidate=None, complete=True, usage=True, attempts=1, cancelled=False):
    rows = []
    for n in range(1, attempts + 1):
        rows.append({'type':'event','at_ms':n,'event':{'type':'request_sent','attempt':n}})
        if usage: rows.append({'type':'event','at_ms':n+1,'event':{'type':'usage','attempt':n,'usage':{'prompt_tokens':10,'completion_tokens':5}}})
    if candidate is None: rows.append({'type':'event','at_ms':7,'event':{'type':'delta','text':'先核对条件。'}})
    rows.append({'type':'terminal','complete':complete,'attempts':attempts,'candidate':candidate,'elapsed_ms':12,'first_meaningful_answer_ms':None})
    return {'stdout':'\n'.join(json.dumps(r, ensure_ascii=False) for r in rows),'stderr':'','returncode':0,'cancelled':cancelled,'timed_out':False,'output_limit':False}


class Coordinator(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.binary = Path(os.environ.get('C5_PREPARER', p.REPO/'target/debug/examples/context_trial_prepare')).resolve()
        cls.runner = Path(os.environ.get('C5_RUNNER', p.REPO/'target/debug/examples/context_trial_run')).resolve()
        if not cls.binary.is_file() or not cls.runner.is_file(): raise RuntimeError('build both context trial examples')
        cls.root = Path(tempfile.mkdtemp(prefix='tingxian-c5-coordinator-')); cls.images = cls.root/'images'; cls.images.mkdir()
        for id_ in ('D03','H03'): (cls.images/(id_+'.png')).write_bytes(fixtures.png())
        print('Synthetic coordinator evidence retained at', cls.root)

    def setUp(self):
        self.bundle = self.root/self.id().split('.')[-1]
        p.prepare(self.binary,self.images,self.bundle,self.runner)
        self.manifest = p.load(self.bundle/'manifest.json'); self.day=p.load(self.bundle/'price-review.json')['checked_on']
        self.auth=p.load(self.bundle/'authorization-template.json');self.auth.update(approved=True,allow_paid_calls=True,max_cost_micro_cny=self.manifest['total_reserve_micro_cny'],valid_on=self.day)
        self.calls=[]

    def candidate(self,task):
        data=p.load(self.bundle/'cases'/task['input']['case_id']/'prepared.json')
        prompt=json.loads(data['AI2']['payload']['messages'][1]['content'])
        return {'schema_version':1,'snapshot_id':prompt['snapshot_id'],'finish_reason':'stop','items':[{'kind':x['event']['kind'],'origin':x['event']['origin'],'refs':[x['ref']],'quote':x['event']['text']} for x in prompt['events']],'omitted':[]}

    def send(self,runner,bundle,task,key,timeout):
        # The committed reservation is readable on a second connection before
        # any synthetic transport side effect. This is the real dispatch seam.
        with sqlite3.connect(bundle/'execution/ledger.sqlite') as db:
            row=db.execute('SELECT state,held,reserve,task FROM jobs WHERE id=?',(task['job_id'],)).fetchone()
        self.assertEqual(row[0],'reserved');self.assertEqual(row[1],row[2]);self.assertEqual(json.loads(row[3]),task)
        self.calls.append(task)
        return result(self.candidate(task) if task['role']=='AI2' else None)

    def run_trial(self,send=None,resume=False,**kw):
        return e.run_batch(self.binary,self.runner,self.bundle,self.auth,'synthetic-process-only',resume,
                           send=send or self.send,currency=lambda key:{'currency':'CNY'},evidence='synthetic',current_day=self.day,**kw)

    def rows(self):
        with sqlite3.connect(self.bundle/'execution/ledger.sqlite') as db:
            db.row_factory=sqlite3.Row;return {r['id']:dict(r) for r in db.execute('SELECT * FROM jobs')}

    def test_all_jobs_reserve_before_dispatch_and_use_real_candidate_only_for_c(self):
        summary=self.run_trial()
        self.assertEqual(len(self.calls),22);self.assertEqual(summary['attempts_held'],22)
        self.assertEqual(sum(t['role']=='AI2' for t in self.calls),4)
        self.assertTrue(all(j['state']=='settled' and j['complete'] for j in summary['jobs']))
        self.assertIsNone(summary['actual_provider_bill_micro_cny'])
        for id_ in ('D02','D03','H02','H03'):
            ids=[t['job_id'] for t in self.calls];self.assertLess(ids.index(id_+'-AI2'),ids.index(id_+'-C'))
            task=next(t for t in self.calls if t['job_id']==id_+'-C');self.assertIn('candidate',task['input']);self.assertFalse(task['fallback_raw'])
        c=json.loads(self.rows()['D02-C']['result']);self.assertIsNone(c['first_meaningful_answer_ms'])
        with self.assertRaises(ValueError):self.run_trial()
        count=len(self.calls);self.run_trial(resume=True);self.assertEqual(len(self.calls),count)

    def test_default_consent_stops_before_currency_or_ledger(self):
        self.auth['approved']=False
        with self.assertRaises(ValueError):self.run_trial()
        self.assertFalse((self.bundle/'execution').exists());self.assertEqual(self.calls,[])

    def test_currency_preflight_accepts_only_available_cny_without_storing_balances(self):
        for available,currency,accepted in [(True,'CNY',True),(True,'USD',False),(False,'CNY',False)]:
            body=json.dumps({'is_available':available,'balance_infos':[{'currency':currency,'total_balance':'synthetic'}]}).encode()
            with patch.object(e.urllib.request,'build_opener') as build:
                build.return_value.open.return_value=io.BytesIO(body)
                if accepted:self.assertEqual(e.check_currency('synthetic-process-only'),{'currency':'CNY','amounts_saved':False,'generation_request':False})
                else:
                    with self.assertRaises(ValueError):e.check_currency('synthetic-process-only')

    def test_crash_keeps_full_reserve_and_explicit_resume_never_replays_unknown(self):
        def crash(*args):self.send(*args);raise RuntimeError('synthetic crash after dispatch')
        with self.assertRaises(RuntimeError):self.run_trial(crash)
        row=self.rows()['D01-A'];self.assertEqual(row['state'],'reserved');self.assertEqual(row['held'],row['reserve'])
        self.run_trial(resume=True)
        self.assertEqual([t['job_id'] for t in self.calls].count('D01-A'),1)
        row=self.rows()['D01-A'];self.assertEqual(row['state'],'interrupted');self.assertEqual(row['held'],row['reserve'])
        self.assertEqual(len(self.calls),22)

    def test_missing_terminal_keeps_unreported_attempts_and_stops(self):
        def incomplete(*args):
            raw=self.send(*args);raw['stdout']='\n'.join(raw['stdout'].splitlines()[:-1]);return raw
        with self.assertRaises(ValueError):self.run_trial(incomplete)
        self.assertEqual(len(self.calls),1);row=self.rows()['D01-A'];self.assertEqual(row['state'],'interrupted');self.assertEqual(row['held'],row['reserve'])
        cost=json.loads(row['result'])['accounting'];self.assertEqual(cost['transport_attempts_held'],2);self.assertIsNone(cost['actual_provider_bill_micro_cny'])

    def test_lost_ai2_response_resumes_with_raw_fallback_not_an_extra_ai2_call(self):
        def crash(*args):
            raw=self.send(*args)
            if args[2]['job_id']=='D02-AI2':raise RuntimeError('synthetic lost AI2 response')
            return raw
        with self.assertRaises(RuntimeError):self.run_trial(crash)
        self.run_trial(resume=True)
        self.assertEqual([t['job_id'] for t in self.calls].count('D02-AI2'),1)
        task=next(t for t in self.calls if t['job_id']=='D02-C');self.assertTrue(task['fallback_raw'])

    def test_resume_refuses_inconsistent_persisted_accounting(self):
        self.run_trial()
        with sqlite3.connect(self.bundle/'execution/ledger.sqlite') as db:db.execute("UPDATE jobs SET held=0 WHERE id='D01-A'")
        count=len(self.calls)
        with self.assertRaises(ValueError):self.run_trial(resume=True)
        self.assertEqual(len(self.calls),count)

    def test_bad_ai2_candidate_falls_back_and_is_not_compression_success(self):
        def bad(*args):
            raw=self.send(*args);task=args[2]
            if task['role']=='AI2':
                candidate=self.candidate(task);candidate['items'][0]['quote']='伪造经历';return result(candidate)
            return raw
        self.run_trial(bad)
        self.assertEqual(len(self.calls),22)
        row=json.loads(self.rows()['D02-AI2']['result']);self.assertFalse(row['complete']);self.assertEqual(row['candidate_validation'],'rejected')
        task=next(t for t in self.calls if t['job_id']=='D02-C');self.assertTrue(task['fallback_raw']);self.assertNotIn('candidate',task['input'])
        self.assertEqual(task['expected_payload_sha256'],p.load(self.bundle/'cases/D02/prepared.json')['B']['payload_sha256'])

    def test_cancel_preserves_accounting_and_stops_remaining_jobs(self):
        def cancel(*args):self.send(*args);return result(complete=False,usage=False,cancelled=True)
        summary=self.run_trial(cancel);self.assertEqual(len(self.calls),1)
        row=self.rows()['D01-A'];self.assertEqual(row['held'],row['reserve']//2)
        self.assertTrue(all(j['state']=='pending' for j in summary['jobs'][1:]))

    def test_failed_reservation_commit_prevents_transport(self):
        with patch.object(e.Journal,'reserve',side_effect=sqlite3.OperationalError('synthetic full disk at reservation')):
            with self.assertRaises(sqlite3.OperationalError):self.run_trial()
        self.assertEqual(self.calls,[]);self.assertTrue(all(r['state']=='pending' and r['held']==0 for r in self.rows().values()))

    def test_concurrent_owner_cannot_dispatch(self):
        directory=self.bundle/'execution';directory.mkdir()
        with e.locked(directory/'coordinator.lock'):
            with self.assertRaises(ValueError):self.run_trial()
        self.assertEqual(self.calls,[]);self.assertFalse((directory/'ledger.sqlite').exists())

    def test_revoked_authorization_stops_next_dispatch(self):
        authorization=self.bundle/'approved-test-only.json';p.write(authorization,self.auth)
        def revoke(*args):
            raw=self.send(*args);authorization.write_text(json.dumps(dict(self.auth,approved=False)));return raw
        with self.assertRaises(ValueError):self.run_trial(revoke,authorization_path=authorization)
        self.assertEqual(len(self.calls),1)

    def test_settlement_handles_retry_usage_duplicates_unknown_and_over_reserve(self):
        job=self.manifest['jobs'][0];price=p.load(self.bundle/'price-review.json')
        raw=result(attempts=2);parsed=e.parse_result(raw,job,price);self.assertEqual(parsed['accounting']['held_micro_cny'],120)
        events=parsed['events'];events.insert(-1,{'event':{'type':'usage','attempt':2,'usage':{'prompt_tokens':1,'completion_tokens':1}}})
        self.assertEqual(e.settle(events,events[-1],job,price)['held_micro_cny'],120)
        events[1]['event']['usage']['prompt_tokens']=1_000_000
        cost=e.settle(events,None,job,price);self.assertGreater(cost['held_micro_cny'],job['reserve_micro_cny'])
        events.insert(0,{'event':{'type':'usage','attempt':True,'usage':{}}});self.assertFalse(e.settle(events,events[-1],job,price)['terminal_valid'])
        self.assertFalse(e.settle([],{'complete':True,'attempts':0},job,price)['terminal_valid'])

    def test_over_estimate_stops_and_retains_known_usage(self):
        def over(*args):
            raw=self.send(*args);events=[json.loads(s) for s in raw['stdout'].splitlines()];events[1]['event']['usage']['prompt_tokens']=10_000_000;raw['stdout']='\n'.join(json.dumps(x) for x in events);return raw
        with self.assertRaises(ValueError):self.run_trial(over)
        self.assertEqual(len(self.calls),1);row=self.rows()['D01-A'];self.assertGreater(row['held'],row['reserve'])

    def test_runner_rejects_direct_execute_without_scoped_grant(self):
        prepared=p.load(self.bundle/'cases/D01/prepared.json');task={'job_id':'D01-A','role':'AI1','arm':'A','input':{'case_id':'D01'},'fallback_raw':False,'expected_payload_sha256':prepared['A']['payload_sha256'],'manifest_sha256':'a'*64}
        raw=subprocess.run([str(self.runner),'--execute'],input=json.dumps(task),text=True,capture_output=True,timeout=10,env={'PATH':os.environ.get('PATH','')})
        self.assertNotEqual(raw.returncode,0);self.assertNotIn('request_sent',raw.stdout)
        local=subprocess.run([str(self.runner)],input=json.dumps(task),text=True,capture_output=True,timeout=10,env={'PATH':os.environ.get('PATH','')});self.assertEqual(local.returncode,0);self.assertEqual(json.loads(local.stdout)['external_calls'],0)

    def test_os_kill_after_commit_retains_reservation_without_replay(self):
        script=self.bundle/'synthetic-kill-driver.py'
        script.write_text('import sys, time\nfrom pathlib import Path\nsys.path.insert(0, '+repr(str(p.HERE))+')\nimport execute as e\ndef send(*args):\n    Path('+repr(str(self.bundle/'reserved.marker'))+').write_text("reserved")\n    time.sleep(60)\ne.run_batch(Path('+repr(str(self.binary))+'),Path('+repr(str(self.runner))+'),Path('+repr(str(self.bundle))+'),'+repr(self.auth)+',"synthetic-process-only",send=send,currency=lambda key:{"currency":"CNY"},evidence="synthetic",current_day='+repr(self.day)+')\n')
        child=subprocess.Popen([sys.executable,'-B',str(script)],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
        try:
            deadline=time.monotonic()+10
            while not (self.bundle/'reserved.marker').exists() and child.poll() is None and time.monotonic()<deadline:time.sleep(.02)
            self.assertTrue((self.bundle/'reserved.marker').exists())
            child.kill();child.communicate(timeout=5)
        finally:
            if child.poll() is None:child.kill();child.communicate(timeout=5)
        row=self.rows()['D01-A'];self.assertEqual(row['state'],'reserved');self.assertEqual(row['held'],row['reserve'])
        self.run_trial(resume=True)
        self.assertEqual(len(self.calls),21);self.assertNotIn('D01-A',[t['job_id'] for t in self.calls])
        self.assertEqual(self.rows()['D01-A']['state'],'interrupted')

    def test_transport_timeout_reaps_only_its_synthetic_child(self):
        child=self.root/'timeout-child.py';child.write_text('#!/usr/bin/python3\nimport sys,time\nsys.stdin.read()\ntime.sleep(60)\n');child.chmod(0o700)
        raw=e.transport(child,self.bundle,{'manifest_sha256':'a'*64},'synthetic-process-only',.05)
        self.assertTrue(raw['timed_out']);self.assertIsNotNone(raw['returncode'])

    def test_transport_output_bound_is_enforced_without_a_provider(self):
        child=self.root/'overflow-child.py';child.write_text('#!/usr/bin/python3\nimport sys\nsys.stdin.read()\nsys.stdout.write("x" * 3000000)\nsys.stdout.flush()\n');child.chmod(0o700)
        raw=e.transport(child,self.bundle,{'manifest_sha256':'a'*64},'synthetic-process-only',5)
        self.assertTrue(raw['output_limit']);self.assertLessEqual(len(raw['stdout'].encode()),e.MAX_LOG)
        self.assertEqual(self.calls,[])


if __name__=='__main__':unittest.main()
