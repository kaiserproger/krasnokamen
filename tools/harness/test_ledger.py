import os
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



def _git(repo, *args):
    import subprocess
    return subprocess.run(['git', '-C', str(repo), *args], check=True, capture_output=True, text=True).stdout


def _commit(repo, message):
    _git(repo, '-c', 'user.email=t@example', '-c', 'user.name=t', 'commit', '-qam', message)
    return _git(repo, 'rev-parse', 'HEAD').strip()


CLIENT_LIB = '''pub fn save(value: &str) -> usize {
    value.len()
}

#[cfg(test)]
mod tests {
    use super::save;

    #[test]
    fn a_save_never_writes_when_the_guard_is_present() {
        assert_eq!(save("x"), 0, "the guard kept the value");
    }
}
'''

CLIENT_LIB_FIXED = '''pub fn save(value: &str) -> usize {
    #[cfg(test)]
    {
        if std::env::var_os("SAVE_TO_A_PATH").is_none() {
            return 0;
        }
    }
    value.len()
}

#[cfg(test)]
mod tests {
    use super::save;

    #[test]
    fn a_save_never_writes_when_the_guard_is_present() {
        assert_eq!(save("x"), 0, "the guard kept the value");
    }
}
'''


class ClientTestPromotionTests(unittest.TestCase):
    """`client-test:<sha>`: the registered client unit ref flips on the fix's reversed production hunks."""

    def setUp(self):
        import os
        import subprocess
        import tempfile

        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.root = self.base / 'core'
        self.root.mkdir()
        subprocess.run(['git', 'init', '-q', str(self.root)], check=True)
        (self.root / 'README.md').write_text('core\n')
        _git(self.root, 'add', '-A')
        _commit(self.root, 'base')
        self.client = self.base / 'client'
        self.client.mkdir()
        subprocess.run(['git', 'init', '-q', str(self.client)], check=True)
        (self.client / 'Cargo.toml').write_text('[package]\nname = "krasnokamen-client"\nversion = "0.1.0"\n')
        (self.client / 'src').mkdir()
        (self.client / 'src/lib.rs').write_text(CLIENT_LIB)
        _git(self.client, 'add', '-A')
        _commit(self.client, 'before')
        (self.client / 'src/lib.rs').write_text(CLIENT_LIB_FIXED)
        _git(self.client, 'add', '-A')
        self.fix = _commit(self.client, 'fix: keep a test save out of the data root')
        self.env = patch.dict('os.environ', {'KRASNOKAMEN_CLIENT_ROOT': str(self.client)})
        self.env.start()
        self.addCleanup(self.env.stop)
        self.ledger_write(
            f'id = "CLIENT-TEST-ROW"\ntitle = "t"\narea = "client"\nrepro = "source"\nseverity = "major"\n'
            f'status = "open"\nsource = "s"\nfix_commit = "client-test:{self.fix[:12]}"\n'
            'regression = ["client:krasnokamen-client:tests::a_save_never_writes_when_the_guard_is_present"]\n')

    def ledger_write(self, body):
        (self.root / 'docs/bugs/rows').mkdir(parents=True, exist_ok=True)
        (self.root / 'docs/bugs/rows/CLIENT-TEST-ROW.toml').write_text('[[bug]]\n' + body)

    def ctx(self):
        artifact = self.base / 'artifacts'
        artifact.mkdir(exist_ok=True)
        return profiles.ProfileContext(artifact)

    def assertion_line(self, text=None):
        source = (text if text is not None else (self.client / 'src/lib.rs').read_text()).splitlines()
        for index, line in enumerate(source, 1):
            if 'the guard kept the value' in line:
                return index
        raise AssertionError('fixture has no assert line')

    def test_promotes_on_a_real_pre_image_reversal_and_records_the_kind(self):
        calls = []

        def run(refs, tree):
            text = (Path(tree) / 'src/lib.rs').read_text()
            reversed_now = 'SAVE_TO_A_PATH' not in text
            calls.append(reversed_now)
            if not reversed_now:
                return {ref: {'state': ledger.GREEN, 'detail': ''} for ref in refs}
            return {ref: {'state': ledger.RED, 'detail': '1 failed',
                          'assertion': {'file': 'src/lib.rs', 'line': self.assertion_line(text),
                                        'error': 'panic'}} for ref in refs}

        receipt = ledger.promote('CLIENT-TEST-ROW', root=self.root, ctx=self.ctx(), client_runner=run)
        self.assertEqual(receipt['control'], 'client_test_reversal')
        self.assertEqual(receipt['reversed_files'], ['src/lib.rs'])
        self.assertEqual(calls, [True, False])
        row = ledger.load_rows(self.root)[0]
        self.assertEqual(row['status'], 'fixed')
        self.assertEqual(row['promotion'], 'docs/bugs/promotions/CLIENT-TEST-ROW.json')

    def test_refuses_a_control_that_stays_green(self):
        green = {'client:krasnokamen-client:tests::a_save_never_writes_when_the_guard_is_present':
                 {'state': ledger.GREEN, 'detail': ''}}
        with self.assertRaises(ledger.Refused) as refused:
            ledger.promote('CLIENT-TEST-ROW', root=self.root, ctx=self.ctx(),
                           client_runner=lambda refs, tree: {ref: dict(green[ref]) for ref in refs})
        self.assertIn('stayed green', str(refused.exception))
        self.assertEqual(ledger.load_rows(self.root)[0]['status'], 'open')

    def test_refuses_a_red_outside_test_code(self):
        line = 1  # `pub fn save(...)`, production code, not an assert
        red = {'client:krasnokamen-client:tests::a_save_never_writes_when_the_guard_is_present':
               {'state': ledger.RED, 'detail': '1 failed',
                'assertion': {'file': 'src/lib.rs', 'line': line, 'error': 'panic'}}}
        green = {ref: {'state': ledger.GREEN, 'detail': ''} for ref in red}

        def run(refs, tree):
            reversed_now = 'SAVE_TO_A_PATH' not in (Path(tree) / 'src/lib.rs').read_text()
            return {ref: dict((red if reversed_now else green)[ref]) for ref in refs}

        with self.assertRaises(ledger.Refused) as refused:
            ledger.promote('CLIENT-TEST-ROW', root=self.root, ctx=self.ctx(), client_runner=run)
        self.assertIn('stayed green', str(refused.exception))

    def test_refuses_when_the_fix_no_longer_reverses(self):
        # A later client commit rewrites the guarded lines: no real pre-image, so no control.
        (self.client / 'src/lib.rs').write_text(CLIENT_LIB_FIXED.replace('value.len()', 'value.len() + 1'))
        _git(self.client, 'add', '-A')
        _commit(self.client, 'later: change the save')
        green = {'client:krasnokamen-client:tests::a_save_never_writes_when_the_guard_is_present':
                 {'state': ledger.GREEN, 'detail': ''}}
        with self.assertRaises(ledger.Refused) as refused:
            ledger.promote('CLIENT-TEST-ROW', root=self.root, ctx=self.ctx(),
                           client_runner=lambda refs, tree: {ref: dict(green[ref]) for ref in refs})
        self.assertIn('does not reverse', str(refused.exception))

    def test_refuses_a_dirty_client_source(self):
        (self.client / 'src/lib.rs').write_text(CLIENT_LIB_FIXED + '\n// local edit\n')
        with self.assertRaises(ledger.Refused) as refused:
            ledger.promote('CLIENT-TEST-ROW', root=self.root, ctx=self.ctx(),
                           client_runner=lambda refs, tree: {})
        self.assertIn('modified source files', str(refused.exception))

    def test_refuses_a_client_test_fix_without_a_client_ref(self):
        self.ledger_write(
            f'id = "CLIENT-TEST-ROW"\ntitle = "t"\narea = "client"\nrepro = "source"\nseverity = "major"\n'
            f'status = "open"\nsource = "s"\nfix_commit = "client-test:{self.fix[:12]}"\n'
            'regression = ["python:tools.x:Y.test_z"]\n')
        with self.assertRaises(ledger.Refused) as refused:
            ledger.promote('CLIENT-TEST-ROW', root=self.root, ctx=self.ctx(),
                           client_runner=lambda refs, tree: {})
        self.assertIn('needs a client:', str(refused.exception))


    def test_run_accepts_a_fixed_client_test_row_without_a_graphical_ref(self):
        import json
        fix = f'client-test:{self.fix[:12]}'
        self.ledger_write(
            f'id = "CLIENT-TEST-ROW"\ntitle = "t"\narea = "client"\nrepro = "source"\nseverity = "major"\n'
            f'status = "fixed"\nsource = "s"\nfix_commit = "{fix}"\n'
            'regression = ["client:krasnokamen-client:tests::a_save_never_writes_when_the_guard_is_present"]\n'
            'client_test_reason = "the build-only guard is compiled out of the shipped client, so no scene can flip it"\n'
            'promotion = "docs/bugs/promotions/CLIENT-TEST-ROW.json"\n')
        (self.root / 'docs/bugs/promotions').mkdir(parents=True, exist_ok=True)
        (self.root / 'docs/bugs/promotions/CLIENT-TEST-ROW.json').write_text(json.dumps({
            'id': 'CLIENT-TEST-ROW', 'verdict': 'promoted', 'fix_commit': fix,
            'control': 'client_test_reversal', 'reversed_files': ['src/lib.rs']}))
        with patch.object(regressions, 'REPO_ROOT', self.root), \
             patch.object(regressions, 'BASE_FIXED', frozenset()):
            details = regressions.run(profiles.ProfileContext(self.base / 'artifacts2'))
        self.assertEqual(details['fixed'], 1)

    def test_run_refuses_a_client_test_row_without_a_reason(self):
        import json
        fix = f'client-test:{self.fix[:12]}'
        self.ledger_write(
            f'id = "CLIENT-TEST-ROW"\ntitle = "t"\narea = "client"\nrepro = "source"\nseverity = "major"\n'
            f'status = "fixed"\nsource = "s"\nfix_commit = "{fix}"\n'
            'regression = ["client:krasnokamen-client:tests::a_save_never_writes_when_the_guard_is_present"]\n'
            'promotion = "docs/bugs/promotions/CLIENT-TEST-ROW.json"\n')
        (self.root / 'docs/bugs/promotions').mkdir(parents=True, exist_ok=True)
        (self.root / 'docs/bugs/promotions/CLIENT-TEST-ROW.json').write_text(json.dumps({
            'id': 'CLIENT-TEST-ROW', 'verdict': 'promoted', 'fix_commit': fix,
            'control': 'client_test_reversal', 'reversed_files': ['src/lib.rs']}))
        with patch.object(regressions, 'REPO_ROOT', self.root), \
             patch.object(regressions, 'BASE_FIXED', frozenset()):
            with self.assertRaises(ValueError) as refused:
                regressions.run(profiles.ProfileContext(self.base / 'artifacts3'))
        self.assertIn('client_test_reason', str(refused.exception))

    def test_default_runner_routes_the_refs_into_the_reversed_copy(self):
        # The injected-runner tests bypass the default lambda; this one pins it: a ref must be parsed
        # into (crate, test), run in the control copy, and come back keyed by the ref.
        seen = []

        def fake_client(ctx, tests, root, repo=None):
            text = (Path(repo) / 'src/lib.rs').read_text()
            assert 'SAVE_TO_A_PATH' not in text, 'the control copy must be reversed'
            seen.append(set(tests))
            line = self.assertion_line(text)
            return {test: (ledger.RED, '1 failed', {'file': 'src/lib.rs', 'line': line, 'error': 'panic'})
                    for test in tests}

        def fake_green(ctx, tests, root, repo=None):
            return {test: (ledger.GREEN, '', None) for test in tests}

        def dispatch(ctx, tests, root, repo=None):
            # Both calls pass the tree; the control copy holds the reversal, the checkout does not.
            reversed_now = 'SAVE_TO_A_PATH' not in (Path(repo) / 'src/lib.rs').read_text()
            return fake_client(ctx, tests, root, repo) if reversed_now else fake_green(ctx, tests, root, repo)

        with patch.object(ledger, '_run_client', side_effect=dispatch):
            receipt = ledger.promote('CLIENT-TEST-ROW', root=self.root, ctx=self.ctx())
        self.assertEqual(receipt['control'], 'client_test_reversal')
        self.assertEqual(seen, [{('krasnokamen-client',
                                  'tests::a_save_never_writes_when_the_guard_is_present')}])


