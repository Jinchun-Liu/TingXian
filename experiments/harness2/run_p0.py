#!/usr/bin/env python3
"""Run pinned official orchestration without keys, AI requests or app access."""
import argparse
import hashlib
import importlib.metadata
import json
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import time

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
import fixtures
import policy


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--limit',type=int,default=24)
    args=parser.parse_args()
    if not 1<=args.limit<=24: parser.error('limit must be 1..24')
    upstream=args.upstream.resolve();root=args.output.resolve()
    actual=subprocess.check_output(['git','-C',str(upstream),'rev-parse','HEAD'],text=True).strip()
    if actual!=policy.UPSTREAM_COMMIT: raise ValueError('Upstream commit changed')
    if subprocess.check_output(['git','-C',str(upstream),'status','--porcelain'],text=True).strip():raise ValueError('Modified upstream source')
    if root.exists():raise ValueError('Use a fresh output directory; prior runs remain intact')
    if shutil.disk_usage(root.parent).free<3*1024**3:raise ValueError('Need estimated 1 GiB plus 2 GiB margin before starting')
    root.mkdir(parents=True)
    sys.path.insert(0,str(upstream))
    from harness2.core.schedule import run_task
    from harness2.core.spec import BenchmarkAdapter
    from adapter import TingXianAdapter,SmokeProposer,digest,write_json,HOST
    cases=fixtures.build(root/'dataset')
    # Freeze independent labels before candidate execution. No model-backed
    # blind evaluation is performed, and the adapter receives no gold path.
    manifest={'experiment':'TX-H2-U-P0-20261107','route':'U','host_commit':subprocess.check_output(['git','-C',str(HERE.parents[1]),'rev-parse','HEAD'],text=True).strip(),'host_packer_sha256':digest(HOST),'upstream_commit':actual,'upstream_spec_sha256':digest(upstream/'harness2/core/spec.py'),'platform':platform.platform(),'python':sys.version,'dependencies':{n:importlib.metadata.version(n) for n in ['python-docx','pypdf','lxml','typing_extensions']},'cases_sha256':digest(root/'dataset/cases.jsonl'),'labels_sha256':digest(root/'dataset/evaluator/labels.jsonl'),'base_policy_sha256':policy.signature(policy.BASE),'mode':'agg','k':1,'paid_execution_enabled':False,'model':'deterministic transport probe; no AI','max_parallel_workers':2,'context_cap':262144,'semantic_evaluation':False,'human_review':'pending','created':time.time()}
    manifest['adapter_files_sha256']={p.name:digest(p) for p in sorted(HERE.glob('*.py'))}
    write_json(root/'experiment-manifest.json',manifest)
    adapter=TingXianAdapter(root/'experiment',root/'dataset');assert isinstance(adapter,BenchmarkAdapter)
    prop=SmokeProposer(adapter.spec.channels['proposer'])
    results=[];consecutive=0
    for task in adapter.tasks()[:args.limit]:
        try:
            result=run_task(task,1,adapter=adapter,prop=prop,runs_root=adapter.spec.runs_root,modes=('agg',),tag='p0',seed=0)
            row={'task':task,'status':result.status,'errors':result.errors}
        except Exception as exc: row={'task':task,'status':'exception','errors':[str(exc)]}
        results.append(row);print(json.dumps(row,ensure_ascii=False),flush=True)
        consecutive=0 if row['status']=='ok' else consecutive+1
        if consecutive>=3:adapter.cancel();break
    unchanged=all(adapter._sources(c['id'])==c['files'] for c in cases)
    artifacts=[]
    for path in sorted((adapter.root/'executions').glob('*/state.json')):
        state=json.loads(path.read_text())
        if state['status']=='completed':artifacts.append({'task':state['task'],'run_id':state['run_id'],'policy':state['policy'],'bundle_sha256':state['artifact_sha256']})
    report={'experiment':manifest['experiment'],'route':'U','tasks_requested':args.limit,'tasks_attempted':len(results),'tasks_completed':sum(r['status']=='ok' for r in results),'execution_attempts':adapter.execution_count,'completed_artifacts':len(artifacts),'editor_calls':len(prop._calls),'original_sources_unchanged':unchanged,'gold_not_scored':adapter.score_calls==0,'model_calls':0,'actual_api_cost':0,'quality_improvement':'not evaluated','production_enabled':False,'results':results,'artifacts':artifacts}
    write_json(root/'smoke-report.json',report)
    # A portable candidate is data only and cannot install/activate itself.
    candidates={}
    for summary_path in adapter.spec.runs_root.rglob('summary.json'):
        summary=json.loads(summary_path.read_text())
        selection=summary.get('selections',{}).get('agg',{})
        if not selection.get('ok'):continue
        tree=policy.read_tree(summary_path.parent/'harnesses'/selection['chosen_label'])
        sig=policy.signature(tree)
        if not any(a['policy']==sig and a['run_id']==selection['chosen_run_id'] for a in artifacts):
            raise ValueError('Selected policy has no matching verified execution')
        candidates.setdefault(sig,{'policy':tree,'policy_sha256':sig,'runs':[]})['runs'].append(selection['chosen_run_id'])
    write_json(root/'strategy-candidate.json',{'format':'tingxian-strategy-candidate-v1','status':'candidate','scope':'synthetic-background-p0','candidates':list(candidates.values()),'evidence':'smoke-report.json','semantic_validated':False,'approved_for_production':False})
    return int(report['tasks_completed']!=args.limit or not unchanged)

if __name__=='__main__':raise SystemExit(main())
