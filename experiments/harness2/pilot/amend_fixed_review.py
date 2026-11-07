"""Versioned B-controller correction; preserve original outputs and all prior costs.
Only still-unattempted fixed slots may execute. Failed slots are never retried.
Existing valid outputs are immutable selector inputs, not reused checkpoints.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE));sys.path.insert(1,str(HERE.parent))
from broker import Broker,encoded,hash_file,MODEL,OUTPUT_CAP
from fixed_review import select_valid
import policy,sandbox


def main():
    p=argparse.ArgumentParser();p.add_argument('--pilot',type=Path,required=True);p.add_argument('--upstream',type=Path,required=True);a=p.parse_args()
    original=a.pilot.resolve();upstream=a.upstream.resolve();root=original/'amendment-01'
    if root.exists():raise ValueError('Amendment exists; do not silently repeat')
    if subprocess.check_output(['git','-C',str(upstream),'rev-parse','HEAD'],text=True).strip()!=policy.UPSTREAM_COMMIT:raise ValueError('Changed upstream')
    if subprocess.check_output(['git','-C',str(upstream),'status','--porcelain'],text=True).strip():raise ValueError('Modified upstream')
    sys.path.insert(0,str(upstream))
    from pilot_adapter import PilotAdapter,HOST
    prior=json.loads((original/'pilot-report.json').read_text());frozen=json.loads((original/'manifest.json').read_text())
    auth=json.loads((original/'authorization.json').read_text())
    if auth['approved_budget_cny']!=30 or frozen['model']!=MODEL or frozen['output_cap']!=OUTPUT_CAP:raise ValueError('Changed authorization/model/output cap')
    if hash_file(HOST)!=frozen['host_sha256'] or hash_file(HERE/'rpc_worker.py')!=frozen['source_sha256']['pilot/rpc_worker.py']:raise ValueError('Extraction/selector worker changed')
    for case in map(json.loads,(original/'dataset/cases.jsonl').read_text().splitlines()):
        for name,h in case['files'].items():
            if hash_file(original/'dataset/inputs'/case['id']/name)!=h:raise ValueError('Changed source')
    targets=[r['task'] for r in prior['outcomes'] if r['group']=='B' and r['status']!='completed']
    if not targets or any(t not in frozen['cases'] for t in targets):raise ValueError('No authorized failed B groups')
    root.mkdir()
    manifest={'kind':'P1-B-controller-amendment-01','targets':targets,'original_manifest_sha256':hash_file(original/'manifest.json'),
        'original_report_sha256':hash_file(original/'pilot-report.json'),'model':MODEL,'output_cap':OUTPUT_CAP,'thinking':'high',
        'scope':'Only unattempted B slots and selection; no retry of failed slots; no A/C0 rerun',
        'original_ledger_sha256':hash_file(original/'evidence-original/budget.sqlite'),
        'same_budget_ledger':True,'total_cap_cny':30,'initial_generation_requests':prior['budget']['attempts'],
        'source_commit':subprocess.check_output(['git','-C',str(HERE.parents[2]),'rev-parse','HEAD'],text=True).strip(),
        'source_sha256':{str(p.relative_to(HERE)):hash_file(p) for p in sorted(HERE.glob('*.py'))},'created':time.time()}
    (root/'manifest.json').write_bytes(encoded(manifest))
    broker=Broker(original/'broker',key=os.environ.get('TX_H2_API_KEY',''))
    if not broker.key:raise ValueError('Dedicated credential required')
    initial_ids={r[0] for r in broker.ledger.db.execute('SELECT id FROM attempts')}
    adapter=PilotAdapter(root/'experiment',original/'dataset',broker)
    outcomes=[dict(row) for row in prior['outcomes']]
    amendment_rows=[]
    for task in targets:
        group_id=task+'-B';row={'task':task,'group':'B','status':'failed','phase':'controller-amendment-01'};candidates=[];slots=[]
        try:
            with broker.group(group_id,3500000):
                for slot in range(4):
                    old_id=group_id+'-'+str(slot);old_run=original/'experiment/executions'/hashlib.sha256(old_id.encode()).hexdigest()[:32]
                    if old_run.exists():
                        state=json.loads((old_run/'state.json').read_text())
                        if state['status']=='completed':
                            artifact=old_run/'artifacts/bundle.meetily.json'
                            if state['policy']!=policy.signature(policy.BASE) or hash_file(artifact)!=state['artifact_sha256']:raise ValueError('Changed prior candidate')
                            candidates.append({'id':str(slot),'path':str(artifact)})
                            slots.append({'slot':slot,'action':'existing immutable candidate','sha256':hash_file(artifact)})
                        else:
                            slots.append({'slot':slot,'action':'failed slot retained; not retried','error':state.get('error')})
                        continue
                    try:
                        new_run=adapter.execute(task,None,'amendment-01/'+group_id+'-slot'+str(slot))
                        if not adapter.delivered(new_run):raise ValueError('Unverified new candidate')
                        artifact=new_run/'artifacts/bundle.meetily.json'
                        candidates.append({'id':str(slot),'path':str(artifact)})
                        slots.append({'slot':slot,'action':'previously unattempted slot executed','sha256':hash_file(artifact)})
                    except Exception as exc:
                        slots.append({'slot':slot,'action':'new slot failed; no retry','error':str(exc)[:1000]})
                        if broker.stopped():raise
                selected,choice=select_valid(broker,root,'amendment-01-'+group_id,candidates)
                def publish():
                    target=root/'submissions'/(group_id+'.json');target.parent.mkdir(exist_ok=True)
                    target.write_bytes((selected/'artifacts/bundle.meetily.json').read_bytes())
                    row.update(status='completed',path=str(target),sha256=hash_file(target),selection=choice)
                broker.adopt(publish)
        except Exception as exc:row['error']=str(exc)[:1200]
        row['slots']=slots
        with broker.lock:
            cost,calls=broker.ledger.db.execute('SELECT COALESCE(SUM(COALESCE(actual,reserve)),0),COUNT(*) FROM attempts WHERE role LIKE ?',(group_id+'/%',)).fetchone()
        row.update(calls=calls,reserved_or_measured_peak_cny=cost/1e6)
        amendment_rows.append(row)
        outcomes=[row if old['task']==task and old['group']=='B' else old for old in outcomes]
        print(json.dumps(row,ensure_ascii=False),flush=True)
    evaluation=root/'evaluation';evaluation.mkdir()
    submissions=[{k:r[k] for k in ('task','group','path','sha256')} for r in outcomes if r['status']=='completed']
    cfg={'labels':str(original/'dataset/evaluator/labels.jsonl'),'submissions':submissions,'output':str(evaluation/'proxy.json')}
    (evaluation/'job.json').write_bytes(encoded(cfg))
    profile=evaluation/'worker.sb';profile.write_text(sandbox.profile(read_files=[HERE/'evaluate.py',Path(cfg['labels']),*[Path(r['path']) for r in submissions]],write_dirs=[evaluation]))
    result=sandbox.execute([sys.executable,'-B',HERE/'evaluate.py',evaluation/'job.json'],profile,evaluation)
    budget=broker.totals()
    report={'phase':'P1-B-controller-amendment-01','original_completed':prior['completed'],'groups_requested':18,
        'completed':sum(r['status']=='completed' for r in outcomes),'budget':budget,
        'added_generation_requests':budget['attempts']-prior['budget']['attempts'],
        'new_generation_request_ids':sorted({r[0] for r in broker.ledger.db.execute('SELECT id FROM attempts')}-initial_ids),
        'model_and_prompts_unchanged':True,'failed_slots_retried':0,'evaluation_process_ok':result.returncode==0,
        'human_review':'pending','semantic_improvement':'not established','production_enabled':False,
        'amendments':amendment_rows,'outcomes':outcomes}
    if result.returncode:report['evaluation_error']=result.stderr[-1000:]
    (root/'amended-report.json').write_bytes(encoded(report));broker.ledger.close()
    print(json.dumps({k:v for k,v in report.items() if k not in ['outcomes','amendments']},ensure_ascii=False),flush=True)
    return 0 if all(r['status']=='completed' for r in amendment_rows) else 1


if __name__=='__main__':raise SystemExit(main())
