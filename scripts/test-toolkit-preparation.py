#!/usr/bin/env python3
"""Synthetic source/version, semantic-boundary and actual portable CLI checks."""
import copy
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'interview/toolkit' / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
h, p = load('harness'), load('preparation')
TASK = {'role': '测试开发实习生', 'organization': '虚构团队', 'requirements': ['契约测试与业务语义', '真实线上故障处理']}
def bundle():
    return {'format': 'meetily-background-v1', 'documents': [
        {'id': 'a'*64, 'hash': 'b'*64, 'name': 'synthetic.txt', 'records': [
            {'kind': 'personal', 'location': '行1', 'quote': '只在模拟环境编写了 null 与零的契约测试，未上线。', 'note': '测试练习，不代表生产事故经验'},
            {'kind': 'practice', 'location': '行2', 'quote': '参考答案：线上先止损，再定位根因。', 'note': '面试练习，不是个人经历'},
            {'kind': 'technical', 'location': '行3', 'quote': 'Schema 通过不等于业务语义正确。', 'note': '通用技术方法'}]}],
        'profile': {'overview': '虚构测试资料', 'role': '测试', 'aliases': ''}}
def archive(b=None, revisions=None, excluded=None, owner='synthetic-owner'):
    return h.archive_bundle(b or bundle(), owner, '', revisions, excluded)
def proposal(plan, part=0):
    packet = plan['parts'][part]
    personal = [rid for rid in packet['record_ids'] if plan['records'][rid]['kind'] in p.PERSONAL]
    technical = [rid for rid in packet['record_ids'] if plan['records'][rid]['kind'] not in p.PERSONAL]
    rows = []
    for requirement in plan['task']['requirements']:
        supported = requirement['id'] == 'req-1' and personal
        rows.append({'id': requirement['id'], 'matches': ([{'record_id': personal[0], 'relevance': 'supporting', 'reason': '仅有模拟环境契约测试证据'}] if supported else []),
                     'references': [{'record_id': rid, 'purpose': '仅作方法参考'} for rid in technical],
                     'gaps': [] if supported else ['本批没有真实线上事故经历的个人证据']})
    stories = []
    if personal:
        sections = {s: [] for s in p.SECTIONS}
        sections['action'] = [{'text': '只在模拟环境编写契约测试，未上线。', 'record_ids': [personal[0]]}]
        stories = [{'title': '模拟契约测试', 'activity': 'unknown', 'sections': sections,
                    'missing': {s: '需要本人补充并核对证据' for s in p.SECTIONS if s != 'action'}}]
    return {'format': 'preparation-part-v1', 'input_hash': packet['input_hash'], 'requirements': rows, 'stories': stories}
