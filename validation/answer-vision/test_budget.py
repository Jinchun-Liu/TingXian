"""Offline accounting/authorization faults; never contacts a model."""
import datetime
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import run


def sent(attempt):
    return {'event': {'type': 'request_sent', 'attempt': attempt}}


def usage(attempt, prompt, completion):
    return {'event': {'type': 'usage', 'attempt': attempt,
                      'usage': {'prompt_tokens': prompt, 'completion_tokens': completion}}}


class Accounting(unittest.TestCase):
    def test_known_usage_zero_is_known_and_maxima_are_not_summed(self):
        result = run.settle([sent(1), usage(1, 10, 5), usage(1, 8, 7),
                             sent(2), usage(2, 0, 0)], {'attempts': 2}, 1000)
        self.assertEqual(result['held_micro_cny'], 10 * 2 + 7 * 8)
        self.assertEqual(result['unknown_calls'], 0)

    def test_one_unknown_retry_retains_full_reserve(self):
        result = run.settle([sent(1), usage(1, 10, 5), sent(2), usage(2, 0, None)],
                            {'attempts': 2}, 1000)
        self.assertEqual(result['held_micro_cny'], 1060)
        self.assertEqual(result['unknown_calls'], 1)

    def test_missing_terminal_never_releases_even_reported_usage(self):
        for events in ([], [sent(1)], [sent(1), usage(1, 0, 0)]):
            result = run.settle(events, None, 1000)
            self.assertEqual(result['held_micro_cny'], 2000)
            self.assertFalse(result['terminal_observed'])

    def test_invalid_usage_is_unknown_not_free(self):
        for invalid in (None, -1, True, 1.5, '2'):
            result = run.settle([sent(1), usage(1, invalid, 0)], {'attempts': 1}, 1000)
            self.assertEqual(result['held_micro_cny'], 1000)

    def test_duplicate_out_of_order_and_mismatched_attempts_rejected(self):
        for events, terminal in (([sent(1), sent(1)], {'attempts': 1}),
                                 ([sent(2)], {'attempts': 1}),
                                 ([sent(True)], {'attempts': 1}),
                                 ([sent(1)], {'attempts': True}),
                                 ([sent(1)], {'attempts': 0}),
                                 ([usage(1, 0, 0)], {'attempts': 0})):
            with self.assertRaises(ValueError): run.settle(events, terminal, 1000)


class Preflight(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='tingxian-budget-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.binary = self.root / 'runner'; self.binary.write_bytes(b'offline-only')
        self.data = self.root / 'case.json'; self.data.write_bytes(b'{}')
        self.manifest = {'cases': [{'max_attempts': 2, 'single_attempt_reserve_micro_cny': 1000,
                                    'checked_on': datetime.date.today().isoformat()}],
                         'maximum_reserved_micro_cny': 2000,
                         'binary_sha256': run.sha(self.binary.read_bytes()),
                         'file_sha256': {'case.json': run.sha(self.data.read_bytes())}}
        self.auth = {'approved': True, 'scope': 'complex-answer-and-vision-synthetic-api-validation',
                     'maximum_total_micro_cny': 30_000_000, 'prior_reserved_micro_cny': 7_643_134}
        self.save()

    def save(self):
        (self.root / 'manifest.json').write_text(json.dumps(self.manifest))
        self.auth['manifest_sha256'] = run.sha((self.root / 'manifest.json').read_bytes())
        (self.root / 'auth.json').write_text(json.dumps(self.auth))

    def check(self):
        return run.preflight(self.binary, self.root, self.root / 'auth.json')

    def test_valid_frozen_preflight_has_no_side_effects(self):
        self.assertEqual(self.check()[2:], (30_000_000, 7_643_134))
        self.assertFalse((self.root / 'execution').exists())

    def test_no_consent_is_rejected_before_credential_or_network(self):
        self.auth['approved'] = False; self.save()
        with patch.dict(os.environ, {}, clear=True), patch.object(run.urllib.request, 'build_opener') as network:
            with self.assertRaisesRegex(ValueError, 'authorization'):
                run.execute(self.binary, self.root, self.root / 'auth.json')
            network.assert_not_called()
        self.assertFalse((self.root / 'execution').exists())

    def test_entire_batch_and_prior_expenditure_are_enforced(self):
        self.auth['maximum_total_micro_cny'] = 7_644_000; self.save()
        with self.assertRaisesRegex(ValueError, 'whole batch'): self.check()
        self.auth['maximum_total_micro_cny'] = 30_000_000
        self.auth['prior_reserved_micro_cny'] = 0; self.save()
        with self.assertRaisesRegex(ValueError, 'prior cost'): self.check()

    def test_tampered_file_binary_and_manifest_are_rejected(self):
        self.data.write_bytes(b'changed')
        with self.assertRaises(ValueError): self.check()
        self.data.write_bytes(b'{}'); self.binary.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'binary changed'): self.check()
        self.binary.write_bytes(b'offline-only')
        (self.root / 'manifest.json').write_text('{}')
        with self.assertRaisesRegex(ValueError, 'frozen'): self.check()

    def test_stale_prices_and_fake_total_are_rejected(self):
        self.manifest['cases'][0]['checked_on'] = '2000-01-01'; self.save()
        with self.assertRaisesRegex(ValueError, 'pricing'): self.check()
        self.manifest['cases'][0]['checked_on'] = datetime.date.today().isoformat()
        self.manifest['maximum_reserved_micro_cny'] = 0; self.save()
        with self.assertRaisesRegex(ValueError, 'batch reserve'): self.check()

    def test_optimized_python_cannot_bypass_authorization(self):
        self.auth['approved'] = False; self.save()
        result = subprocess.run([sys.executable, '-B', '-O', str(run.HERE / 'run.py'),
                                 '--binary', str(self.binary), '--output', str(self.root),
                                 '--execute', '--authorization', str(self.root / 'auth.json')],
                                capture_output=True, text=True, timeout=5,
                                env={'PATH': os.environ.get('PATH', '')})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('explicit authorization is missing', result.stderr)
        self.assertFalse((self.root / 'execution').exists())


if __name__ == '__main__': unittest.main()