class GraphicalControlBackfillTests(unittest.TestCase):
    """`ledger graphical-control`: record a promoted row's graphical red later, never promote."""

    ROW = ('id = "GRAPHICAL-ROW"\ntitle = "t"\narea = "harness"\nrepro = "source"\nseverity = "major"\n'
           'status = "fixed"\nsource = "s"\nfix_commit = "{fix}"\n'
           'regression = ["harness:scene-shot:native-dynamic-context"]\n'
           'promotion = "docs/bugs/promotions/GRAPHICAL-ROW.json"\n')
    REF = 'harness:scene-shot:native-dynamic-context'

    def setUp(self):
        import subprocess
        import tempfile

        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.root = self.base / 'core'
        self.root.mkdir()
        subprocess.run(['git', 'init', '-q', str(self.root)], check=True)
        (self.root / 'f.txt').write_text('old\n')
        _git(self.root, 'add', '-A')
        _commit(self.root, 'base')
        (self.root / 'f.txt').write_text('new\n')
        _git(self.root, 'add', '-A')
        self.fix = _commit(self.root, 'fix: the row')
        (self.root / 'docs/bugs/rows').mkdir(parents=True)
        (self.root / 'docs/bugs/rows/GRAPHICAL-ROW.toml').write_text('[[bug]]\n' + self.ROW.format(fix=self.fix))
        (self.root / 'docs/bugs/promotions').mkdir(parents=True)
        self.promotion = {
            'id': 'GRAPHICAL-ROW', 'verdict': 'promoted', 'fix_commit': self.fix,
            'tree_head': self.fix, 'tree_sha256': 'a' * 64, 'control': 'reversed_fix_hunks',
            'reversed_files': ['f.txt'],
            'negative_control': {self.REF: {'state': 'unrunnable', 'detail': 'no build token'}},
        }
        self.write_promotion()

    def write_promotion(self):
        import json
        (self.root / 'docs/bugs/promotions/GRAPHICAL-ROW.json').write_text(
            json.dumps(self.promotion, indent=2) + '\n')

    def read_promotion(self):
        import json
        return json.loads((self.root / 'docs/bugs/promotions/GRAPHICAL-ROW.json').read_text())

    def control_tree(self, reverse=True):
        import subprocess
        tree = self.base / 'control'
        subprocess.run(['git', '-C', str(self.root), 'worktree', 'add', '--detach', str(tree), 'HEAD'],
                       check=True, capture_output=True)
        if reverse:
            patch, _, _ = ledger.non_test_patch(self.root, self.fix)
            subprocess.run(['git', '-C', str(tree), 'apply', '-R', '-'], input=patch, text=True, check=True)
        fingerprint = ledger.tree_fingerprint(tree)
        self.promotion['control_tree_sha256'] = fingerprint['sha256']
        self.promotion['control_tree_head'] = fingerprint['head']
        self.write_promotion()
        return tree

    def ctx(self):
        artifact = self.base / 'artifacts'
        artifact.mkdir(exist_ok=True)
        return profiles.ProfileContext(artifact)

    def runner(self, state=ledger.RED, detail='native-dynamic-context assertion failed: animated_face'):
        return lambda ctx, refs, tree, client: {
            ref: {'state': state, 'detail': detail, 'receipt': '/tmp/run/result.json'} for ref in refs}

    def test_records_a_red_on_the_reversed_tree(self):
        updated = ledger.graphical_control('GRAPHICAL-ROW', root=self.root, control_tree=self.control_tree(),
                                           ctx=self.ctx(), runner=self.runner())
        entry = updated['negative_control'][self.REF]
        self.assertEqual(entry['state'], 'red')
        self.assertEqual(entry['assertion'], 'animated_face')
        self.assertEqual(len(updated['graphical_control_backfill']), 1)
        on_disk = self.read_promotion()
        self.assertEqual(on_disk['negative_control'][self.REF]['state'], 'red')
        self.assertEqual(ledger.load_rows(self.root)[0]['status'], 'fixed')

    def test_refuses_a_tree_without_the_reversal(self):
        tree = self.control_tree(reverse=False)
        with self.assertRaises(ledger.Refused) as refused:
            ledger.graphical_control('GRAPHICAL-ROW', root=self.root, control_tree=tree, ctx=self.ctx(),
                                     runner=self.runner())
        self.assertIn('not the reversed control', str(refused.exception))

    def test_refuses_a_green_run(self):
        with self.assertRaises(ledger.Refused) as refused:
            ledger.graphical_control('GRAPHICAL-ROW', root=self.root, control_tree=self.control_tree(),
                                     ctx=self.ctx(), runner=self.runner(state=ledger.GREEN, detail=''))
        self.assertIn('not red on the reversed tree', str(refused.exception))
        self.assertEqual(self.read_promotion()['negative_control'][self.REF]['state'], 'unrunnable')

    def test_dry_run_leaves_the_receipt_alone(self):
        ledger.graphical_control('GRAPHICAL-ROW', root=self.root, control_tree=self.control_tree(),
                                 ctx=self.ctx(), runner=self.runner(), dry_run=True)
        self.assertEqual(self.read_promotion()['negative_control'][self.REF]['state'], 'unrunnable')

    def test_refuses_a_receipt_of_the_candidate_tree(self):
        import json
        self.control_tree()
        receipt = self.base / 'result.json'
        receipt.write_text(json.dumps({
            'status': 'failed', 'tree': {'head': self.fix, 'sha256': 'a' * 64},
            'details': {'named_scenario': {'id': 'native-dynamic-context', 'assertions': {'animated_face': False}}},
        }))
        with self.assertRaises(ledger.Refused) as refused:
            ledger.graphical_control('GRAPHICAL-ROW', root=self.root, receipt=receipt, ctx=self.ctx(),
                                     runner=self.runner())
        self.assertIn('not the reversed control', str(refused.exception))

    def test_refuses_a_receipt_of_another_tree(self):
        import json
        self.control_tree()
        receipt = self.base / 'result.json'
        receipt.write_text(json.dumps({
            'status': 'failed', 'tree': {'head': self.fix, 'sha256': 'c' * 64},
            'details': {'named_scenario': {'id': 'native-dynamic-context', 'assertions': {'animated_face': False}}},
        }))
        with self.assertRaises(ledger.Refused) as refused:
            ledger.graphical_control('GRAPHICAL-ROW', root=self.root, receipt=receipt, ctx=self.ctx(),
                                     runner=self.runner())
        self.assertIn("not the promotion's reversed control", str(refused.exception))

    def test_refuses_a_promotion_without_a_recorded_control_tree(self):
        tree = self.control_tree()
        self.promotion.pop('control_tree_sha256', None)
        self.write_promotion()
        with self.assertRaises(ledger.Refused) as refused:
            ledger.graphical_control('GRAPHICAL-ROW', root=self.root, control_tree=tree,
                                     ctx=self.ctx(), runner=self.runner())
        self.assertIn('records no control_tree_sha256', str(refused.exception))

    def test_refuses_a_control_tree_that_is_not_the_recorded_one(self):
        self.control_tree()
        other = self.base / 'other'
        import subprocess
        subprocess.run(['git', '-C', str(self.root), 'worktree', 'add', '--detach', str(other), 'HEAD'],
                       check=True, capture_output=True)
        patch, _, _ = ledger.non_test_patch(self.root, self.fix)
        subprocess.run(['git', '-C', str(other), 'apply', '-R', '-'], input=patch, text=True, check=True)
        (other / 'f.txt').write_text('old\nhand edit\n')
        with self.assertRaises(ledger.Refused) as refused:
            ledger.graphical_control('GRAPHICAL-ROW', root=self.root, control_tree=other, ctx=self.ctx(),
                                     runner=self.runner())
        self.assertIn("not the promotion's reversed control", str(refused.exception))

    def test_records_the_assertion_from_the_run_receipt(self):
        """A generic control detail is not evidence; the failed assertion comes from the run's receipt."""
        import json
        receipt = self.base / 'run/result.json'
        receipt.parent.mkdir()
        receipt.write_text(json.dumps({
            'status': 'failed',
            'details': {'named_scenario': {'id': 'native-dynamic-context',
                                           'assertions': {'animated_face': False}}},
        }))

        def runner(ctx, refs, tree, client):
            return {ref: {'state': ledger.RED, 'detail': "client receipt is 'failed', not passed",
                          'receipt': str(receipt)} for ref in refs}

        updated = ledger.graphical_control('GRAPHICAL-ROW', root=self.root,
                                           control_tree=self.control_tree(), ctx=self.ctx(), runner=runner)
        self.assertEqual(updated['negative_control'][self.REF]['assertion'], 'animated_face')

    def add_row(self, ident, proving=None, ref=None):
        body = self.ROW.format(fix=self.fix).replace('GRAPHICAL-ROW', ident)
        if proving is not None:
            body += f'proving_assertions = {{ "{ref or self.REF}" = "{proving}" }}\n'
        (self.root / f'docs/bugs/rows/{ident}.toml').write_text('[[bug]]\n' + body)

    def test_refuses_a_scene_shared_with_another_row_without_proving_assertions(self):
        self.add_row('OTHER-ROW')
        with self.assertRaises(ledger.Refused) as refused:
            ledger.graphical_control('GRAPHICAL-ROW', root=self.root, control_tree=self.control_tree(),
                                     ctx=self.ctx(), runner=self.runner())
        self.assertIn('is cited by another row too', str(refused.exception))
        self.assertIn('proving_assertions', str(refused.exception))

    def test_refuses_a_shared_scene_that_failed_another_rows_assertion(self):
        import json
        self.add_row('OTHER-ROW')
        (self.root / 'docs/bugs/rows/GRAPHICAL-ROW.toml').write_text(
            '[[bug]]\n' + self.ROW.format(fix=self.fix) + f'proving_assertions = {{ "{self.REF}" = "expected_name" }}\n')
        tree = self.control_tree()
        receipt = self.base / 'run/result.json'
        receipt.parent.mkdir()
        receipt.write_text(json.dumps({
            'status': 'failed',
            'details': {'named_scenario': {'id': 'native-dynamic-context',
                                           'assertions': {'another_rows_name': False}}},
        }))

        def runner(ctx, refs, tree, client):
            return {ref: {'state': ledger.RED, 'detail': 'failed', 'receipt': str(receipt)} for ref in refs}

        with self.assertRaises(ledger.Refused) as refused:
            ledger.graphical_control('GRAPHICAL-ROW', root=self.root, control_tree=tree,
                                     ctx=self.ctx(), runner=runner)
        self.assertIn("not the row's 'expected_name'", str(refused.exception))

    def test_accepts_a_shared_scene_that_failed_the_rows_own_assertion(self):
        import json
        self.add_row('OTHER-ROW')
        (self.root / 'docs/bugs/rows/GRAPHICAL-ROW.toml').write_text(
            '[[bug]]\n' + self.ROW.format(fix=self.fix) + f'proving_assertions = {{ "{self.REF}" = "expected_name" }}\n')
        receipt = self.base / 'run/result.json'
        receipt.parent.mkdir()
        receipt.write_text(json.dumps({
            'status': 'failed',
            'details': {'named_scenario': {'id': 'native-dynamic-context',
                                           'assertions': {'expected_name': False}}},
        }))

        def runner(ctx, refs, tree, client):
            return {ref: {'state': ledger.RED, 'detail': 'failed', 'receipt': str(receipt)} for ref in refs}

        updated = ledger.graphical_control('GRAPHICAL-ROW', root=self.root,
                                           control_tree=self.control_tree(), ctx=self.ctx(), runner=runner)
        self.assertEqual(updated['negative_control'][self.REF]['assertion'], 'expected_name')

    def test_refuses_a_receipt_whose_server_was_not_built_from_the_control(self):
        import json
        self.control_tree()
        receipt = self.base / 'result.json'
        receipt.write_text(json.dumps({
            'status': 'failed', 'tree': {'head': self.fix, 'sha256': self.promotion['control_tree_sha256']},
            'details': {'named_scenario': {'id': 'native-dynamic-context', 'assertions': {'animated_face': False}}},
        }))
        with self.assertRaises(ledger.Refused) as refused:
            ledger.graphical_control('GRAPHICAL-ROW', root=self.root, receipt=receipt, ctx=self.ctx(),
                                     runner=self.runner())
        self.assertIn('names no server binary built from', str(refused.exception))

    def test_refuses_an_open_row(self):
        (self.root / 'docs/bugs/rows/GRAPHICAL-ROW.toml').write_text(
            '[[bug]]\n' + self.ROW.format(fix=self.fix).replace('status = "fixed"', 'status = "open"')
            .replace('promotion = "docs/bugs/promotions/GRAPHICAL-ROW.json"\n', ''))
        with self.assertRaises(ledger.Refused) as refused:
            ledger.graphical_control('GRAPHICAL-ROW', root=self.root, control_tree=self.control_tree(),
                                     ctx=self.ctx(), runner=self.runner())
        self.assertIn('it never promotes', str(refused.exception))

    def test_records_a_validated_receipt_of_the_reversed_tree(self):
        import hashlib
        import json
        import subprocess

        client = self.base / 'client'
        client.mkdir()
        subprocess.run(['git', 'init', '-q', str(client)], check=True)
        (client / 'src.rs').write_text('fn main() {}\n')
        _git(client, 'add', '-A')
        client_head = _commit(client, 'client')
        binary = self.base / 'krasnokamen-client'
        binary.write_bytes(b'client-binary')
        digest = hashlib.sha256(b'client-binary').hexdigest()
        (self.base / 'manifest.json').write_text(json.dumps({
            'kind': 'client', 'repo': 'client', 'head': client_head, 'dirty': False, 'binary_sha256': digest}))
        (self.root / 'docs/bugs/rows/GRAPHICAL-ROW.toml').write_text(
            '[[bug]]\n' + self.ROW.format(fix=self.fix)
            + f'proving_assertions = {{ "{self.REF}" = "animated_face" }}\n')
        tree = self.control_tree()
        server_sha = 'e' * 64
        builds = self.root / '.analysis/ledger-builds'
        builds.mkdir(parents=True)
        (builds / f'{server_sha}-{self.promotion["control_tree_sha256"]}.json').write_text(json.dumps({
            'sha256': server_sha,
            'tree': {'head': self.fix, 'sha256': self.promotion['control_tree_sha256']}}))
        receipt = self.base / 'result.json'
        receipt.write_text(json.dumps({
            'status': 'failed', 'tree': {'head': self.fix, 'sha256': self.promotion['control_tree_sha256']},
            'server_binary': {'sha256': server_sha},
            'client_binary': {'path': str(binary), 'sha256': digest,
                              'manifest_path': str(self.base / 'manifest.json'), 'head': client_head,
                              'dirty': False},
            'details': {'named_scenario': {'id': 'native-dynamic-context',
                                           'assertions': {'animated_face': False, 'set_up': True}}},
        }))
        with patch.dict('os.environ', {'KRASNOKAMEN_CLIENT_ROOT': str(client)}):
            updated = ledger.graphical_control('GRAPHICAL-ROW', root=self.root, receipt=receipt, ctx=self.ctx(),
                                               runner=self.runner())
        entry = updated['negative_control'][self.REF]
        self.assertEqual(entry['state'], 'red')
        self.assertEqual(entry['assertion'], 'animated_face')
        self.assertEqual(entry['receipt'], str(receipt.resolve()))



