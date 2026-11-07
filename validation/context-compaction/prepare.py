"""C5 offline planning and consent preflight. No execution/credential/network path.

Freeze six fictional cases and A/B requests. C's long-case request is intentionally
absent until a separately obtained AI2 candidate passes production validation.
A successful preflight is not a model call, human acceptance, or release approval.
"""
import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import subprocess
from zoneinfo import ZoneInfo

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
SCOPE = 'tingxian-context-c5-fictional-six-v1'
CORPUS_SHA = '3b431f3903f9fc1c1480b0342da81e9914176c14ded18cfee5cafdc86bf28239'
ENDPOINT = 'https://api.deepseek.com/chat/completions'
MODEL = 'deepseek-flash'
AUTH_FIELDS = {'approved', 'scope', 'manifest_sha256', 'data_scope', 'model', 'endpoint',
               'max_cost_micro_cny', 'max_transport_attempts', 'valid_on', 'allow_paid_calls',
               'allow_private_data', 'allow_installed_app_switch', 'allow_strategy_promotion'}
SOURCES = ['Cargo.lock', 'crates/interview-core/examples/context_trial_prepare.rs',
           'crates/interview-core/src/session_memory.rs', 'crates/interview-core/src/compaction.rs',
           'crates/interview-core/src/model.rs', 'crates/interview-core/src/request_budget.rs',
           'crates/interview-core/src/context_budget.rs', 'crates/interview-core/src/vision.rs',
           'crates/interview-core/src/request_config.rs', 'crates/interview-core/src/providers.rs',
           'validation/context-compaction/cases.json', 'validation/context-compaction/visuals.json',
           'validation/context-compaction/prepare.py', 'validation/context-compaction/price-review.json',
           'validation/answer-vision/render-fixtures.swift']
sha = lambda data: hashlib.sha256(data).hexdigest()


def require(ok, message):
    if not ok:
        raise ValueError(str(message))


def today():
    return dt.datetime.now(ZoneInfo('Asia/Shanghai')).date().isoformat()


def read(path):
    require(path.is_file() and not path.is_symlink(), f'expected regular file: {path.name}')
    require(path.stat().st_size <= 16 * 1024 * 1024, 'file too large')
    return path.read_bytes()


def load(path):
    return json.loads(read(path))


def write(path, value):
    with path.open('x', encoding='utf-8') as f:
        json.dump(value, f, ensure_ascii=False, indent=2)
        f.write('\n'); f.flush(); os.fsync(f.fileno())


def invoke(binary, entry, cwd):
    # Only PATH is inherited; never copy application credentials into the child.
    result = subprocess.run([str(binary)], input=json.dumps(entry, ensure_ascii=False),
                            text=True, cwd=cwd, capture_output=True, timeout=15,
                            env={'PATH': os.environ.get('PATH', '')}, check=False)
    require(result.returncode == 0, 'offline request validation failed: ' + result.stderr[:500])
    data = json.loads(result.stdout)
    require(data['external_calls'] == 0 and data['quality'] is None, 'not an offline result')
    require(data['model'] == MODEL and data['endpoint'] == ENDPOINT and data['case_corpus_sha256'] == CORPUS_SHA,
            'runner identity or frozen corpus changed')
    return data


def validate_prices(price, current_day=None):
    require(price['model'] == MODEL and price['endpoint'] == ENDPOINT, 'price identity changed')
    require(price['currency'] == 'CNY' and price['basis'] == 'peak_cache_miss', 'expected conservative CNY rates')
    for k in ('input_micro_cny_per_token', 'output_micro_cny_per_token', 'image_upper_tokens'):
        require(type(price[k]) is int and 0 < price[k] <= 100000, f'invalid rate/bound: {k}')
    require(price['input_micro_cny_per_token'] >= 2 and price['output_micro_cny_per_token'] >= 8,
            'a lower price needs a newly reviewed planner, not a stale override')
    require(price['image_upper_tokens'] >= 1024, 'image bound is below the reviewed reserve')
    require(price['pricing_url'] == 'https://api-docs.deepseek.com/zh-cn/quick_start/pricing/' and
            price['vision_url'] == 'https://api-docs.deepseek.com/guides/vision/', 'unreviewed price sources')
    date = dt.date.fromisoformat(price['checked_on'])
    require(date.isoformat() == price['checked_on'], 'invalid price date')
    if current_day is not None:
        require(price['checked_on'] == current_day, 'refresh official pricing and image bound before paid execution')


