#!/usr/bin/env python3
"""Synthetic offline packet preparation, not model or realtime latency."""
import argparse
import importlib.util
import json
import resource
import statistics
import subprocess
import sys
import time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def load(name):
    spec=importlib.util.spec_from_file_location(name,ROOT/'interview/toolkit'/(name+'.py'))
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module
h,p=load('harness'),load('preparation')
def child(count,preview):
    bundle={'format':'meetily-background-v1','documents':[{'id':f'{i+1:064x}','hash':f'{i+2:064x}','name':f'fictional-{i}.txt','records':[{'kind':'personal','location':'行1','quote':f'虚构记录 {i}：只在模拟环境进行参数化测试，未上线，未处理生产事故。','note':'模拟资料，保留贡献限制。'}]} for i in range(count)],'profile':{'overview':'合成计时资料','role':'测试开发','aliases':''}}
    archive=h.archive_bundle(bundle,'synthetic-timing','测试开发')
    task={'role':'测试开发','organization':'虚构团队','requirements':['契约测试与边界条件','数据库事务与幂等','真实生产故障处理']}
    methods=h.preparation_policy(h.load_registry(ROOT/"interview/toolkit/registry.json"),task,preview)
    durations=[];cpu=[]
    for n in range(6):
        before=resource.getrusage(resource.RUSAGE_SELF);start=time.perf_counter();plan=p.plan(archive,task,method_pack=methods);elapsed=(time.perf_counter()-start)*1000
        after=resource.getrusage(resource.RUSAGE_SELF)
        if n:durations.append(elapsed);cpu.append((after.ru_utime+after.ru_stime-before.ru_utime-before.ru_stime)*1000)
    manifest=p.manifest(plan)
    peak=resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return {'methods_preview':preview,'method_rule_count':len(methods['rule_ids']) if methods else 0,'records':count,'samples':5,'plan_p50_ms':statistics.median(durations),'plan_p95_ms':max(durations),'cpu_p50_ms':statistics.median(cpu),'process_peak_rss_bytes':peak if sys.platform=='darwin' else peak*1024,'packets':len(manifest['parts']),'total_input_upper':manifest['total_input_upper'],'peak_packet_input_upper':max(v['input_upper'] for v in manifest['parts']),'output_reserved_per_packet':manifest['limits']['output_reserved'],'context_tokens':manifest['limits']['context_tokens'],'real_api_requests':0}
def main():
    a=argparse.ArgumentParser(description=__doc__);a.add_argument('--child',type=int);a.add_argument('--preview-methods',action='store_true');args=a.parse_args()
    if args.child:
        print(json.dumps(child(args.child,args.preview_methods)));return
    results=[]
    for preview in (False,True):
        for count in (10,100,1000,2000):
            command=[sys.executable,'-B',__file__,'--child',str(count)]+(['--preview-methods'] if preview else [])
            result=subprocess.run(command,capture_output=True,text=True,check=True,timeout=120)
            results.append(json.loads(result.stdout));print(f'Completed {count} synthetic records; preview={preview}',file=sys.stderr)
    print(json.dumps({'scope':'Offline Python planning only; archive built before measurement; one warmup + five samples, p95 is max of this small sample; fresh process per size; RSS includes interpreter/archive and planning, not an incremental component peak. No models, provider tokens or end-to-end latency.','results':results},ensure_ascii=False))
if __name__=='__main__':main()