LOADER_BATTLE_OLD = '''def pack_loaded_once_per_connection(events):
    successful = sum(1 for event in events if event.get("action") == 0)
    return successful >= 2
'''

LOADER_BATTLE_NEW = '''def pack_loaded_once_per_connection(events):
    actors = {event["actor"] for event in events if event.get("action") == 0}
    return len(actors) >= 2
'''

LOADER_TEST = '''import unittest

from battle import pack_loaded_once_per_connection


class PackTest(unittest.TestCase):
    def test_pair(self):
        assert pack_loaded_once_per_connection([{"actor": "a", "action": 0}])
'''


class LoaderFixTests(unittest.TestCase):
    """`loader:<sha>`: the loader python ref flips on the Loader fix's reversed production hunks."""

    REF = 'loader:python:tools:test_battle.PackTest.test_pair'

    def setUp(self):
        import subprocess
        import tempfile

        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.root = self.base / 'core'
        self.root.mkdir()
        subprocess.run(['git', 'init', '-q', str(self.root)], check=True)
        (self.root / 'README.md').write_text('core\n')
        _git(self.root, 'add', '-A')
        _commit(self.root, 'base')
        self.loader = self.base / 'loader'
        (self.loader / 'tools').mkdir(parents=True)
        subprocess.run(['git', 'init', '-q', str(self.loader)], check=True)
        (self.loader / 'README.md').write_text('loader\n')
        (self.loader / 'tools/battle.py').write_text(LOADER_BATTLE_OLD)
        (self.loader / 'tools/test_battle.py').write_text(LOADER_TEST)
        _git(self.loader, 'add', '-A')
        _commit(self.loader, 'before')
        (self.loader / 'tools/battle.py').write_text(LOADER_BATTLE_NEW)
        _git(self.loader, 'add', '-A')
        self.fix = _commit(self.loader, 'fix: every actor')
        self.env = patch.dict('os.environ', {'KRASNOKAMEN_LOADER_ROOT': str(self.loader)})
        self.env.start()
        self.addCleanup(self.env.stop)
        self.write_row()

    def write_row(self, control_files='["tools/battle.py"]'):
        (self.root / 'docs/bugs/rows').mkdir(parents=True, exist_ok=True)
        (self.root / 'docs/bugs/rows/LOADER-ROW.toml').write_text(
            '[[bug]]\nid = "LOADER-ROW"\ntitle = "t"\narea = "harness"\nrepro = "source"\n'
            'severity = "major"\nstatus = "open"\nsource = "s"\n'
            f'fix_commit = "loader:{self.fix[:12]}"\nregression = ["{self.REF}"]\n'
            f'control_files = {control_files}\n')

    def ctx(self):
        artifact = self.base / 'artifacts'
        artifact.mkdir(exist_ok=True)
        return profiles.ProfileContext(artifact)

    def runner(self, calls=None):
        def run(refs, tree):
            text = (Path(tree) / 'tools/battle.py').read_text()
            reversed_now = 'actors = {' not in text
            if calls is not None:
                calls.append(reversed_now)
            if not reversed_now:
                return {ref: {'state': ledger.GREEN, 'detail': ''} for ref in refs}
            test = Path(tree) / 'tools/test_battle.py'
            line = [index for index, text in enumerate(test.read_text().splitlines(), 1)
                    if 'assert pack_loaded' in text][0]
            return {ref: {'state': ledger.RED, 'detail': 'test assertion failed',
                          'assertion': {'file': str(test), 'line': line, 'error': 'AssertionError'}}
                    for ref in refs}
        return run

    def test_promotes_on_the_narrowed_real_pre_image(self):
        calls = []
        receipt = ledger.promote('LOADER-ROW', root=self.root, ctx=self.ctx(), loader_runner=self.runner(calls))
        self.assertEqual(receipt['control'], 'loader_reversal')
        self.assertEqual(receipt['reversed_files'], ['tools/battle.py'])
        self.assertEqual(calls, [True, False])
        self.assertEqual(receipt['loader_checkout']['head'], self.fix)
        self.assertEqual(ledger.load_rows(self.root)[0]['status'], 'fixed')

    def test_refuses_a_test_file_in_control_files(self):
        self.write_row('["tools/test_battle.py"]')
        with self.assertRaises(ledger.Refused) as refused:
            ledger.promote('LOADER-ROW', root=self.root, ctx=self.ctx(), loader_runner=self.runner())
        self.assertIn('a test or docs path', str(refused.exception))

    def test_refuses_without_control_files(self):
        (self.root / 'docs/bugs/rows/LOADER-ROW.toml').write_text(
            '[[bug]]\nid = "LOADER-ROW"\ntitle = "t"\narea = "harness"\nrepro = "source"\n'
            'severity = "major"\nstatus = "open"\nsource = "s"\n'
            f'fix_commit = "loader:{self.fix[:12]}"\nregression = ["{self.REF}"]\n')
        with self.assertRaises(ledger.Refused) as refused:
            ledger.promote('LOADER-ROW', root=self.root, ctx=self.ctx(), loader_runner=self.runner())
        self.assertIn('control_files', str(refused.exception))

    def test_refuses_a_red_outside_the_kept_test_file(self):
        def run(refs, tree):
            text = (Path(tree) / 'tools/battle.py').read_text()
            if 'actors = {' in text:
                return {ref: {'state': ledger.GREEN, 'detail': ''} for ref in refs}
            return {ref: {'state': ledger.RED, 'detail': 'test assertion failed',
                          'assertion': {'file': str(Path(tree) / 'tools/battle.py'), 'line': 1,
                                        'error': 'AssertionError'}} for ref in refs}
        with self.assertRaises(ledger.Refused) as refused:
            ledger.promote('LOADER-ROW', root=self.root, ctx=self.ctx(), loader_runner=run)
        self.assertIn('stayed green', str(refused.exception))

    def test_loader_ref_parses_and_rejects_a_traversal(self):
        self.assertEqual(regressions._parse_ref(self.REF),
                         ('loader', 'python', 'tools:test_battle.PackTest.test_pair'))
        self.assertEqual(regressions._parse_ref(
            'loader:gradle:java-agent:dev.krasnokamen.agent.javaagent.MinecraftClientObservationTest'),
            ('loader', 'gradle', 'java-agent:dev.krasnokamen.agent.javaagent.MinecraftClientObservationTest'))
        for bad in ('loader:python:../evil:test_x.Test.test_y', 'loader:python:tools:no_dot',
                    'loader:gradle:java-agent:not a selector', 'loader:python:tools'):
            with self.assertRaises(ValueError):
                regressions._parse_ref(bad)

    def test_run_accepts_a_fixed_loader_row(self):
        import json
        fix = f'loader:{self.fix[:12]}'
        (self.root / 'docs/bugs/rows/LOADER-ROW.toml').write_text(
            '[[bug]]\nid = "LOADER-ROW"\ntitle = "t"\narea = "harness"\nrepro = "source"\n'
            'severity = "major"\nstatus = "fixed"\nsource = "s"\n'
            f'fix_commit = "{fix}"\nregression = ["{self.REF}"]\n'
            'control_files = ["tools/battle.py"]\n'
            'promotion = "docs/bugs/promotions/LOADER-ROW.json"\n')
        (self.root / 'docs/bugs/promotions').mkdir(parents=True, exist_ok=True)
        (self.root / 'docs/bugs/promotions/LOADER-ROW.json').write_text(json.dumps({
            'id': 'LOADER-ROW', 'verdict': 'promoted', 'fix_commit': fix,
            'control': 'loader_reversal', 'reversed_files': ['tools/battle.py']}))
        with patch.object(regressions, 'REPO_ROOT', self.root), \
             patch.object(regressions, 'BASE_FIXED', frozenset()):
            details = regressions.run(profiles.ProfileContext(self.base / 'artifacts3'))
        self.assertEqual(details['fixed'], 1)


    def test_promotes_a_gradle_loader_ref(self):
        gradle = 'loader:gradle:java-agent:dev.krasnokamen.agent.javaagent.MinecraftClientObservationTest'
        (self.root / 'docs/bugs/rows/LOADER-ROW.toml').write_text(
            '[[bug]]\nid = "LOADER-ROW"\ntitle = "t"\narea = "harness"\nrepro = "source"\n'
            'severity = "major"\nstatus = "open"\nsource = "s"\n'
            f'fix_commit = "loader:{self.fix[:12]}"\nregression = ["{gradle}"]\n'
            'control_files = ["tools/battle.py"]\n')
        java = self.loader / 'java-agent'
        observation = java / 'src/main/java/dev/krasnokamen/agent/javaagent/Observation.java'
        observation.parent.mkdir(parents=True)
        observation.write_text('class O { int hands; }\n')
        _git(self.loader, 'add', '-A')
        _commit(self.loader, 'java base')
        observation.write_text('class O { int hands; int off; }\n')
        _git(self.loader, 'add', '-A')
        _commit(self.loader, 'fix: the observation')
        fix = _git(self.loader, 'rev-parse', 'HEAD').strip()
        (self.root / 'docs/bugs/rows/LOADER-ROW.toml').write_text(
            '[[bug]]\nid = "LOADER-ROW"\ntitle = "t"\narea = "harness"\nrepro = "source"\n'
            'severity = "major"\nstatus = "open"\nsource = "s"\n'
            f'fix_commit = "loader:{fix[:12]}"\nregression = ["{gradle}"]\n'
            'control_files = ["java-agent/src/main/java/dev/krasnokamen/agent/javaagent/Observation.java"]\n')
        test_file = (java / 'src/test/java/dev/krasnokamen/agent/javaagent/MinecraftClientObservationTest.java')
        test_file.parent.mkdir(parents=True)
        test_file.write_text('public class MinecraftClientObservationTest {}\n')

        def run(refs, tree):
            text = (Path(tree) / 'java-agent/src/main/java/dev/krasnokamen/agent/javaagent/Observation.java').read_text()
            if 'int off;' in text:
                return {ref: {'state': ledger.GREEN, 'detail': ''} for ref in refs}
            return {ref: {'state': ledger.RED, 'detail': 'assert',
                          'assertion': {'file': str(Path(tree) / 'java-agent/src/test/java/dev/krasnokamen/agent/javaagent/MinecraftClientObservationTest.java'),
                                        'line': 1, 'error': 'AssertionError'}} for ref in refs}

        receipt = ledger.promote('LOADER-ROW', root=self.root, ctx=self.ctx(), loader_runner=run)
        self.assertEqual(receipt['control'], 'loader_reversal')
        self.assertEqual(receipt['reversed_files'],
                         ['java-agent/src/main/java/dev/krasnokamen/agent/javaagent/Observation.java'])