def reserve(request, price):
    m = request['measurement']
    require(m['within_known_budget'] and m['output_tokens'] == request['output_cap'], 'invalid request budget')
    require(m['estimator'] == 'serialized-utf8-bytes-v1-not-tokenizer', 'unknown estimator')
    # Safety is charged as well as reserved, to avoid pretending byte estimates
    # are exact provider tokens. Reasoning is disabled, not separately double billed.
    upper = m['serialized_text_bytes'] + m['safety_tokens'] + m['image_count'] * price['image_upper_tokens']
    require(upper + m['output_tokens'] <= m['window_tokens'], 'reviewed image reserve exceeds window')
    one = upper * price['input_micro_cny_per_token'] + m['output_tokens'] * price['output_micro_cny_per_token']
    return {'input_upper_tokens': upper, 'output_cap': m['output_tokens'],
            'max_attempts': request['max_attempts'], 'reserve_micro_cny': one * request['max_attempts']}


def prepare(binary, images, output):
    binary = binary.resolve(strict=True)
    require(sha(read(HERE / 'cases.json')) == CORPUS_SHA, 'frozen corpus changed')
    cases = load(HERE / 'cases.json'); price = load(HERE / 'price-review.json'); validate_prices(price)
    require([c['id'] for c in cases] == ['D01', 'D02', 'D03', 'H01', 'H02', 'H03'], 'unexpected corpus')
    # Inspect both real images before creating an output bundle; reject the old
    # context_measure data URL placeholder and arbitrary remote image URLs.
    image_data = {}
    for c in cases:
        if c['image']:
            b = read(images / (c['id'] + '.png'))
            require(b.startswith(b'\x89PNG\r\n\x1a\n') and len(b) <= 10 * 1024 * 1024, 'invalid PNG')
            image_data[c['id']] = b
    output.mkdir(); (output / 'images').mkdir(); (output / 'cases').mkdir()
    for id_, b in image_data.items():
        with (output / 'images' / (id_ + '.png')).open('xb') as f: f.write(b)
    jobs, reviews = [], []
    for c in cases:
        entry = {'case_id': c['id']}
        if c['image']:
            entry.update(image='images/' + c['id'] + '.png', image_sha256=sha(image_data[c['id']]))
        data = invoke(binary, entry, output)
        directory = output / 'cases' / c['id']; directory.mkdir()
        write(directory / 'input.json', entry); write(directory / 'prepared.json', data)
        for arm in ('A', 'B', 'C'):
            request = data[arm] if arm != 'C' else data['C']['request']
            if request is None:
                deferred = data['C']; n = deferred['input_reservation_tokens']
                budget = {'input_upper_tokens': n, 'output_cap': deferred['output_cap'],
                          'max_attempts': 2, 'reserve_micro_cny': 2 * (n * price['input_micro_cny_per_token'] + deferred['output_cap'] * price['output_micro_cny_per_token'])}
            else:
                budget = reserve(request, price)
            jobs.append({'id': c['id'] + '-' + arm, 'case_id': c['id'], 'arm': arm, 'role': 'AI1',
                         'state': 'deferred_ai2' if request is None else 'prepared',
                         'depends_on': [c['id'] + '-AI2'] if request is None else [], **budget})
            reviews.append({'case_id': c['id'], 'arm': arm, 'split': c['split'], 'reviewer': None,
                            'criteria': [{'item': s, 'met': None, 'evidence': ''} for s in c['required']],
                            'natural_spoken': None, 'short_concise_or_long_complete': None,
                            'unsupported_experience_upgrade': None, 'semantic_passed': None})
        if data['AI2'] is not None:
            jobs.append({'id': c['id'] + '-AI2', 'case_id': c['id'], 'role': 'AI2', 'state': 'prepared',
                         'depends_on': [], **reserve(data['AI2'], price)})
    require(len(jobs) == 22 and sum(j['max_attempts'] for j in jobs) == 40, 'unexpected call bounds')
    write(output / 'review-template.json', reviews)
    write(output / 'price-review.json', price)
    write(output / 'measurements-template.json', {
        'actual_model_evidence': False,
        'entries': [{'job_id': j['id'], 'role': j['role'], 'attempts': [], 'complete': None,
                     'failure': None, 'first_meaningful_answer_ms': None, 'complete_ms': None,
                     'cpu_ms': None, 'peak_rss_bytes': None, 'cache_hit_tokens': None,
                     'prompt_tokens': None, 'output_tokens': None, 'reasoning_tokens': None,
                     'compaction_promoted': None, 'fallback_to_B': None} for j in jobs],
        'rules': ['No empty/filler token counts as first meaningful answer.',
                  'Failures remain in denominator; per-attempt unknown usage stays unknown.',
                  'Report AI2 cost separately and in total; savings cannot be inferred from smaller inputs.',
                  'Two cases per class are a pilot, not a reliable population p95 or 60-minute meeting benchmark.']})
    write(output / 'ledger-template.json', {'external_calls': 0, 'actual_cost_micro_cny': None,
          'entries': [{'job_id': j['id'], 'status': 'not_started', 'usage': None,
                       'reserved_micro_cny': j['reserve_micro_cny'], 'charged_micro_cny': None} for j in jobs],
          'rule': 'Before any child transport, fsync the full max-attempt reservation; unknown usage retains reserve; failures remain in the denominator. This template does not execute or book money.'})
    files = {p.relative_to(output).as_posix(): sha(read(p)) for p in sorted(output.rglob('*')) if p.is_file()}
    manifest = {'version': 1, 'scope': SCOPE, 'cases_sha256': CORPUS_SHA, 'model': MODEL, 'endpoint': ENDPOINT,
                'data_scope': 'frozen_fictional_only', 'created_on': today(), 'strategy': 'legacy',
                'harness2_enabled': False, 'app_window': 262144, 'source_version': 'c5-fictional-sources-v1',
                'jobs': jobs, 'base_calls': 22, 'max_transport_attempts': 40,
                'total_reserve_micro_cny': sum(j['reserve_micro_cny'] for j in jobs),
                'file_sha256': files, 'source_sha256': {p: sha(read(REPO / p)) for p in SOURCES},
                'binary_sha256': sha(binary.read_bytes()),
                'git_head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip(),
                'worktree_dirty': bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=REPO, text=True).strip()),
                'external_calls': 0, 'execution_available': False,
                'C_rule': 'Revalidate a real candidate against the sealed snapshot; materialize exact C request, remeasure, and remain under its fixed reservation. AI2 failure is a recorded fallback to B, not successful compression.'}
    write(output / 'manifest.json', manifest)
    auth = {'approved': False, 'scope': SCOPE, 'manifest_sha256': sha(read(output / 'manifest.json')),
            'data_scope': 'frozen_fictional_only', 'model': MODEL, 'endpoint': ENDPOINT,
            'max_cost_micro_cny': 0, 'max_transport_attempts': 40, 'valid_on': today(),
            'allow_paid_calls': False, 'allow_private_data': False,
            'allow_installed_app_switch': False, 'allow_strategy_promotion': False}
    write(output / 'authorization-template.json', auth)
    return {'manifest_sha256': auth['manifest_sha256'], 'base_calls': 22, 'max_attempts': 40,
            'estimated_reserve_micro_cny': manifest['total_reserve_micro_cny'], 'external_calls': 0}


