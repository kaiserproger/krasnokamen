import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.harness import ledger, profiles, regressions


class LedgerReviewTests(unittest.TestCase):
    def test_regressions_routes_python_and_client(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ledger.write_rows(root, '''[[bug]]
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
            ledger.write_rows(root, '''[[bug]]
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


ROW = '''[[bug]]
id = "{ident}"
title = "{ident} title"
area = "harness"
repro = "source"
severity = "major"
status = "open"
source = "example"
regression = []
repro_check = """an operator note that mentions
status = "open"
as a quoted line, not as this row's status"""
'''


class RowFileTests(unittest.TestCase):
    """One row per file: the split, the loader's refusals, and show/edit."""

    def root(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        return Path(directory.name)

    def rows(self, count=2):
        return ''.join(ROW.format(ident=f'BUG-{n}') + '\n' for n in range(count))

    def test_split_round_trips_every_row_then_removes_the_single_file(self):
        import tomllib
        root = self.root()
        (root / 'docs/bugs').mkdir(parents=True)
        original = self.rows(3)
        (root / 'docs/bugs/ledger.toml').write_text(original)
        self.assertEqual(ledger.split(root), 3)
        self.assertFalse((root / 'docs/bugs/ledger.toml').exists())
        self.assertEqual(ledger.load_rows(root), tomllib.loads(original)['bug'])
        for name in ('BUG-0', 'BUG-1', 'BUG-2'):
            self.assertEqual((root / 'docs/bugs/rows' / f'{name}.toml').read_text(), ROW.format(ident=name))

    def test_split_re_runs_on_a_newer_single_file(self):
        root = self.root()
        (root / 'docs/bugs').mkdir(parents=True)
        (root / 'docs/bugs/ledger.toml').write_text(self.rows(1))
        self.assertEqual(ledger.split(root), 1)
        (root / 'docs/bugs/ledger.toml').write_text(self.rows(2))
        self.assertEqual(ledger.split(root), 2)
        self.assertEqual([row['id'] for row in ledger.load_rows(root)], ['BUG-0', 'BUG-1'])

    def test_split_overwrites_a_stale_row_file(self):
        root = self.root()
        (root / 'docs/bugs').mkdir(parents=True)
        (root / 'docs/bugs/rows').mkdir()
        (root / 'docs/bugs/rows/BUG-0.toml').write_text(ROW.format(ident='BUG-0').replace('major', 'minor') + '\n')
        (root / 'docs/bugs/ledger.toml').write_text(self.rows(1))
        # The single file is authoritative: a stale row edit is rewritten, not merged.
        self.assertEqual(ledger.split(root), 1)
        self.assertIn('severity = "major"', ledger.show(root, 'BUG-0'))

    def test_edit_sets_a_field_and_skips_multi_line_strings(self):
        root = self.root()
        ledger.write_rows(root, self.rows(1))
        ledger.edit(root, 'BUG-0', ['title = "changed"'])
        row = ledger.load_rows(root)[0]
        self.assertEqual(row['title'], 'changed')
        self.assertIn('status = "open"\nas a quoted line', ledger.show(root, 'BUG-0'))
        self.assertEqual(row['repro_check'].splitlines()[1], 'status = "open"')

    def test_edit_refuses_status_promotion_id_and_unknown_fields(self):
        root = self.root()
        ledger.write_rows(root, self.rows(1))
        before = ledger.show(root, 'BUG-0')
        for assignment, reason in (('status = "fixed"', 'only .*ledger promote'),
                                   ('promotion = "p"', 'only .*ledger promote'),
                                   ('id = "OTHER"', 'the id is the row file name'),
                                   ('nonsense = "x"', 'unknown field nonsense')):
            with self.subTest(assignment=assignment):
                with self.assertRaisesRegex(ledger.Refused, reason):
                    ledger.edit(root, 'BUG-0', [assignment])
                self.assertEqual(ledger.show(root, 'BUG-0'), before)

    def test_edit_refuses_a_value_that_does_not_parse_and_dry_run_writes_nothing(self):
        root = self.root()
        ledger.write_rows(root, self.rows(1))
        before = ledger.show(root, 'BUG-0')
        with self.assertRaisesRegex(ledger.Refused, 'not a valid row value'):
            ledger.edit(root, 'BUG-0', ['title = "unterminated'])
        ledger.edit(root, 'BUG-0', ['title = "dry"'], dry_run=True)
        self.assertEqual(ledger.show(root, 'BUG-0'), before)

    def test_promote_flips_the_real_status_not_the_quoted_one(self):
        root = self.root()
        ledger.write_rows(root, self.rows(1))
        path = ledger.row_path(root, 'BUG-0')
        receipt = {'id': 'BUG-0'}
        ledger._record_promotion(root, 'BUG-0', receipt)
        row = ledger.load_rows(root)[0]
        self.assertEqual(row['status'], 'fixed')
        self.assertEqual(row['promotion'], 'docs/bugs/promotions/BUG-0.json')
        self.assertEqual(row['repro_check'].splitlines()[1], 'status = "open"')
        self.assertEqual(path.read_text().count('promotion = '), 1)

    def test_promote_restores_the_row_when_it_does_not_reload(self):
        root = self.root()
        ledger.write_rows(root, self.rows(1))
        original = ledger.show(root, 'BUG-0')
        with patch.object(ledger, 'load_rows', side_effect=ValueError('unreadable ledger')):
            with self.assertRaises(ValueError):
                ledger._record_promotion(root, 'BUG-0', {'id': 'BUG-0'})
        self.assertEqual(ledger.show(root, 'BUG-0'), original)



class ServerRegistryTests(unittest.TestCase):
    """The release-server registry is keyed per (binary, tree): a harness-only fix leaves the binary
    identical on the candidate and on its reverted control tree, and both must be citable."""

    def registry(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        (root / '.analysis/ledger-builds').mkdir(parents=True)
        return root, root / '.analysis/ledger-builds'

    def test_register_server_writes_one_entry_per_tree(self):
        root, registry = self.registry()
        with patch.object(ledger, 'REPO_ROOT', root):
            ledger._register_server('a' * 64, {'head': 'b' * 40, 'sha256': 'c' * 64})
            ledger._register_server('a' * 64, {'head': 'b' * 40, 'sha256': 'd' * 64})
            self.assertEqual(ledger._registration(registry, 'a' * 64, {'head': 'b' * 40, 'sha256': 'c' * 64}),
                             {'sha256': 'a' * 64, 'tree': {'head': 'b' * 40, 'sha256': 'c' * 64}})
            self.assertEqual(ledger._registration(registry, 'a' * 64, {'head': 'b' * 40, 'sha256': 'd' * 64}),
                             {'sha256': 'a' * 64, 'tree': {'head': 'b' * 40, 'sha256': 'd' * 64}})
            self.assertIsNone(ledger._registration(registry, 'a' * 64, {'head': 'b' * 40, 'sha256': 'e' * 64}))

    def test_registration_still_reads_a_legacy_binary_named_entry(self):
        _, registry = self.registry()
        legacy = {'sha256': 'a' * 64, 'tree': {'head': 'b' * 40, 'sha256': 'e' * 64}}
        (registry / f'{"a" * 64}.json').write_text(__import__('json').dumps(legacy) + '\n')
        self.assertEqual(ledger._registration(registry, 'a' * 64, legacy['tree']), legacy)
        self.assertIsNone(ledger._registration(registry, 'a' * 64, {'head': 'b' * 40, 'sha256': 'f' * 64}))
