"""Fixed-policy comparator: four slots, failures retained, choose valid outputs."""
from pathlib import Path


def select_valid(broker, root, group_id, candidates):
    if not candidates:
        raise RuntimeError('No valid fixed-policy candidates')
    if len(candidates) == 1:
        choice = {'selected': candidates[0]['id'], 'reason': 'Only one valid candidate; no paid selector needed'}
    else:
        cfg = {'job_id': group_id + '-selection', 'role': 'reviewer', 'candidates': candidates}
        choice = broker.run_worker(Path(root) / 'selectors' / group_id, cfg,
                                   read_files=[Path(c['path']) for c in candidates])
    selected = next(c for c in candidates if c['id'] == choice['selected'])
    return Path(selected['path']).parent.parent, choice


def run_fixed(adapter, broker, root, task, group_id):
    candidates, failures = [], []
    for slot in range(4):
        try:
            run = adapter.execute(task, None, group_id + '-' + str(slot))
            if not adapter.delivered(run):
                raise RuntimeError('Candidate artifact did not verify')
            candidates.append({'id': str(slot), 'path': str(run / 'artifacts/bundle.meetily.json')})
        except Exception as exc:
            failures.append({'slot': slot, 'error': str(exc)[:1200]})
            if broker.stopped():
                raise
    selected, choice = select_valid(broker, root, group_id, candidates)
    return selected, choice, failures