def verify(bundle, binary):
    m = load(bundle / 'manifest.json')
    require(m['scope'] == SCOPE and m['cases_sha256'] == CORPUS_SHA, 'wrong trial scope')
    require(m['model'] == MODEL and m['endpoint'] == ENDPOINT and m['strategy'] == 'legacy' and not m['harness2_enabled'], 'trial identity changed')
    require(m['app_window'] == 262144 and m['data_scope'] == 'frozen_fictional_only', 'window/data scope changed')
    for path, expected in m['file_sha256'].items():
        p = Path(path); require(not p.is_absolute() and '..' not in p.parts, 'unsafe bundle path')
        require(sha(read(bundle / p)) == expected, f'bundle changed: {path}')
    require(set(m['source_sha256']) == set(SOURCES), 'source coverage changed')
    for path, expected in m['source_sha256'].items():
        require(sha(read(REPO / path)) == expected, f'source changed: {path}')
    require(sha(binary.read_bytes()) == m['binary_sha256'], 'preparer binary changed')
    require(len(m['jobs']) == m['base_calls'] == 22 and len({j['id'] for j in m['jobs']}) == 22, 'invalid jobs')
    require(sum(j['max_attempts'] for j in m['jobs']) == m['max_transport_attempts'] == 40, 'invalid attempt count')
    require(sum(j['reserve_micro_cny'] for j in m['jobs']) == m['total_reserve_micro_cny'], 'invalid reserve')
    return m


