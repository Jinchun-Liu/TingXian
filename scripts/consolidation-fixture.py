"""Deterministic synthetic proposals, never a substitute for model evidence."""
import copy

def groups(c, parent, budget=None, merge=True):
    ids=[s['id'] for s in parent['stories']]
    return {'format':'consolidation-groups-v1','input_hash':c.catalog(parent,budget)['input_hash'],
            'groups':[{'story_ids':row,'reason':'合成分组建议，仍需核对项目身份'} for row in ([ids] if merge and ids else [[sid] for sid in ids])]}

def responses(plan):
    sources={r['record_id']:r for r in plan['parent']['source_records']}
    result=[]
    for item in plan['parts']:
        row={'format':'consolidation-part-v1','input_hash':item['input_hash'],'kind':item['kind'],'omitted':[],'conflicts':[]}
        if item['kind']=='requirement':
            original=item['requirement']
            row['requirement']={k:copy.deepcopy(original[k]) for k in ('id','matches','references')}
            gaps=list(dict.fromkeys(g['text'] for g in item['input_gaps']))
            row['requirement']['gaps']=gaps
            row['gap_review']=[{'id':g['id'],'status':'retained','reason':'合成方案保留原始缺口，等待证据','evidence_ids':[],'output_gap':gaps.index(g['text'])} for g in item['input_gaps']]
        else:
            ids=item['record_ids']
            row['stories']=[{'title':'合成跨块契约测试','activity':'unknown',
                'sections':{'context':[],'task':[],'action':[{'text':sources[rid]['claim'],'record_ids':[rid]} for rid in ids], 'result':[],'learning':[]},
                'missing':{k:'需要本人补充可核对证据' for k in ('context','task','result','learning')},
                'from_story_ids':[s['id'] for s in item['stories']]}]
            if len(ids)>1:row['conflicts']=[{'text':'来源对职责描述不一致，需本人核对；未自动裁定','record_ids':ids[:2]}]
        result.append(row)
    return result
