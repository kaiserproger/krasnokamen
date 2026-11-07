"""Nightly retention removes only completed campaigns it owns."""
import importlib.machinery
import hashlib
import json
import importlib.util
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / 'dev' / 'scenario-nightly'
LOADER = importlib.machinery.SourceFileLoader('scenario_nightly', str(SCRIPT))
SPEC = importlib.util.spec_from_loader(LOADER.name, LOADER)
nightly = importlib.util.module_from_spec(SPEC)
LOADER.exec_module(nightly)


class ScenarioNightlyTests(unittest.TestCase):
    def test_campaign_records_both_pass_and_failure_fingerprints(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            binary = root / 'solaris-v2'
            binary.write_bytes(b'server')
            (root / 'manifest.json').write_text(json.dumps({'head': 'server-head'}))
            args = SimpleNamespace(receipts=root / 'campaigns', server_bin=binary, repo=root,
                                   vanilla_data=root, seeds=2, start_seed=0, steps=3, bots=2,
                                   max_bytes=1000000, budget_seconds=60, port=27380)
            args.receipts.mkdir()
            fingerprint = {'head': 'harness-head', 'sha256': 'harness-diff'}

            def run(argv, *, cwd, env, stdout, stderr, timeout):
                seed = int(argv[argv.index('--seed') + 1])
                receipt_dir = Path(env['SOLARIS_VALIDATION_ROOT']) / 'run'
                receipt_dir.mkdir(parents=True)
                receipt = {'status': 'passed' if seed == 0 else 'failed', 'tree': fingerprint,
                           'details': {'protocol_matrix': {'status': 'pass' if seed == 0 else 'fail'}}}
                (receipt_dir / 'result.json').write_text(json.dumps(receipt))
                return SimpleNamespace(returncode=seed)

            with patch('tools.harness.ledger.tree_fingerprint', return_value=fingerprint), \
                    patch.object(nightly.subprocess, 'run', side_effect=run):
                self.assertEqual(nightly.run_campaign(args), 1)
            result = json.loads(next(args.receipts.glob('campaign-*/result.json')).read_text())
            for record in (result['passed_seeds'][0], result['failures'][0]):
                self.assertEqual(record['server_head'], 'server-head')
                self.assertEqual(record['server_sha256'], hashlib.sha256(b'server').hexdigest())
                self.assertEqual(record['harness_tree'], fingerprint)
                self.assertIn('--seed', record['command'])
                self.assertEqual((record['steps'], record['bots']), (3, 2))
            self.assertEqual(result['passed_seeds'][0]['status'], 'passed')
            self.assertEqual(result['failures'][0]['status'], 'failed')
            self.assertFalse((Path(result['passed_seeds'][0]['original_receipt'])).exists())
            self.assertTrue(Path(result['failures'][0]['receipt']).exists())

    def test_pass_summary_preserves_seed_command_and_tree_before_cleanup(self):
        with tempfile.TemporaryDirectory() as tmp:
            receipt = Path(tmp) / 'result.json'
            payload = {'status': 'passed', 'tree': {'head': 'abc', 'sha256': 'def'}}
            receipt.write_text(json.dumps(payload))
            record = nightly.passed_receipt({'seed': 7, 'steps': 16, 'bots': 3,
                                             'server_sha256': 'serverhash', 'server_head': 'servercommit',
                                             'harness_tree': payload['tree'],
                                             'command': 'run --seed 7'}, receipt, payload)
            self.assertEqual(record['seed'], 7)
            self.assertEqual(record['steps'], 16)
            self.assertEqual(record['bots'], 3)
            self.assertEqual(record['server_sha256'], 'serverhash')
            self.assertEqual(record['server_head'], 'servercommit')
            self.assertEqual(record['harness_tree'], payload['tree'])
            self.assertEqual(record['command'], 'run --seed 7')
            self.assertEqual(record['tree'], payload['tree'])
            self.assertFalse(record['original_receipt_retained'])
            self.assertEqual(record['receipt_sha256'], hashlib.sha256(receipt.read_bytes()).hexdigest())

    def test_prune_old_campaigns_without_touching_current_or_unrelated(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            old = root / 'campaign-001'
            current = root / 'campaign-002'
            unrelated = root / 'unrelated'
            for directory in (old, current, unrelated):
                directory.mkdir()
                (directory / 'result.json').write_text(json.dumps({'seeds_run': 1, 'seed': 7}))
            (old / 'world').mkdir()
            (old / 'world' / 'data').write_bytes(b'x' * 1000)
            nightly.prune(root, current, 200)
            self.assertEqual(json.loads((old / 'result.json').read_text()),
                             {'seeds_run': 1, 'seed': 7, 'artifacts_retained': False})
            self.assertFalse((old / 'world').exists())
            self.assertTrue(current.exists())
            self.assertTrue(unrelated.exists())

    def test_over_budget_current_is_reported_not_removed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            current = root / 'campaign-001'
            current.mkdir()
            (current / 'result.json').write_bytes(b'abcde')
            self.assertEqual(nightly.prune(root, current, 1), 5)
            self.assertTrue(current.exists())
