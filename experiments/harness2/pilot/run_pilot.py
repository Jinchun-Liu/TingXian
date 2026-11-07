"""Six development cases only. Paid execution requires explicit config + key."""
import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import threading
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE));sys.path.insert(1, str(HERE.parent))
from broker import Broker, encoded, hash_file, MODEL
import policy
import sandbox

CASES = ['clear-a', 'practice-a', 'conflict-a', 'duplicate-a', 'long-a', 'ambiguity-a']


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--upstream', type=Path, required=True)
    parser.add_argument('--dataset', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--authorization', type=Path, required=True)
    args = parser.parse_args()
    upstream = args.upstream.resolve(); root = args.output.resolve(); source = args.dataset.resolve()
    auth = json.loads(args.authorization.read_text())
    if auth.get('approved_budget_cny') != 30 or auth.get('cases') != CASES or auth.get('stage') != 'P1-six-dev-pilot':
        raise ValueError('Explicit six-case 30 CNY authorization required')
    if subprocess.check_output(['git', '-C', str(upstream), 'rev-parse', 'HEAD'], text=True).strip() != policy.UPSTREAM_COMMIT:
        raise ValueError('Changed upstream')
    if subprocess.check_output(['git', '-C', str(upstream), 'status', '--porcelain'], text=True).strip():
        raise ValueError('Modified upstream')
    sys.path.insert(0, str(upstream))
    from harness2.core.schedule import run_task
    from pilot_adapter import PilotAdapter, PilotProposer, HOST
    lock = json.loads((HERE.parent / 'upstream-lock.json').read_text())
    if hash_file(HOST) != lock['host_packer_sha256']:raise ValueError('Changed host validator')
    if shutil.disk_usage(root.parent).free < 3 * 1024**3:raise ValueError('At least 3 GiB free required')
    root.mkdir(parents=True, exist_ok=True)
    if (root / 'manifest.json').exists():raise ValueError('Pilot was already started; preserve it and explicitly audit resume')
    cases = [r for r in map(json.loads, (source / 'cases.jsonl').read_text().splitlines()) if r['id'] in CASES]
    if len(cases) != 6 or any(r['split'] != 'dev' for r in cases):raise ValueError('Only the frozen six dev cases are allowed')
    dataset = root / 'dataset';dataset.mkdir();(dataset / 'inputs').mkdir();(dataset / 'evaluator').mkdir()
    for case in cases:
        shutil.copytree(source / 'inputs' / case['id'], dataset / 'inputs' / case['id'])
        for name, digest in case['files'].items():
            if hash_file(dataset / 'inputs' / case['id'] / name) != digest:raise ValueError('Changed fixture')
    (dataset / 'cases.jsonl').write_text(''.join(json.dumps(c, ensure_ascii=False) + '\n' for c in cases))
    labels = [r for r in map(json.loads, (source / 'evaluator/labels.jsonl').read_text().splitlines()) if r['id'] in CASES]
    (dataset / 'evaluator/labels.jsonl').write_text(''.join(json.dumps(c, ensure_ascii=False) + '\n' for c in labels))
    broker = Broker(root / 'broker', key=os.environ.get('TX_H2_API_KEY', ''))
    if not broker.key:raise ValueError('Dedicated TX_H2_API_KEY not supplied; no application credential read')
    signal.signal(signal.SIGINT, lambda *_: broker.stop())
    manifest = {'stage': 'P1-six-dev-pilot', 'route': 'U', 'cases': CASES, 'groups': ['A', 'B', 'C0'],
        'upstream_commit': policy.UPSTREAM_COMMIT, 'host_sha256': hash_file(HOST), 'model': MODEL,
        'thinking': 'high', 'output_cap': 8192, 'max_request_context': 262144, 'k': 1,
        'budget_cny': 30, 'max_calls': 700, 'max_parallel_workers': 2, 'authorization': auth,
        'source_sha256': {str(p.relative_to(HERE.parent)): hash_file(p) for p in sorted(HERE.parent.rglob('*.py'))},
        'labels_sha256': hash_file(dataset / 'evaluator/labels.jsonl'), 'human_labels_reviewed': False,
        'cost_basis': 'peak cache-miss CNY upper estimate; actual provider invoice unknown',
        'rates': {'input_per_million': 2, 'output_per_million': 8},
        'price_url': 'https://api-docs.deepseek.com/zh-cn/quick_start/pricing/',
        'price_checked': '2026-11-07', 'created': time.time()}
    (root / 'manifest.json').write_bytes(encoded(manifest))
    adapter = PilotAdapter(root / 'experiment', dataset, broker)
    outcomes = []; guard = threading.Lock()
    def execute_group(task, group):
        group_id = task + '-' + group
        # Same monetary cap for B and C0, but actual costs are reported rather
        # than silently padded. They may not qualify as a matched-cost result.
        cap = 1000000 if group == 'A' else 3500000
        row = {'task': task, 'group': group, 'status': 'failed', 'started': time.time()}
        try:
            with broker.group(group_id, cap):
                if group == 'A':
                    selected = adapter.execute(task, None, group_id)
                elif group == 'B':
                    candidates = []
                    for n in range(4):
                        path = adapter.execute(task, None, group_id + '-' + str(n))
                        candidates.append({'id': str(n), 'path': str(path / 'artifacts/bundle.meetily.json')})
                    choice_dir = root / 'selectors' / group_id
                    cfg = {'job_id': group_id + '-selection', 'role': 'reviewer', 'candidates': candidates}
                    choice = broker.run_worker(choice_dir, cfg, read_files=[Path(c['path']) for c in candidates])
                    selected = Path(candidates[int(choice['selected'])]['path']).parent.parent
                    row['selection'] = choice
                else:
                    prop = PilotProposer(adapter.spec.channels['proposer'], broker, root / 'proposers' / group_id, group_id=group_id)
                    result = run_task(task, 1, adapter=adapter, prop=prop, runs_root=adapter.spec.runs_root, modes=('agg',), tag='pilot', seed=0)
                    summary = next(p for p in adapter.spec.runs_root.rglob('summary.json') if p.parent.name == task)
                    state = json.loads(summary.read_text()); selection = state.get('selections', {}).get('agg', {})
                    row.update(core_status=result.status, core_errors=result.errors, editor_calls=len(prop._calls))
                    if result.status != 'ok' or not selection.get('ok'):raise RuntimeError('Core candidate pipeline incomplete')
                    selected = adapter.root / 'executions' / hashlib.sha256(selection['chosen_run_id'].encode()).hexdigest()[:32]
                    row['selected_policy'] = selection['chosen_label']
                if not adapter.delivered(selected) or broker.stopped():raise RuntimeError('No adopted verified artifact')
                target = root / 'submissions' / (group_id + '.json');target.parent.mkdir(exist_ok=True)
                shutil.copyfile(selected / 'artifacts/bundle.meetily.json', target)
                row.update(status='completed', path=str(target), sha256=hash_file(target))
        except Exception as exc:
            row['error'] = str(exc)[:1200]
        with broker.lock:
            cost, calls, unknown = broker.ledger.db.execute('SELECT COALESCE(SUM(COALESCE(actual,reserve)),0),COUNT(*),SUM(CASE WHEN actual IS NULL THEN 1 ELSE 0 END) FROM attempts WHERE role LIKE ?', (group_id + '/%',)).fetchone()
        row.update(ended=time.time(), calls=calls, reserved_or_measured_peak_cny=cost / 1000000, unknown_usage_calls=unknown or 0)
        with guard:
            outcomes.append(row)
            if len(outcomes) >= 3 and all(r['status'] != 'completed' for r in outcomes[-3:]):
                broker.stop()
            (root / 'outcomes.json').write_bytes(encoded(outcomes))
            print(json.dumps(row, ensure_ascii=False), flush=True)
        return row
    # At most two groups at once; source files, run namespaces and budgets are
    # independent. Each C0 proposer sees only its own task/candidate context.
    with ThreadPoolExecutor(max_workers=2) as pool:
        pending = [pool.submit(execute_group, task, group) for task in CASES for group in ('A', 'B', 'C0')]
        for future in as_completed(pending):future.result()
    # Freeze all submissions before evaluator sees labels; no feedback returns.
    evaluation = root / 'evaluation';evaluation.mkdir()
    submissions = [{k: row[k] for k in ('task', 'group', 'path', 'sha256')} for row in outcomes if row['status'] == 'completed']
    cfg = {'labels': str(dataset / 'evaluator/labels.jsonl'), 'submissions': submissions, 'output': str(evaluation / 'proxy.json')}
    (evaluation / 'job.json').write_bytes(encoded(cfg))
    profile = evaluation / 'worker.sb'
    profile.write_text(sandbox.profile(read_files=[HERE / 'evaluate.py', dataset / 'evaluator/labels.jsonl', *[Path(r['path']) for r in submissions]], write_dirs=[evaluation]))
    result = sandbox.execute([sys.executable, '-B', HERE / 'evaluate.py', evaluation / 'job.json'], profile, evaluation)
    summary = {'groups_requested': 18, 'completed': sum(r['status'] == 'completed' for r in outcomes),
               'budget': broker.totals(), 'evaluation_process_ok': result.returncode == 0,
               'human_review': 'pending', 'semantic_improvement': 'not established',
               'production_enabled': False, 'outcomes': outcomes}
    if result.returncode:summary['evaluation_error'] = result.stderr[-1000:]
    (root / 'pilot-report.json').write_bytes(encoded(summary));print(json.dumps({k:v for k,v in summary.items() if k!='outcomes'}, ensure_ascii=False), flush=True)
    broker.ledger.close()
    return 0 if summary['completed'] == 18 else 1


if __name__ == '__main__':
    raise SystemExit(main())
