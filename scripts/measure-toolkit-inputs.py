#!/usr/bin/env python3
"""Prepare 12 paired inputs locally. No network, model calls, or quality score."""
import argparse
import hashlib
import importlib.util
import json
import statistics
import subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('toolkit',ROOT/'interview/toolkit/harness.py')
h=importlib.util.module_from_spec(spec);spec.loader.exec_module(h)
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--style-mode',choices=['full-cards','compact-candidate'],default='full-cards')
    args=parser.parse_args()
    if args.binary.name not in ('toolkit_prepare','toolkit_prepare.exe'):raise ValueError('Use the prepare-only toolkit example')
    registry=h.load_registry(ROOT/'interview/toolkit/registry.json')
    cases=json.loads((ROOT/'interview/toolkit/validation/spoken.json').read_text())
    results=[]
    for case in cases:
        mode='meeting' if case['category']=='meeting' else 'interview'
        policy=h.compile_policy(registry,h.route(case['question'],mode,'测试开发'),preview=True)
        method=policy['method_text'] if args.style_mode=='compact-candidate' else policy['text']
        payload={k:case[k] for k in ('question','facts','dialogue')}
        payload.update(method_text=method,preferences={'scenario':mode})
        completed=subprocess.run([str(args.binary.resolve())],input=json.dumps(payload,ensure_ascii=False),text=True,capture_output=True,check=True,timeout=15)
        prepared=json.loads(completed.stdout);report=prepared['report']
        if not report['user_context_identical'] or report['model_calls']!=0:raise ValueError('Invalid comparison boundary')
        report.update(case_id=case['id'],rule_ids=policy['method_rule_ids'] if args.style_mode=='compact-candidate' else policy['rule_ids'],style_reference_ids=policy['style_rule_ids'],routing_compile_ms=policy['elapsed_ms'],model_quality='not_run',read_aloud='not_run',input_byte_reduction_vs_legacy=1-report['candidate_text_bytes']/report['legacy_text_bytes'])
        results.append(report)
    elapsed=sorted(r['routing_compile_ms'] for r in results)
    result={'style_mode':args.style_mode,'style_semantic_equivalence':'unvalidated candidate, not exact rule deduplication','schema_version':'toolkit-paired-preparation-1','samples':len(results),'results':results,
            'median_input_byte_reduction_vs_legacy':statistics.median(r['input_byte_reduction_vs_legacy'] for r in results),
            'routing_compile_p50_ms':statistics.median(elapsed),'routing_compile_p95_ms':elapsed[-1],
            'measurement_scope':'local deterministic routing/assembly only; excludes process spawn, retrieval, ASR, network, model',
            'provider_token_usage':None,'actual_cost_cny':None,'paid_calls':0,'human_quality':'not_run','cpu_memory_peaks':'not_measured','default_policy_changed':False,
            'source_hashes':{n:hashlib.sha256((ROOT/n).read_bytes()).hexdigest() for n in ['crates/interview-core/src/model.rs','crates/interview-core/src/toolkit_policy.rs','interview/toolkit/registry.json','interview/toolkit/validation/spoken.json']}}
    with args.out.open('x') as f:json.dump(result,f,ensure_ascii=False,indent=2)
    print(json.dumps({k:result[k] for k in ['samples','median_input_byte_reduction_vs_legacy','routing_compile_p95_ms','paid_calls']},ensure_ascii=False))
if __name__=='__main__':main()
