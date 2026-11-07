#!/usr/bin/env python3
"""Synthetic offline planning overhead; does not measure model quality or fees."""
import copy
import importlib.util
import json
from pathlib import Path
import resource
import statistics
import sys
import time

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('checks',ROOT/'scripts/test-toolkit-consolidation.py')
t=importlib.util.module_from_spec(spec);spec.loader.exec_module(t)
p,c,f=t.p,t.c,t.f
b=t.t.bundle();template=b['documents'][0]
b['documents']=[{**copy.deepcopy(template),'id':f'{i+1:064x}','name':f'合成项目{i}.md','records':[copy.deepcopy(template['records'][0])]} for i in range(20)]
view=t.t.archive(b);task=p.task_spec(t.t.TASK)
cap=max(p.upper(p.messages(task,[p.model_record(r)],'0'*64)) for r in view['records'])+10
def build(bundle):
    base=p.plan(t.t.archive(bundle),t.t.TASK,input_cap=cap)
    parent=p.collect(base,[t.t.proposal(base,i) for i in range(len(base['parts']))],'fixture')
    return parent,c.plan(parent,f.groups(c,parent,merge=False))
times=[]
for _ in range(6):
    start=time.perf_counter();parent,plan=build(b);out=c.collect(plan,f.responses(plan),'fixture');times.append((time.perf_counter()-start)*1000)
changed=copy.deepcopy(b);changed['documents'][1]['records'][0]['quote']=changed['documents'][1]['records'][0]['quote'].replace('null','NULL')
_,next_plan=build(changed)
before={r['input_hash'] for r in plan['parts']};after={r['input_hash'] for r in next_plan['parts']}
current=c.manifest(plan)
report={'mode':'synthetic_fixture','documents':20,'records':len(view['records']),'repetitions':len(times),
        'preparation_planned_requests':len(parent['budget']['parts']), 'additional_consolidation_planned_requests':current['planned_requests'],
        'preparation_input_upper_cap':cap,'preparation_fixture_partition':'One source per packet to expose cache invalidation; not the default production partition',
        'preparation_input_upper':parent['budget']['total_input_upper'],'additional_consolidation_input_upper':current['total_input_upper'],
        'consolidation_largest_input_upper':max(current['catalog_input_upper'],*(r['input_upper'] for r in current['parts'])),
        'consolidation_limits':current['limits'],'same_inputs_reusable_parts':len(before),
        'single_source_change_reusable_parts':len(before&after),'single_source_change_new_parts':len(after-before),
        'single_source_change_catalog_changed':plan['catalog']['input_hash']!=next_plan['catalog']['input_hash'],
        'offline_pipeline_ms_p50':statistics.median(times),'offline_pipeline_ms_p95':max(times),
        'process_peak_rss_bytes':resource.getrusage(resource.RUSAGE_SELF).ru_maxrss*(1 if sys.platform=='darwin' else 1024),
        'model_calls':0,'actual_input_tokens':None,'model_latency_ms':None,'actual_cost_cny':None,'semantic_quality_validated':False,
        'candidate_json_bytes':len(p.canonical(out).encode()),
        'limits_note':'Six local repeats; p95 is nearest rank (max). UTF-8 upper estimates are not model tokens. Added consolidation work is not a cost saving. Four-role review and retries are not included in planned request counts.'}
print(json.dumps(report,ensure_ascii=False,indent=2))