class GradleLoaderReportTests(unittest.TestCase):
    """`_gradle_junit`: a Loader Java ref's state comes from its JUnit report, assertions only."""

    SELECTOR = 'dev.krasnokamen.agent.javaagent.MinecraftClientObservationTest'

    def report(self, body):
        import tempfile
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        module = Path(directory.name) / 'java-agent'
        (module / 'src/test/java/dev/krasnokamen/agent/javaagent').mkdir(parents=True)
        (module / 'src/test/java/dev/krasnokamen/agent/javaagent/MinecraftClientObservationTest.java').write_text('x\n' * 20)
        results = module / 'build/test-results/test'
        results.mkdir(parents=True)
        (results / 'TEST-dev.krasnokamen.agent.javaagent.MinecraftClientObservationTest.xml').write_text(
            '<testsuite name="%s">%s</testsuite>' % (self.SELECTOR, body))
        return module

    def test_assertion_failure_is_red_with_its_source_line(self):
        module = self.report(
            '<testcase name="exposesHands"><failure type="java.lang.AssertionError" message="expected">'
            'java.lang.AssertionError: expected\n\tat dev.krasnokamen.agent.javaagent.'
            'MinecraftClientObservationTest.exposesHands(MinecraftClientObservationTest.java:42)'
            '</failure></testcase>')
        state, detail, assertion = ledger._gradle_junit(module, self.SELECTOR)
        self.assertEqual(state, ledger.RED)
        self.assertEqual(assertion['error'], 'AssertionError')
        self.assertEqual(assertion['line'], 42)
        self.assertTrue(assertion['file'].endswith('MinecraftClientObservationTest.java'))

    def test_other_exception_is_unrunnable(self):
        module = self.report(
            '<testcase name="x"><failure type="java.lang.NullPointerException">'
            'java.lang.NullPointerException\n\tat dev.krasnokamen.agent.javaagent.'
            'MinecraftClientObservationTest.x(MinecraftClientObservationTest.java:7)</failure></testcase>')
        self.assertEqual(ledger._gradle_junit(module, self.SELECTOR)[0], ledger.UNRUNNABLE)

    def test_passing_and_skipped_reports(self):
        module = self.report('<testcase name="a"/><testcase name="b"><skipped/></testcase>')
        self.assertEqual(ledger._gradle_junit(module, self.SELECTOR)[0], ledger.GREEN)
        module = self.report('<testcase name="b"><skipped/></testcase>')
        self.assertEqual(ledger._gradle_junit(module, self.SELECTOR)[0], ledger.UNRUNNABLE)

    def test_missing_report_is_unrunnable(self):
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(ledger._gradle_junit(Path(directory), self.SELECTOR)[0], ledger.UNRUNNABLE)


