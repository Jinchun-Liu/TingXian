"""Only this trusted parent has network/credentials. Children use bounded JSON pipes."""
from contextlib import contextmanager
import hashlib
import json
import math
import os
from pathlib import Path
import selectors
import ssl
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import control
import sandbox

ENDPOINT = 'https://api.deepseek.com/chat/completions'
MODEL = 'deepseek-flash'
UNITS_PER_YUAN = 1_000_000
# Official peak, cache-miss rates verified 2026-11-07. Off-peak/cache discounts
# are deliberately not used to release budget. Amounts are upper estimates.
INPUT_RATE = 2
OUTPUT_RATE = 8
OUTPUT_CAP = 8192
CONTEXT_CAP = 262144


def encoded(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode()


def hash_file(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


class Broker:
    def __init__(self, root, key='', transport=None):
        self.root = Path(root).resolve()
        self.root.mkdir(parents=True, exist_ok=True)
        self.ledger = control.Ledger(self.root / 'budget.sqlite', 30 * UNITS_PER_YUAN, 700)
        self.key = key
        self.transport = transport or self._http
        self.lock = threading.RLock()
        self.scope = threading.local()
        self.slots = threading.BoundedSemaphore(2)
        self.started = time.monotonic()
        self.cancel_file = self.root / 'CANCEL'

    def stopped(self):
        if self.cancel_file.exists():
            self.ledger.cancel()
        return self.ledger.cancelled()

    def stop(self):
        self.ledger.cancel()

    @contextmanager
    def group(self, group_id, limit_units):
        old = getattr(self.scope, 'value', None)
        self.scope.value = (group_id, limit_units)
        try:
            yield
        finally:
            self.scope.value = old

    def _http(self, payload):
        if not self.key or any(c.isspace() for c in self.key):
            raise ValueError('Missing/invalid dedicated pilot credential')
        tls = ssl.create_default_context()
        if Path('/etc/ssl/cert.pem').is_file():
            tls.load_verify_locations(cafile='/etc/ssl/cert.pem')
        opener = urllib.request.build_opener(NoRedirect, urllib.request.HTTPSHandler(context=tls))
        request = urllib.request.Request(ENDPOINT, data=encoded(payload), headers={
            'Content-Type': 'application/json', 'Authorization': 'Bearer ' + self.key,
        })
        try:
            with opener.open(request, timeout=150) as response:
                raw = response.read(2 * 1024 * 1024 + 1)
        except urllib.error.HTTPError as exc:
            if exc.code in (401, 402, 403):
                self.stop()
            raise RuntimeError(f'Provider HTTP {exc.code}; no automatic retry') from None
        if len(raw) > 2 * 1024 * 1024:
            raise ValueError('Oversized provider response')
        return json.loads(raw)

    def request(self, job, number, role, system, data):
        if role not in ('extractor', 'optimizer', 'reviewer', 'probe'):
            raise ValueError('Unknown role')
        if not isinstance(system, str) or len(system.encode()) > 32000:
            raise ValueError('Invalid system prompt')
        scope = getattr(self.scope, 'value', None)
        if scope is None:
            raise ValueError('A fixed group budget is required')
        group_id, group_limit = scope
        user = encoded(data).decode()
        if self.key and (self.key in user or self.key in system):
            raise ValueError('Credential in model input')
        # Text-only JSON, no tools, search, files, image or arbitrary URL fields.
        payload = {'model': MODEL, 'messages': [{'role': 'system', 'content': system},
                   {'role': 'user', 'content': user}], 'stream': False,
                   'thinking': {'type': 'enabled'}, 'reasoning_effort': 'high',
                   'max_tokens': OUTPUT_CAP, 'response_format': {'type': 'json_object'}}
        input_bound = len(encoded(payload['messages'])) + 1024
        signature = hashlib.sha256(encoded({'job': job, 'number': number, 'role': role,
            'payload': payload, 'group': group_id})).hexdigest()
        path = self.root / 'calls' / signature
        worst = input_bound * INPUT_RATE + OUTPUT_CAP * OUTPUT_RATE
        with self.lock:
            if self.stopped() or time.monotonic() - self.started > 5400:
                self.stop()
                raise control.Stopped('Pilot cancelled or 90-minute time cap reached')
            if path.exists():
                state = json.loads((path / 'state.json').read_text())
                result = path / 'result.json'
                if state['status'] == 'completed' and result.is_file() and hash_file(result) == state['result_sha256']:
                    return json.loads(result.read_text())['content']
                raise RuntimeError('Earlier request unresolved; no silent paid retry')
            spent = self.ledger.db.execute('SELECT COALESCE(SUM(COALESCE(actual,reserve)),0) FROM attempts WHERE role LIKE ?', (group_id + '/%',)).fetchone()[0]
            if spent + worst > group_limit:
                raise control.Stopped('Group budget exhausted before dispatch')
            self.ledger.reserve(signature, group_id + '/' + role, worst, input_bound, OUTPUT_CAP, CONTEXT_CAP, 2048)
            path.mkdir(parents=True)
            state = {'status': 'reserved', 'role': role, 'group': group_id, 'signature': signature,
                     'reserved_micro_cny': worst, 'input_bound': input_bound, 'output_cap': OUTPUT_CAP,
                     'started': time.time()}
            (path / 'state.json').write_bytes(encoded(state))
            (path / 'request.json').write_bytes(encoded(payload))
        try:
            response = self.transport(payload)
            usage = response.get('usage') or {}
            prompt = usage.get('prompt_tokens')
            completion = usage.get('completion_tokens')
            measured = all(type(n) is int and n >= 0 for n in (prompt, completion))
            if measured and (prompt > input_bound or completion > OUTPUT_CAP):
                self.stop()
                raise ValueError('Provider usage exceeded reserved token bounds')
            actual = prompt * INPUT_RATE + completion * OUTPUT_RATE if measured else None
            self.ledger.finish(signature, actual, usage or None)
            if self.stopped():
                raise control.Stopped('Late result from cancelled pilot was not adopted')
            choice = response['choices'][0]
            if choice.get('finish_reason') != 'stop':
                raise ValueError('Incomplete/refused response: ' + str(choice.get('finish_reason')))
            content = choice['message'].get('content')
            if not isinstance(content, str) or not content.strip():
                raise ValueError('Empty model answer')
            value = json.loads(content)
            if not isinstance(value, dict):
                raise ValueError('JSON object required')
            saved = {'content': value, 'usage': usage or None, 'model': response.get('model'),
                     'system_fingerprint': response.get('system_fingerprint'), 'response_id': response.get('id'),
                     'estimated_peak_micro_cny': actual, 'usage_known': measured}
            (path / 'result.json').write_bytes(encoded(saved))
            state.update(status='completed', ended=time.time(), result_sha256=hash_file(path / 'result.json'),
                         estimated_peak_micro_cny=actual, usage_known=measured)
            (path / 'state.json').write_bytes(encoded(state))
            return value
        except BaseException as exc:
            # A failed/unknown request retains its reservation. If usage was
            # returned before a validation error, its measured bound is retained.
            state.update(status='failed', ended=time.time(), error=type(exc).__name__)
            (path / 'state.json').write_bytes(encoded(state))
            raise

    def run_worker(self, run, config, read_dirs=(), read_files=()):
        run = Path(run).resolve()
        run.mkdir(parents=True, exist_ok=True)
        (run / 'job.json').write_bytes(encoded(config))
        code = HERE / 'rpc_worker.py'
        profile = run / 'worker.sb'
        profile.write_text(sandbox.profile(read_files=[code, HERE.parent / 'policy.py', *read_files],
                                           read_dirs=read_dirs, write_dirs=[run]))
        if sys.platform != 'darwin':
            raise RuntimeError('Mac sandbox required')
        command = ['/usr/bin/sandbox-exec', '-f', str(profile), sys.executable, '-B', str(code), str(run / 'job.json')]
        with self.slots, (run / 'stderr.txt').open('wb') as errors:
            process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors,
                cwd=run, env={'PATH': '/usr/bin:/bin', 'PYTHONDONTWRITEBYTECODE': '1', 'PYTHONNOUSERSITE': '1'})
            selector = selectors.DefaultSelector(); selector.register(process.stdout, selectors.EVENT_READ)
            number = 0
            try:
                while True:
                    if self.stopped():
                        raise control.Stopped('Cancelled worker')
                    if not selector.select(timeout=0.2):
                        if process.poll() is not None:
                            raise RuntimeError('Worker ended without a final result')
                        continue
                    raw = process.stdout.readline(1024 * 1024 + 1)
                    if not raw or len(raw) > 1024 * 1024:
                        raise ValueError('Worker protocol length/EOF')
                    message = json.loads(raw)
                    if message['type'] == 'request':
                        number += 1
                        value = self.request(config['job_id'], number, config['role'], message['system'], message['data'])
                        process.stdin.write(encoded({'value': value}) + b'\n'); process.stdin.flush()
                    elif message['type'] == 'done':
                        process.wait(timeout=5)
                        if process.returncode or self.stopped():
                            raise RuntimeError('Worker failed or cancelled')
                        return message['result']
                    elif message['type'] == 'error':
                        raise RuntimeError('Worker validation: ' + message['message'][:1200])
                    else:
                        raise ValueError('Unknown worker protocol')
            finally:
                selector.close()
                if process.poll() is None:
                    process.terminate()
                    try: process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill(); process.wait(timeout=5)
                process.stdin.close(); process.stdout.close()

    def totals(self):
        with self.lock:
            count, held, known, unknown = self.ledger.db.execute('SELECT COUNT(*),COALESCE(SUM(COALESCE(actual,reserve)),0),COALESCE(SUM(actual),0),SUM(CASE WHEN actual IS NULL THEN 1 ELSE 0 END) FROM attempts').fetchone()
            return {'attempts': count, 'reserved_or_measured_peak_cny': held / UNITS_PER_YUAN,
                    'measured_usage_peak_estimate_cny': known / UNITS_PER_YUAN, 'unknown_usage_calls': unknown or 0,
                    'actual_provider_bill_cny': None, 'limit_cny': 30, 'cancelled': self.stopped()}