class Tests(unittest.TestCase):
    def test_background_method_preview_is_scoped_and_changes_input_identity(self):
        registry=h.load_registry(ROOT/'interview/toolkit/registry.json')
        self.assertIsNone(h.preparation_policy(registry,TASK))
        methods=h.preparation_policy(registry,TASK,True)
        self.assertIn('CV:C13',methods['rule_ids']);self.assertIn('INT:G03',methods['rule_ids'])
        self.assertNotIn('CV:C15',methods['rule_ids']);self.assertFalse(any(r.startswith(('SP:','LIVE:','INT:R')) for r in methods['rule_ids']))
        before=p.plan(archive(),TASK);after=p.plan(archive(),TASK,method_pack=methods)
        self.assertNotEqual(before['parts'][0]['input_hash'],after['parts'][0]['input_hash'])
        self.assertIn(methods['text'],p.input_packet(after,1)['messages'][0]['content'])
        self.assertGreater(after['parts'][0]['input_upper'],before['parts'][0]['input_upper'])
        with self.assertRaisesRegex(ValueError,'incomplete'):p.collect(after,[proposal(before)])
    def test_ranked_role_mapping_and_story_preserve_exact_evidence_and_unknowns(self):
        view = archive();plan = p.plan(view,TASK);result = p.collect(plan,[proposal(plan)],'fixture')
        self.assertEqual(view['records'][0]['claim'],bundle()['documents'][0]['records'][0]['quote'])
        self.assertNotEqual(view['records'][0]['claim'],view['records'][0]['extraction_note'])
        self.assertEqual(len(result['role_view'][0]['matches']),1)
        self.assertEqual(result['role_view'][1]['matches'],[])
        self.assertTrue(result['stories'][0]['missing']['result'])
        self.assertFalse(result['quality_pass']);self.assertFalse(result['application_imported'])
        self.assertIsNone(result['model_usage']);self.assertIsNone(result['actual_cost_cny'])
        self.assertEqual(result['source_records'],[plan['records'][rid] for rid in sorted(plan['records'])])
        rendered=p.render(result);self.assertIn('只在模拟环境',rendered);self.assertIn('待语义核对',rendered);self.assertIn('参考材料',rendered)
        candidate=p.team_candidate(result)
        for claim in candidate['claim_evidence_map']: self.assertTrue(set(claim['evidence_refs'])<=set(plan['records']))
        self.assertEqual(candidate['artifact']['candidate_hash'],result['candidate_hash'])
    def test_practice_and_general_reference_cannot_become_personal_experience(self):
        plan=p.plan(archive(),TASK);value=proposal(plan)
        practice=next(rid for rid,r in plan['records'].items() if r['kind']=='practice')
        value['requirements'][0]['matches'][0]['record_id']=practice
        with self.assertRaisesRegex(ValueError,'personal experience'):p.collect(plan,[value])
        value=proposal(plan);value['stories'][0]['sections']['action'][0]['record_ids']=[practice]
        with self.assertRaisesRegex(ValueError,'personal experience'):p.collect(plan,[value])
    def test_missing_unknown_or_foreign_references_and_unproven_status_are_rejected(self):
        plan=p.plan(archive(),TASK)
        for change in [lambda v:v.update(quality_pass=True),lambda v:v['requirements'].pop(),
                       lambda v:v['stories'][0]['missing'].pop('result'),
                       lambda v:v['stories'][0].update(activity='completed'),
                       lambda v:v['stories'][0]['sections']['action'][0].update(record_ids=['f'*64])]:
            value=proposal(plan);change(value)
            with self.assertRaises(ValueError):p.collect(plan,[value])
    def test_manual_revision_and_exclusion_invalidate_only_matching_inputs(self):
        original=archive();rid=original['records'][0]['record_id']
        rev={'record_id':rid,'claim':'本人修订：只做计划，尚未编写测试','basis':'本人核对','revision_id':'rev-1','activity':'planned'}
        changed=archive(revisions=[rev]);plan=p.plan(changed,TASK)
        self.assertNotIn(rid,plan['records']);self.assertEqual(len(plan['records']),3)
        new=next(r for r in plan['records'].values() if r['claim_status']=='user_asserted')
        self.assertEqual(new['source_refs'][0]['quote'],original['records'][0]['claim'])
        old_plan=p.plan(original,TASK)
        with self.assertRaisesRegex(ValueError,'incomplete'):p.collect(plan,[proposal(old_plan)])
        value=proposal(plan);value['stories'][0]['activity']='completed'
        with self.assertRaisesRegex(ValueError,'upgrade'):p.collect(plan,[value])
        value['stories'][0]['activity']='planned';self.assertTrue(p.collect(plan,[value])['stories'])
        excluded_plan=p.plan(archive(revisions=[rev],excluded=['a'*64]),TASK)
        out=p.collect(excluded_plan,[value]);self.assertEqual(out['stories'],[]);self.assertEqual(out['source_records'],[])
        self.assertEqual(out['ignored_stale_inputs'],[value['input_hash']]);self.assertTrue(out['role_view'][0]['gaps'])
    def test_revision_chain_rejects_two_current_assertions_for_one_source(self):
        original=archive();rid=original['records'][0]['record_id']
        first={'record_id':rid,'claim':'仅作计划','basis':'本人核对','revision_id':'rev1','activity':'planned'}
        revised=archive(revisions=[first]);new_id=revised['records'][-1]['record_id']
        stale={**first,'claim':'改为进行中','revision_id':'rev2','activity':'in_progress'}
        with self.assertRaisesRegex(ValueError,'Stale'):archive(revisions=[first,stale])
        final=archive(revisions=[first,{**stale,'record_id':new_id}]);current=[r for r in final['records'] if not r.get('superseded')]
        self.assertEqual(len(current),3);self.assertEqual(current[-1]['activity_status'],'in_progress')
        self.assertEqual(final['records'][-1]['source_refs'],original['records'][0]['source_refs'])
    def test_bounded_packets_resume_coverage_and_cross_packet_reference_rejection(self):
        view=archive();task=p.task_spec(TASK)
        cap=max(p.upper(p.messages(task,[p.model_record(r)],'0'*64)) for r in view['records'])+5
        plan=p.plan(view,TASK,input_cap=cap);self.assertEqual(len(plan['parts']),3)
        responses=[proposal(plan,i) for i in range(3)]
        with self.assertRaisesRegex(ValueError,'incomplete'):p.collect(plan,responses[:2])
        with self.assertRaisesRegex(ValueError,'Duplicate'):p.collect(plan,responses+[responses[0]])
        self.assertEqual(len(p.collect(plan,responses)['accepted_inputs']),3)
        wrong=copy.deepcopy(responses[0]);foreign=plan['parts'][1]['record_ids'][0]
        wrong['requirements'][0]['references']=[{'record_id':foreign,'purpose':'不能跨输入冒认看过'}]
        with self.assertRaisesRegex(ValueError,'out-of-packet'):p.validate_part(plan,wrong)
        for i in range(1,4):
            packet=p.input_packet(plan,i)
            self.assertLessEqual(packet['input_upper'],cap)
            self.assertLessEqual(packet['input_upper']+packet['limits']['output_reserved']+packet['limits']['safety_reserved'],262144)
        with self.assertRaises(ValueError):p.plan(view,TASK,context_tokens=8192)
        self.assertEqual(p.plan(view,TASK,context_tokens=1048576)['limits']['context_tokens'],262144)
    def test_identical_packets_reuse_but_task_owner_rules_and_sources_bind_cache(self):
        plan=p.plan(archive(),TASK);responses=[proposal(plan)]
        self.assertEqual(plan['parts'][0]['input_hash'],p.plan(archive(),TASK)['parts'][0]['input_hash'])
        for other in [p.plan(archive(owner='different-owner'),TASK),p.plan(archive(),{**TASK,'role':'后端实习生'}),p.plan(archive(),TASK,output_reserve=4096)]:
            with self.assertRaisesRegex(ValueError,'incomplete'):p.collect(other,responses)
        changed=bundle();changed['documents'][0]['records'][0]['quote']='同一文件的新版本：仍只是计划。'
        with self.assertRaisesRegex(ValueError,'incomplete'):p.collect(p.plan(archive(changed),TASK),responses)
        # Profiles are not source evidence; changing summary must not force a
        # new host call for unchanged exact records/task/prompt/budgets.
        changed=bundle();changed['profile']['overview']='只修改摘要'
        fresh=p.plan(archive(changed),TASK);self.assertNotEqual(fresh['snapshot_hash'],plan['snapshot_hash'])
        self.assertEqual(p.collect(fresh,responses)['accepted_inputs'],[responses[0]['input_hash']])
    def test_no_silent_truncation_credentials_or_html_execution(self):
        with self.assertRaises(ValueError):p.plan(archive(),{**TASK,'requirements':TASK['requirements']*20})
        value=proposal(p.plan(archive(),TASK));value['stories'][0]['title']='<script>alert(1)</script>'
        output=p.render(p.collect(p.plan(archive(),TASK),[value]));self.assertNotIn('<script>',output)
        value['stories'][0]['title']='sk-'+'synthetic0123456789'*2
        with self.assertRaises(ValueError):p.collect(p.plan(archive(),TASK),[value])
    def test_single_source_update_reuses_other_unchanged_packets(self):
        original=bundle();template=original['documents'][0]
        original['documents']=[{**copy.deepcopy(template),'id':f'{i+1:064x}','name':f'synthetic-{i}.txt','records':[copy.deepcopy(template['records'][0])]} for i in range(4)]
        view=archive(original);task=p.task_spec(TASK)
        cap=max(p.upper(p.messages(task,[p.model_record(r)],'0'*64)) for r in view['records'])+5
        before=p.plan(view,TASK,input_cap=cap);self.assertEqual(len(before['parts']),4)
        responses=[proposal(before,i) for i in range(4)]
        changed=copy.deepcopy(original);changed['documents'][1]['records'][0]['quote']=changed['documents'][1]['records'][0]['quote'].replace('null','NULL')
        after=p.plan(archive(changed),TASK,input_cap=cap)
        old={v['input_hash'] for v in before['parts']};new={v['input_hash'] for v in after['parts']}
        self.assertEqual(len(old & new),3)
        responses+=[proposal(after,i) for i,part in enumerate(after['parts']) if part['input_hash'] not in old]
        out=p.collect(after,responses,'fixture');self.assertEqual(len(out['accepted_inputs']),4);self.assertEqual(len(out['ignored_stale_inputs']),1)
    def test_cli_roundtrip_and_exact_candidate_team_review_contract(self):
        root=Path(tempfile.mkdtemp(prefix='tingxian-preparation-test-'))
        for name,value in [('bundle.json',bundle()),('task.json',TASK)]: (root/name).write_text(json.dumps(value,ensure_ascii=False))
        base=[sys.executable,'-B',str(ROOT/'interview/toolkit/harness.py')]
        args=['--bundle',str(root/'bundle.json'),'--task',str(root/'task.json'),'--library-id','synthetic-owner']
        run=lambda action,extra=[]:subprocess.run(base+[action]+args+extra,check=True,capture_output=True,text=True,timeout=10).stdout
        manifest=json.loads(run('preparation-plan'));packet=json.loads(run('preparation-input',['--part','1']))
        self.assertEqual(manifest['parts'][0]['input_hash'],packet['input_hash'])
        plan=p.plan(archive(),TASK);(root/'responses.json').write_text(json.dumps([proposal(plan)],ensure_ascii=False))
        extra=['--proposals',str(root/'responses.json'),'--execution-mode','fixture']
        artifact=json.loads(run('preparation-collect',extra));team=json.loads(run('preparation-collect',extra+['--output-format','team']))
        self.assertEqual(team['artifact'],artifact)
        self.assertIn('项目故事卡',run('preparation-collect',extra+['--output-format','markdown']))
        reviews={role:{'schema_version':'review-1','reviewer':role,'mode':'coaching' if role=='consultant' else 'artifact_review','candidate_id':team['id'],'candidate_hash':h.digest(team),'findings':[],'missing_evidence':[],'prioritized_actions':[],'status':'reviewed'} for role in h.ROLES}
        result=h.review_round(team,reviews,set(plan['records']))
        self.assertTrue(result['structurally_complete'])
        changed=copy.deepcopy(team);changed['artifact']['stories'][0]['title']='评审后隐藏修改'
        with self.assertRaisesRegex(ValueError,'Stale'):h.review_round(changed,reviews,set(plan['records']))

if __name__=='__main__':unittest.main()
