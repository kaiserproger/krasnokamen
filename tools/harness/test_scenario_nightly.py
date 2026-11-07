"""Nightly retention removes only completed campaigns it owns."""
import importlib.machinery
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
