"""C5 coordinator. Default is consent preflight, not execution.

--execute is the only CLI path which can read the process credential or call the
fixed provider. Tests inject synthetic transport/currency checks at run_batch;
there is no CLI switch for bypassing consent, source hashes or cost reservation.
"""
import argparse
import contextlib
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import threading
import time
import urllib.request

import prepare as p

MAX_LOG = 2 * 1024 * 1024

def integer(value):
    return type(value) is int and value >= 0


def settle(events, terminal, job, price):
    """Keep unknown attempts at full reserve; repeated usage cannot lower cost."""
    maximum = job['max_attempts']; per = job['reserve_micro_cny'] // maximum
    sent, usage = [], {}
    valid = True
    for row in events:
        event = row.get('event', {})
        if not isinstance(event, dict): valid = False; continue
        if event.get('type') == 'request_sent':
            n = event.get('attempt')
            if not integer(n) or n != len(sent) + 1 or n > maximum: valid = False
            else: sent.append(n)
        elif event.get('type') == 'usage':
            n, u = event.get('attempt'), event.get('usage')
            if not integer(n) or n not in sent or not isinstance(u, dict): valid = False; continue
            a, b = u.get('prompt_tokens'), u.get('completion_tokens')
            if not integer(a) or not integer(b): continue
            previous = usage.get(n, {'input_tokens': 0, 'output_tokens': 0})
            usage[n] = {'input_tokens': max(a, previous['input_tokens']), 'output_tokens': max(b, previous['output_tokens']), 'raw': u}
    complete_terminal = (valid and isinstance(terminal, dict) and type(terminal.get('complete')) is bool
                         and integer(terminal.get('attempts')) and terminal['attempts'] == len(sent)
                         and (terminal['complete'] is False or len(sent) > 0))
    observed = sum(v['input_tokens'] * price['input_micro_cny_per_token'] + v['output_tokens'] * price['output_micro_cny_per_token'] for v in usage.values())
    known = set(usage)
    if complete_terminal:
        held = observed + (len(sent) - len(known)) * per
        unknown = len(sent) - len(known)
    else:
        # A process may die after posting but before writing its first event.
        held = max(job['reserve_micro_cny'], observed + (maximum - len(known)) * per)
        unknown = maximum - len(known)
    return {'held_micro_cny': held, 'peak_rate_usage_estimate_micro_cny': observed,
            'usage': {str(n): value for n, value in usage.items()}, 'unknown_attempts': unknown, 'dispatches_observed': len(sent),
            'transport_attempts_held': len(sent) if complete_terminal else maximum,
            'terminal_valid': complete_terminal, 'actual_provider_bill_micro_cny': None}


