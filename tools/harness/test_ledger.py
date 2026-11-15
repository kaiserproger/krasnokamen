import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.harness import ledger, profiles, regressions


class LedgerReviewTests(unittest.TestCase):
    def test_regressions_routes_python_and_client(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            (root / 'docs/bugs/ledger.toml').write_text('''[[bug]]
id = "X"
title = "x"
area = "harness"
repro = "source"
severity = "major"
status = "open"
source = "x"
fix_commit = "abcdef012345"
regression = ["python:pkg.test:Case.test_x", "client:client-crate:tests::x"]
''')
            with patch.object(regressions, 'REPO_ROOT', root), patch.object(regressions, 'BASE_FIXED', frozenset()), \
                 patch.object(ledger, '_run_python', return_value={('pkg.test', 'Case.test_x'): (ledger.GREEN, '')}) as py, \
                 patch.object(ledger, '_run_client', return_value={('client-crate', 'tests::x'): (ledger.GREEN, '')}) as client:
                regressions.run(profiles.ProfileContext(root))
                py.assert_called_once()
                client.assert_not_called()
                regressions.run(profiles.ProfileContext(root, include_regression_client=True))
                client.assert_called_once()

    def test_client_commit_validates_in_ledger_run(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            (root / 'docs/bugs/ledger.toml').write_text('''[[bug]]
id = "X"
title = "x"
area = "harness"
repro = "source"
severity = "major"
status = "open"
source = "x"
fix_commit = "client:abcdef012345"
regression = []
''')
            with patch.object(regressions, 'REPO_ROOT', root), patch.object(regressions, 'BASE_FIXED', frozenset()):
                self.assertEqual(regressions.run(profiles.ProfileContext(root))['open'], 1)

    def test_client_zero_pass_one_failure_is_red(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(ledger, 'client_repo', return_value=Path(directory)), \
                 patch.object(ledger, '_exec', side_effect=[
                     (True, '{"packages": [{"name": "c", "targets": [{"name": "c", "kind": ["lib"]}]}]}'),
                     (False, 'test result: FAILED. 0 passed; 1 failed; 0 ignored')]):
                result = ledger._run_client(None, {('c', 'test')}, Path(directory))
                self.assertEqual(result[('c', 'test')][0], ledger.RED)

    def test_python_requires_executed_tests_and_distinguishes_errors(self):
        cases = [
            (True, 'Ran 0 tests in 0.001s\n\nOK\n', ledger.UNRUNNABLE),
            (True, 'Ran 1 test in 0.001s\n\nOK (skipped=1)\n', ledger.UNRUNNABLE),
            (False, 'ERROR: test_x\nSyntaxError: bad\nRan 1 test in 0.001s\nFAILED (errors=1)', ledger.UNRUNNABLE),
            (False, 'FAIL: test_x\nRan 1 test in 0.001s\nFAILED (failures=1)', ledger.RED),
            (True, 'Ran 1 test in 0.001s\n\nOK\n', ledger.GREEN),
        ]
        for ok, text, expected in cases:
            with self.subTest(text=text), patch.object(ledger, '_exec', return_value=(ok, text)):
                self.assertEqual(ledger._run_python(None, {('m', 'test_x')}, Path('.'))[('m', 'test_x')][0], expected)

    def test_client_promotion_checks_every_uncovered_ref(self):
        refs = ['harness:entities:native-v2-craft-drag', 'cargo:mc-core:test_x']
        row = {'regression': refs}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ctx = profiles.ProfileContext(root)
            control = {'red': [refs[0]], 'client_head': 'before'}
            restore = {'green': [refs[0]], 'client_head': 'after'}
            tree = {'head': 'a', 'dirty': False, 'sha256': 'b'}
            with patch.object(ledger, 'client_fix_commit', return_value='a'), \
                 patch.object(ledger, 'check_client_control_receipt', return_value=control), \
                 patch.object(ledger, 'check_client_restore_receipt', return_value=restore), \
                 patch.object(ledger, '_check_client_control_delta', return_value={}):
                def runner(ctx, selected, tree, client):
                    self.assertEqual(selected, refs[1:])
                    return {refs[1]: {'state': ledger.RED, 'detail': 'failed'}}
                with self.assertRaisesRegex(ledger.Refused, 'candidate refs not green'):
                    ledger._promote_client_fix('X', row=row, fix='client:abcdef0', root=root, tree_now=tree,
                        dry_run=True, ctx=ctx, client_receipt='restore', client_control_receipt='control', runner=runner)
