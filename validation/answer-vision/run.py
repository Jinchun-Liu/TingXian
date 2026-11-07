"""Prepare by default; explicitly authorized execution uses the production Rust runner.

Only this frozen fictional corpus is supported. No application database, account
configuration, screen capture, tools, model judging or user documents are read.
Money is integer micro-CNY, conservatively using peak cache-miss rates.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import urllib.request
import urllib.error

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
CAP = 30_000_000
sha = lambda data: hashlib.sha256(data).hexdigest()


def require(condition, message):
    # These checks must remain active under python -O.
    if not condition:
        raise ValueError(str(message))


def write(path, value):
    with path.open('x', encoding='utf-8') as f:
        json.dump(value, f, ensure_ascii=False, indent=2)
        f.write('\n'); f.flush(); os.fsync(f.fileno())


def prepare(binary, images, output, selected):
    text = json.loads((HERE / 'cases.json').read_text(encoding='utf-8'))
    visuals = json.loads((HERE / 'visuals.json').read_text(encoding='utf-8'))
    cases = text['cases'] + visuals['cases']
    ids = [c['id'] for c in cases]
    require(len(ids) == len(set(ids)) == 47, 'expected 47 unique cases')
    if selected:
        require(len(selected) == len(set(selected)) and set(selected) <= set(ids), 'unknown or repeated case')
        cases = [c for c in cases if c['id'] in selected]
    output.mkdir()
    (output / 'cases').mkdir(); (output / 'images').mkdir()
    receipts, reviews = [], []
    for case in cases:
        entry = {'question': case['question'], 'facts': text['facts'], 'dialogue': case.get('dialogue', []), 'history': []}
        if case.get('suggestion'):
            entry['history'] = [{'question': '过去的问题', 'confirmed_answer': None, 'suggested_answer': case['suggestion']}]
        if case['kind'] == 'vision':
            source = images / (case['id'] + '.png')
            require(source.is_file() and not source.is_symlink(), source)
            image = source.read_bytes()
            require(image[:8] == b'\x89PNG\r\n\x1a\n' and len(image) <= 10 * 1024 * 1024, 'invalid PNG fixture')
            require(int.from_bytes(image[16:20], 'big') == case['width'] and int.from_bytes(image[20:24], 'big') == case['height'], 'fixture dimensions changed')
            name = 'images/' + case['id'] + '.png'
            with (output / name).open('xb') as f: f.write(image)
            entry.update(image=name, image_sha256=sha(image))
        directory = output / 'cases' / case['id']; directory.mkdir()
        write(directory / 'input.json', entry)
        result = subprocess.run([str(binary)], input=json.dumps(entry, ensure_ascii=False), text=True, encoding='utf-8', cwd=output, capture_output=True, timeout=10, check=True, env={'PATH': os.environ.get('PATH', '')})
        prepared = json.loads(result.stdout)
        require(prepared['type'] == 'prepared', 'unexpected runner output')
        receipt = prepared['receipt']
        require(receipt['max_attempts'] == 2 and receipt['model'] == 'deepseek-flash', 'unexpected model/attempt limit')
        write(directory / 'prepared.json', prepared)
        receipts.append({'id': case['id'], 'kind': case['kind'], **receipt})
        reviews.append({'id': case['id'], 'criteria': [{'item': x, 'met': None, 'evidence': ''} for x in case['criteria']], 'critical': [{'item': x, 'met': None, 'evidence': ''} for x in case.get('critical', [])], 'reviewer': None, 'semantic_passed': None})
    write(output / 'review-template.json', reviews)
    hashes = {p.relative_to(output).as_posix(): sha(p.read_bytes()) for p in sorted(output.rglob('*')) if p.is_file()}
    source_names = ['crates/interview-core/src/model.rs', 'crates/interview-core/src/vision.rs', 'crates/interview-core/src/context_budget.rs', 'crates/interview-core/src/providers.rs', 'crates/interview-core/examples/acceptance_case.rs', 'validation/answer-vision/run.py', 'validation/answer-vision/cases.json', 'validation/answer-vision/visuals.json', 'validation/answer-vision/render-fixtures.swift']
    manifest = {'version': 1, 'scope': '47-case fictional core/API acceptance; no native capture or meeting validation', 'cases': receipts, 'file_sha256': hashes, 'source_sha256': {n: sha((REPO / n).read_bytes()) for n in source_names}, 'binary_sha256': sha(binary.read_bytes()), 'source_commit': subprocess.check_output(['git', '-C', str(REPO), 'rev-parse', 'HEAD'], text=True).strip(), 'worktree_dirty': bool(subprocess.check_output(['git', '-C', str(REPO), 'status', '--porcelain'])), 'maximum_reserved_micro_cny': sum(c['single_attempt_reserve_micro_cny'] * 2 for c in receipts), 'model_calls': 0, 'semantic_review': 'pending', 'real_execution_authorized': False}
    write(output / 'manifest.json', manifest)
    write(output / 'authorization.example.json', {'approved': False, 'manifest_sha256': sha((output / 'manifest.json').read_bytes()), 'maximum_total_micro_cny': CAP, 'prior_reserved_micro_cny': 7_643_134, 'scope': 'complex-answer-and-vision-synthetic-api-validation', 'note': 'Example only. Existing six-case Harness authorization does not authorize this run. Prior amount is conservative usage estimate, not an invoice.'})
    print(json.dumps({'prepared': len(cases), 'maximum_reserved_cny': manifest['maximum_reserved_micro_cny'] / 1e6, 'model_calls': 0, 'output': str(output)}, ensure_ascii=False))


def valid_token(value):
    return isinstance(value, int) and not isinstance(value, bool) and value >= 0


def settle(events, terminal, reserved_per_attempt):
    sent, usage = set(), {}
    for row in events:
        event = row.get('event', {})
        if event.get('type') == 'request_sent':
            attempt = event.get('attempt')
            if not valid_token(attempt) or attempt not in (1, 2) or attempt != len(sent) + 1:
                raise ValueError('unexpected, repeated or out-of-order attempt number')
            sent.add(attempt)
        if event.get('type') == 'usage':
            attempt, value = event.get('attempt'), event.get('usage')
            if not valid_token(attempt) or attempt not in sent or not isinstance(value, dict): raise ValueError('usage without matching request')
            p, c = value.get('prompt_tokens'), value.get('completion_tokens')
            if valid_token(p) and valid_token(c):
                old = usage.get(attempt, (0, 0)); usage[attempt] = (max(old[0], p), max(old[1], c))
    if terminal is None:
        return {'held_micro_cny': 2 * reserved_per_attempt, 'attempts_observed': len(sent), 'unreported_attempts_possible': 2 - len(sent), 'unknown_calls': len(sent), 'usage': usage, 'terminal_observed': False}
    if not valid_token(terminal.get('attempts')) or terminal.get('attempts') != len(sent) or sent != set(range(1, len(sent) + 1)) or len(sent) > 2:
        raise ValueError('terminal/request count mismatch')
    measured = sum(p * 2 + c * 8 for p, c in usage.values())
    unknown = len(sent - set(usage))
    return {'held_micro_cny': measured + unknown * reserved_per_attempt, 'attempts_observed': len(sent), 'unreported_attempts_possible': 0, 'unknown_calls': unknown, 'usage': usage, 'terminal_observed': True}


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args): return None


def preflight(binary, output, authorization):
    manifest = json.loads((output / 'manifest.json').read_text(encoding='utf-8'))
    auth = json.loads(authorization.read_text(encoding='utf-8'))
    require(auth.get('approved') is True and auth.get('scope') == 'complex-answer-and-vision-synthetic-api-validation', 'explicit authorization is missing')
    require(auth['manifest_sha256'] == sha((output / 'manifest.json').read_bytes()), 'authorization does not match frozen cases')
    cap, held = auth['maximum_total_micro_cny'], auth['prior_reserved_micro_cny']
    require(valid_token(cap) and valid_token(held) and 7_643_134 <= held < cap <= CAP, 'invalid cap or prior cost reservation')
    reserves = [c['single_attempt_reserve_micro_cny'] for c in manifest['cases']]
    require(reserves and all(valid_token(r) and r > 0 for r in reserves), 'invalid per-attempt reserve')
    require(all(c['max_attempts'] == 2 for c in manifest['cases']) and manifest['maximum_reserved_micro_cny'] == sum(reserves) * 2, 'invalid batch reserve')
    require(held + manifest['maximum_reserved_micro_cny'] <= cap, 'whole batch cannot fit within remaining authorized budget')
    require(all(c['checked_on'] == datetime.date.today().isoformat() for c in manifest['cases']), 'reverify current pricing and image bounds before a later-day run')
    require(sha(binary.read_bytes()) == manifest['binary_sha256'], 'binary changed after preparation')
    for name, digest in manifest['file_sha256'].items():
        path = output / name
        require(not Path(name).is_absolute() and '..' not in Path(name).parts, 'unsafe manifest path')
        require(path.is_file() and not path.is_symlink() and sha(path.read_bytes()) == digest, name)
    return manifest, auth, cap, held


def execute(binary, output, authorization):
    manifest, auth, cap, held = preflight(binary, output, authorization)
    key = os.environ.get('TINGXIAN_VALIDATION_API_KEY', '').strip()
    if not key: raise ValueError('set TINGXIAN_VALIDATION_API_KEY only for the authorized process')
    # Refuse implicit rerun/resume. Every interrupted request remains reserved.
    run = output / 'execution'; run.mkdir()
    write(run / 'authorization.json', auth)
    opener = urllib.request.build_opener(NoRedirect)
    req = urllib.request.Request('https://api.deepseek.com/user/balance', headers={'Authorization': 'Bearer ' + key})
    with opener.open(req, timeout=15) as response: account = json.loads(response.read(128 * 1024))
    currencies = sorted({x.get('currency') for x in account.get('balance_infos', []) if isinstance(x, dict) and isinstance(x.get('currency'), str)})
    if currencies != ['CNY']: raise ValueError('CNY billing must be verified before generation')
    write(run / 'currency.json', {'currencies': currencies, 'amounts_saved': False, 'generation_request': False})
    ledger = (run / 'ledger.jsonl').open('x', encoding='utf-8')
    def record(row):
        ledger.write(json.dumps(row, ensure_ascii=False) + '\n'); ledger.flush(); os.fsync(ledger.fileno())
    record({'event': 'initial', 'cap_micro_cny': cap, 'held_micro_cny': held})
    outcomes = []; failures = 0
    try:
        for case in manifest['cases']:
            reserve = case['single_attempt_reserve_micro_cny'] * 2
            if held + reserve > cap: raise ValueError('authorized budget exhausted')
            record({'event': 'reserved', 'id': case['id'], 'reserve_micro_cny': reserve, 'held_micro_cny': held + reserve})
            before = held; held += reserve
            env = {'PATH': os.environ.get('PATH', ''), 'TINGXIAN_VALIDATION_API_KEY': key, 'TINGXIAN_ACCEPTANCE_AUTHORIZED': '1'}
            payload = (output / 'cases' / case['id'] / 'input.json').read_text(encoding='utf-8')
            child = subprocess.Popen([str(binary), '--run'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=output, env=env, text=True, encoding='utf-8')
            try:
                raw, stderr = child.communicate(payload, timeout=35)
            except BaseException:
                child.kill(); child.communicate(timeout=5)
                record({'event': 'interrupted', 'id': case['id'], 'held_micro_cny': held, 'note': 'two worst-case attempts remain reserved'})
                raise
            raw, stderr = raw.replace(key, '[REDACTED]'), stderr.replace(key, '[REDACTED]')
            write(run / (case['id'] + '-transport.json'), {'stdout': raw, 'stderr': stderr, 'returncode': child.returncode})
            events = [json.loads(line) for line in raw.splitlines() if line.strip()]
            terminals = [e for e in events if e.get('type') == 'terminal']
            terminal = terminals[0] if len(terminals) == 1 and events[-1] == terminals[0] else None
            settled = settle(events, terminal, case['single_attempt_reserve_micro_cny'])
            held = before + settled['held_micro_cny']
            record({'event': 'settled', 'id': case['id'], **settled, 'cumulative_held_micro_cny': held})
            answer = ''.join(e['event']['text'] for e in events if e.get('event', {}).get('type') == 'delta')
            completed = terminal is not None and terminal.get('complete') is True and child.returncode == 0
            outcome = {'id': case['id'], 'kind': case['kind'], 'complete': completed, 'returncode': child.returncode, 'answer': answer, 'events': events, 'stderr': stderr, 'cost': settled, 'image_sha256': case['image_sha256'], 'semantic_passed': None}
            write(run / (case['id'] + '.json'), outcome); outcomes.append({k: outcome[k] for k in ('id', 'kind', 'complete', 'semantic_passed')})
            print(json.dumps({'case': case['id'], 'complete': completed, 'cumulative_held_cny_including_prior': held / 1e6}), flush=True)
            if not settled['terminal_observed'] or settled['held_micro_cny'] > reserve or held > cap:
                raise ValueError('uncertain completion or estimate exceeded; no further calls')
            failures = 0 if completed else failures + 1
            if failures >= 3: raise ValueError('three consecutive incomplete answers; no further calls')
    finally:
        write(run / 'summary.json', {'cases': outcomes, 'held_micro_cny_including_prior': held, 'cap_micro_cny': cap, 'actual_provider_bill_cny': None, 'semantic_acceptance': 'pending review', 'native_capture_tested': False})
        ledger.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--images', type=Path)
    parser.add_argument('--cases', nargs='+')
    parser.add_argument('--authorization', type=Path)
    parser.add_argument('--execute', action='store_true')
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    if args.execute:
        if not args.authorization: parser.error('--execute requires --authorization')
        execute(binary, output, args.authorization.resolve())
    else:
        if not args.images: parser.error('preparation requires --images')
        prepare(binary, args.images.resolve(), output, args.cases)


if __name__ == '__main__': main()
