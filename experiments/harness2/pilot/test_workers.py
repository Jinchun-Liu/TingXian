"""Exercise real Mac worker isolation + official orchestration with fake transport."""
import argparse
import json
from pathlib import Path
import sys

p=argparse.ArgumentParser();p.add_argument('--upstream',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE));sys.path.insert(1,str(HERE.parent));sys.path.insert(0,str(a.upstream.resolve()))
from broker import Broker
from pilot_adapter import PilotAdapter,PilotProposer,HOST
from harness2.core.schedule import run_task
import fixtures,policy

root=a.output.resolve();root.mkdir(parents=True,exist_ok=False);fixtures.build(root/'dataset')
calls=[]
def fake(payload):
    system=payload['messages'][0]['content'];data=json.loads(payload['messages'][1]['content']);calls.append(payload)
    if '仅有version、order' in system:
        value={'version':1,'order':'constraints_first','extraction':'优先保留限制和连续引文。','review':'不要自动确认，保留冲突。'}
    elif '"selected"' in system:
        value={'selected':data['candidates'][0]['id'],'reason':'mock selection; not semantic evidence'}
    elif isinstance(data,dict) and 'segments' in data:
        value={'records':[{'kind':'technical','location':r['location'],'quote':r['text'][:650],'note':'synthetic test only'} for r in data['segments']]}
    else:
        value={'overview':'synthetic test only','role':'','aliases':'','settings':dict.fromkeys(['role','organization','context','terminology','call_names'],'')}
    return {'choices':[{'finish_reason':'stop','message':{'content':json.dumps(value,ensure_ascii=False)}}],'usage':{'prompt_tokens':100,'completion_tokens':20}}

b=Broker(root/'broker',key='synthetic-runtime-canary',transport=fake)
ad=PilotAdapter(root/'experiment',root/'dataset',b)
with b.group('clear-a-C0',3500000):
    prop=PilotProposer(ad.spec.channels['proposer'],b,root/'proposers')
    r=run_task('clear-a',1,adapter=ad,prop=prop,runs_root=ad.spec.runs_root,modes=('agg',),tag='offline',seed=0)
assert r.status=='ok',r.errors
assert ad.execution_count==4 and len(prop._calls)==3
outputs=list((ad.root/'executions').glob('*/artifacts/bundle.meetily.json'));assert len(outputs)==4
with b.group('clear-a-B',3500000):
    result=b.run_worker(root/'selector',{'job_id':'select','role':'reviewer','candidates':[{'id':str(i),'path':str(p)} for i,p in enumerate(outputs)]},read_files=outputs)
assert result['selected']=='0'
assert all('synthetic-runtime-canary' not in json.dumps(p) for p in calls)
for file in (root/'broker/calls').rglob('*.json'):
    assert 'synthetic-runtime-canary' not in file.read_text()
summary={'official_core_status':r.status,'execution_count':ad.execution_count,'editor_calls':len(prop._calls),'broker_requests':len(calls),'real_api_calls':0,'selector_passed':True,'credential_absent_from_requests_and_artifacts':True}
(root/'test-report.json').write_text(json.dumps(summary,indent=2)+'\n');print(json.dumps(summary));b.ledger.close()