class LoaderControlGuardTests(unittest.TestCase):
    """B2: a control file the fix creates or deletes is not a pre-image."""

    def setUp(self):
        import subprocess
        import tempfile

        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.loader = Path(directory.name) / 'loader'
        (self.loader / 'tools').mkdir(parents=True)
        subprocess.run(['git', 'init', '-q', str(self.loader)], check=True)
        (self.loader / 'tools/battle.py').write_text('old\n')
        _git(self.loader, 'add', '-A')
        _commit(self.loader, 'before')

    def test_refuses_a_file_the_fix_creates(self):
        (self.loader / 'tools/added.py').write_text('new\n')
        _git(self.loader, 'add', '-A')
        fix = _commit(self.loader, 'fix: adds a module')
        with self.assertRaises(ledger.Refused) as refused:
            ledger._loader_control_patch(self.loader, fix, ['tools/added.py'])
        self.assertIn('creates or deletes', str(refused.exception))

    def test_refuses_a_file_the_fix_deletes(self):
        _git(self.loader, 'rm', '-q', 'tools/battle.py')
        fix = _commit(self.loader, 'fix: deletes the module')
        with self.assertRaises(ledger.Refused) as refused:
            ledger._loader_control_patch(self.loader, fix, ['tools/battle.py'])
        self.assertIn('creates or deletes', str(refused.exception))

    def test_accepts_a_modified_file(self):
        (self.loader / 'tools/battle.py').write_text('new\n')
        _git(self.loader, 'add', '-A')
        fix = _commit(self.loader, 'fix: modifies the module')
        patch, files = ledger._loader_control_patch(self.loader, fix, ['tools/battle.py'])
        self.assertEqual(files, ['tools/battle.py'])
        self.assertIn('+new', patch)


