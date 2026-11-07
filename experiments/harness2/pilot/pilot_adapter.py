"""P1 official-core seam. Fixed validator; only declared policy text can change."""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import threading
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
# Load P0 under a distinct name to avoid collision with this module.
spec = importlib.util.spec_from_file_location('p0_adapter', HERE.parent / 'adapter.py')
p0 = importlib.util.module_from_spec(spec);spec.loader.exec_module(p0)
from harness2.core.proposer import Proposer, Reply
from harness2.core.spec import BenchSpec, BenchPrompts, ChannelSpec
import policy
from broker import MODEL, encoded, hash_file

HOST = p0.HOST


class PilotAdapter(p0.TingXianAdapter):
    def __init__(self, root, dataset, broker):
        super().__init__(root, dataset)
        self.broker = broker
        channel = ChannelSpec('opencode', MODEL)
        self.spec = BenchSpec(name='tingxian-p1', runs_root=self.root / 'runs', install_mode='workspace_config',
            prompts=BenchPrompts(work='整理虚构背景资料。保留可靠来源、类别、限制和冲突，设置仅为待核对建议。'),
            channels={'proposer': channel, 'selector': channel}, allow_subagents=False,
            leak_globs=('labels.jsonl', 'evaluation.jsonl'))

    def execute(self, task_id, harness_dir, run_id, *, timeout=None):
        tree = policy.read_tree(harness_dir or self.base_harness())
        hashes = self._sources(task_id)
        fingerprint = hashlib.sha256(encoded({'task': task_id, 'sources': hashes,
            'policy': policy.signature(tree), 'host': hash_file(HOST),
            'worker': hash_file(HERE / 'rpc_worker.py'), 'broker': hash_file(HERE / 'broker.py'),
            'model': MODEL, 'thinking': 'high', 'output_cap': 8192})).hexdigest()
        run = self.root / 'executions' / hashlib.sha256(run_id.encode()).hexdigest()[:32]
        with self.lock:
            if self.broker.stopped():raise RuntimeError('Pilot cancelled')
            if run.exists():
                old = json.loads((run / 'state.json').read_text())
                if old['fingerprint'] != fingerprint:raise ValueError('Changed execution namespace')
                if self.delivered(run):return run
                raise ValueError('Incomplete prior execution; explicit new experiment required')
            run.mkdir(parents=True)
            state = {'task': task_id, 'run_id': run_id, 'fingerprint': fingerprint,
                     'policy': policy.signature(tree), 'status': 'running', 'started': time.time()}
            p0.write_json(run / 'state.json', state);self._log(state);self.execution_count += 1
        try:
            cfg = {'job_id': run_id, 'role': 'extractor', 'sources': str(self.task_files(task_id)),
                'out': str(run / 'artifacts'), 'host': str(HOST), 'policy': tree,
                'baseline': policy.signature(tree) == policy.signature(policy.BASE)}
            # Official recursion may use its own threads; carry the fixed task budget
            # explicitly rather than relying on inherited thread-local state.
            budget_scope = getattr(self.broker.scope, 'value', None) or (task_id + '-C0', 3500000)
            with self.broker.group(*budget_scope):
                result = self.broker.run_worker(run, cfg, read_dirs=[self.task_files(task_id)], read_files=[HOST])
            self._sources(task_id)
            def publish():
                state.update(status='completed', ended=time.time(), result=result, artifact_sha256=result['bundle_sha256'])
                p0.write_json(run / 'state.json', state)
                (run / 'trajectory.md').write_text('[ACTION] Fixed background extraction, quote/hash checks and pack.\n[OBSERVATION] ' + json.dumps(result, ensure_ascii=False))
            self.broker.adopt(publish)
            self._log(state)
            return run
        except BaseException as exc:
            state.update(status='cancelled' if self.broker.stopped() else 'failed', ended=time.time(), error=str(exc)[:1200])
            p0.write_json(run / 'state.json', state);self._log(state)
            (run / 'trajectory.md').write_text('[ERROR] ' + state['error'])
            raise

    def cancel(self):
        self.broker.stop()
        super().cancel()


class PilotProposer(Proposer):
    def __init__(self, spec, broker, root, group_id=None):
        super().__init__(spec=spec, attempts=1)
        self.broker = broker; self.root = Path(root); self.root.mkdir(parents=True, exist_ok=True)
        current = getattr(broker.scope, 'value', None)
        self.budget_scope = (group_id, 3500000) if group_id else current
        if self.budget_scope is None: raise ValueError('Explicit proposer budget scope required')

    def call(self, mission, context_dir, label, **kwargs):
        if label.startswith('experience.'):
            raise RuntimeError('C0 does not create or activate cross-task experience')
        if label.startswith('select.'):
            raise RuntimeError('k=1 does not require a candidate selector')
        identity = hashlib.sha256((str(context_dir) + label).encode()).hexdigest()[:32]
        run = self.root / identity
        cfg = {'job_id': 'optimizer-' + identity, 'role': 'optimizer', 'context': str(Path(context_dir).resolve()),
               'mission': mission, 'out': str(run / 'harness')}
        with self.broker.group(*self.budget_scope):
            result = self.broker.run_worker(run, cfg, read_dirs=[Path(context_dir).resolve()])
        policy.read_tree(run / 'harness')
        p0.write_json(run / 'result.json', result)
        reply = Reply(label=label, text='<notes>Bounded declarative JSON strategy; no gold feedback or experience activation.</notes>',
                      ok=True, attempts=1, workspace=run / 'harness', channel=MODEL, audit=run / 'result.json')
        self._calls.append(reply)
        return reply
