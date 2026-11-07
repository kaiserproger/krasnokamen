import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.harness import profiles, regressions


class RegressionLedgerTests(unittest.TestCase):
    def test_nextest_listing_accepts_compiler_output(self):
        result = regressions._nextest_listing('Compiling example\\n{"rust-suites": {}}\\n')
        self.assertEqual(result['rust-suites'], {})

    def test_unknown_exact_test_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            (root / 'docs/bugs/ledger.toml').write_text('''[[bug]]
id = "EXAMPLE"
title = "missing test"
area = "harness"
repro = "cargo"
severity = "major"
status = "fixed"
source = "example"
regression = ["cargo:mc-core:missing::test"]
''')
            ctx = profiles.ProfileContext(root)
            def list_empty(argv, ctx, name):
                (root / name).write_text('{"rust-suites": {}}')
            with patch.object(regressions, 'REPO_ROOT', root), patch.object(profiles, '_run_command', list_empty):
                with self.assertRaisesRegex(RuntimeError, 'EXAMPLE.*unknown'):
                    regressions.run(ctx)
            ledger = root / 'docs/bugs/ledger.toml'
            ledger.write_text(ledger.read_text().replace('status = "fixed"', 'status = "open"'))
            with patch.object(regressions, 'REPO_ROOT', root), patch.object(profiles, '_run_command', list_empty):
                with self.assertRaisesRegex(RuntimeError, 'EXAMPLE.*unknown'):
                    regressions.run(ctx)

    def test_gameplay_fix_can_use_cargo_check(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            (root / 'docs/bugs/ledger.toml').write_text('''[[bug]]
id = "GAMEPLAY"
title = "gameplay"
area = "gameplay"
repro = "source"
severity = "major"
status = "fixed"
source = "example"
regression = ["cargo:mc-core:some_test"]
''')
            def run_command(argv, ctx, name):
                if name.endswith('-list.log'):
                    (root / name).write_text('{"rust-suites":{"lib":{"package-name":"mc-core","testcases":{"some_test":{"filter-match":{"status":"matches"},"ignored":false}}}}}')
            with patch.object(regressions, 'REPO_ROOT', root), patch.object(profiles, '_run_command', run_command):
                self.assertEqual(regressions.run(profiles.ProfileContext(root))['fixed'], 1)

    def test_manual_gameplay_fix_needs_client_check(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            (root / 'docs/bugs/ledger.toml').write_text('''[[bug]]
id = "OWNER"
title = "manual repro"
area = "gameplay"
repro = "manual"
severity = "major"
status = "fixed"
source = "example"
regression = ["cargo:mc-core:some_test"]
''')
            with patch.object(regressions, 'REPO_ROOT', root), patch.object(profiles, '_run_command', side_effect=AssertionError("must reject before executing")):
                with self.assertRaisesRegex(ValueError, 'OWNER.*real-client'):
                    regressions.run(profiles.ProfileContext(root))

    def test_manual_repro_rejects_tcp_scenario(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            (root / 'docs/bugs/ledger.toml').write_text('''[[bug]]
id = "OWNER"
title = "manual repro"
area = "gameplay"
repro = "manual"
severity = "major"
status = "fixed"
source = "example"
regression = ["harness:inventory:inventory-client"]
''')
            with patch.object(regressions, 'REPO_ROOT', root), patch.object(profiles, '_run_command', side_effect=AssertionError("must reject before executing")):
                with self.assertRaisesRegex(ValueError, 'OWNER.*real-client'):
                    regressions.run(profiles.ProfileContext(root))

    def test_player_visible_fix_requires_real_client_check(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            (root / 'docs/bugs/ledger.toml').write_text('''[[bug]]
id = "CLIENT"
title = "visible"
area = "client"
repro = "source"
severity = "major"
status = "fixed"
source = "example"
regression = ["cargo:mc-core:some_test"]
''')
            with patch.object(regressions, 'REPO_ROOT', root):
                with self.assertRaisesRegex(ValueError, 'CLIENT.*real-client'):
                    regressions.run(profiles.ProfileContext(root))

    def test_client_prepared_is_not_a_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            (root / 'docs/bugs/ledger.toml').write_text('''[[bug]]
id = "VISIBLE"
title = "visible"
area = "client"
repro = "client"
severity = "major"
status = "fixed"
source = "example"
regression = ["harness:entities:native-v2-craft-drag"]
''')
            receipt = root / 'child'
            receipt.mkdir()
            (receipt / 'result.json').write_text('{"status":"prepared"}')
            def run_child(argv, ctx, name, **kwargs):
                self.assertNotIn('SOLARIS_FIRST_START_RECOVERY_BINARY', kwargs['env'])
                (root / name).write_text(f'[harness] artifact={receipt}\n')
            ctx = profiles.ProfileContext(root, include_regression_client=True)
            with patch.dict('os.environ', {'SOLARIS_FIRST_START_RECOVERY_BINARY': '/mutation'}), patch.object(regressions, 'REPO_ROOT', root), patch.object(profiles, '_run_command', run_child), patch.object(regressions, '_release_server', return_value=root / 'server'):
                with self.assertRaisesRegex(RuntimeError, 'VISIBLE.*prepared'):
                    regressions.run(ctx)
            (receipt / 'result.json').write_text('{"status":"passed","details":{"server_binary_sha256":"wrong"}}')
            ctx.client_evidence['regressions_server'] = {'sha256': 'expected'}
            with patch.object(regressions, 'REPO_ROOT', root), patch.object(profiles, '_run_command', run_child), patch.object(regressions, '_release_server', return_value=root / 'server'):
                with self.assertRaisesRegex(RuntimeError, 'VISIBLE.*release server'):
                    regressions.run(ctx)

    def test_invalid_ledgers_fail_closed(self):
        valid = '''[[bug]]
id = "B"
title = "bug"
area = "gameplay"
repro = "source"
severity = "major"
status = "open"
source = "report"
regression = []
'''
        cases = [
            '', '[[bugs]]\nid = "typo"\n', 'bug = []\n',
            valid.replace('area = "gameplay"', 'area = "unknown"'),
            valid.replace('repro = "source"', 'repro = "unknown"'),
            valid.replace('severity = "major"', 'severity = "unknown"'),
            valid.replace('status = "open"', 'status = "unknown"'),
            valid + 'unexpected = "value"\n',
            valid.replace('regression = []', 'regression = ["harness:scene-shot"]'),
        ]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            ledger = root / 'docs/bugs/ledger.toml'
            for text in cases:
                with self.subTest(text=text):
                    ledger.write_text(text)
                    with patch.object(regressions, 'REPO_ROOT', root):
                        with self.assertRaises(ValueError):
                            regressions.run(profiles.ProfileContext(root))

    def test_client_scenario_argv(self):
        native = regressions._client_command('entities', 'native-v2-craft-drag')
        self.assertEqual(native[-3:], ['--client', 'native', '--windowed'])
        java = regressions._client_command('entities', 'java-v2-p5-parity')
        self.assertEqual(java[-2:], ['--client', 'java'])
        from tools.harness.scenarios import SCENARIOS
        for scenario, entry in SCENARIOS.items():
            if entry.evidence in ('graphical', 'tcp', 'native') and entry.profile not in ('loader-live', 'dh-gate'):
                argv = regressions._client_command(entry.profile, scenario)
                self.assertEqual(argv[5], scenario)
                self.assertEqual(argv[6:], ['--client', 'native', '--windowed'] if scenario.startswith('native-') else ['--client', 'java'])
        with self.assertRaises(ValueError):
            regressions._client_command('entities', 'no-such-scenario')

    def test_open_checks_are_run(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            (root / 'docs/bugs/ledger.toml').write_text('''[[bug]]
id = "OPEN"
title = "partial"
area = "gameplay"
repro = "source"
severity = "major"
status = "open"
source = "report"
regression = ["cargo:mc-core:some_test"]
''')
            calls = []
            def run_command(argv, ctx, name):
                calls.append(argv)
                if name.endswith('-list.log'):
                    (root / name).write_text('{"rust-suites":{"lib":{"package-name":"mc-core","testcases":{"some_test":{"filter-match":{"status":"matches"},"ignored":false}}}}}')
            with patch.object(regressions, 'REPO_ROOT', root), patch.object(profiles, '_run_command', run_command):
                regressions.run(profiles.ProfileContext(root))
            self.assertEqual([call[2] for call in calls], ['list', 'run'])

    def test_assertion_failure_is_attributed_to_its_bug(self):
        cargo = {'mc-core': {'module::broken': {'BROKEN'}, 'module::other': {'OTHER'}}}
        log = '        FAIL [   0.120s] (1/2) mc-core module::broken\n'
        self.assertEqual(regressions._failed_ids(log, cargo), {'BROKEN'})

    def test_failing_assertion_reports_only_matching_bug(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            (root / 'docs/bugs/ledger.toml').write_text('''[[bug]]
id = "BROKEN"
title = "broken"
area = "gameplay"
repro = "source"
severity = "major"
status = "open"
source = "report"
regression = ["cargo:mc-core:module::broken"]
[[bug]]
id = "OTHER"
title = "other"
area = "gameplay"
repro = "source"
severity = "major"
status = "open"
source = "report"
regression = ["cargo:mc-core:module::other"]
''')
            def run_command(argv, ctx, name):
                if name.endswith('-list.log'):
                    (root / name).write_text('{"rust-suites":{"lib":{"package-name":"mc-core","testcases":{"module::broken":{"filter-match":{"status":"matches"}},"module::other":{"filter-match":{"status":"matches"}}}}}}')
                else:
                    (root / name).write_text('        FAIL [   0.120s] (1/2) mc-core module::broken\n')
                    raise RuntimeError('assertion failed')
            with patch.object(regressions, 'REPO_ROOT', root), patch.object(profiles, '_run_command', run_command):
                with self.assertRaises(RuntimeError) as error:
                    regressions.run(profiles.ProfileContext(root))
            self.assertIn('BROKEN', str(error.exception))
            self.assertNotIn('OTHER', str(error.exception))

    def test_release_server_identity_is_recorded(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / 'release/solaris-v2'
            binary.parent.mkdir()
            binary.write_bytes(b'candidate')
            ctx = profiles.ProfileContext(root)
            with patch.dict('os.environ', {'CARGO_TARGET_DIR': str(root)}), patch.object(profiles, '_run_command') as command:
                self.assertEqual(regressions._release_server(ctx, profiles), binary)
            self.assertIn('--release', command.call_args.args[0])
            self.assertEqual(ctx.client_evidence['regressions_server']['sha256'], __import__('hashlib').sha256(b'candidate').hexdigest())

    def test_red_repro_is_not_run_as_a_gate(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            (root / 'docs/bugs/ledger.toml').write_text('''[[bug]]
id = "RED"
title = "known red"
area = "gameplay"
repro = "cargo"
severity = "major"
status = "open"
source = "report"
regression = []
repro_check = "cargo:mc-core:known_red"
''')
            with patch.object(regressions, 'REPO_ROOT', root), patch.object(profiles, '_run_command') as command:
                regressions.run(profiles.ProfileContext(root))
                command.assert_not_called()