class ClientControlSandboxTests(unittest.TestCase):
    """B3: a control copy runs in a scratch environment so a reversed guard cannot touch the host."""

    def setUp(self):
        import shutil
        import tempfile

        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.root = self.base / 'core'
        self.root.mkdir()
        self.real = self.base / 'client'
        self.real.mkdir()
        self.copy = self.base / 'control'
        self.copy.mkdir()
        self.original = dict(os.environ)

    def tearDown(self):
        os.environ.clear()
        os.environ.update(self.original)

    def _run(self, repo):
        seen = {}

        def exec_(ctx, argv, name, cwd, env):
            seen.update(env)
            return False, ''

        os.environ['KRASNOKAMEN_CLIENT_ROOT'] = str(self.real)
        os.environ['KRASNOKAMEN_CLIENT_SETTINGS'] = '/home/owner/client-settings.json'
        with patch.object(ledger, '_exec', exec_), \
                patch.object(ledger, 'client_repo', lambda root=ledger.REPO_ROOT: self.real):
            ledger._run_client(None, {('crate', 'module::test')}, self.root, repo)
        return seen

    def test_a_control_copy_runs_in_a_scratch_environment(self):
        seen = self._run(self.copy)
        scratch = str(self.root / '.analysis/client-sandbox')
        for key in ('HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME', 'APPDATA', 'LOCALAPPDATA'):
            self.assertTrue(seen[key].startswith(scratch), (key, seen[key]))
        self.assertTrue(seen['KRASNOKAMEN_CLIENT_SETTINGS'].startswith(scratch), seen['KRASNOKAMEN_CLIENT_SETTINGS'])
        self.assertNotIn('KRASNOKAMEN_CLIENT_ROOT', seen)

    def test_the_repository_itself_keeps_the_operator_environment(self):
        seen = self._run(self.real)
        self.assertEqual(seen['KRASNOKAMEN_CLIENT_SETTINGS'], '/home/owner/client-settings.json')
        self.assertEqual(seen['HOME'], self.original['HOME'])


