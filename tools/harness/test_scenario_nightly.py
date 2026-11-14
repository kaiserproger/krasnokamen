"""Nightly retention removes only completed campaigns it owns."""
import importlib.machinery
import hashlib
import json
import importlib.util
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'dev' / 'scenario-nightly'
LOADER = importlib.machinery.SourceFileLoader('scenario_nightly', str(SCRIPT))
SPEC = importlib.util.spec_from_loader(LOADER.name, LOADER)
nightly = importlib.util.module_from_spec(SPEC)
LOADER.exec_module(nightly)


class ScenarioNightlyTests(unittest.TestCase):
    def test_pass_summary_preserves_seed_command_and_tree_before_cleanup(self):
        with tempfile.TemporaryDirectory() as tmp:
            receipt = Path(tmp) / 'result.json'
            payload = {'status': 'passed', 'tree': {'head': 'abc', 'sha256': 'def'}}
            receipt.write_text(json.dumps(payload))
            record = nightly.passed_receipt({'seed': 7, 'steps': 16, 'bots': 3,
                                             'server_sha256': 'serverhash',
                                             'command': 'run --seed 7'}, receipt, payload)
            self.assertEqual(record['seed'], 7)
            self.assertEqual(record['steps'], 16)
            self.assertEqual(record['bots'], 3)
            self.assertEqual(record['server_sha256'], 'serverhash')
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
                (directory / 'result.json').write_bytes(b'abcde')
            self.assertEqual(nightly.prune(root, current, 5), 5)
            self.assertFalse(old.exists())
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
