"""Kill a portable worker mid-request, then resume through real loopback HTTP.

Only synthetic text and a fixture credential are used. No provider requests,
installed application, permissions, user library or existing tasks are touched.
Artifacts are retained in a new temporary directory for inspection.
"""
import collections
import hashlib
import http.server
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import threading

SOURCE = Path(__file__).resolve().parents[1] / 'interview/skills/interview-background'


def run():
    root = Path(tempfile.mkdtemp(prefix='tingxian-background-recovery-'))
    exported = root / 'portable-skill'
    shutil.copytree(SOURCE, exported, ignore=shutil.ignore_patterns('__pycache__'))
    sources = root / 'sources'
    sources.mkdir()
    source = sources / 'fictional-practice.txt'
    source.write_text('虚构练习：null、零和缺失字段分开测试；没有生产经验。\n' * 650, encoding='utf-8')
    source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
    checkpoint = root / 'job.sqlite'
    blocked, release = threading.Event(), threading.Event()
    lock = threading.Lock()
    observed, bodies = [], []

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            raw = self.rfile.read(int(self.headers['Content-Length']))
            body = json.loads(raw)
            data = json.loads(body['messages'][1]['content'])
            key = hashlib.sha256(raw).hexdigest()
            with lock:
                observed.append(key)
                bodies.append(body)
                position = len(observed)
            if position == 2:
                blocked.set()
                if not release.wait(20):
                    return
            if isinstance(data, dict) and 'segments' in data:
                row = data['segments'][0]
                content = {'records': [{'kind': 'practice', 'location': row['location'], 'quote': row['text'], 'note': '虚构练习，并非生产经历'}]}
            else:
                content = {'overview': '虚构练习资料，无生产经验。', 'role': '', 'aliases': ''}
            response = json.dumps({'usage': {'prompt_tokens': 100, 'completion_tokens': 30}, 'choices': [{'finish_reason': 'stop', 'message': {'content': json.dumps(content, ensure_ascii=False)}}]}, ensure_ascii=False).encode()
            try:
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(response)))
                self.end_headers()
                self.wfile.write(response)
            except (BrokenPipeError, ConnectionResetError):
                pass  # The killed worker must never adopt this late response.

    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever, kwargs={'poll_interval': 0.05}, daemon=True)
    thread.start()
    credential = 'fixture-credential-only-never-external'
    request = {'folder': str(sources), 'checkpoint': str(checkpoint), 'api': {'model': 'loopback-fixture', 'endpoint': f'http://127.0.0.1:{server.server_port}/chat/completions', 'key': credential, 'timeout': 10}}
    command = [sys.executable, '-B', str(exported / 'scripts/background.py'), 'desktop']
    env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'PYTHONDONTWRITEBYTECODE': '1', 'NO_PROXY': '127.0.0.1'}
    if os.name == 'nt':
        env['SYSTEMROOT'] = os.environ.get('SYSTEMROOT', 'C:\\Windows')
    payload = json.dumps(request, ensure_ascii=False)
    first = None

    def worker():
        return subprocess.run(command, input=payload, cwd=root, env=env, text=True, encoding='utf-8', capture_output=True, timeout=30)

    try:
        first = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=root, env=env, text=True, encoding='utf-8')
        first.stdin.write(payload)
        first.stdin.close()
        first.stdin = None
        assert blocked.wait(15), 'worker never reached its second HTTP request'
        with sqlite3.connect(checkpoint) as db:
            assert db.execute('SELECT done,total FROM files').fetchone()[0] == 1
            assert dict(db.execute('SELECT status,count(*) FROM attempts GROUP BY status')) == {'pending': 1, 'returned': 1}
        competing = worker()
        assert competing.returncode != 0 and '仍在运行' in competing.stderr
        assert len(observed) == 2, 'a competing process sent duplicate requests'
        first.kill()
        first.communicate(timeout=5)
        assert first.returncode != 0
        release.set()
        resumed = worker()
        assert resumed.returncode == 0, resumed.stderr
        result = json.loads(resumed.stdout.splitlines()[-1])
        counts = collections.Counter(observed)
        assert counts[observed[0]] == 1, 'completed chunk was sent again'
        assert counts[observed[1]] == 2, 'unreturned chunk was not retried exactly once on explicit resume'
        assert all(n == 1 for k, n in counts.items() if k != observed[1])
        metrics = result['metrics']
        assert metrics['calls'] == len(observed)
        assert metrics['unknown_calls'] == 1, 'lost response was reported as free or measured'
        assert metrics['sum_prompt_tokens'] == (len(observed) - 1) * 100
        assert metrics['sum_completion_tokens'] == (len(observed) - 1) * 30
        before = len(observed)
        completed = worker()
        assert completed.returncode == 0, completed.stderr
        assert json.loads(completed.stdout.splitlines()[-1]) == result
        assert len(observed) == before, 'completed task sent new requests'
        assert credential.encode() not in checkpoint.read_bytes()
        assert hashlib.sha256(source.read_bytes()).hexdigest() == source_hash
        with sqlite3.connect(checkpoint) as db:
            assert db.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
            assert db.execute("SELECT value FROM meta WHERE key='status'").fetchone()[0] == 'complete'
            assert db.execute('SELECT done=total FROM files').fetchone()[0] == 1
        for body in bodies:
            bound = sum(len(x['content'].encode()) for x in body['messages']) + 128 + body['max_tokens'] + 2048
            assert bound <= 262144
        report = {'passed': True, 'portable_worker': True, 'process_killed_mid_request': True, 'concurrent_worker_blocked': True, 'completed_chunk_repeated': False, 'completed_task_reissued': False, 'unknown_calls': 1, 'metrics': metrics, 'loopback_requests': len(observed), 'real_api_requests': 0, 'source_unchanged': True, 'checkpoint_integrity': 'ok', 'worker_sha256': hashlib.sha256((exported / 'scripts/background.py').read_bytes()).hexdigest(), 'work': str(root)}
        (root / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
        print(json.dumps(report, ensure_ascii=False))
    finally:
        if first and first.poll() is None:
            first.kill()
            first.communicate(timeout=5)
        release.set()
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


if __name__ == '__main__':
    run()
