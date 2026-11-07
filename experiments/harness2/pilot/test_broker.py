import json
from pathlib import Path
import sys
from concurrent.futures import ThreadPoolExecutor
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from broker import Broker, encoded, OUTPUT_CAP
import control

ROOT = Path(sys.argv.pop(1)).resolve()
ROOT.mkdir(parents=True, exist_ok=False)


def fake(payload, usage=True):
    return {'choices': [{'finish_reason': 'stop', 'message': {'content': '{"ok":true}'}}],
            **({'usage': {'prompt_tokens': 100, 'completion_tokens': 20}} if usage else {})}


class TestBroker(unittest.TestCase):
    def test01_reserve_and_reuse_and_no_key_in_files(self):
        calls = []
        def transport(payload):
            calls.append(payload)
            return fake(payload)
        b = Broker(ROOT / 'reuse', key='synthetic-canary-secret', transport=transport)
        with b.group('one', 1000000):
            for _ in range(2):self.assertEqual(b.request('same', 1, 'probe', '仅返回JSON', {}), {'ok': True})
        self.assertEqual(len(calls), 1)
        self.assertEqual(b.totals()['attempts'], 1)
        self.assertTrue(all(b'synthetic-canary-secret' not in p.read_bytes() for p in b.root.rglob('*') if p.is_file()))
        b.ledger.close()

    def test02_unknown_and_failed_requests_no_automatic_retry(self):
        b = Broker(ROOT / 'unknown', transport=lambda payload: fake(payload, False))
        with b.group('one', 1000000):b.request('a', 1, 'probe', 'JSON', {})
        self.assertEqual(b.totals()['unknown_usage_calls'], 1)
        self.assertGreater(b.totals()['reserved_or_measured_peak_cny'], 0)
        def fail(_):raise TimeoutError('synthetic timeout')
        b.transport = fail
        with b.group('one', 1000000):
            self.assertRaises(TimeoutError, b.request, 'b', 1, 'probe', 'JSON', {})
            self.assertRaisesRegex(RuntimeError, 'unresolved', b.request, 'b', 1, 'probe', 'JSON', {})
        self.assertEqual(b.totals()['attempts'], 2)
        b.ledger.close()

    def test03_context_and_group_limits_prevent_network(self):
        calls = []
        b = Broker(ROOT / 'limits', transport=lambda p: calls.append(p))
        with b.group('tiny', 1):self.assertRaises(control.Stopped, b.request, 'a', 1, 'probe', 'JSON', {})
        with b.group('large', 30000000):self.assertRaises(ValueError, b.request, 'b', 1, 'probe', 'JSON', '文' * 100000)
        self.assertEqual(calls, [])
        self.assertEqual(b.totals()['attempts'], 0)
        b.ledger.close()

    def test04_late_result_and_usage_overrun_stop(self):
        b = Broker(ROOT / 'cancel', transport=lambda p: (b.stop() or fake(p)))
        with b.group('one', 1000000):self.assertRaises(control.Stopped, b.request, 'a', 1, 'probe', 'JSON', {})
        self.assertFalse(any((p / 'result.json').exists() for p in (b.root / 'calls').iterdir()))
        b.ledger.close()
        def excess(p):
            value = fake(p);value['usage']['completion_tokens'] = OUTPUT_CAP + 1;return value
        b = Broker(ROOT / 'overrun', transport=excess)
        with b.group('one', 1000000):self.assertRaises(ValueError, b.request, 'a', 1, 'probe', 'JSON', {})
        self.assertTrue(b.totals()['cancelled']);b.ledger.close()

    def test05_concurrent_group_reservations(self):
        import threading
        entered = threading.Event(); release = threading.Event()
        def transport(payload):entered.set();release.wait(3);return fake(payload)
        b = Broker(ROOT / 'parallel', transport=transport)
        def request(i):
            with b.group('same', 100000):return b.request(str(i), 1, 'probe', 'JSON', {})
        try:
            with ThreadPoolExecutor(max_workers=2) as pool:
                first = pool.submit(request, 1);self.assertTrue(entered.wait(3))
                second = pool.submit(request, 2)
                self.assertRaises(control.Stopped, second.result)
                release.set();self.assertEqual(first.result(), {'ok': True})
        finally:release.set();b.ledger.close()


if __name__ == '__main__':
    unittest.main(verbosity=2)
