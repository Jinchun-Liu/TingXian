#!/usr/bin/env python3
"""Cross-packet contract tests using invented evidence and fixture outputs only."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[1]
def load(name,path):
    spec=importlib.util.spec_from_file_location(name,path);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
t=load('prep_tests',ROOT/'scripts/test-toolkit-preparation.py')
c=load('consolidation',ROOT/'interview/toolkit/consolidation.py')
f=load('fixture',ROOT/'scripts/consolidation-fixture.py')
p,h=c.p,t.h

def setup():
    b=t.bundle();template=b['documents'][0]
    b['documents']=[{**copy.deepcopy(template),'id':f'{i+1:064x}','name':f'合成证据{i}.md','records':[copy.deepcopy(template['records'][i%3])]} for i in range(4)]
    b['documents'][3]['records'][0]['quote']='合成同名项目记录：只做方案评审，没有编写测试。'
    view=t.archive(b);task=p.task_spec(t.TASK)
    cap=max(p.upper(p.messages(task,[p.model_record(r)],'0'*64)) for r in view['records'])+5
    base=p.plan(view,t.TASK,input_cap=cap);proposals=[t.proposal(base,i) for i in range(len(base['parts']))]
    parent=p.collect(base,proposals,'fixture');group=f.groups(c,parent);plan=c.plan(parent,group)
    return b,cap,base,proposals,parent,group,plan

class Tests(unittest.TestCase):
    def test_cross_packet_merge_retains_originals_gaps_conflicts_and_review_hash(self):
        _,_,base,_,parent,_,plan=setup();self.assertEqual(len(base['parts']),4);self.assertEqual(len(parent['stories']),2)
        before=copy.deepcopy(parent);out=c.collect(plan,f.responses(plan),'fixture')
        self.assertEqual(parent,before);self.assertEqual(len(out['stories']),1)
        self.assertEqual(out['source_records'],parent['source_records']);self.assertEqual(out['consolidation']['parent_candidate'],parent)
        self.assertEqual(len(out['consolidation']['conflicts']),1);self.assertEqual(out['consolidation']['conflicts'][0]['status'],'unresolved')
        self.assertFalse(out['quality_pass']);self.assertFalse(out['harness2_enabled']);self.assertIsNone(out['actual_cost_cny'])
        self.assertEqual(out['consolidation']['budget']['planned_requests'],4);self.assertEqual(out['model_calls_executed_by_tool'],0)
        rendered=p.render(out);self.assertIn('未决冲突',rendered);self.assertIn('跨块整合候选',rendered)
        team=p.team_candidate(out);self.assertEqual(team['artifact'],out);self.assertTrue(any('不一致' in x['claim'] for x in team['claim_evidence_map']))
        reviews={role:{'schema_version':'review-1','reviewer':role,'mode':'coaching' if role=='consultant' else 'artifact_review','candidate_id':team['id'],'candidate_hash':h.digest(team),'findings':[],'missing_evidence':[],'prioritized_actions':[],'status':'reviewed'} for role in h.ROLES}
        self.assertTrue(h.review_round(team,reviews,set(base['records']))['structurally_complete'])
        changed=copy.deepcopy(team);changed['artifact']['consolidation']['conflicts']=[]
        with self.assertRaisesRegex(ValueError,'Stale'):h.review_round(changed,reviews,set(base['records']))
    def test_same_title_can_stay_separate_and_all_story_ids_must_be_accounted(self):
        *_,parent,group,plan=setup()
        split=f.groups(c,parent,merge=False);other=c.plan(parent,split)
        self.assertEqual(len(c.collect(other,f.responses(other))['stories']),2)
        for change in [lambda g:g['groups'][0]['story_ids'].pop(),lambda g:g['groups'][0]['story_ids'].append(g['groups'][0]['story_ids'][0]),lambda g:g.update(input_hash='f'*64),lambda g:g['groups'][0].update(reason='')]:
            bad=copy.deepcopy(group);change(bad)
            with self.assertRaises(ValueError):c.plan(parent,bad)
        rs=f.responses(plan);rs[-1]['stories'][0]['from_story_ids'].pop()
        with self.assertRaises(ValueError):c.collect(plan,rs)
    def test_omitted_evidence_requires_a_reason_and_remains_auditable(self):
        *_,plan=setup();rs=f.responses(plan);rid=rs[0]['requirement']['references'].pop()['record_id']
        with self.assertRaisesRegex(ValueError,'disappeared'):c.collect(plan,rs)
        rs[0]['omitted']=[{'record_id':rid,'reason':'重复通用参考，未作为个人证据'}]
        out=c.collect(plan,rs);self.assertIn(rid,[r['record_id'] for r in out['source_records']]);self.assertIn('省略选材',p.render(out))
        rs[0]['omitted'][0]['reason']=''
        with self.assertRaises(ValueError):c.collect(plan,rs)
    def test_gap_resolution_requires_selected_personal_evidence(self):
        *_,plan=setup();rs=f.responses(plan);gap=rs[0]['gap_review'][0]
        self.assertTrue(gap)
        bad=copy.deepcopy(rs);bad[0]['gap_review'].pop()
        with self.assertRaisesRegex(ValueError,'Every input gap'):c.collect(plan,bad)
        bad=copy.deepcopy(rs);bad[0]['gap_review'][0]['output_gap']=999
        with self.assertRaisesRegex(ValueError,'remain visible'):c.collect(plan,bad)
        practice=next(r['record_id'] for r in plan['parent']['source_records'] if r['kind']=='practice')
        gap.update(status='resolved',output_gap=None,evidence_ids=[practice])
        with self.assertRaisesRegex(ValueError,'personal sources'):c.collect(plan,rs)
        gap['evidence_ids']=[rs[0]['requirement']['matches'][0]['record_id']]
        self.assertEqual(c.collect(plan,rs)['consolidation']['parts'][0]['gap_review'][0]['status'],'resolved')
        # A valid citation is not proof that the gap is semantically resolved.
        self.assertFalse(c.collect(plan,rs)['quality_pass'])
    def test_cross_project_evidence_activity_reference_kind_and_conflict_guards(self):
        *_,plan=setup();original=f.responses(plan)
        for mutate in [lambda rs:rs[-1]['stories'][0].update(activity='completed'),
                       lambda rs:rs[-1]['conflicts'][0].update(record_ids=[plan['parts'][-1]['record_ids'][0]]),
                       lambda rs:rs[-1]['conflicts'][0].update(record_ids=['f'*64,'e'*64]),
                       lambda rs:rs[-1]['stories'][0]['sections']['action'][0].update(record_ids=[next(r['record_id'] for r in plan['parent']['source_records'] if r['kind']=='practice')]),
                       lambda rs:rs[0]['requirement']['matches'][0].update(record_id=next(r['record_id'] for r in plan['parent']['source_records'] if r['kind']=='practice'))]:
            bad=copy.deepcopy(original);mutate(bad)
            with self.assertRaises(ValueError):c.collect(plan,bad)
        bad=copy.deepcopy(original);story=bad[-1]['stories'][0];a,b=story['from_story_ids'];one=copy.deepcopy(story);two=copy.deepcopy(story)
        one['from_story_ids']=[a];two['from_story_ids']=[b];bad[-1]['stories']=[one,two]
        with self.assertRaisesRegex(ValueError,'different project'):c.collect(plan,bad)
    def test_budget_refuses_oversize_without_partial_mutation_or_truncation(self):
        *_,parent,group,plan=setup();before=copy.deepcopy(parent)
        with self.assertRaisesRegex(ValueError,'budget'):c.catalog(parent,c.limits(input_cap=32))
        small=c.limits(input_cap=5000);smallgroup=f.groups(c,parent,small)
        with self.assertRaisesRegex(ValueError,'budget'):c.plan(parent,smallgroup,small)
        self.assertEqual(before,parent)
        with self.assertRaises(ValueError):c.limits(context_tokens=8192)
        self.assertEqual(c.limits(context_tokens=1048576)['context_tokens'],262144)
        for i in range(len(plan['parts'])):
            item=c.input_packet(plan,i+1);self.assertEqual(p.upper(item['messages']),item['input_upper'])
            self.assertLessEqual(item['input_upper']+item['limits']['output_reserved']+item['limits']['safety_reserved'],262144)
    def test_cache_is_per_input_and_stale_partial_outputs_do_not_replace_parent(self):
        *_,parent,group,plan=setup();rs=f.responses(plan)
        self.assertEqual(c.plan(parent,group)['parts'],plan['parts'])
        for bad in [rs[:-1],rs+[rs[0]]]:
            with self.assertRaises(ValueError):c.collect(plan,bad)
        changed=copy.deepcopy(parent);changed['candidate_hash']='f'*64;changed['role_view'][0]['matches'][0]['reason']='更具体的相关性理由'
        after=c.plan(changed,f.groups(c,changed));old={x['input_hash'] for x in plan['parts']};new={x['input_hash'] for x in after['parts']}
        self.assertEqual(len(old&new),2)
        with self.assertRaisesRegex(ValueError,'incomplete'):c.collect(after,rs)
        extra=[r for r in f.responses(after) if r['input_hash'] not in old];out=c.collect(after,rs+extra)
        self.assertEqual(len(out['consolidation']['ignored_stale_inputs']),1)
        changed['library_id']='other-owner'
        with self.assertRaisesRegex(ValueError,'Stale'):c.plan(changed,group)
    def test_real_cli_scope_parent_rebuild_and_outputs(self):
        b,cap,_,proposals,parent,group,plan=setup()
        folder=Path(tempfile.mkdtemp(prefix='tingxian-consolidation-cli-'))
        scope={'format':'tingxian-preparation-scope-v1','library_id':'synthetic-owner','bundle':b,'revisions':[],'excluded':[],'index_version':'1'}
        for name,value in [('scope',scope),('task',t.TASK),('proposals',proposals),('groups',group),('responses',f.responses(plan))]:(folder/(name+'.json')).write_text(json.dumps(value,ensure_ascii=False))
        base=[sys.executable,'-B',str(ROOT/'interview/toolkit/harness.py')]
        args=['--scope',str(folder/'scope.json'),'--task',str(folder/'task.json'),'--proposals',str(folder/'proposals.json'),'--input-cap',str(cap),'--execution-mode','fixture']
        def run(action,extra=(),check=True):return subprocess.run(base+[action]+args+list(extra),capture_output=True,text=True,check=check,timeout=10)
        self.assertEqual(json.loads(run('consolidation-catalog').stdout)['input_hash'],group['input_hash'])
        groupsargs=['--groups',str(folder/'groups.json')]
        manifest=json.loads(run('consolidation-plan',groupsargs).stdout);packet=json.loads(run('consolidation-input',groupsargs+['--part','1']).stdout)
        self.assertEqual(packet['input_hash'],manifest['parts'][0]['input_hash'])
        more=groupsargs+['--responses',str(folder/'responses.json')];out=json.loads(run('consolidation-collect',more).stdout)
        self.assertEqual(out,c.collect(plan,f.responses(plan),'fixture'))
        self.assertEqual(json.loads(run('consolidation-collect',more+['--output-format','team']).stdout)['artifact'],out)
        self.assertIn('未决冲突',run('consolidation-collect',more+['--output-format','markdown']).stdout)
        # Withdrawal or another owner invalidates the rebuilt parent before consolidation.
        for patch in [{'excluded':[b['documents'][0]['id']]},{'library_id':'other-owner'},{'revisions':[{'record_id':parent['source_records'][0]['record_id'],'claim':'本人重新核对，仅做计划','basis':'本人核对','revision_id':'rev1','activity':'planned'}]}]:
            (folder/'scope.json').write_text(json.dumps({**scope,**patch},ensure_ascii=False))
            self.assertNotEqual(run('consolidation-collect',more,False).returncode,0)

if __name__=='__main__':unittest.main()