def check_authorization(bundle, binary, auth, current_day=None):
    m = verify(bundle, binary); day = current_day or today()
    require(set(auth) == AUTH_FIELDS, 'unexpected/missing authorization fields')
    require(auth['approved'] is True and auth['allow_paid_calls'] is True, 'this trial has no paid authorization')
    require(auth['scope'] == SCOPE and auth['manifest_sha256'] == sha(read(bundle / 'manifest.json')), 'authorization belongs to another manifest/trial')
    for field in ('data_scope', 'model', 'endpoint'):
        require(auth[field] == m[field], f'authorization mismatch: {field}')
    for field in ('allow_private_data', 'allow_installed_app_switch', 'allow_strategy_promotion'):
        require(auth[field] is False, f'{field} is outside this trial')
    require(auth['valid_on'] == day, 'authorization date expired')
    require(type(auth['max_cost_micro_cny']) is int and auth['max_cost_micro_cny'] >= m['total_reserve_micro_cny'], 'authorized cost cap below worst-case reserve')
    require(type(auth['max_transport_attempts']) is int and auth['max_transport_attempts'] == m['max_transport_attempts'], 'attempt scope changed')
    validate_prices(load(bundle / 'price-review.json'), day)
    return {'preflight': 'passed', 'external_calls': 0, 'execution_available': False,
            'note': 'Preflight only. This tool has no paid executor, does not consume consent, and cannot promote a runtime strategy.'}


def materialize(bundle, binary, case_id, candidate, output):
    m = verify(bundle, binary)
    require(case_id in {'D02', 'D03', 'H02', 'H03'}, 'only long-case C can materialize')
    entry = load(bundle / 'cases' / case_id / 'input.json'); entry['candidate'] = load(candidate)
    result = invoke(binary.resolve(strict=True), entry, bundle)
    require(result['C']['state'] == 'validated_candidate_not_evidence_of_model_call', 'candidate was not validated')
    budget = reserve(result['C']['request'], load(bundle / 'price-review.json'))
    cap = next(j for j in m['jobs'] if j['id'] == case_id + '-C')['reserve_micro_cny']
    require(budget['reserve_micro_cny'] <= cap, 'derived C exceeds authorized reservation')
    write(output, {'manifest_sha256': sha(read(bundle / 'manifest.json')), 'case_id': case_id,
                   'candidate_file_sha256': sha(read(candidate)), 'C': result['C'], 'budget': budget,
                   'external_calls': 0, 'model_provenance_verified': False})
    return {'state': 'locally_validated_not_model_accepted', 'external_calls': 0, 'reserve': budget}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest='command', required=True)
    a = sub.add_parser('prepare'); a.add_argument('--binary', type=Path, required=True); a.add_argument('--images', type=Path, required=True); a.add_argument('--out', type=Path, required=True)
    a = sub.add_parser('check-authorization'); a.add_argument('--binary', type=Path, required=True); a.add_argument('--bundle', type=Path, required=True); a.add_argument('--authorization', type=Path, required=True)
    a = sub.add_parser('materialize'); a.add_argument('--binary', type=Path, required=True); a.add_argument('--bundle', type=Path, required=True); a.add_argument('--case', required=True); a.add_argument('--candidate', type=Path, required=True); a.add_argument('--out', type=Path, required=True)
    args = p.parse_args()
    if args.command == 'prepare': result = prepare(args.binary, args.images, args.out)
    elif args.command == 'check-authorization': result = check_authorization(args.bundle, args.binary, load(args.authorization))
    else: result = materialize(args.bundle, args.binary, args.case, args.candidate, args.out)
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
