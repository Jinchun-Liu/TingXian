"""Official Harness² seam -> fixed TingXian background evidence packer (P0 only)."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import threading
import time

from harness2.core.proposer import Proposer, Reply
from harness2.core.spec import BenchSpec, BenchPrompts, ChannelSpec
from harness2.core.tree import SurfaceSpec
import policy
import sandbox

HERE=Path(__file__).resolve().parent
HOST=HERE.parents[1]/'interview/skills/interview-background/scripts/background.py'


def digest(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write_json(p,obj): Path(p).write_text(json.dumps(obj,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')

class TextSurface:
    name='tingxian-text'
    def surface(self):
        return SurfaceSpec(name='opencode',always_on=tuple(policy.BASE),always_on_max=16000,always_on_cap_reason='Bounded text-only strategy',skill_dir='skills',skill_name_rule='must_match',trigger_hint='',autotrigger_file='systemprompt.md',trigger_line='',subagent_requires_mode=True,plugin_dir=None,hooks_file=None)

class TingXianAdapter:
    def __init__(self,root,dataset):
        self.root=Path(root).resolve();self.root.mkdir(parents=True,exist_ok=True)
        self.dataset=Path(dataset).resolve()
        self.cases={r['id']:r for r in map(json.loads,(self.dataset/'cases.jsonl').read_text().splitlines())}
        self.substrate=TextSurface();channel=ChannelSpec('opencode','local/p0-deterministic')
        self.spec=BenchSpec(name='tingxian-p0',runs_root=self.root/'runs',install_mode='workspace_config',prompts=BenchPrompts(work='将虚构背景文件整理为保留逐字引文及位置的待核对资料包。'),channels={'proposer':channel,'selector':channel},allow_subagents=False,leak_globs=('labels.jsonl','evaluation.jsonl'))
        self.lock=threading.RLock();self.slots=threading.BoundedSemaphore(2)
        self.execution_count=0;self.score_calls=0
        self.base_harness().mkdir(exist_ok=True)
        for name,text in policy.BASE.items():
            p=self.base_harness()/name;p.parent.mkdir(parents=True,exist_ok=True)
            if p.exists() and p.read_text()!=text: raise ValueError('Baseline changed')
            if not p.exists():p.write_text(text,encoding='utf-8')
    def tasks(self,split='all'):
        if split not in ('all','dev','blind'): raise ValueError('Unknown split')
        return [k for k,v in self.cases.items() if split=='all' or v['split']==split]
    def domain_of(self,task_id): return self.cases[task_id]['domain']
    def task_prompt(self,task_id): return self.cases[task_id]['task']
    def task_files(self,task_id): return self.dataset/'inputs'/self.cases[task_id]['id']
    def base_harness(self): return self.root/'base-harness'
    def validate_tree(self,tree): policy.validate_tree(tree)
    def cancelled(self): return (self.root/'cancelled.json').exists()
    def cancel(self):
        with self.lock: write_json(self.root/'cancelled.json',{'cancelled':True,'at':time.time()})
    def _sources(self,task_id):
        root=self.task_files(task_id)
        if root.is_symlink(): raise ValueError('Symlink sources')
        paths=list(root.rglob('*'))
        if any(p.is_symlink() for p in paths): raise ValueError('Symlink source entry')
        current={p.relative_to(root).as_posix():digest(p) for p in paths if p.is_file()}
        if current!=self.cases[task_id]['files']: raise ValueError('Source set/content changed; new experiment required')
        return current
    def _log(self,obj):
        with self.lock:
            with (self.root/'runs.jsonl').open('a',encoding='utf-8') as f:
                f.write(json.dumps(obj,ensure_ascii=False)+'\n');f.flush()
    def execute(self,task_id,harness_dir,run_id,*,timeout=None):
        if not isinstance(run_id,str) or not run_id or len(run_id)>1024: raise ValueError('Invalid run id')
        tree=policy.read_tree(harness_dir or self.base_harness());source_hashes=self._sources(task_id)
        fingerprint=hashlib.sha256(json.dumps({'task':task_id,'sources':source_hashes,'policy':policy.signature(tree),'host':digest(HOST),'worker':digest(HERE/'worker.py'),'model':'p0-mechanical-no-api'},sort_keys=True).encode()).hexdigest()
        run=self.root/'executions'/hashlib.sha256(run_id.encode()).hexdigest()[:32]
        with self.lock:
            if self.cancelled(): raise RuntimeError('Experiment cancelled; no new execution')
            if run.exists():
                old=json.loads((run/'state.json').read_text())
                if old['fingerprint']!=fingerprint: raise ValueError('Run id belongs to a different candidate/source/model namespace')
                if old['status']=='completed' and self.delivered(run): return run
                raise ValueError('Prior execution incomplete or cancelled; preserve evidence and use a new run id explicitly')
            run.mkdir(parents=True)
            state={'run_id':run_id,'task':task_id,'fingerprint':fingerprint,'policy':policy.signature(tree),'status':'running','started':time.time(),'model_calls':0,'usage':None,'cost_units':0,'cost_kind':'no API calls; not a measured model cost'}
            write_json(run/'state.json',state);self._log(state);self.execution_count+=1
        write_json(run/'policy.json',policy.validate_tree(tree))
        out=run/'artifacts';out.mkdir();profile=run/'worker.sb'
        profile.write_text(sandbox.profile(read_files=[HERE/'worker.py',HOST],read_dirs=[self.task_files(task_id)],write_dirs=[run]))
        try:
            with self.slots:
                if self.cancelled(): raise RuntimeError('Cancelled while waiting for worker slot')
                result=sandbox.execute([sys.executable,'-B',HERE/'worker.py','--sources',self.task_files(task_id),'--out',out,'--host',HOST,'--policy',run/'policy.json'],profile,run,min(timeout or 60,900))
            if result.returncode: raise RuntimeError(result.stderr[-1600:] or f'Worker exited {result.returncode}')
            details=json.loads(result.stdout)
            self._sources(task_id)
            # The host validator rechecks literal quotes, category schema and
            # source hashes. A successful transport is not semantic approval.
            with self.lock:
                if self.cancelled(): raise RuntimeError('Cancelled; completed worker artifacts are not adopted')
                state.update(status='completed',ended=time.time(),result=details,artifact_sha256=digest(out/'bundle.meetily.json'))
                write_json(run/'state.json',state)
                (run/'trajectory.md').write_text('[ACTION] Fixed sandboxed evidence packer; policy '+state['policy']+'\n[OBSERVATION] '+json.dumps(details)+'\n',encoding='utf-8')
                self._log(state)
            return run
        except BaseException as exc:
            with self.lock:
                state.update(status='cancelled' if self.cancelled() else 'failed',ended=time.time(),error=str(exc)[:1800]);write_json(run/'state.json',state);self._log(state)
                (run/'trajectory.md').write_text('[ERROR] '+state['error'],encoding='utf-8')
            raise
    def delivered(self,run_dir):
        p=Path(run_dir);state_path=p/'state.json';artifact=p/'artifacts/bundle.meetily.json'
        if not state_path.is_file() or not artifact.is_file():return False
        state=json.loads(state_path.read_text())
        return state['status']=='completed' and state.get('artifact_sha256')==digest(artifact)
    def trajectory(self,run_dir,*,obs_trunc): return (Path(run_dir)/'trajectory.md').read_text()[:obs_trunc]
    def deliverable_text(self,run_dir):
        return (Path(run_dir)/'artifacts/bundle.meetily.json').read_text() if self.delivered(run_dir) else ''
    def deliverable_files(self,run_dir): return {'bundle.meetily.json':self.deliverable_text(run_dir)} if self.delivered(run_dir) else {}
    def score(self,run_dir,task_id):
        self.score_calls+=1
        # Gold is deliberately outside this adapter. Semantic evaluation is a
        # separate process/stage after candidates are frozen and submitted.
        return None

class SmokeProposer(Proposer):
    """Scripted policy edit only: proves official orchestration, not intelligence."""
    def call(self,mission,context_dir,label,**kwargs):
        context=Path(context_dir);workspace=None
        if label.startswith('experience.'):
            text='NO_ENTRY'  # No auto-activation or cross-task personal memory.
        elif label.startswith('select.'):
            raise RuntimeError('P0 uses k=1; no model/hidden-label selector is configured')
        else:
            workspace=context/'harness';path=workspace/'systemprompt.md'
            v=json.loads(path.read_text());v.update(order='constraints_first',review='逐文件核对限制与冲突，输出保持待人工确认；不自动应用。')
            path.write_text(json.dumps(v,ensure_ascii=False),encoding='utf-8')
            text='<notes>Deterministic P0 policy edit; no quality or cost improvement claimed.</notes>'
        reply=Reply(label=label,text=text,ok=True,attempts=1,workspace=workspace)
        self._calls.append(reply);return reply