class RegistryKeyTests(unittest.TestCase):
    """F1: one binary built from two trees (candidate and its Python-only control) stays citable."""

    def test_two_trees_for_one_binary_are_both_kept(self):
        import hashlib
        import json
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sha = 'a' * 64
            candidate = {'head': 'b' * 40, 'dirty': False, 'sha256': hashlib.sha256(b'').hexdigest()}
            control = {'head': 'b' * 40, 'dirty': True, 'sha256': 'c' * 64}
            with patch.object(ledger, 'REPO_ROOT', root):
                ledger._register_server(sha, candidate)
                ledger._register_server(sha, control)
                registry = root / '.analysis/ledger-builds'
                self.assertEqual(len(list(registry.glob('*.json'))), 2)
                self.assertEqual(ledger._registration(registry, sha, candidate)['tree']['sha256'], candidate['sha256'])
                self.assertEqual(ledger._registration(registry, sha, control)['tree']['sha256'], control['sha256'])
                self.assertIsNone(ledger._registration(registry, sha, {'head': 'd' * 40, 'sha256': 'e' * 64}))
                receipt = json.dumps({'server_binary_sha256': sha})
                self.assertEqual(ledger._server_built_from(root, receipt, candidate), sha)
                self.assertEqual(ledger._server_built_from(root, receipt, control), sha)
                with self.assertRaises(ledger.Refused):
                    ledger._server_built_from(root, receipt, {'head': 'd' * 40, 'sha256': 'e' * 64})


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
            # Two clean trees differ only by their head: both entries must survive (REVIEW-wave2-12 m3).
            ledger._register_server('a' * 64, {'head': 'f' * 40, 'sha256': 'c' * 64})
            self.assertEqual(len(list(registry.glob('*.json'))), 3)
            self.assertEqual(ledger._registration(registry, 'a' * 64, {'head': 'f' * 40, 'sha256': 'c' * 64}),
                             {'sha256': 'a' * 64, 'tree': {'head': 'f' * 40, 'sha256': 'c' * 64}})
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

