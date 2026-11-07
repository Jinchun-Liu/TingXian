"""P0 mechanical extractor, deliberately NOT an AI quality benchmark."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import sys


def load_host(path):
    spec=importlib.util.spec_from_file_location('fixed_background',path)
    host=importlib.util.module_from_spec(spec);spec.loader.exec_module(host)
    return host


def run(source_root, output, host_path, policy_file):
    host=load_host(host_path);policy=json.loads(policy_file.read_text())
    inventory=host.inventory(source_root)
    ordered=list(inventory.items())
    if policy['order']=='constraints_first': ordered.sort(key=lambda item:('限制' not in item[1][1] and '冲突' not in item[1][1],item[1][1]))
    docs=[];files=[]
    for key,(path,name) in ordered:
        original=hashlib.sha256(path.read_bytes()).hexdigest()
        try:
            h,parts=host.source(path,name)
        except ValueError as exc:
            files.append({'name':name,'hash':original,'status':'skipped','reason':str(exc)});continue
        records=[]
        for part in parts:
            # All source text is quoted as technical material for this transport
            # probe. No personal facts or automatic settings are inferred.
            rows=[{'kind':'technical','location':r['location'],'quote':r['text'][:650],'note':'P0机械引文；类别与语义未经模型或人工确认。'} for r in part]
            records.extend(host.records({'records':rows},part))
        docs.append({'id':key,'name':name,'hash':h,'records':records})
        files.append({'name':name,'hash':h,'status':'completed','parts':len(parts),'records':len(records)})
        assert hashlib.sha256(path.read_bytes()).hexdigest()==original
    summary={'overview':'虚构资料接入测试。以下引文仅验证传输、定位和格式；未作个人背景判断，不可自动应用。','role':'','aliases':'','settings':dict.fromkeys(['role','organization','context','terminology','call_names'],'')}
    bundle=host.pack(docs,summary)
    host.verify_documents({'folder':str(source_root)},docs)
    output.mkdir(exist_ok=True,parents=True)
    for name,obj in [('bundle.meetily.json',bundle),('files.json',files)]:
        with (output/name).open('x',encoding='utf-8') as f: json.dump(obj,f,ensure_ascii=False,indent=2)
    return {'documents':len(docs),'files':len(files),'skipped':sum(f['status']=='skipped' for f in files),'model_calls':0,'quality_evaluated':False}

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--sources',type=Path,required=True);p.add_argument('--out',type=Path,required=True);p.add_argument('--host',type=Path,required=True);p.add_argument('--policy',type=Path,required=True)
    a=p.parse_args();print(json.dumps(run(a.sources,a.out,a.host,a.policy)))