@contextlib.contextmanager
def locked(path):
    with path.open('a+b') as f:
        if f.tell() == 0: f.write(b'0'); f.flush(); os.fsync(f.fileno())
        try:
            if os.name == 'nt':
                import msvcrt
                f.seek(0); msvcrt.locking(f.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(f.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as e:
            raise ValueError('another coordinator owns this trial') from e
        try: yield
        finally:
            if os.name == 'nt':
                f.seek(0); msvcrt.locking(f.fileno(), msvcrt.LK_UNLCK, 1)
            else: fcntl.flock(f.fileno(), fcntl.LOCK_UN)


class Journal:
    def __init__(self, directory, manifest, auth, evidence, resume):
        database = directory / 'ledger.sqlite'; exists = database.exists()
        p.require(not exists or resume, 'existing ledger requires explicit --resume; no implicit rerun')
        self.db = sqlite3.connect(database)
        try: self._initialize(directory, manifest, auth, evidence, exists)
        except BaseException:
            self.db.close(); raise

    def _initialize(self, directory, manifest, auth, evidence, exists):
        self.db.row_factory = sqlite3.Row
        self.db.execute('PRAGMA synchronous=FULL')
        self.db.execute('PRAGMA journal_mode=DELETE')
        self.db.execute('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL)')
        self.db.execute('CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY,role TEXT NOT NULL,state TEXT NOT NULL,reserve INTEGER NOT NULL,held INTEGER NOT NULL DEFAULT 0,attempts INTEGER NOT NULL DEFAULT 0,request_hash TEXT,task TEXT,result TEXT)')
        identity = {'manifest_sha256': auth['manifest_sha256'], 'authorization': auth,
                    'evidence_mode': evidence, 'actual_bill': None, 'initial_jobs': manifest['jobs']}
        rows = self.db.execute('SELECT value FROM metadata WHERE key=?', ('identity',)).fetchone()
        if rows is None:
            p.require(not exists, 'incomplete initial ledger; do not overwrite or assume no cost')
            with self.db:
                self.db.execute('INSERT INTO metadata VALUES (?,?)', ('identity', json.dumps(identity, sort_keys=True)))
                self.db.executemany('INSERT INTO jobs(id,role,state,reserve) VALUES (?,?,?,?)', [(j['id'], j['role'], 'pending', j['reserve_micro_cny']) for j in manifest['jobs']])
        else:
            p.require(json.loads(rows['value']) == identity, 'ledger authorization, manifest or evidence mode changed')
            expected = {j['id']: (j['role'], j['reserve_micro_cny']) for j in manifest['jobs']}
            records = self.db.execute('SELECT * FROM jobs').fetchall()
            p.require(len(records) == len(expected), 'ledger lost jobs')
            price = p.load(directory.parent / 'price-review.json')
            specs = {j['id']: j for j in manifest['jobs']}
            for row in records:
                p.require(expected.get(row['id']) == (row['role'], row['reserve']) and row['state'] in {'pending', 'reserved', 'interrupted', 'settled'}, 'invalid ledger state')
                p.require(integer(row['held']) and integer(row['attempts']), 'invalid ledger accounting')
                if row['state'] == 'pending':
                    p.require(row['held'] == row['attempts'] == 0 and row['task'] is None and row['result'] is None, 'pending job has inconsistent charges')
                else:
                    task = json.loads(row['task'])
                    p.require(task['job_id'] == row['id'] and task['role'] == row['role'] and task['manifest_sha256'] == auth['manifest_sha256'] and task['expected_payload_sha256'] == row['request_hash'], 'ledger task identity mismatch')
                    if row['result'] is not None:
                        result = json.loads(row['result']); cost = settle(result['events'], result['terminal'], specs[row['id']], price)
                        p.require(cost == result['accounting'] and cost['held_micro_cny'] == row['held'] and cost['transport_attempts_held'] == row['attempts'], 'ledger usage/charges disagree; do not resume')
                    else:
                        p.require(row['state'] != 'settled' and row['held'] == row['reserve'] and row['attempts'] == specs[row['id']]['max_attempts'], 'lost terminal or reservation')
                if row['state'] in {'reserved', 'interrupted'}: p.require(row['held'] >= row['reserve'], 'interrupted reservation was lost')
            with self.db:
                self.db.execute("UPDATE jobs SET state='interrupted' WHERE state='reserved'")
        # SQLite FULL commits precede every child; additionally sync the directory
        # so initial database creation survives a host crash on macOS/Linux.
        if os.name != 'nt':
            fd = os.open(directory, os.O_RDONLY)
            try: os.fsync(fd)
            finally: os.close(fd)

    def row(self, id_): return self.db.execute('SELECT * FROM jobs WHERE id=?', (id_,)).fetchone()
    def held(self): return self.db.execute('SELECT COALESCE(SUM(held),0) FROM jobs').fetchone()[0]
    def attempts(self): return self.db.execute('SELECT COALESCE(SUM(attempts),0) FROM jobs').fetchone()[0]

    def reserve(self, job, task, cap, max_attempts):
        p.require(self.row(job['id'])['state'] == 'pending', 'job has already been attempted')
        p.require(self.held() + job['reserve_micro_cny'] <= cap, 'authorized money exhausted')
        p.require(self.attempts() + job['max_attempts'] <= max_attempts, 'authorized attempts exhausted')
        with self.db:
            changed = self.db.execute("UPDATE jobs SET state='reserved',held=reserve,attempts=?,request_hash=?,task=? WHERE id=? AND state='pending'", (job['max_attempts'], task['expected_payload_sha256'], json.dumps(task, ensure_ascii=False), job['id'])).rowcount
            p.require(changed == 1, 'job was concurrently reserved')

    def finish(self, job, result):
        cost = result['accounting']
        with self.db:
            changed = self.db.execute("UPDATE jobs SET state=?,held=?,attempts=?,result=? WHERE id=? AND state='reserved'", ('settled' if cost['terminal_valid'] else 'interrupted', cost['held_micro_cny'], cost['transport_attempts_held'], json.dumps(result, ensure_ascii=False), job['id'])).rowcount
            p.require(changed == 1, 'reservation ownership lost')

    def summary(self):
        rows = self.db.execute('SELECT id,role,state,held,attempts,result FROM jobs ORDER BY rowid').fetchall()
        return {'held_micro_cny': self.held(), 'attempts_held': self.attempts(), 'actual_provider_bill_micro_cny': None,
                'jobs': [{'id': r['id'], 'role': r['role'], 'state': r['state'], 'held_micro_cny': r['held'],
                          'attempts_held': r['attempts'], 'complete': json.loads(r['result'])['complete'] if r['result'] else None} for r in rows],
                'semantic_acceptance': 'not reviewed', 'native_device_acceptance': False}

    def close(self): self.db.close()


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args): return None


def check_currency(key):
    opener = urllib.request.build_opener(NoRedirect)
    request = urllib.request.Request('https://api.deepseek.com/user/balance', headers={'Authorization': 'Bearer ' + key})
    try:
        with opener.open(request, timeout=15) as response:
            raw = response.read(128 * 1024 + 1)
        p.require(len(raw) <= 128 * 1024, 'currency response too large')
        value = json.loads(raw)
        p.require(value.get('is_available') is True, 'provider reports insufficient/unavailable balance')
        currencies = {i.get('currency') for i in value.get('balance_infos', []) if isinstance(i, dict)}
        p.require(currencies == {'CNY'}, 'CNY billing could not be verified')
    except Exception as e:
        raise ValueError('provider currency preflight failed; no generation started') from None
    return {'currency': 'CNY', 'amounts_saved': False, 'generation_request': False}


def transport(runner, bundle, task, key, timeout):
    """Bounded stdout/stderr, timeout/interrupt kill; never log the credential."""
    env = {'PATH': os.environ.get('PATH', ''), 'TINGXIAN_C5_AUTHORIZED_MANIFEST': task['manifest_sha256'], 'TINGXIAN_C5_API_KEY': key}
    child = subprocess.Popen([str(runner), '--execute'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=bundle, env=env)
    buffers = [bytearray(), bytearray()]; overflow = threading.Event()
    def consume(pipe, buf):
        while True:
            part = pipe.read(8192)
            if not part: break
            room = MAX_LOG - len(buf); buf.extend(part[:max(0, room)])
            if len(part) > room:
                overflow.set()
                try: child.kill()
                except OSError: pass
        pipe.close()
    threads = [threading.Thread(target=consume, args=(pipe, buf), daemon=True) for pipe, buf in zip((child.stdout, child.stderr), buffers)]
    for thread in threads: thread.start()
    cancelled = False; timed_out = False
    try:
        child.stdin.write(json.dumps(task, ensure_ascii=False).encode()); child.stdin.close()
        child.wait(timeout=timeout)
    except KeyboardInterrupt:
        cancelled = True
        try: child.send_signal(signal.SIGINT); child.wait(timeout=3)
        except (OSError, subprocess.TimeoutExpired): child.kill(); child.wait(timeout=5)
    except subprocess.TimeoutExpired:
        timed_out = True; child.kill(); child.wait(timeout=5)
    except BaseException:
        child.kill(); child.wait(timeout=5)
        raise
    finally:
        for thread in threads: thread.join(timeout=5)
    return {'stdout': buffers[0].decode('utf-8', errors='replace').replace(key, '[REDACTED]'),
            'stderr': buffers[1].decode('utf-8', errors='replace').replace(key, '[REDACTED]'),
            'returncode': child.returncode, 'cancelled': cancelled, 'timed_out': timed_out, 'output_limit': overflow.is_set()}


def parse_result(raw, job, price):
    events = []; malformed = False
    for line in raw['stdout'].splitlines():
        if not line.strip(): continue
        try:
            row = json.loads(line); p.require(isinstance(row, dict), 'nonobject event'); events.append(row)
        except (ValueError, TypeError): malformed = True
    terminals = [r for r in events if r.get('type') == 'terminal']
    terminal = terminals[0] if len(terminals) == 1 and events[-1:] == terminals and not malformed else None
    if raw.get('output_limit') or raw.get('timed_out'): terminal = None
    cost = settle(events, terminal, job, price)
    complete = cost['terminal_valid'] and terminal['complete'] is True and raw['returncode'] == 0 and not raw.get('cancelled')
    return {'complete': complete, 'terminal': terminal, 'accounting': cost, 'events': events,
            'transport': raw, 'semantic_passed': None, 'first_meaningful_answer_ms': None}


def task_for(job, journal, bundle, binary, manifest):
    def frozen(name):
        raw = p.read(bundle / name)
        p.require(p.sha(raw) == manifest['file_sha256'][name], 'case changed during dispatch preparation')
        return json.loads(raw)
    entry = frozen('cases/' + job['case_id'] + '/input.json')
    prepared = frozen('cases/' + job['case_id'] + '/prepared.json')
    fallback = False
    if job['state'] == 'deferred_ai2':
        dependency = journal.row(job['case_id'] + '-AI2')
        p.require(dependency['state'] != 'pending', 'AI2 dependency was not attempted')
        result = json.loads(dependency['result']) if dependency['result'] else None
        candidate = result.get('terminal', {}).get('candidate') if result and result.get('terminal') else None
        if result and result['complete'] and candidate is not None:
            entry['candidate'] = candidate
            # Revalidate even if the child said it was successful. A changed
            # candidate or stale reference is not trusted because of JSON alone.
            try:
                derived = p.invoke(binary, entry, bundle); request = derived['C']['request']
                p.require(p.reserve(request, p.load(bundle / 'price-review.json'))['reserve_micro_cny'] <= job['reserve_micro_cny'], 'C exceeds fixed reserve')
            except (ValueError, subprocess.SubprocessError):
                entry.pop('candidate', None); fallback = True; request = prepared['B']
        else: fallback = True; request = prepared['B']
    elif job['role'] == 'AI2': request = prepared['AI2']
    elif job['arm'] == 'C': request = prepared['C']['request']
    else: request = prepared[job['arm']]
    return {'job_id': job['id'], 'role': job['role'], 'arm': job.get('arm'), 'input': entry,
            'fallback_raw': fallback, 'expected_payload_sha256': request['payload_sha256'],
            'manifest_sha256': p.sha(p.read(bundle / 'manifest.json'))}, request['total_timeout_secs'] + 10


def verify_runner(manifest, runner):
    p.require(manifest.get('version') == 2 and manifest.get('execution_available') is True,
              'prepare a v2 manifest with its explicit --runner before execution')
    p.require(runner.is_file() and not runner.is_symlink() and p.sha(runner.read_bytes()) == manifest['runner_sha256'], 'execution binary changed')


def run_batch(binary, runner, bundle, auth, key, resume=False, *, send=transport, currency=check_currency, evidence='real-api-unreviewed', current_day=None, authorization_path=None):
    p.check_authorization(bundle, binary, auth, current_day)
    manifest = p.verify(bundle, binary); verify_runner(manifest, runner)
    p.require(key.strip(), 'set a process-only TINGXIAN_C5_API_KEY after approval')
    directory = bundle / 'execution'; directory.mkdir(exist_ok=True)
    with locked(directory / 'coordinator.lock'):
        journal = Journal(directory, manifest, auth, evidence, resume)
        try:
            currency_result = currency(key)
            p.require(currency_result.get('currency') == 'CNY', 'CNY billing not verified')
            with journal.db:
                journal.db.execute('INSERT INTO metadata VALUES (?,?)',
                    ('currency-check-' + str(time.time_ns()), json.dumps({'currency':'CNY','amounts_saved':False,'evidence_mode':evidence})))
            price = p.load(bundle / 'price-review.json')
            jobs = {j['id']: j for j in manifest['jobs']}
            order = [j for id_ in ('D01','D02','D03','H01','H02','H03') for j in (id_+'-A',id_+'-B',id_+'-AI2',id_+'-C') if j in jobs]
            failures = 0
            for id_ in order:
                job = jobs[id_]
                if journal.row(id_)['state'] != 'pending': continue
                # Re-read consent, corpus/source/image/binary binding before each
                # dispatch. No long-running process silently uses stale prices.
                if authorization_path is not None:
                    p.require(p.load(authorization_path) == auth, 'authorization changed/revoked; stop before the next request')
                p.check_authorization(bundle, binary, auth, current_day); verify_runner(manifest, runner)
                task, timeout = task_for(job, journal, bundle, binary, manifest)
                journal.reserve(job, task, auth['max_cost_micro_cny'], auth['max_transport_attempts'])
                # From this point any exception/crash retains the full committed
                # reservation; resume skips this job, never silently retries it.
                raw = send(runner, bundle, task, key, timeout)
                result = parse_result(raw, job, price); result['fallback_raw'] = task['fallback_raw']
                if job['role'] == 'AI2':
                    result['candidate_validation'] = 'not_promoted'
                    if result['complete']:
                        entry = dict(task['input'], candidate=result['terminal'].get('candidate'))
                        try:
                            derived = p.invoke(binary, entry, bundle)
                            p.require(derived['C']['state'] == 'validated_candidate_not_evidence_of_model_call', 'missing candidate')
                            result['candidate_validation'] = 'quotes_and_snapshot_validated'
                        except (ValueError, subprocess.SubprocessError):
                            result['complete'] = False; result['candidate_validation'] = 'rejected'
                journal.finish(job, result)
                print(json.dumps({'job': id_, 'complete': result['complete'], 'held_micro_cny': journal.held(), 'evidence': evidence}), flush=True)
                if not result['accounting']['terminal_valid'] or result['accounting']['held_micro_cny'] > job['reserve_micro_cny'] or journal.held() > auth['max_cost_micro_cny']:
                    raise ValueError('uncertain/over-reserve outcome; no further calls in this invocation')
                if raw.get('cancelled'): break
                failures = 0 if result['complete'] else failures + 1
                if failures >= 3: raise ValueError('three consecutive incomplete tasks; stop for review')
            return journal.summary()
        finally:
            # The DB is authoritative, even when export fails or the disk is
            # full; do not overwrite prior summaries or release reserved cost.
            summary = journal.summary(); summary['evidence_mode'] = evidence
            name = 'summary-' + str(time.time_ns()) + '.json'
            try: p.write(directory / name, summary)
            finally: journal.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True); parser.add_argument('--runner', type=Path, required=True)
    parser.add_argument('--bundle', type=Path, required=True); parser.add_argument('--authorization', type=Path, required=True)
    parser.add_argument('--execute', action='store_true'); parser.add_argument('--resume', action='store_true')
    args = parser.parse_args(); auth = p.load(args.authorization)
    p.check_authorization(args.bundle, args.binary, auth); verify_runner(p.verify(args.bundle, args.binary), args.runner)
    if not args.execute:
        print(json.dumps({'preflight':'passed','generation_requests':0,'note':'--execute and scoped consent are required; no key read'})); return
    key = os.environ.get('TINGXIAN_C5_API_KEY', '')
    result = run_batch(args.binary.resolve(), args.runner.resolve(), args.bundle.resolve(), auth, key, args.resume,
                       authorization_path=args.authorization.resolve())
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == '__main__': main()