class RestoreBytecodeTests(unittest.TestCase):
    """A reversed fix and its restore can share a python source's size and second.

    CPython then accepts the control's .pyc for the restored source, so the restore runs the
    control's code and a valid promotion is refused (SCENARIO-RANDOM-BREAK-NO-PROGRESS,
    2026-11-25: the two ordinals dicts differ by one digit and the stages ran in one second)."""

    def test_materialize_drops_python_bytecode_caches(self):
        import tempfile
        forget = getattr(ledger, '_forget_bytecode', None)
        self.assertIsNotNone(forget, 'the restore must remove ignored bytecode caches')
        with tempfile.TemporaryDirectory() as directory:
            tree = Path(directory)
            cache = tree / 'tools' / 'harness' / '__pycache__'
            cache.mkdir(parents=True)
            (cache / 'protocol_input_verbs.cpython-314.pyc').write_bytes(b'\x00')
            keep = tree / 'crates' / '__pycache__'
            keep.mkdir(parents=True)
            (keep / 'x.pyc').write_bytes(b'\x00')
            forget(tree)
            self.assertFalse(cache.exists())
            self.assertTrue(keep.exists(), 'only the python refs under tools/ are run from this tree')

    def test_python_ref_runs_without_writing_bytecode(self):
        import tempfile
        seen = {}

        def fake_exec(ctx, argv, tag, tree, env):
            seen['env'] = env
            return True, 'Ran 1 test in 0.1s\n\nOK\n'

        with tempfile.TemporaryDirectory() as directory, \
                patch.object(ledger, '_exec', fake_exec):
            results = ledger._run_python(object(), {('tools.harness.test_x', 'T.test_y')}, Path(directory))
        self.assertEqual(results[('tools.harness.test_x', 'T.test_y')][0], ledger.GREEN)
        self.assertEqual((seen.get('env') or {}).get('PYTHONDONTWRITEBYTECODE'), '1',
                         'a python ref must not leave a .pyc for the control and the restore to share')

class GradleJUnitLayoutTests(unittest.TestCase):
    """A Loader that sets its build dir per Minecraft version keeps the JUnit XML under it.

    `<module>/build/<version>/test-results/test/TEST-*.xml` was invisible to the locator, so a
    reversed loader fix reported "no JUnit report" and the promotion of
    LOADER-MCP-ATTACK-IGNORES-USE-ITEM-GATE was refused with a red control (2026-11-25)."""

    REPORT = '''<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="dev.krasnokamen.agent.javaagent.MinecraftAttackInputTest" tests="1" failures="1">
  <testcase name="usingItemRefuses" classname="dev.krasnokamen.agent.javaagent.MinecraftAttackInputTest">
    <failure type="java.lang.AssertionError" message="expected true">java.lang.AssertionError: expected true
        at dev.krasnokamen.agent.javaagent.MinecraftAttackInputTest.usingItemRefuses(MinecraftAttackInputTest.java:99)</failure>
  </testcase>
</testsuite>
'''

    @staticmethod
    def _source(module: Path):
        source = module / 'src' / 'test' / 'java' / 'dev' / 'krasnokamen' / 'agent' / 'javaagent' / 'MinecraftAttackInputTest.java'
        source.parent.mkdir(parents=True, exist_ok=True)
        source.write_text('final class MinecraftAttackInputTest {}\n')

    def test_versioned_build_directory_report_is_read(self):
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            module = Path(directory) / 'java-agent'
            results = module / 'build' / '26.3' / 'test-results' / 'test'
            results.mkdir(parents=True)
            (results / 'TEST-dev.krasnokamen.agent.javaagent.MinecraftAttackInputTest.xml').write_text(self.REPORT)
            self._source(module)
            state, detail, assertion = ledger._gradle_junit(
                module, 'dev.krasnokamen.agent.javaagent.MinecraftAttackInputTest')
        self.assertEqual(state, ledger.RED, detail)
        self.assertIsNotNone(assertion)
        self.assertEqual(assertion.get('line'), 99)

    def test_plain_layout_still_reads(self):
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            module = Path(directory) / 'java-agent'
            results = module / 'build' / 'test-results' / 'test'
            results.mkdir(parents=True)
            (results / 'TEST-dev.krasnokamen.agent.javaagent.MinecraftAttackInputTest.xml').write_text(self.REPORT)
            self._source(module)
            state, detail, assertion = ledger._gradle_junit(
                module, 'dev.krasnokamen.agent.javaagent.MinecraftAttackInputTest')
        self.assertEqual(state, ledger.RED, detail)

    def test_mockito_verification_failure_is_an_assertion(self):
        report = self.REPORT.replace(
            'type="java.lang.AssertionError" message="expected true">java.lang.AssertionError: expected true',
            'type="org.mockito.exceptions.verification.NoInteractionsWanted" message="no interactions">'
            'org.mockito.exceptions.verification.NoInteractionsWanted: No interactions wanted')
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            module = Path(directory) / 'java-agent'
            results = module / 'build' / 'test-results' / 'test'
            results.mkdir(parents=True)
            (results / 'TEST-dev.krasnokamen.agent.javaagent.MinecraftAttackInputTest.xml').write_text(report)
            self._source(module)
            state, detail, assertion = ledger._gradle_junit(
                module, 'dev.krasnokamen.agent.javaagent.MinecraftAttackInputTest')
        self.assertEqual(state, ledger.RED, detail)
