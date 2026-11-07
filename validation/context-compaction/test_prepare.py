"""Offline gate tests; synthetic retained files only, no API/real application DB."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
import zlib

spec = importlib.util.spec_from_file_location('c5_prepare', Path(__file__).with_name('prepare.py'))
p = importlib.util.module_from_spec(spec); spec.loader.exec_module(p)


def png():
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 1, 1, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(b'\x00\xff\xff\xff\xff')) + chunk(b'IEND', b'')


class TrialPlan(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.binary = Path(os.environ.get('C5_PREPARER', p.REPO / 'target/debug/examples/context_trial_prepare')).resolve()
        if not cls.binary.is_file():
            raise RuntimeError('Build context_trial_prepare and supply C5_PREPARER before this suite')
        cls.root = Path(tempfile.mkdtemp(prefix='tingxian-c5-offline-'))
        cls.images = cls.root / 'images'; cls.images.mkdir()
        for id_ in ('D03', 'H03'): (cls.images / (id_ + '.png')).write_bytes(png())
        print('Synthetic fixtures retained at', cls.root)

    def setUp(self):
        self.bundle = self.root / self.id().split('.')[-1]
        self.result = p.prepare(self.binary, self.images, self.bundle)
        self.manifest = p.load(self.bundle / 'manifest.json')
        self.day = p.load(self.bundle / 'price-review.json')['checked_on']

    def authorization(self):
        auth = p.load(self.bundle / 'authorization-template.json')
        auth.update(approved=True, allow_paid_calls=True, max_cost_micro_cny=self.manifest['total_reserve_micro_cny'], valid_on=self.day)
        return auth

    def test_real_planner_bounds_and_short_no_extra_call(self):
        m = p.verify(self.bundle, self.binary)
        self.assertEqual((m['base_calls'], m['max_transport_attempts']), (22, 40))
        self.assertEqual(sum(j['role'] == 'AI2' for j in m['jobs']), 4)
        self.assertFalse(m['execution_available'])
        for id_ in ('D01', 'H01'):
            data = p.load(self.bundle / 'cases' / id_ / 'prepared.json')
            self.assertIsNone(data['AI2']); self.assertEqual(data['A'], data['C']['request'])
        for id_ in ('D02', 'D03', 'H02', 'H03'):
            self.assertIsNone(p.load(self.bundle / 'cases' / id_ / 'prepared.json')['C']['request'])
        self.assertEqual(self.result['external_calls'], 0)

    def test_default_and_old_trial_authorizations_rejected(self):
        with self.assertRaises(ValueError):
            p.check_authorization(self.bundle, self.binary, p.load(self.bundle / 'authorization-template.json'), self.day)
        auth = self.authorization(); auth['scope'] = 'Harness2-six-trial'
        with self.assertRaises(ValueError): p.check_authorization(self.bundle, self.binary, auth, self.day)
        auth = self.authorization(); auth['manifest_sha256'] = '0' * 64
        with self.assertRaises(ValueError): p.check_authorization(self.bundle, self.binary, auth, self.day)

    def test_exact_consent_is_preflight_only_and_does_not_mutate_ledger(self):
        before = p.read(self.bundle / 'ledger-template.json')
        result = p.check_authorization(self.bundle, self.binary, self.authorization(), self.day)
        self.assertEqual(result['preflight'], 'passed'); self.assertFalse(result['execution_available'])
        self.assertEqual(before, p.read(self.bundle / 'ledger-template.json'))

    def test_caps_boolean_traps_private_scope_and_expiry_rejected(self):
        changes = [{'max_cost_micro_cny': True}, {'max_cost_micro_cny': self.manifest['total_reserve_micro_cny'] - 1},
                   {'max_transport_attempts': 22}, {'allow_private_data': True}, {'allow_strategy_promotion': True},
                   {'allow_installed_app_switch': True}, {'valid_on': '2000-01-01'}, {'approved': 1}]
        for change in changes:
            with self.subTest(change=change):
                auth = self.authorization(); auth.update(change)
                with self.assertRaises(ValueError): p.check_authorization(self.bundle, self.binary, auth, self.day)

    def test_price_staleness_and_image_bound_not_silently_accepted(self):
        price = p.load(self.bundle / 'price-review.json')
        with self.assertRaises(ValueError): p.validate_prices(price, '2000-01-01')
        for update in [{'image_upper_tokens': 0}, {'image_upper_tokens': 1}, {'input_micro_cny_per_token': 1}, {'model': 'other-model'}]:
            bad = dict(price, **update)
            with self.assertRaises(ValueError): p.validate_prices(bad)

    def test_modified_png_or_request_rejected(self):
        target = self.bundle / 'images/D03.png'; target.write_bytes(target.read_bytes() + b'changed')
        with self.assertRaises(ValueError): p.verify(self.bundle, self.binary)

    def test_modified_runner_and_source_binding_rejected(self):
        copied = self.root / 'different-binary'; copied.write_bytes(b'not the runner')
        with self.assertRaises(ValueError): p.verify(self.bundle, copied)
        m = p.load(self.bundle / 'manifest.json'); m['source_sha256'].pop('crates/interview-core/src/compaction.rs')
        (self.bundle / 'manifest.json').write_text(json.dumps(m))
        with self.assertRaises(ValueError): p.verify(self.bundle, self.binary)

    def test_candidate_materialization_revalidates_snapshot_and_negation(self):
        prepared = p.load(self.bundle / 'cases/D02/prepared.json'); snapshot = prepared['AI2']['snapshot']
        # Test-only fixture, never bundled as a real AI2 result.
        prompt = json.loads(prepared['AI2']['payload']['messages'][1]['content'])
        candidate = {'schema_version': 1, 'snapshot_id': prompt['snapshot_id'], 'finish_reason': 'stop',
                     'items': [{'kind': e['event']['kind'], 'origin': e['event']['origin'], 'refs': [e['ref']], 'quote': e['event']['text']} for e in prompt['events']], 'omitted': []}
        path = self.bundle / 'test-candidate.json'; p.write(path, candidate)
        result = p.materialize(self.bundle, self.binary, 'D02', path, self.bundle / 'C-derived.json')
        self.assertEqual(result['external_calls'], 0)
        derived = p.load(self.bundle / 'C-derived.json'); self.assertFalse(derived['model_provenance_verified'])
        self.assertIn('没有处理过真实生产事故', json.dumps(derived, ensure_ascii=False))
        with self.assertRaises(ValueError): p.materialize(self.bundle, self.binary, 'H02', path, self.bundle / 'wrong.json')
        candidate['items'][0]['quote'] = '我处理过真实生产事故。'; path.write_text(json.dumps(candidate))
        with self.assertRaises(ValueError): p.materialize(self.bundle, self.binary, 'D02', path, self.bundle / 'invalid.json')
        with self.assertRaises(ValueError): p.materialize(self.bundle, self.binary, 'D01', path, self.bundle / 'short.json')

    def test_placeholder_or_missing_images_rejected_before_bundle(self):
        images = self.root / 'bad-images'; images.mkdir()
        for id_ in ('D03', 'H03'): (images / (id_ + '.png')).write_bytes(b'synthetic-budget-only-not-recognition')
        out = self.root / 'bad-output'
        with self.assertRaises(ValueError): p.prepare(self.binary, images, out)
        self.assertFalse(out.exists())

    def test_overwrite_and_execution_flags_rejected(self):
        with self.assertRaises(FileExistsError): p.prepare(self.binary, self.images, self.bundle)
        result = subprocess.run([str(self.binary), '--run'], input='{}', text=True, capture_output=True, timeout=5,
                                env={'PATH': os.environ.get('PATH', ''), 'TINGXIAN_ACCEPTANCE_AUTHORIZED': '1'})
        self.assertNotEqual(result.returncode, 0); self.assertEqual(result.stdout, '')

    def test_zero_unknown_or_over_window_reservation_rejected(self):
        request = p.load(self.bundle / 'cases/D03/prepared.json')['A']; price = p.load(self.bundle / 'price-review.json')
        self.assertFalse(request['measurement']['complete_local_estimate'])
        result = p.reserve(request, price)
        self.assertEqual(result['input_upper_tokens'], request['measurement']['serialized_text_bytes'] + request['measurement']['safety_tokens'] + 1024)
        bad = copy.deepcopy(request); bad['measurement']['window_tokens'] = 1
        with self.assertRaises(ValueError): p.reserve(bad, price)


if __name__ == '__main__': unittest.main()
