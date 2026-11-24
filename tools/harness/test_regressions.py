import json
import hashlib
import os
import re
import shutil
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.harness import ledger, profiles, regressions


class RegressionLedgerTests(unittest.TestCase):
    def setUp(self):
        # Scratch ledgers below are `fixed` without promotion receipts: grandfather them.
        patcher = patch.object(regressions, 'BASE_FIXED', frozenset({'DOOR', 'SLEEP', 'EXAMPLE', 'GAMEPLAY', 'OWNER', 'CLIENT', 'VISIBLE'}))
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_profile_verdict_validation(self):
        for profile, verdict in [('unknown-profile', 'all'), ('village-qa', 'unknown'),
                                 ('loader-live', 'all')]:
            with self.subTest(profile=profile, verdict=verdict):
                with self.assertRaises(ValueError):
                    regressions._profile_command(profile, verdict)

    def test_chisel_optional_admission_routes_only_that_verdict_to_java(self):
        command = regressions._profile_command('chisel', 'vanilla_optional_admission')
        self.assertEqual(command[-3:], ['--client', 'java', '--windowed'])
        self.assertEqual(regressions._profile_command('chisel', 'all')[-3:],
                         ['--client', 'native', '--windowed'])
        self.assertEqual(regressions._profile_command('village-qa', 'all')[-3:],
                         ['--client', 'native', '--windowed'])
        for value in (None, False, 'true'):
            with self.assertRaises(RuntimeError):
                regressions._profile_verdict(
                    {'details': {'verdicts': {'vanilla_optional_admission': value}}},
                    'chisel', 'vanilla_optional_admission')
        regressions._profile_verdict(
            {'details': {'verdicts': {'vanilla_optional_admission': True}}},
            'chisel', 'vanilla_optional_admission')

    def test_autonomy_profile_uses_named_verdict(self):
        regressions._profile_command('village-autonomy-qa', 'walk_to_village')
        regressions._profile_command('village-autonomy-qa', 'all')
        receipt = {'details': {'scenario': {'autonomy_acceptance': {
            'behaviour': {'walk_to_village': 'PASS'}}}}}
        regressions._profile_verdict(receipt, 'village-autonomy-qa', 'walk_to_village')
        for profile in ('civilization-seed-qa', 'village-owner-order-qa'):
            regressions._profile_command(profile, 'all')
            with self.assertRaises(ValueError):
                regressions._profile_command(profile, 'walk_to_village')

    def test_civilization_ref_runs_the_default_seed_with_the_long_timeout(self):
        command = regressions._profile_command('civilization-seed-qa', 'new_house')
        self.assertEqual(command[-2:], ['--timeout-seconds', '7200'])
        for name in ('VILLAGE_AUTONOMY_SEED', 'VILLAGE_AUTONOMY_CENTER'):
            with patch.dict('os.environ', {name: '100'}):
                with self.assertRaisesRegex(ValueError, name):
                    regressions._profile_command('civilization-seed-qa', 'new_house')

    def test_building_verdict_is_keyed_by_blueprint(self):
        command = regressions._profile_command('buildings-qa', 'wall.L1_built')
        # Only the wall is built and checked; the other blueprints neither run nor decide this verdict.
        self.assertEqual(command[:2], ['env', 'BUILDINGS_QA_SELECT=wall'])
        self.assertIn('--timeout-seconds', command)
        self.assertNotIn('BUILDINGS_QA_SELECT', ' '.join(regressions._profile_command('buildings-qa', 'all')))
        with self.assertRaises(ValueError):
            regressions._profile_command('buildings-qa', 'unknown.L1_built')
        receipt = {'details': {'scenario': {'buildings_acceptance': {'blueprints': {
            'wall': {'behaviour': {'L1_built': 'PASS'}}}}}}}
        regressions._profile_verdict(receipt, 'buildings-qa', 'wall.L1_built')

    def test_profile_rejects_partial_building_run(self):
        with patch.dict('os.environ', {'BUILDINGS_QA_STOP_AFTER_ORDER': '1'}):
            with self.assertRaisesRegex(ValueError, 'BUILDINGS_QA_STOP_AFTER_ORDER'):
                regressions._profile_command('buildings-qa', 'wall.L1_built')

    def test_boolean_profile_verdicts_fail_closed(self):
        regressions._profile_command('civilization-seed-qa', 'new_house')
        receipt = {'status': 'passed', 'details': {'verdicts': {'new_house': False}}}
        with self.assertRaisesRegex(RuntimeError, 'new_house'):
            regressions._profile_verdict(receipt, 'civilization-seed-qa', 'new_house')
        receipt['details']['verdicts']['new_house'] = True
        regressions._profile_verdict(receipt, 'civilization-seed-qa', 'new_house')
        with self.assertRaisesRegex(RuntimeError, 'all'):
            regressions._profile_verdict(receipt, 'civilization-seed-qa', 'all')
        receipt['details']['verdicts']['all'] = True
        regressions._profile_verdict(receipt, 'civilization-seed-qa', 'all')

    def test_native_matrix_rejects_unknown_verdict(self):
        from tools.harness.backends.native_matrix import frozen_native_rows, verdict_key
        rows = frozen_native_rows()
        regressions._profile_command('native-matrix', verdict_key(rows[0]['id']))
        with self.assertRaises(ValueError):
            regressions._profile_command('native-matrix', '26.3.not-a-frozen-scenario')

    def test_native_matrix_refuses_override_for_regressions(self):
        for name in ('KRASNOKAMEN_MATRIX_PATH', 'KRASNOKAMEN_MATRIX_LEDGER', 'KRASNOKAMEN_MATRIX_RUN_ROOT'):
            with self.subTest(name=name), patch.dict(os.environ, {name: '/override'}):
                with self.assertRaisesRegex(ValueError, name):
                    regressions._profile_command('native-matrix', 'all')

    def test_native_matrix_rejects_true_partial_verdict(self):
        receipt = {'status': 'passed', 'details': {
            'matrix_full_run': False, 'verdicts': {'26.3.example': True}}}
        with self.assertRaisesRegex(regressions.VerdictNotMet, 'full frozen matrix'):
            regressions._profile_verdict(receipt, 'native-matrix', '26.3.example')
        receipt['details']['matrix_full_run'] = True
        regressions._profile_verdict(receipt, 'native-matrix', '26.3.example')
        del receipt['details']['verdicts']
        with self.assertRaises(regressions.VerdictNotMet):
            regressions._profile_verdict(receipt, 'native-matrix', 'all')

    def test_native_matrix_true_verdict_still_requires_passing_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            child = root / 'child'
            child.mkdir()
            (child / 'result.json').write_text(json.dumps({
                'profile': 'native-matrix', 'status': 'failed',
                'details': {'verdicts': {'26.3.example': True}, 'failed_rows': [],
                            'blocked_rows': [{'id': 'windows', 'reason': 'unsupported'}]}}))
            ctx = profiles.ProfileContext(root)
            ctx.client_evidence['regressions_server'] = {'sha256': 'expected'}
            def run_child(argv, ctx, name, **kwargs):
                (root / name).write_text(f'[harness] artifact={child}\n')
                raise RuntimeError('matrix incomplete')
            with patch.object(profiles, '_run_command', run_child):
                with self.assertRaises(regressions.ReceiptNotPassed):
                    regressions._client_receipt(ctx, profiles, [], {}, 'native-matrix',
                                                '26.3.example', True)

    def test_profile_verdicts_fail_closed_and_share_one_run(self):
        import json
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            ledger.write_rows(root, ''.join(
                f'''[[bug]]
id = "{ident}"
title = "visible"
area = "client"
repro = "client"
severity = "major"
status = "fixed"
source = "example"
regression = ["harness:village-qa:{verdict}"]
'''
                for ident, verdict in [('DOOR', 'door_walk'), ('SLEEP', 'sleep_return')]))
            receipt = root / 'child'
            receipt.mkdir()
            calls = []
            def run_child(argv, ctx, name, **kwargs):
                calls.append(argv)
                (root / name).write_text(f'[harness] artifact={receipt}\n')
            result = {'status': 'passed', 'details': {'server_binary_sha256': 'expected',
                      'scenario': {'settlement_acceptance': {'behaviour': {
                          'door_walk': 'PASS', 'sleep_return': 'FAIL'}}}}}
            (receipt / 'result.json').write_text(json.dumps(result))
            ctx = profiles.ProfileContext(root, include_regression_client=True)
            ctx.client_evidence['regressions_server'] = {'sha256': 'expected'}
            with patch.object(regressions, 'REPO_ROOT', root), patch.object(profiles, '_run_command', run_child), patch.object(regressions, '_release_server', return_value=root / 'server'):
                with self.assertRaisesRegex(RuntimeError, 'SLEEP.*sleep_return'):
                    regressions.run(ctx)
                self.assertEqual(len(calls), 1)
                calls.clear()
                result['details']['scenario']['settlement_acceptance']['behaviour']['sleep_return'] = 'PASS'
                (receipt / 'result.json').write_text(json.dumps(result))
                self.assertEqual(regressions.run(ctx)['fixed'], 2)
                self.assertEqual(len(calls), 1)
                result['details']['unrelated'] = result['details'].pop('server_binary_sha256')
                (receipt / 'result.json').write_text(json.dumps(result))
                with self.assertRaisesRegex(RuntimeError, 'release server'):
                    regressions.run(ctx)

    def test_unknown_exact_test_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            ledger.write_rows(root, '''[[bug]]
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
            ledger.write_rows(root, ledger.document(root).replace('status = "fixed"', 'status = "open"'))
            with patch.object(regressions, 'REPO_ROOT', root), patch.object(regressions, 'BASE_FIXED', frozenset()), patch.object(profiles, '_run_command', list_empty):
                with self.assertRaisesRegex(RuntimeError, 'EXAMPLE.*unknown'):
                    regressions.run(ctx)

    def test_gameplay_fix_can_use_cargo_check(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            ledger.write_rows(root, '''[[bug]]
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
            ledger.write_rows(root, '''[[bug]]
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
            ledger.write_rows(root, '''[[bug]]
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

    def test_command_repro_fix_needs_cargo_check(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            ledger.write_rows(root, '''[[bug]]
id = "PRUNE"
title = "offline command"
area = "gameplay"
repro = "command"
severity = "major"
status = "fixed"
source = "example"
regression = ["python:tools.harness.test_example:Example.test_case"]
''')
            with patch.object(regressions, 'REPO_ROOT', root), patch.object(profiles, '_run_command', side_effect=AssertionError("must reject before executing")):
                with self.assertRaisesRegex(ValueError, 'PRUNE.*cargo check'):
                    regressions.run(profiles.ProfileContext(root))

    def test_player_visible_fix_requires_real_client_check(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            ledger.write_rows(root, '''[[bug]]
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
            ledger.write_rows(root, '''[[bug]]
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
                self.assertNotIn('KRASNOKAMEN_FIRST_START_RECOVERY_BINARY', kwargs['env'])
                (root / name).write_text(f'[harness] artifact={receipt}\n')
            ctx = profiles.ProfileContext(root, include_regression_client=True)
            with patch.dict('os.environ', {'KRASNOKAMEN_FIRST_START_RECOVERY_BINARY': '/mutation'}), patch.object(regressions, 'REPO_ROOT', root), patch.object(profiles, '_run_command', run_child), patch.object(regressions, '_release_server', return_value=root / 'server'):
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
            (root / 'docs/bugs/rows').mkdir(parents=True)
            for text in cases:
                with self.subTest(text=text):
                    # Written raw: the point is what the loader and the gate say about a bad row file.
                    shutil.rmtree(root / 'docs/bugs/rows')
                    (root / 'docs/bugs/rows').mkdir(parents=True)
                    (root / 'docs/bugs/rows/B.toml').write_text(text)
                    with patch.object(regressions, 'REPO_ROOT', root):
                        with self.assertRaises(ValueError):
                            regressions.run(profiles.ProfileContext(root))

    def test_unknown_client_scenario_is_rejected(self):
        with self.assertRaises(ValueError):
            regressions._client_command('entities', 'no-such-scenario')


    def test_loader_live_ref_parse_and_command(self):
        ref = 'harness:loader-live:loader-fabric-screen-admission'
        self.assertEqual(regressions._parse_ref(ref), ('harness', 'loader-live', 'loader-fabric-screen-admission'))
        argv = regressions._client_command('loader-live', 'loader-fabric-screen-admission')
        self.assertEqual(argv[5], 'loader-fabric-screen-admission')
        self.assertEqual(argv[6:], ['--platform', 'fabric', '--server-bin', '{server_bin}',
                                    '--plugins-dir', '{plugins_dir}', '--timeout-seconds', '420'])

    def test_battle_recovery_refs_are_graphical_and_take_only_the_env_server(self):
        for scenario in ('battle-kit-census', 'battle-crash-handoff', 'battle-auth-hydration'):
            ref = f'harness:battle-crash-recovery:{scenario}'
            with self.subTest(ref=ref):
                self.assertTrue(regressions.is_graphical_ref(ref))
                argv = regressions._client_command('battle-crash-recovery', scenario)
                self.assertEqual(argv[1:], ['-m', 'tools.harness', 'run', 'scenario', scenario])

    def test_loader_live_ref_fails_closed(self):
        for bad in ('harness:loader-live:', 'harness:loader-live:no-such-scenario',
                    'harness:loader-live:native-low-roof-house', 'harness:dh-gate:all',
                    'harness:loader-live:loader-fabric-screen-admission:extra'):
            with self.subTest(bad=bad):
                with self.assertRaises(ValueError):
                    regressions._parse_ref(bad)

    def test_scenario_ref_red_only_on_assertion_failure(self):
        scene_receipt = {'status': 'failed', 'error': 'scene-shot assertion failed: house-certification_contains_0',
                         'details': {'named_scenario': {'id': 'native-low-roof-house'}}}
        self.assertTrue(ledger._scenario_assertion_failed(scene_receipt, 'native-low-roof-house'))
        loader_receipt = {'status': 'failed',
                          'error': 'loader-fabric-screen-admission assertion failed: fabric_screen_service_admitted',
                          'details': {'named_scenario': {'id': 'loader-fabric-screen-admission'},
                                      'failure': {'phase': 'assertion'}}}
        self.assertTrue(ledger._scenario_assertion_failed(loader_receipt, 'loader-fabric-screen-admission'))
        flake = {'status': 'failed', 'error': 'real fabric client did not reach Play',
                 'details': {'named_scenario': {'id': 'loader-fabric-screen-admission'}}}
        self.assertFalse(ledger._scenario_assertion_failed(flake, 'loader-fabric-screen-admission'))
        wrong = {'status': 'failed', 'error': 'scene-shot assertion failed: x', 'details': {'named_scenario': {'id': 'other'}}}
        self.assertFalse(ledger._scenario_assertion_failed(wrong, 'native-low-roof-house'))
        # No structured scenario id: free text never decides (no `check in text` fallback).
        self.assertFalse(ledger._scenario_assertion_failed(
            {'status': 'failed', 'error': 'native-low-roof-house assertion failed: x', 'details': {}}, 'native-low-roof-house'))

    def test_profile_all_and_blueprint_verdicts_are_red_on_their_own_assertion(self):
        failed = lambda name, **more: {'status': 'failed', 'error': f'autocraft-qa-settlements assertion failed: {name}', **more}
        # `all` of a profile that reports no verdict map: its own assertion failing is the red.
        self.assertTrue(ledger._failing_verdict(failed('raw_material_refused'), 'autocraft-qa', 'all'))
        # settlement-map-qa publishes a boolean verdict map: red is an explicit false, never a bare assertion text.
        mapped = lambda value: failed('viewer_drawn', details={'verdicts': {'all': value}})
        self.assertTrue(ledger._failing_verdict(mapped(False), 'settlement-map-qa', 'all'))
        self.assertFalse(ledger._failing_verdict(mapped(None), 'settlement-map-qa', 'all'))
        self.assertFalse(ledger._failing_verdict(failed('viewer_drawn'), 'settlement-map-qa', 'all'))
        # A connection assertion, a failure before the scenario body and a missing receipt error are not.
        self.assertFalse(ledger._failing_verdict(failed('in_play'), 'autocraft-qa', 'all'))
        self.assertFalse(ledger._failing_verdict(failed('x', details={'failure': {'phase': 'client_start'}}), 'autocraft-qa', 'all'))
        self.assertFalse(ledger._failing_verdict({'status': 'failed', 'error': 'timed out'}, 'autocraft-qa', 'all'))
        # Only explicit behavioural steps for this blueprint and level are red.
        for step in ('project', 'built', 'blocks', 'order_available', 'upgrade_available'):
            with self.subTest(step=step):
                self.assertTrue(ledger._failing_verdict(
                    failed(f'buildings_wall_L3_{step}'), 'buildings-qa', 'wall.L3_upgrade'))
        for step in ('funded', 'planner_fenced', 'fence_waiting', 'not_overtaken',
                     'level_exists', 'camera_space', 'unknown_future_step'):
            with self.subTest(step=step):
                self.assertFalse(ledger._failing_verdict(
                    failed(f'buildings_wall_L3_{step}'), 'buildings-qa', 'wall.L3_upgrade'))
        self.assertFalse(ledger._failing_verdict(failed('buildings_gate_L3_blocks'), 'buildings-qa', 'wall.L3_upgrade'))
        self.assertFalse(ledger._failing_verdict(failed('buildings_wall_L1_project'), 'buildings-qa', 'wall.L3_upgrade'))
        self.assertFalse(ledger._failing_verdict(failed('buildings_house_L1_built'), 'buildings-qa', 'wall.L1_built'))
        self.assertFalse(ledger._failing_verdict(failed('in_play'), 'buildings-qa', 'wall.L1_built'))
        self.assertTrue(ledger._failing_verdict(failed('buildings_house_L1_broken_cell_repaired'),
                                               'buildings-qa', 'house.L1_broken_cell_repaired'))
        self.assertFalse(ledger._failing_verdict(failed('buildings_house_L1_funded'),
                                                'buildings-qa', 'house.L1_broken_cell_repaired'))

    def test_failed_profile_run_is_red_for_the_verdict_its_scenario_report_records(self):
        # village-qa records its six verdicts, then fails the aggregate `all_behaviours_pass`: the failed
        # result.json has no `details.scenario`, the probe's scenario.json beside it does.
        failed = {'status': 'failed', 'error': 'village-qa-settlements assertion failed: all_behaviours_pass',
                  'details': {'artifact_dir': '/client/artifacts'}}
        report = {'id': 'village-qa-settlements', 'passed': False,
                  'settlement_acceptance': {'behaviour': {'door_walk': 'PASS', 'miner_deposit': 'FAIL',
                                                          'lumberjack': 'FAIL'}}}
        with tempfile.TemporaryDirectory() as directory:
            probe = Path(directory) / 'village-qa'
            probe.mkdir()
            (probe / 'scenario.json').write_text(json.dumps(report))
            regressions._attach_failed_scenario(failed, Path(directory))
        self.assertTrue(ledger._failing_verdict(failed, 'village-qa', 'miner_deposit'))
        self.assertTrue(ledger._failing_verdict(failed, 'village-qa', 'lumberjack'))
        # A verdict that passed, or that was never recorded, is not red for an unrelated failure.
        self.assertFalse(ledger._failing_verdict(failed, 'village-qa', 'door_walk'))
        self.assertFalse(ledger._failing_verdict(failed, 'village-qa', 'night_door'))
        # No report, or two of them, attaches nothing: the failure stays unrunnable.
        bare = {'status': 'failed', 'error': failed['error'], 'details': {}}
        with tempfile.TemporaryDirectory() as directory:
            for name in ('village-qa', 'other'):
                (Path(directory) / name).mkdir()
                (Path(directory) / name / 'scenario.json').write_text(json.dumps(report))
            regressions._attach_failed_scenario(bare, Path(directory))
        self.assertNotIn('scenario', bare['details'])
        self.assertFalse(ledger._failing_verdict(bare, 'village-qa', 'miner_deposit'))

    def test_scenario_ref_connection_assertions_and_argv_text_are_not_red(self):
        for name in ('in_play', 'restart_client_entered_play', 'reconnect_entered_play', 'client_joined'):
            receipt = {'status': 'failed', 'error': f'native-v2-craft-drag assertion failed: {name}',
                       'details': {'named_scenario': {'id': 'native-v2-craft-drag'}}}
            with self.subTest(name=name):
                self.assertFalse(ledger._scenario_assertion_failed(receipt, 'native-v2-craft-drag'))
        joined = {'status': 'failed', 'error': 'native-v2-craft-drag assertion failed: crafted',
                  'details': {'named_scenario': {'id': 'native-v2-craft-drag'}, 'failure': {'phase': 'join'}}}
        self.assertFalse(ledger._scenario_assertion_failed(joined, 'native-v2-craft-drag'))
        # The command line (argv) of a failed child is not receipt evidence.
        argv_only = {'status': 'failed', 'error': 'child exited 1: python -m tools.harness run scenario native-v2-craft-drag',
                     'details': {'named_scenario': {'id': 'native-v2-craft-drag'}}}
        self.assertFalse(ledger._scenario_assertion_failed(argv_only, 'native-v2-craft-drag'))

    def test_open_checks_are_run(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            ledger.write_rows(root, '''[[bug]]
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

    def test_failing_assertion_reports_only_matching_bug(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            ledger.write_rows(root, '''[[bug]]
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

    def test_red_repro_is_not_run_as_a_gate(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs').mkdir(parents=True)
            ledger.write_rows(root, '''[[bug]]
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


GIT_ENV = {'GIT_AUTHOR_NAME': 't', 'GIT_AUTHOR_EMAIL': 't@t', 'GIT_COMMITTER_NAME': 't', 'GIT_COMMITTER_EMAIL': 't@t', 'GIT_CONFIG_GLOBAL': '/dev/null'}
ROW = '''[[bug]]
id = "BUG"
title = "t"
area = "{area}"
repro = "source"
severity = "major"
status = "open"
source = "x"
regression = ["{ref}"]
{extra}'''


class TestOnlyFixTests(unittest.TestCase):
    """A fix whose every hunk is a corrected test oracle (docs/bugs/README.md, `test_only_fix`).

    The control runs twice: the pre-fix test hunks restored must go red on current code (A), and a
    committed defect-injection patch must turn the corrected test red (B), so the guard's direction is
    proven and not merely 'the old assertion was red'. Both reds must be assertion failures inside
    the changed test file, and the same test must flip in both."""

    OLD = 'class ExampleTests:\n    def test_guard(self):\n        assert value == "old"\n'
    NEW = 'class ExampleTests:\n    def test_guard(self):\n        assert value == "ok"\n'
    OTHER = 'class OtherTests:\n    def test_other(self):\n        assert True\n'
    OTHER_NEW = 'class OtherTests:\n    def test_other(self):\n        assert True  # corrected\n'
    REF = 'python:tools.harness.test_example:ExampleTests.test_guard'
    REF_OTHER = 'python:tools.harness.test_other:OtherTests.test_other'

    def git(self, root, *args):
        return subprocess.run(['git', '-C', str(root), *args], check=True, capture_output=True, text=True,
                              env={**os.environ, **GIT_ENV}).stdout.strip()

    def defect_diff(self, root, write):
        """Make `write`'s change, return its diff, then put the tree back."""
        write()
        diff = subprocess.run(['git', '-C', str(root), 'diff', '--no-color'], check=True, capture_output=True,
                              text=True, env={**os.environ, **GIT_ENV}).stdout
        self.git(root, 'checkout', '--', '.')
        return diff

    def scratch(self, production_hunk=False, covered=True, defect_text=None,
                defect_path='docs/bugs/test-only/BUG-defect.patch', write_patch=True,
                defect=None, later_edit=False, sha_override=None, two_files=False,
                refs=None, control_a_files=None):
        """Repo whose fix commit changes only the test oracle, plus a committed defect patch.

        `defect` is a callable(root) -> diff for a case's own mutation (a test-file hunk, docs, a
        build file); `later_edit` commits another edit to the test file after the fix, so the fix's
        test hunks no longer reverse onto the candidate (control A cannot run at head)."""
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name) / 'repo'
        for part in ('src', 'tools/harness', 'docs/bugs/test-only', 'docs/playable', 'docs/real-client-regression'):
            (root / part).mkdir(parents=True)
        self.git(root, 'init', '-q')
        (root / 'src/engine.rs').write_text('value = ok\n')
        # Tracked harness inputs and build files: a case may try to inject its defect there.
        (root / 'Cargo.toml').write_text('[package]\nname = "example"\n')
        (root / 'docs/playable/real-client-playable-loop.json').write_text('{"steps": []}\n')
        (root / 'docs/bugs/README.md').write_text('# bugs\n')
        (root / 'tools/harness/test_example.py').write_text(self.OLD)
        (root / 'tools/harness/test_other.py').write_text(self.OTHER)
        self.git(root, 'add', '-A')
        self.git(root, 'commit', '-qm', 'base')
        if covered:
            (root / 'tools/harness/test_example.py').write_text(self.NEW)
            if two_files:
                (root / 'tools/harness/test_other.py').write_text(self.OTHER_NEW)
        else:
            # The fix corrects a test in another file than the row's ref names.
            (root / 'tools/harness/test_other.py').write_text(self.OTHER_NEW)
        if production_hunk:
            (root / 'src/engine.rs').write_text('value = ok # touched\n')
        self.git(root, 'add', '-A')
        self.git(root, 'commit', '-qm', 'fix')
        self.fix = self.git(root, 'rev-parse', 'HEAD')
        if later_edit:
            (root / 'tools/harness/test_example.py').write_text(self.NEW + '\n# later\n')
            self.git(root, 'add', '-A')
            self.git(root, 'commit', '-qm', 'later edit of the same test file')
        # The defect the corrected oracle claims to catch: a production behavioural mutation.
        default = self.defect_diff(root, lambda: (root / 'src/engine.rs').write_text('value = defect\n'))
        text = defect(root) if defect is not None else default
        if write_patch:
            (root / defect_path).write_text(defect_text if defect_text is not None else text)
        defect_sha = hashlib.sha256((root / defect_path).read_bytes()).hexdigest() if write_patch else None
        self.git(root, 'add', '-A') if write_patch else None
        self.git(root, 'commit', '-qm', 'defect patch') if write_patch else None
        ledger.write_rows(root, f'''[[bug]]
id = "BUG"
title = "t"
area = "harness"
repro = "source"
severity = "major"
status = "open"
source = "x"
regression = {json.dumps(refs or [self.REF, self.REF_OTHER])}
fix_commit = "{self.fix}"
repro_check = "old oracle asserted value == old; vanilla 26.3 data says ok"
test_only_fix = true
control_b_patch = "{defect_path}"
{("control_a_files = " + json.dumps(control_a_files)) if control_a_files else ""}
{"" if sha_override else f'control_b_sha256 = "{defect_sha}"' if defect_sha else ""}
''')
        if sha_override:
            ledger.edit(root, 'BUG', [f'control_b_sha256 = "{sha_override}"'])
        self.git(root, 'add', '-A')
        self.git(root, 'commit', '-qm', 'ledger')
        self.artifacts = Path(directory.name) / 'artifacts'
        self.artifacts.mkdir()
        return root

    def runner(self, control_a_red=True, defect_red=True, a_assertion=None, b_assertion=None,
               b_red_refs=(REF,), b_state=ledger.RED, b_line=3):
        """The four steps in order: A (old oracle restored), B (defect injected), restore."""
        seen = []

        def entry(tree, red, assertion_file=None, line=3):
            if not red:
                return {'state': ledger.GREEN, 'detail': ''}
            return {'state': ledger.RED, 'detail': 'test assertion failed',
                    'assertion': {'file': assertion_file or str(tree / 'tools/harness/test_example.py'),
                                  'line': line, 'error': 'AssertionError'}}

        def run(_ctx, refs, tree, _client):
            test = (tree / 'tools/harness/test_example.py').read_text()
            other = (tree / 'tools/harness/test_other.py').read_text()
            defect = 'defect' in (tree / 'src/engine.rs').read_text()
            seen.append({'test': test, 'other': other, 'defect': defect, 'red': defect or '"old"' in test})
            old_oracle = '"old"' in test
            out = {}
            for ref in refs:
                if not defect:
                    out[ref] = entry(tree, old_oracle and control_a_red, a_assertion)
                elif b_state != ledger.RED:
                    out[ref] = {'state': b_state, 'detail': 'build error'}
                else:
                    red = defect_red and ref in b_red_refs
                    out[ref] = entry(tree, red, b_assertion, b_line)
            return out

        run.seen = seen
        return run

    def promote(self, root, runner, **options):
        return ledger.promote('BUG', root=root, ctx=profiles.ProfileContext(self.artifacts),
                              runner=runner, **options)

    def test_promotion_runs_the_old_oracle_control_and_the_defect_injection(self):
        root = self.scratch()
        runner = self.runner()
        receipt = self.promote(root, runner)
        self.assertEqual(receipt['control'], 'test_hunks')
        self.assertEqual(receipt['control_tree'], 'head')
        self.assertEqual(receipt['control_b']['patch'], 'docs/bugs/test-only/BUG-defect.patch')
        self.assertEqual(len(receipt['control_b']['sha256']), 64)
        self.assertEqual(receipt['control_b']['proving'], [self.REF])
        self.assertEqual(receipt['control_b']['negative_control'][self.REF]['state'], ledger.RED)
        self.assertEqual(receipt['negative_control'][self.REF]['state'], ledger.RED)
        self.assertEqual(receipt['restored'][self.REF]['state'], ledger.GREEN)
        # A gone: the old oracle, B: the injected defect, restore: the corrected test on current code.
        self.assertEqual([entry['red'] for entry in runner.seen], [True, True, False])
        # B ran on the CORRECTED test: the defect injection changes production code, never the test.
        self.assertEqual(runner.seen[1]['test'], self.NEW)
        self.assertTrue(runner.seen[1]['defect'])
        written = next(row for row in ledger.load_rows(root) if row['id'] == 'BUG')
        self.assertEqual(written['status'], 'fixed')

    def test_a_fix_commit_with_production_hunks_is_refused(self):
        root = self.scratch(production_hunk=True)
        runner = self.runner()
        with self.assertRaisesRegex(ledger.Refused, 'src/engine.rs is production code'):
            self.promote(root, runner)
        self.assertEqual(runner.seen, [])

    def test_patch_mode_and_the_flag_are_refused(self):
        root = self.scratch()
        ledger.edit(root, 'BUG', [f'fix_commit = "patch:{"0" * 64}"'])
        self.git(root, 'add', '-A')
        self.git(root, 'commit', '-qm', 'patch-mode fix')
        with self.assertRaisesRegex(ledger.Refused, 'test_only_fix is only for a committed fix_commit'):
            self.promote(root, self.runner(), fix_patch='docs/bugs/test-only/BUG-defect.patch')

    def test_an_old_oracle_that_is_green_on_current_code_is_refused(self):
        root = self.scratch()
        runner = self.runner(control_a_red=False)
        with self.assertRaisesRegex(ledger.Refused, 'did not go red at an assertion'):
            self.promote(root, runner)

    def test_a_defect_red_outside_the_reversed_hunk_is_refused(self):
        # ReviewD123Harness1 (BIOME): a red at another assert in the same file proves nothing about
        # the corrected assertion, so control B's line must lie inside a hunk the fix reversed.
        root = self.scratch()
        runner = self.runner(b_line=40)
        with self.assertRaisesRegex(ledger.Refused, 'inside a hunk the fix reversed'):
            self.promote(root, runner)
        self.assertEqual([entry['red'] for entry in runner.seen], [True, True],
                         'control A ran, then B was judged out of range')

    def test_a_defect_control_that_stays_green_is_refused(self):
        root = self.scratch()
        runner = self.runner(defect_red=False)
        with self.assertRaisesRegex(ledger.Refused, 'defect-injection control.*did not go red at an assertion'):
            self.promote(root, runner)
        # Control A still ran and went red at its assertion: the refusal is about B, not about A.
        self.assertEqual(len(runner.seen), 2)
        self.assertTrue(runner.seen[0]['red'], 'control A ran with the old oracle')
        self.assertTrue(runner.seen[1]['defect'], 'control B ran with the defect injected')

    def test_a_missing_defect_patch_is_refused(self):
        root = self.scratch(write_patch=False)
        runner = self.runner()
        with self.assertRaisesRegex(ledger.Refused, 'is not committed at HEAD'):
            self.promote(root, runner)
        self.assertEqual(runner.seen, [])

    def test_an_uncommitted_defect_patch_is_refused(self):
        root = self.scratch()
        (root / 'docs/bugs/test-only/BUG-defect.patch').write_text('uncommitted change\n')
        runner = self.runner()
        with self.assertRaisesRegex(ledger.Refused, 'worktree copy .* differs from HEAD'):
            self.promote(root, runner)
        self.assertEqual(runner.seen, [])

    def test_a_recorded_hash_mismatch_is_refused(self):
        root = self.scratch(sha_override='0' * 64)
        runner = self.runner()
        with self.assertRaisesRegex(ledger.Refused, 'control_b_sha256 is'):
            self.promote(root, runner)
        self.assertEqual(runner.seen, [])

    def test_a_defect_patch_that_does_not_apply_is_refused(self):
        text = ('diff --git a/src/engine.rs b/src/engine.rs\n--- a/src/engine.rs\n+++ b/src/engine.rs\n'
                '@@ -1,1 +1,1 @@\n-value = not this tree\n+value = defect\n')
        root = self.scratch(defect_text=text)
        runner = self.runner()
        with self.assertRaisesRegex(ledger.Refused, 'does not apply on the tree being promoted'):
            self.promote(root, runner)

    def test_refs_that_do_not_run_the_changed_test_file_are_refused(self):
        root = self.scratch(covered=False)
        # The fix corrected tools/harness/test_other.py; the row names only the other file's test.
        ledger.edit(root, 'BUG', [f'regression = ["{self.REF}"]'])
        self.git(root, 'add', '-A')
        self.git(root, 'commit', '-qm', 'row names one file')
        runner = self.runner()
        with self.assertRaisesRegex(ledger.Refused, 'no ref whose test is defined in tools/harness/test_other.py'):
            self.promote(root, runner)
        self.assertEqual(runner.seen, [])

    def test_a_defect_that_edits_the_test_file_is_refused(self):
        def defect(root):
            return self.defect_diff(root, lambda: (root / 'tools/harness/test_example.py').write_text(
                self.NEW + '\n# injected\n'))
        root = self.scratch(defect=defect)
        runner = self.runner()
        with self.assertRaisesRegex(ledger.Refused, 'changes test files'):
            self.promote(root, runner)
        self.assertEqual(runner.seen, [])

    def test_a_defect_under_docs_bugs_is_left_out_and_refused(self):
        def defect(root):
            return self.defect_diff(root, lambda: (root / 'docs/bugs/README.md').write_text('# injected\n'))
        root = self.scratch(defect=defect)
        runner = self.runner()
        with self.assertRaisesRegex(ledger.Refused, 'leaves part of itself out'):
            self.promote(root, runner)

    def test_a_defect_that_edits_a_harness_input_under_docs_is_refused(self):
        def defect(root):
            return self.defect_diff(root, lambda: (root / 'docs/playable/real-client-playable-loop.json').write_text('{}\n'))
        root = self.scratch(defect=defect)
        runner = self.runner()
        with self.assertRaisesRegex(ledger.Refused, 'harness input under docs/'):
            self.promote(root, runner)

    def test_a_defect_that_edits_build_configuration_is_refused(self):
        def defect(root):
            return self.defect_diff(root, lambda: (root / 'Cargo.toml').write_text('[package]\nname = "defect"\n'))
        root = self.scratch(defect=defect)
        runner = self.runner()
        with self.assertRaisesRegex(ledger.Refused, 'changes build configuration'):
            self.promote(root, runner)

    def test_a_red_panic_outside_the_changed_test_file_is_not_control_a(self):
        root = self.scratch()
        runner = self.runner(a_assertion=lambda tree: str(tree / 'src/engine.rs'))
        with self.assertRaisesRegex(ledger.Refused, 'did not go red at an assertion.*not control A'):
            self.promote(root, runner)

    def test_a_defect_red_on_an_unrelated_ref_is_not_proving(self):
        root = self.scratch()
        runner = self.runner(b_red_refs={self.REF_OTHER})
        with self.assertRaisesRegex(ledger.Refused, 'is not defined in .*: a bare name from another file'):
            self.promote(root, runner)

    def test_a_defect_red_on_no_changed_test_does_not_overlap_control_a(self):
        root = self.scratch()
        runner = self.runner(b_red_refs=set())
        with self.assertRaisesRegex(ledger.Refused, 'did not go red at an assertion'):
            self.promote(root, runner)

    def test_a_defect_that_breaks_the_build_is_unrunnable_not_red(self):
        root = self.scratch()
        runner = self.runner(b_state=ledger.UNRUNNABLE)
        with self.assertRaisesRegex(ledger.Refused, 'did not go red at an assertion'):
            self.promote(root, runner)

    def test_a_fix_that_cannot_reverse_at_head_refuses_a_fix_commit_control(self):
        root = self.scratch(later_edit=True)
        runner = self.runner()
        with self.assertRaisesRegex(ledger.Refused, 'control A cannot run against current code'):
            self.promote(root, runner)
        self.assertEqual(runner.seen, [])

    def test_control_a_files_narrows_the_reversal_to_the_row_s_own_file(self):
        root = self.scratch(two_files=True, refs=[self.REF],
                            control_a_files=['tools/harness/test_example.py'])
        runner = self.runner()
        receipt = self.promote(root, runner)
        self.assertEqual(receipt['reversed_files'], ['tools/harness/test_example.py'])
        # Control A restored only the row's file: the other corrected oracle stayed corrected.
        self.assertIn('"old"', runner.seen[0]['test'])
        self.assertEqual(runner.seen[0]['other'], self.OTHER_NEW)
        self.assertEqual(receipt['control_b']['proving'], [self.REF])

    def test_control_a_files_naming_an_unchanged_file_is_refused(self):
        root = self.scratch(control_a_files=['tools/harness/test_absent.py'])
        with self.assertRaisesRegex(ledger.Refused, 'which the fix commit does not change'):
            self.promote(root, self.runner())

    def test_a_production_file_in_the_fix_commit_is_refused_even_with_control_a_files(self):
        # The narrowing chooses what control A reverses; it must never hide a production file from
        # the "a test_only_fix commit changes only test files" refusal (ReviewD123Harness2 item 1).
        root = self.scratch(production_hunk=True, control_a_files=['tools/harness/test_example.py'])
        runner = self.runner()
        with self.assertRaisesRegex(ledger.Refused, 'src/engine.rs is production code'):
            self.promote(root, runner)
        self.assertEqual(runner.seen, [])

    def test_the_row_flag_needs_its_evidence(self):
        base = '''[[bug]]
id = "B"
title = "bug"
area = "harness"
repro = "source"
severity = "major"
status = "open"
source = "report"
regression = []
repro_check = "old oracle wrote the wrong value"
'''
        cases = [
            base + 'test_only_fix = "yes"\n',
            base.replace('repro_check = "old oracle wrote the wrong value"\n', '') + 'test_only_fix = true\n',
            base + 'test_only_fix = true\ncontrol_b_patch = "/tmp/absolute.patch"\n',
            base + 'test_only_fix = true\n',
            base + 'control_b_patch = "docs/bugs/test-only/defect.patch"\n',
            base + 'test_only_fix = true\ncontrol_b_patch = "docs/bugs/test-only/defect.patch"\n',
            base + 'test_only_fix = true\ncontrol_b_patch = "docs/bugs/test-only/defect.patch"\n'
                   'control_b_sha256 = "not-a-hash"\n',
            base + 'test_only_fix = true\ncontrol_b_patch = "docs/bugs/test-only/defect.patch"\n'
                   'control_b_sha256 = "' + '0' * 64 + '"\n',
            base + 'test_only_fix = true\ncontrol_b_patch = "docs/bugs/test-only/defect.patch"\n'
                   'control_b_sha256 = "' + '0' * 64 + '"\nfix_commit = "client:' + '0' * 40 + '"\n',
            base + 'control_a_files = ["crates/mc-core/src/plants_tests.rs"]\n',
            base + 'test_only_fix = true\ncontrol_a_files = []\n',
            base + 'test_only_fix = true\ncontrol_a_files = ["/etc/passwd.rs"]\n',
            base + 'test_only_fix = true\ncontrol_a_files = ["docs/bugs/x.rs"]\n',
        ]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'docs/bugs/rows').mkdir(parents=True)
            for text in cases:
                with self.subTest(text=text):
                    shutil.rmtree(root / 'docs/bugs/rows')
                    (root / 'docs/bugs/rows').mkdir(parents=True)
                    (root / 'docs/bugs/rows/B.toml').write_text(text)
                    with patch.object(regressions, 'REPO_ROOT', root), \
                         patch.object(regressions, 'BASE_FIXED', frozenset()):
                        with self.assertRaises(ValueError):
                            regressions.run(profiles.ProfileContext(root))

    def promotion_root(self, test_only, control='test_hunks', control_b_sha=None, fix='a' * 40):
        """A repo with one row and its promotion receipt, for `_promotion_errors` alone."""
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        (root / 'docs/bugs/rows').mkdir(parents=True)
        (root / 'docs/bugs/promotions').mkdir(parents=True)
        fields = ['test_only_fix = true'] if test_only else []
        if control_b_sha:
            fields += [f'control_b_sha256 = "{control_b_sha}"',
                       'control_b_patch = "docs/bugs/test-only/X-defect.patch"']
        (root / 'docs/bugs/rows/X.toml').write_text(f'''[[bug]]
id = "X"
title = "t"
area = "harness"
repro = "source"
severity = "minor"
status = "fixed"
source = "x"
regression = ["python:tools.harness.test_example:ExampleTests.test_guard"]
fix_commit = "{fix}"
{chr(10).join(fields)}
promotion = "docs/bugs/promotions/X.json"
''')
        (root / 'docs/bugs/promotions/X.json').write_text(json.dumps({
            'id': 'X', 'verdict': 'promoted', 'fix_commit': fix, 'control': control,
            'control_b': {'sha256': control_b_sha} if control_b_sha else None}))
        return root

    def test_a_narrowed_row_needs_its_receipt_to_reverse_the_named_files(self):
        root = self.promotion_root(test_only=True, control='test_hunks', control_b_sha='0' * 64)
        row_path = root / 'docs/bugs/rows/X.toml'
        row_path.write_text(row_path.read_text() + 'control_a_files = ["tools/harness/test_example.py"]\n')
        receipt_path = root / 'docs/bugs/promotions/X.json'
        receipt = json.loads(receipt_path.read_text())
        receipt['reversed_files'] = ['tools/harness/test_other.py']
        receipt_path.write_text(json.dumps(receipt))
        with patch.object(regressions, 'REPO_ROOT', root):
            row = next(row for row in ledger.load_rows(root) if row['id'] == 'X')
            errors = regressions._promotion_errors('X', row)
        self.assertTrue(any('control_a_files' in error for error in errors), errors)
        receipt['reversed_files'] = ['tools/harness/test_example.py']
        receipt_path.write_text(json.dumps(receipt))
        with patch.object(regressions, 'REPO_ROOT', root):
            row = next(row for row in ledger.load_rows(root) if row['id'] == 'X')
            self.assertEqual(regressions._promotion_errors('X', row), [])

    def test_a_receipt_may_not_claim_a_test_only_control_without_the_flag(self):
        root = self.promotion_root(test_only=False, control='test_hunks')
        with patch.object(regressions, 'REPO_ROOT', root):
            row = next(row for row in ledger.load_rows(root) if row['id'] == 'X')
            errors = regressions._promotion_errors('X', row)
        self.assertTrue(any('without test_only_fix' in error for error in errors), errors)

    def test_a_test_only_receipt_needs_its_control_and_defect_hash(self):
        root = self.promotion_root(test_only=True, control='reversed_fix_hunks')
        with patch.object(regressions, 'REPO_ROOT', root):
            row = next(row for row in ledger.load_rows(root) if row['id'] == 'X')
            self.assertTrue(any('must record control = test_hunks' in error
                                for error in regressions._promotion_errors('X', row)))
        root = self.promotion_root(test_only=True, control='test_hunks', control_b_sha='0' * 64)
        with patch.object(regressions, 'REPO_ROOT', root):
            row = next(row for row in ledger.load_rows(root) if row['id'] == 'X')
            self.assertEqual(regressions._promotion_errors('X', row), [])


class StalePatchTests(unittest.TestCase):
    """A patch-mode receipt whose patch no longer reverses is a report in L2, strict at promote.

    ReviewD123Harness3 N1 / Main X1/B1: an ordinary edit of a historical fix must never turn L2 red, so
    `_promotion_errors` is silent about a missing or stale patch (run() reports the backlog) but fatal
    when the committed patch's non-test hunks differ from the row's digest. `ledger promote`/`--dry-run`
    refuses when the patch does not reverse."""

    def root(self, with_patch=True):
        import tempfile, hashlib
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        (root / 'docs/bugs/rows').mkdir(parents=True)
        (root / 'docs/bugs/fix-patches').mkdir(parents=True)
        text = 'diff --git a/nope.rs b/nope.rs\n--- a/nope.rs\n+++ b/nope.rs\n@@ -1 +1 @@\n-a\n+b\n'
        digest = hashlib.sha256(text.encode()).hexdigest()
        (root / 'docs/bugs/rows/X.toml').write_text(f'''[[bug]]
id = "X"
title = "t"
area = "harness"
repro = "source"
severity = "minor"
status = "fixed"
source = "x"
regression = []
fix_commit = "patch:{digest}"
promotion = "docs/bugs/promotions/X.json"
''')
        (root / 'docs/bugs/promotions').mkdir(parents=True)
        (root / 'docs/bugs/promotions/X.json').write_text(
            '{"id": "X", "verdict": "promoted", "fix_commit": "patch:%s", "control": "reversed_fix_hunks"}'
            % digest)
        if with_patch:
            (root / 'docs/bugs/fix-patches/X.patch').write_text(text)
        return root

    def test_a_missing_patch_file_is_reported_not_fatal(self):
        # Main B1: seven historical rows carry only promotions/<ID>.json; they must not turn L2 red.
        root = self.root(with_patch=False)
        with patch.object(regressions, 'REPO_ROOT', root):
            row = next(row for row in ledger.load_rows(root) if row['id'] == 'X')
            errors = regressions._promotion_errors('X', row)
            stale = regressions._stale_rows([row])
        self.assertEqual(errors, [])
        self.assertEqual(stale, [('X', 'no committed patch')])

    def test_the_digest_is_the_non_test_hunks_like_promote(self):
        # X1: a curated --fix-patch may carry a test hunk; promote hashes only its non-test hunks, so
        # regressions must compute the same digest or it fails rows promote accepted.
        import hashlib
        root = self.root()
        rows = root / 'docs/bugs/rows/X.toml'
        text = ('diff --git a/src/engine.rs b/src/engine.rs\n--- a/src/engine.rs\n+++ b/src/engine.rs\n'
                '@@ -1 +1 @@\n-a\n+b\n'
                'diff --git a/tests/new_case.rs b/tests/new_case.rs\n--- a/tests/new_case.rs\n+++ b/tests/new_case.rs\n'
                '@@ -1 +1 @@\n-a\n+b\n')
        (root / 'docs/bugs/fix-patches/X.patch').write_text(text)
        hunks = ('diff --git a/src/engine.rs b/src/engine.rs\n--- a/src/engine.rs\n+++ b/src/engine.rs\n'
                 '@@ -1 +1 @@\n-a\n+b\n')
        digest = hashlib.sha256(hunks.encode()).hexdigest()
        old = rows.read_text().split('fix_commit = "')[1].split('"')[0]
        body = rows.read_text().replace(old, 'patch:' + digest)
        rows.write_text(body)
        receipt = json.loads((root / 'docs/bugs/promotions/X.json').read_text())
        receipt['fix_commit'] = 'patch:' + digest
        (root / 'docs/bugs/promotions/X.json').write_text(json.dumps(receipt))
        with patch.object(regressions, 'REPO_ROOT', root):
            row = next(row for row in ledger.load_rows(root) if row['id'] == 'X')
            errors = regressions._promotion_errors('X', row)
        self.assertEqual(errors, [], 'a test hunk in the file must not change the digest')

    def test_a_stale_patch_is_a_report_not_an_error(self):
        root = self.root()
        with patch.object(regressions, 'REPO_ROOT', root):
            row = next(row for row in ledger.load_rows(root) if row['id'] == 'X')
            errors = regressions._promotion_errors('X', row)
            stale = regressions._stale_rows([row])
        self.assertEqual(errors, [], 'staleness must not fail the L2 gate')
        self.assertEqual(stale, [('X', 'does not reverse')], 'it is still reported as the backlog')

    def test_the_patch_matching_the_digest_wins_over_an_older_one_of_the_other_location(self):
        # Both docs/bugs/fix-patches/ and docs/bugs/promotions/ may hold a patch for one row (a re-promotion
        # on a real pre-image left the first, invented-side file behind). The row names one digest: the file
        # that hashes to it is the row's patch, so the older file must not fail L2.
        root = self.root()
        (root / 'nope.rs').write_text('b\n')  # the post-change text the hunk classifier reads
        (root / 'docs/bugs/promotions/X.patch').write_text((root / 'docs/bugs/fix-patches/X.patch').read_text())
        (root / 'docs/bugs/fix-patches/X.patch').write_text(
            'diff --git a/nope.rs b/nope.rs\n--- a/nope.rs\n+++ b/nope.rs\n@@ -1 +1 @@\n-a\n+invented\n')
        with patch.object(regressions, 'REPO_ROOT', root):
            row = next(row for row in ledger.load_rows(root) if row['id'] == 'X')
            errors = regressions._promotion_errors('X', row)
        self.assertEqual(errors, [])
        (root / 'docs/bugs/promotions/X.patch').write_text(
            'diff --git a/nope.rs b/nope.rs\n--- a/nope.rs\n+++ b/nope.rs\n@@ -1 +1 @@\n-a\n+other\n')
        with patch.object(regressions, 'REPO_ROOT', root):
            errors = regressions._promotion_errors('X', row)
        self.assertEqual(len(errors), 1, 'with no file matching the digest the mismatch is still fatal')

class FixturePanicTests(unittest.TestCase):
    """A control red must be an assertion, not a fixture that panicked before the test body ran.

    ReviewD123Harness (Mechanics rows TOTEM, DEATH-PROTECTION-COMPONENT, SPECIAL-001 were promoted on
    a `Registry("invalid map color registry")` panic at tick_tests.rs:91). The rule has three classes:
    (a) a confirmed fixture/setup panic refuses; (b) an assert macro, including one in a shared helper
    inside a `*_tests.rs` file, is a red; (c) any other panic must be named by the row as its proving
    assertion file:line, or it refuses."""

    def tree(self):
        import tempfile
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        (root / 'crates/mc-core/src').mkdir(parents=True)
        (root / 'crates/mc-core/src/tick_tests.rs').write_text(
            'fn registry() -> Registry {\n'
            '    load().expect("registry loads")\n'
            '}\n'
            '\n'
            '#[test]\n'
            'fn tick() {\n'
            '    let registry = registry();\n'
            '    assert_eq!(1, 1);\n'
            '}\n')
        (root / 'crates/mc-core/src/core_journal_backpressure_tests.rs').write_text(
            '// helper asserted by tests at 295/319/345\n' * 20 +
            'fn pressure_is_within_budget(state: &State) {\n'
            '    assert!(state.queued <= 32, "backpressure");\n'
            '}\n')
        return root

    def test_a_fixture_panic_is_classified_fixture(self):
        root = self.tree()
        self.assertEqual(ledger._panic_class(root, ('crates/mc-core/src/tick_tests.rs', 2),
                                             'registry loads'), 'fixture')
        self.assertEqual(ledger._panic_class(root, ('crates/mc-core/src/tick_tests.rs', 2),
                                             'Registry("invalid map color registry")'), 'fixture')
        self.assertEqual(ledger._panic_class(root, ('crates/mc-core/src/tick_tests.rs', 8),
                                             'called `Option::unwrap()` on a `None` value'), 'fixture')

    def test_an_assert_macro_outside_test_paths_is_not_an_assertion(self):
        # ReviewD123Harness3 N3b: production code and test-support modules outside _TEST_PATH panic
        # through assert! too; that is class (c), not evidence about the oracle.
        root = self.tree()
        (root / 'crates/mc-core/src/test_support.rs').write_text(
            'fn fixture() {\n    assert!(true);\n}\n')
        self.assertEqual(ledger._panic_class(root, ('crates/mc-core/src/test_support.rs', 2),
                                             'assertion failed'), 'unknown')

    def test_an_inline_test_module_in_a_production_file_is_an_assertion(self):
        # Review wave 2 minor 6: SETTLEMENT-LAYOUT-PROJECT-SHOWS-UNKNOWN-BLUEPRINT's control red is
        # an `assert_ne!` inside `#[cfg(test)] mod tests` of settlements/ui_text.rs, a production
        # path. The reversed patch removes three arms above it, so the control tree's line (1021)
        # is not HEAD's (1024): the class is read from the tree the panic came from.
        root = self.tree()
        file = root / 'crates/mc-core/src/ui_text.rs'
        file.write_text(
            'pub(crate) fn blueprint(id: &str) -> &\'static str {\n'
            '    id\n'
            '}\n'
            '\n'
            '#[cfg(test)]\n'
            'mod tests {\n'
            '    #[test]\n'
            '    fn every_layout_project_has_a_name() {\n'
            '        assert_ne!(super::blueprint("perimeter"), "Unknown blueprint");\n'
            '    }\n'
            '}\n')
        self.assertEqual(
            ledger._panic_class(root, ('crates/mc-core/src/ui_text.rs', 9),
                                'assertion `left != right` failed'), 'assertion')
        self.assertEqual(
            ledger._panic_class(root, ('crates/mc-core/src/ui_text.rs', 2),
                                'assertion `left != right` failed'), 'unknown')

    def test_a_cargo_red_without_a_panic_line_needs_the_row_to_name_it(self):
        # N3a: a `-> Result<()>` test fails through `?` with no panic line; that is not an assertion.
        entry = {'state': ledger.RED, 'detail': 'FAIL'}
        refused = ledger._classify_control('B', {'cargo:mc-core:x::t': entry}, {}, Path('/nonexistent'))
        self.assertEqual(refused['cargo:mc-core:x::t']['state'], ledger.UNPROVEN_PANIC)
        named = {'proving_assertions': {'cargo:mc-core:x::t': 'crates/mc-core/src/x_tests.rs:1'}}
        self.assertEqual(ledger._classify_control('B', {'cargo:mc-core:x::t': entry}, named,
                                                  Path('/nonexistent'))['cargo:mc-core:x::t']['state'], ledger.RED)

    def test_an_assert_macro_is_a_red_even_in_a_shared_helper(self):
        root = self.tree()
        self.assertEqual(ledger._panic_class(root, ('crates/mc-core/src/tick_tests.rs', 8),
                                             'assertion failed'), 'assertion')
        self.assertEqual(
            ledger._panic_class(root, ('crates/mc-core/src/core_journal_backpressure_tests.rs', 22),
                                'assertion failed: backpressure'), 'assertion')

    def test_an_assert_macro_with_a_custom_message_is_an_assertion(self):
        # Wave-2 review N2: `assert!(cond, "msg")` panics with exactly "msg"; the source line decides.
        root = self.tree()
        self.assertEqual(ledger._panic_class(root, ('crates/mc-core/src/tick_tests.rs', 8),
                                             'the tick keeps its registry'), 'assertion')
        self.assertEqual(
            ledger._panic_class(root, ('crates/mc-core/src/core_journal_backpressure_tests.rs', 22),
                                'queued batches must stay within the budget'), 'assertion')

    def test_the_unwrap_line_rule_pins_both_directions(self):
        # Wave-2 review N6: an `.expect`/`.unwrap` line is not evidence unless its message is an
        # assertion, while an assert macro wrapping an unwrap is the assertion itself.
        root = self.tree()
        (root / 'crates/mc-core/src/native_chunk_tests.rs').write_text(
            'fn present(state: &State) {\n'
            '    let chunk = state.chunk(0, 0).expect("chunk present");\n'
            '}\n'
            '\n'
            '#[test]\n'
            'fn counts() {\n'
            '    assert_eq!(state().len().unwrap(), 1);\n'
            '}\n')
        self.assertEqual(ledger._panic_class(root, ('crates/mc-core/src/native_chunk_tests.rs', 2),
                                             'chunk present'), 'unknown')
        self.assertEqual(
            ledger._panic_class(root, ('crates/mc-core/src/native_chunk_tests.rs', 7),
                                'assertion `left == right` failed\n  left: 2\n right: 1'),
            'assertion')

    def test_an_unknown_helper_panic_needs_the_row_to_name_it(self):
        root = self.tree()
        location = ('crates/mc-core/src/core_journal_backpressure_tests.rs', 21)
        self.assertEqual(ledger._panic_class(root, location, 'some helper failure'), 'unknown')
        entry = {'state': ledger.RED, 'detail': 'FAIL',
                 'assertion': {'file': location[0], 'line': location[1], 'error': 'panic', 'class': 'unknown'}}
        unproven = ledger._classify_control('B', {'r': entry}, {}, root)['r']
        self.assertEqual(unproven['state'], ledger.UNPROVEN_PANIC)
        named = {'proving_assertions': {'r': f'{location[0]}:{location[1]}'}}
        self.assertEqual(ledger._classify_control('B', {'r': entry}, named, root)['r']['state'], ledger.RED)

    def test_a_fixture_panic_entry_refuses_at_the_promote_boundary(self):
        root = self.tree()
        entry = {'state': ledger.RED, 'detail': 'FAIL',
                 'assertion': {'file': 'crates/mc-core/src/tick_tests.rs', 'line': 2, 'error': 'panic',
                               'class': 'fixture'}}
        self.assertEqual(ledger._classify_control('B', {'r': entry}, {}, root)['r']['state'],
                         ledger.FIXTURE_FAILURE)

    def test_an_assert_with_a_custom_message_is_an_assertion(self):
        # REVIEW-wave2-10 minor 4: `assert!(cond, "custom text")` panics with its own message, so
        # the message alone must not demote the red to unproven-panic.
        root = self.tree()
        (root / 'crates/mc-core/src/tick_tests.rs').write_text(
            'fn tick() {\n'
            '    assert!(false, "a normal connection end is not an abnormal close");\n'
            '}\n')
        self.assertEqual(
            ledger._panic_class(root, ('crates/mc-core/src/tick_tests.rs', 2),
                                'a normal connection end is not an abnormal close'),
            'assertion')

    def test_a_setup_panic_on_its_own_line_stays_unknown(self):
        # REVIEW-wave2-11 minor-6: only assert*! may be line-start evidence. A `panic!("…")` in a
        # let-else body is a fixture/setup death, so it still needs the row to name it.
        root = self.tree()
        (root / 'crates/mc-core/src/tick_tests.rs').write_text(
            'fn tick() {\n'
            '    panic!("world fixture missing");\n'
            '}\n')
        self.assertEqual(
            ledger._panic_class(root, ('crates/mc-core/src/tick_tests.rs', 2),
                                'world fixture missing'),
            'unknown')

class TestOnlyEvidenceTests(unittest.TestCase):
    """The evidence a test-only promotion parses out of a run."""

    def test_cargo_panics_are_keyed_by_test_and_file(self):
        text = ('        FAIL [   0.123s] mc-edge host_telemetry_tests::bad\n'
                "thread 'host_telemetry_tests::bad' panicked at crates/mc-edge/src/host_telemetry_tests.rs:120:5:\n"
                'assertion `left == right` failed\n')
        # Keyed by (crate, name): a bare name collides across modules (ReviewD123Harness1 9a).
        self.assertEqual(ledger._panic_locations(text),
                         {('mc-edge', 'bad'): ('crates/mc-edge/src/host_telemetry_tests.rs', 120,
                                               'assertion `left == right` failed')})
        two_modules = ('        FAIL [   0.123s] mc-edge tests::shared\n'
                       "thread 'tests::shared' panicked at crates/mc-edge/src/tests.rs:10:1:\n"
                       'assertion failed\n'
                       '        FAIL [   0.123s] mc-core other::shared\n'
                       "thread 'other::shared' panicked at crates/mc-core/src/other.rs:20:1:\n"
                       'assertion failed\n')
        self.assertEqual(ledger._panic_locations(two_modules), {
            ('mc-edge', 'shared'): ('crates/mc-edge/src/tests.rs', 10, 'assertion failed'),
            ('mc-core', 'shared'): ('crates/mc-core/src/other.rs', 20, 'assertion failed')})

    def test_changed_lines_exclude_context_and_operators_span_calls(self):
        # ReviewD123Harness3 N8: only lines the reversal actually changed count (11 here), while an
        # operator starting at the red line may span a changed line further down.
        diff = ('diff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n'
                '@@ -10,3 +10,3 @@ fn f() {\n ctx\n-old\n+new\n ctx\n')
        self.assertEqual(ledger._changed_old_lines(diff), {'x.rs': {11}})
        self.assertEqual(ledger._changed_old_lines(''), {})
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'x.rs').write_text('fn f() {\n    assert!(matches!(\n        a, 1 | 2\n    ));\n}\n')
            self.assertEqual(ledger._operator_span(root, 'x.rs', 2), (2, 4))

    def test_a_setup_unwrap_panic_is_not_an_assertion(self):
        # `#[track_caller]` puts `.unwrap()` at the test's own line; the class is what refuses it.
        text = ('        FAIL [   0.123s] mc-edge tests::setup\n'
                "thread 'tests::setup' panicked at crates/mc-edge/src/tests.rs:44:9:\n"
                'called `Option::unwrap()` on a `None` value\n')
        locations = ledger._panic_locations(text)
        self.assertEqual(list(locations), [('mc-edge', 'setup')])
        self.assertEqual(ledger._panic_class(Path('/nonexistent'), locations[('mc-edge', 'setup')][:2],
                                             locations[('mc-edge', 'setup')][2]), 'fixture')

    def test_a_python_assertion_ignores_a_mentions_elsewhere(self):
        # Only the failing block's own exception counts (ReviewD123Harness1 9c).
        text = ('FAIL: test_guard (tools.harness.test_example.ExampleTests.test_guard)\n'
                'Traceback (most recent call last):\n'
                '  File "/repo/tools/harness/test_example.py", line 3, in test_guard\n'
                'RuntimeError: AssertionError mentioned in a message\n')
        self.assertIsNone(ledger._python_assertion(text))
        text = ('Traceback (most recent call last):\n'
                '  File "/repo/tools/harness/test_example.py", line 7, in test_a\n'
                'KeyError: 1\n'
                'Traceback (most recent call last):\n'
                '  File "/repo/tools/harness/test_example.py", line 12, in test_b\n'
                'AssertionError: 1 != 2\n')
        self.assertEqual(ledger._python_assertion(text),
                         {'file': '/repo/tools/harness/test_example.py', 'line': 12, 'error': 'AssertionError'})

    def test_a_python_assertion_uses_the_innermost_frame(self):
        text = ('FAIL: test_guard (tools.harness.test_example.ExampleTests.test_guard)\n'
                'Traceback (most recent call last):\n'
                '  File "/repo/tools/harness/test_example.py", line 3, in test_guard\n'
                'AssertionError: 1 != 2\n')
        self.assertEqual(ledger._python_assertion(text),
                         {'file': '/repo/tools/harness/test_example.py', 'line': 3, 'error': 'AssertionError'})
        self.assertIsNone(ledger._python_assertion('RuntimeError: boom\n'))

    def test_a_ref_maps_to_its_own_changed_file_through_mod_declarations(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for path, text in {
                'crates/mc-core/src/lib.rs': '#[cfg(test)]\nmod farming_tests;\n#[cfg(test)]\nmod fluids_tests;\n',
                'crates/mc-core/src/farming_tests.rs': '#[path = "plants_tests.rs"]\nmod plants_tests;\n',
                'crates/mc-core/src/plants_tests.rs': 'fn test() {}\n',
                'crates/mc-core/src/fluids_tests.rs': '#[path = "fluid_parity_tests.rs"]\nmod parity;\n',
                'crates/mc-core/src/fluid_parity_tests.rs': 'fn test() {}\n',
                'crates/mc-edge/src/lib.rs': 'mod host_telemetry;\n',
                'crates/mc-edge/src/host_telemetry.rs':
                    '#[cfg(test)]\n#[path = "host_telemetry_tests.rs"]\nmod host_telemetry_tests;\n',
                'crates/mc-edge/src/host_telemetry_tests.rs': 'fn test() {}\n',
            }.items():
                (root / path).parent.mkdir(parents=True, exist_ok=True)
                (root / path).write_text(text)
            plants = ['crates/mc-core/src/plants_tests.rs']
            self.assertEqual(ledger._ref_test_file(
                root, 'cargo:mc-core:farming_tests::plants_tests::native_random_sampler_grows_chorus_flowers',
                plants), 'crates/mc-core/src/plants_tests.rs')
            self.assertIsNone(ledger._ref_test_file(root, 'cargo:mc-core:fluids_tests::parity::x', plants))
            self.assertEqual(ledger._ref_test_file(
                root, 'cargo:mc-core:fluids_tests::parity::physics_water_selects_nearest_descent',
                ['crates/mc-core/src/fluid_parity_tests.rs']), 'crates/mc-core/src/fluid_parity_tests.rs')
            self.assertEqual(ledger._ref_test_file(
                root, 'cargo:mc-edge:host_telemetry::host_telemetry_tests::'
                      'sampler_reads_the_throttle_counters_from_the_cgroup_fixture',
                ['crates/mc-edge/src/host_telemetry_tests.rs']), 'crates/mc-edge/src/host_telemetry_tests.rs')
            self.assertIsNone(ledger._ref_test_file(
                root, 'cargo:mc-core:falling::tests::x', plants))
            self.assertEqual(ledger._ref_test_file(
                root, 'python:tools.other.test_x:ExampleTests.test_guard', ['tools/other/test_x.py']),
                'tools/other/test_x.py')
            self.assertIsNone(ledger._ref_test_file(
                root, 'python:tools.harness.test_other:ExampleTests.test_guard', plants))


class ClientRefTests(unittest.TestCase):
    """A client fix's test runs in the client checkout, so a client row can register a real ref."""

    def test_a_client_ref_parses_and_names_crate_and_test(self):
        ref = 'client:krasnokamen-client:session::actions::fast_click_tests::press_released_before_the_next_tick_still_starts_the_attack'
        self.assertEqual(regressions._parse_ref(ref),
                         ('client', 'krasnokamen-client',
                          'session::actions::fast_click_tests::press_released_before_the_next_tick_still_starts_the_attack'))
        with self.assertRaises(ValueError):
            regressions._parse_ref('client:krasnokamen-client')          # no test named
        with self.assertRaises(ValueError):
            regressions._parse_ref('client:../evil:crate::test')         # not a crate name

    def test_a_client_ref_is_unrunnable_without_a_client_checkout(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ctx = profiles.ProfileContext(Path(directory) / 'artifacts')
            (Path(directory) / 'artifacts').mkdir()
            ref = 'client:krasnokamen-client:session::actions::fast_click_tests::x'
            with patch.dict(os.environ, {'KRASNOKAMEN_CLIENT_ROOT': str(root / 'absent')}):
                out = ledger.run_refs(ctx, [ref], root, False)
            self.assertEqual(out[ref]['state'], ledger.UNRUNNABLE)
            self.assertIn('no client checkout', out[ref]['detail'])

    def run_client_packages(self, packages, refs, summary='test result: ok. 1 passed; 0 failed'):
        calls = []
        def execute(ctx, argv, tag, repo, env):
            calls.append(argv)
            if argv[1] == 'metadata':
                return True, 'mem-run: scope\n' + json.dumps({'packages': packages}) + '\n'
            return True, summary
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(ledger, 'client_repo', return_value=Path(directory)), \
                patch.object(regressions, 'tree_build_command', side_effect=lambda repo, argv: (argv, None)), \
                patch.object(ledger, '_exec', side_effect=execute):
            result = ledger._run_client(None, set(refs), Path(directory))
        return result, calls

    def test_client_lib_package_uses_library_target(self):
        package = {'name': 'client', 'targets': [
            {'name': 'client', 'kind': ['lib']}, {'name': 'ui', 'kind': ['bin']}]}
        result, calls = self.run_client_packages([package], [('client', 'unit::test')])
        self.assertEqual(result[('client', 'unit::test')], (ledger.GREEN, '', None))
        self.assertEqual(calls[-1], ['cargo', 'test', '-p', 'client', '--lib', '--', 'unit::test'])

    def test_client_bin_only_package_uses_named_binary_target(self):
        package = {'name': 'krasnokamen-launcher', 'targets': [
            {'name': 'krasnokamen-launcher', 'kind': ['bin']}]}
        result, calls = self.run_client_packages([package], [('krasnokamen-launcher', 'bootstrap::tests')])
        self.assertEqual(result[('krasnokamen-launcher', 'bootstrap::tests')], (ledger.GREEN, '', None))
        self.assertEqual(calls[-1], ['cargo', 'test', '-p', 'krasnokamen-launcher',
                                     '--bin', 'krasnokamen-launcher', '--', 'bootstrap::tests'])
        self.assertEqual(sum(argv[1] == 'metadata' for argv in calls), 1)

    def test_client_unknown_or_ambiguous_target_fails_closed(self):
        for packages in ([], [{'name': 'launcher', 'targets': []}],
                         [{'name': 'launcher', 'targets': [
                             {'name': 'one', 'kind': ['bin']}, {'name': 'two', 'kind': ['bin']}]}],
                         [{'name': 'launcher', 'targets': [
                             {'name': 'disabled', 'kind': ['bin'], 'test': False}]}]):
            with self.subTest(packages=packages):
                result, calls = self.run_client_packages(packages, [('launcher', 'test')])
                state, detail, _assertion = result[('launcher', 'test')]
                self.assertEqual(state, ledger.UNRUNNABLE)
                self.assertIn('launcher', detail)
                self.assertEqual(len(calls), 1)  # metadata only, never guessed cargo test

    def test_client_bin_ref_cannot_pass_by_selecting_no_test(self):
        package = {'name': 'launcher', 'targets': [{'name': 'launcher', 'kind': ['bin']}]}
        result, _ = self.run_client_packages([package], [('launcher', 'missing')],
                                             summary='test result: ok. 0 passed; 0 failed')
        self.assertEqual(result[('launcher', 'missing')], (ledger.UNRUNNABLE, 'ran no matching test', None))


class LedgerRefContractTests(unittest.TestCase):
    """Every ref in the real ledger resolves: a ref nobody can run is not a regression guard."""

    def test_every_ledger_ref_parses(self):
        from pathlib import Path

        rows = ledger.load_rows(Path(__file__).resolve().parents[2])
        broken = []
        for row in rows:
            for ref in row['regression']:
                try:
                    regressions._parse_ref(ref)
                except ValueError as error:
                    broken.append(f"{row['id']}: {ref}: {error}")
        self.assertEqual(broken, [], "unresolvable ledger refs: " + '; '.join(broken))

    def test_screens_voice_qa_is_a_runnable_profile_with_its_recorded_verdicts(self):
        # The profile and its backend always existed; the resolver tables did not, so all 17
        # harness:screens-voice-qa:* refs in the ledger were unrunnable.
        from pathlib import Path

        self.assertIn('screens-voice-qa', regressions._WINDOWED_PROFILES)
        verdicts = regressions._PROFILE_VERDICTS['screens-voice-qa'][1]
        source = (Path(__file__).parent / 'backends' / 'screens_voice_qa.py').read_text()
        recorded = set(re.findall(r'verdicts\["([a-z0-9_]+)"\]', source))
        self.assertTrue(recorded)
        self.assertEqual(recorded - verdicts, set(), "verdicts the backend records but the resolver rejects")
        for verdict in sorted(recorded):
            regressions._profile_command('screens-voice-qa', verdict)


class LedgerToolTests(unittest.TestCase):
    def git(self, root, *args):
        done = subprocess.run(['git', '-C', str(root), *args], check=True, capture_output=True, text=True, env={**os.environ, **GIT_ENV})
        return done.stdout.strip()

    def scratch(self, area='gameplay', ref='cargo:mc-core:mod::t', with_fix_commit=True, under_wt=False):
        """Tiny repo: base (buggy), fix (fixes engine.rs, adds a test file), ledger (row naming the fix)."""
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name) / ('wt/repo' if under_wt else 'repo')
        (root / 'src').mkdir(parents=True)
        (root / 'docs/bugs').mkdir(parents=True)
        self.git(root, 'init', '-q')
        (root / 'src/engine.rs').write_text('value = buggy\n')
        (root / 'src/util.rs').write_text('u1\n')
        self.git(root, 'add', '-A')
        self.git(root, 'commit', '-qm', 'base')
        self.base = self.git(root, 'rev-parse', 'HEAD')
        (root / 'src/engine.rs').write_text('value = fixed\n')
        (root / 'tests').mkdir()
        (root / 'tests/new_case.rs').write_text('check\n')
        self.git(root, 'add', '-A')
        self.git(root, 'commit', '-qm', 'fix')
        self.fix = self.git(root, 'rev-parse', 'HEAD')
        extra = f'fix_commit = "{self.fix}"\n' if with_fix_commit else ''
        ledger.write_rows(root, ROW.format(area=area, ref=ref, extra=extra))
        self.git(root, 'add', '-A')
        self.git(root, 'commit', '-qm', 'ledger')
        self.head = self.git(root, 'rev-parse', 'HEAD')
        self.artifacts = Path(directory.name) / 'artifacts'
        self.artifacts.mkdir()
        return root

    def runner(self, red_when_buggy=True):
        seen = []
        def run(ctx, refs, tree, client):
            seen.append(((tree / 'src/engine.rs').read_text(), (tree / 'tests/new_case.rs').exists()))
            buggy = 'buggy' in (tree / 'src/engine.rs').read_text()
            state = ledger.RED if buggy and red_when_buggy else ledger.GREEN
            # A red names its assertion like a real runner does; a bare FAIL on a cargo ref is class (c).
            assertion = {'file': 'crates/mc-core/src/tests.rs', 'line': 1, 'error': 'panic', 'class': 'assertion'}
            return {ref: ({'state': state, 'detail': '', 'assertion': assertion} if state == ledger.RED
                          else {'state': state, 'detail': ''}) for ref in refs}
        run.seen = seen
        return run

    def promote(self, root, runner, **kwargs):
        return ledger.promote('BUG', root=root, ctx=profiles.ProfileContext(self.artifacts), runner=runner, **kwargs)

    def test_missing_fix_commit_is_refused(self):
        root = self.scratch(with_fix_commit=False)
        runner = self.runner()
        with self.assertRaisesRegex(ledger.Refused, 'no fix_commit'):
            self.promote(root, runner)
        self.assertEqual(runner.seen, [])

    def test_client_row_needs_a_real_client_receipt_of_the_tree_being_promoted(self):
        root = self.scratch(area='client', ref='harness:entities:native-v2-craft-drag')
        runner = self.runner()
        with self.assertRaisesRegex(ledger.Refused, 'real-client receipt'):
            self.promote(root, runner)
        def receipt(tree, status='passed'):
            path = root / '.analysis/validation/20261107T000000-x/result.json'
            path.parent.mkdir(parents=True, exist_ok=True)
            body = {'status': status, 'profile': 'entities', 'ended_at': '2999-01-01T00:00:00+00:00',
                    'details': {'named_scenario': {'id': 'native-v2-craft-drag'}, 'server_binary_sha256': 'abc'}}
            if tree is not None:
                body['tree'] = tree
            path.write_text(json.dumps(body))
            return path
        now = ledger.tree_fingerprint(root)
        with self.assertRaisesRegex(ledger.Refused, 'not a passed result.json'):
            self.promote(root, runner, client_receipt=receipt(now, status='failed'))
        # A receipt without the fingerprint the harness records at run start proves nothing, whatever
        # the tree that now holds the file or its timestamp say.
        with self.assertRaisesRegex(ledger.Refused, 'records no tree fingerprint'):
            self.promote(root, runner, client_receipt=receipt(None))
        with self.assertRaisesRegex(ledger.Refused, 'records no tree fingerprint'):
            self.promote(root, runner, client_receipt=receipt({'head': self.head, 'dirty': False}))
        # A run on another revision (before the fix, or before later commits) is not the candidate.
        for other in (self.base, self.fix):
            with self.assertRaisesRegex(ledger.Refused, 'not at the tree being promoted'):
                self.promote(root, runner, client_receipt=receipt({**now, 'head': other}))
        # Nor is a run with other uncommitted changes.
        with self.assertRaisesRegex(ledger.Refused, 'different uncommitted changes'):
            self.promote(root, runner, client_receipt=receipt({**now, 'sha256': '0' * 64}))
        def register(tree):
            registry = root / '.analysis/ledger-builds'
            registry.mkdir(parents=True, exist_ok=True)
            (registry / 'abc.json').write_text(json.dumps({'sha256': 'abc', 'tree': {'head': tree['head'], 'sha256': tree['sha256']}}))
        # The scenarios run whatever KRASNOKAMEN_SERVER_BINARY names: a binary the tooling did not build from
        # the candidate (pinned or foreign), or built from an older HEAD, proves nothing about this source.
        with self.assertRaisesRegex(ledger.Refused, 'names no server binary built from the candidate'):
            self.promote(root, runner, client_receipt=receipt(now))
        register({**now, 'head': self.base})
        with self.assertRaisesRegex(ledger.Refused, 'built from a different tree'):
            self.promote(root, runner, client_receipt=receipt(now))
        register({**now, 'sha256': '1' * 64})
        with self.assertRaisesRegex(ledger.Refused, 'built from a different tree'):
            self.promote(root, runner, client_receipt=receipt(now))
        self.assertEqual(runner.seen, [])
        register(now)
        self.promote(root, runner, client_receipt=receipt(now), dry_run=True)
        # The ref the receipt covers carried the red control, so it is rerun on restore.
        self.assertEqual(len(runner.seen), 2)

    def client_repo(self):
        """A second repository beside the server one: the client the `client:<sha>` fix lives in."""
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name) / 'solaris-client'
        root.mkdir(parents=True)
        self.git(root, 'init', '-q')
        (root / 'Client.java').write_text('v1\n')
        self.git(root, 'add', '-A')
        self.git(root, 'commit', '-qm', 'client base')
        self.client_base = self.git(root, 'rev-parse', 'HEAD')
        (root / 'Client.java').write_text('v2\n')
        self.git(root, 'commit', '-qam', 'client fix')
        self.client_fix = self.git(root, 'rev-parse', 'HEAD')
        self.client_root = patch.dict(os.environ, {'KRASNOKAMEN_CLIENT_ROOT': str(root)})
        self.client_root.start()
        self.addCleanup(self.client_root.stop)
        return root

    def client_row(self, client_sha=None, ref='harness:entities:native-v2-craft-drag'):
        root = self.scratch(area='client', ref=ref)
        fix = f'client:{client_sha}' if client_sha else ''
        text = ledger.document(root).replace(
            f'fix_commit = "{self.fix}"', f'fix_commit = "{fix}"')
        if ref == 'harness:entities:native-v2-craft-drag':
            text += '\nproving_assertions = { "harness:entities:native-v2-craft-drag" = "initial_drag_keeps_server_cursor" }\n'
        ledger.write_rows(root, text)
        self.git(root, 'commit', '-qam', 'client row')
        return root

    def client_receipt(self, root, client_head, *, verdict=True, status=None, register=True, server_tree=None,
                       pin_binary_sha='d' * 64, pin_head=None, dirty=False, manifest=True,
                       assertions=None, client_binary=True, pin_override=None):
        """A real-client result.json: its client binary bound to its own pin manifest with `client_head`."""
        tree = server_tree or ledger.tree_fingerprint(root)
        stamp = abs(hash((client_head, verdict, status, dirty, pin_binary_sha)))
        pin_dir = root.parent / f'pin-{stamp}'
        path = root / f'.analysis/validation/{stamp}-x/result.json'
        path.parent.mkdir(parents=True, exist_ok=True)
        details = {'named_scenario': {'id': 'native-v2-craft-drag'},
                   'verdicts': {'native-v2-craft-drag': verdict}, 'server_binary_sha256': 'abc'}
        if assertions is not None:
            details['scenario'] = {'scenario': {'id': 'native-v2-craft-drag', 'assertions': assertions}}
        body = {'status': status or ('passed' if verdict else 'failed'), 'profile': 'entities',
                'ended_at': '2999-01-01T00:00:00+00:00', 'details': details, 'tree': tree}
        if client_binary:
            body['client_binary'] = {'path': str(pin_dir / 'solaris-client'), 'sha256': pin_binary_sha,
                                     'manifest_path': str(pin_dir / 'manifest.json'),
                                     'head': pin_head or client_head, 'dirty': dirty}
        path.write_text(json.dumps(body))
        if manifest:
            pin = pin_override or {'repo': 'client', 'kind': 'client', 'head': client_head, 'dirty': dirty,
                                   'diff_sha256': 'e' * 64, 'binary': str(pin_dir / 'solaris-client'),
                                   'binary_sha256': pin_binary_sha}
            pin_dir.mkdir(exist_ok=True)
            (pin_dir / 'manifest.json').write_text(json.dumps(pin))
        if register:
            registry = root / '.analysis/ledger-builds'
            registry.mkdir(parents=True, exist_ok=True)
            (registry / 'abc.json').write_text(json.dumps(
                {'sha256': 'abc', 'tree': {'head': tree['head'], 'sha256': tree['sha256']}}))
        return path

    def test_a_python_test_ref_is_a_registered_check(self):
        # A tools/** fix has no cargo check: its own unittest is the guard, and promote has to run it.
        ref = 'python:tools.harness.test_plan:InnovationTests.test_new_workspace_dependency_is_innovation'
        self.assertEqual(regressions._parse_ref(ref),
                         ('python', 'tools.harness.test_plan', 'InnovationTests.test_new_workspace_dependency_is_innovation'))
        with self.assertRaises(ValueError):
            regressions._parse_ref('python:tools.harness.test_plan')          # no test named
        with self.assertRaises(ValueError):
            regressions._parse_ref('python:../etc:passwd.test')               # not a module

    def test_a_failing_python_ref_goes_red_and_a_missing_one_is_unrunnable(self):
        root = self.scratch(ref='python:tools.harness.test_plan:InnovationTests.test_new_workspace_dependency_is_innovation')
        # The scratch repo has no tools/ tree, so the ref cannot run there: unrunnable, never green.
        ctx = profiles.ProfileContext(self.artifacts)
        out = ledger.run_refs(ctx, ['python:tools.harness.test_plan:InnovationTests.test_new_workspace_dependency_is_innovation'], root, False)
        state = out['python:tools.harness.test_plan:InnovationTests.test_new_workspace_dependency_is_innovation']['state']
        self.assertIn(state, (ledger.UNRUNNABLE, ledger.RED))

    def test_client_fix_rejects_unrelated_assertion_and_accepts_proving_assertion(self):
        self.client_repo()
        root = self.client_row(client_sha=self.client_fix[:12])
        wrong = self.client_receipt(root, self.client_base, verdict=False,
                                    assertions={'native-v2-ledger-painting': False})
        restore = self.client_receipt(root, self.client_fix, verdict=True)
        with self.assertRaisesRegex(ledger.Refused, 'not red on any of the row'):
            self.promote(root, self.runner(), client_control_receipt=wrong, client_receipt=restore, dry_run=True)
        control = self.client_receipt(root, self.client_base, verdict=False,
                                      assertions={'initial_drag_keeps_server_cursor': False})
        restore = self.client_receipt(root, self.client_fix, verdict=True)
        receipt = self.promote(root, self.runner(), client_control_receipt=control,
                              client_receipt=restore, dry_run=True)
        self.assertTrue(receipt['client_fix'])
        self.assertEqual(receipt['control_tree'], 'client')
        self.assertEqual(receipt['negative_control']['client_head'], self.client_base)
        self.assertEqual(receipt['restored']['client_head'], self.client_fix)
        self.assertIn('harness:entities:native-v2-craft-drag', receipt['negative_control']['red'])
        # The runner is never invoked: receipts are the evidence for a client fix.
        self.assertEqual(self.runner().seen, [])
        self.promote(root, self.runner(), client_control_receipt=control, client_receipt=restore)
        self.assertIn('status = "fixed"', ledger.document(root))

    def test_client_control_is_candidate_with_only_fix_reversed(self):
        client = self.client_repo()
        root = self.client_row(client_sha=self.client_fix[:12])
        (client / 'later.txt').write_text('later change\n')
        self.git(client, 'add', 'later.txt')
        self.git(client, 'commit', '-qm', 'later candidate')
        candidate = self.git(client, 'rev-parse', 'HEAD')
        self.git(client, 'revert', '--no-edit', self.client_fix)
        exact_control = self.git(client, 'rev-parse', 'HEAD')
        restore = self.client_receipt(root, candidate)
        def promote_with(control_head):
            control = self.client_receipt(root, control_head, verdict=False,
                                          assertions={'initial_drag_keeps_server_cursor': False})
            return self.promote(root, self.runner(), client_control_receipt=control,
                                client_receipt=restore, dry_run=True)
        receipt = promote_with(exact_control)
        self.assertEqual(receipt['negative_control']['client_head'], exact_control)
        self.assertTrue(receipt['client_control_delta']['fix_patch_id'])
        with self.assertRaisesRegex(ledger.Refused, 'extra paths: later.txt'):
            promote_with(self.client_base)
        (client / 'unrelated.txt').write_text('unrelated\n')
        self.git(client, 'add', 'unrelated.txt')
        self.git(client, 'commit', '-qm', 'unrelated control change')
        unrelated = self.git(client, 'rev-parse', 'HEAD')
        with self.assertRaisesRegex(ledger.Refused, 'extra paths: unrelated.txt'):
            promote_with(unrelated)

    def test_client_control_that_stays_green_is_refused(self):
        self.client_repo()
        root = self.client_row(client_sha=self.client_fix[:12])
        with self.assertRaisesRegex(ledger.Refused, 'not red on any of the row'):
            self.promote(root, self.runner(), client_control_receipt=self.client_receipt(root, self.client_base),
                         client_receipt=self.client_receipt(root, self.client_fix), dry_run=True)

    def test_client_control_that_already_has_the_fix_is_refused(self):
        self.client_repo()
        root = self.client_row(client_sha=self.client_fix[:12])
        with self.assertRaisesRegex(ledger.Refused, 'more than the fix non-test hunks'):
            self.promote(root, self.runner(),
                         client_control_receipt=self.client_receipt(root, self.client_fix, verdict=False, assertions={'initial_drag_keeps_server_cursor': False}),
                         client_receipt=self.client_receipt(root, self.client_fix), dry_run=True)

    def test_client_restore_on_the_pre_fix_client_is_refused(self):
        self.client_repo()
        root = self.client_row(client_sha=self.client_fix[:12])
        with self.assertRaisesRegex(ledger.Refused, 'does not contain the fix'):
            self.promote(root, self.runner(),
                         client_control_receipt=self.client_receipt(root, self.client_base, verdict=False, assertions={'initial_drag_keeps_server_cursor': False}),
                         client_receipt=self.client_receipt(root, self.client_base), dry_run=True)

    def test_unknown_client_commit_is_refused(self):
        self.client_repo()
        root = self.client_row(client_sha='0' * 12)
        with self.assertRaisesRegex(ledger.Refused, 'not a commit of the client repository'):
            self.promote(root, self.runner(), dry_run=True)

    def test_receipt_without_a_client_binary_is_refused(self):
        self.client_repo()
        root = self.client_row(client_sha=self.client_fix[:12])
        control = self.client_receipt(root, self.client_base, verdict=False,
                                      assertions={'initial_drag_keeps_server_cursor': False})
        body = json.loads(control.read_text())
        body.pop('client_binary')
        control.write_text(json.dumps(body))
        with self.assertRaisesRegex(ledger.Refused, 'records no client binary'):
            self.promote(root, self.runner(), client_control_receipt=control,
                         client_receipt=self.client_receipt(root, self.client_fix), dry_run=True)

    def test_receipt_without_a_pin_manifest_is_refused(self):
        self.client_repo()
        root = self.client_row(client_sha=self.client_fix[:12])
        control = self.client_receipt(root, self.client_base, verdict=False, manifest=False,
                                      assertions={'initial_drag_keeps_server_cursor': False})
        with self.assertRaisesRegex(ledger.Refused, r'records no client pin manifest|pin manifest .* unreadable'):
            self.promote(root, self.runner(), client_control_receipt=control,
                         client_receipt=self.client_receipt(root, self.client_fix), dry_run=True)

    def test_client_binary_that_the_pin_does_not_describe_is_refused(self):
        self.client_repo()
        root = self.client_row(client_sha=self.client_fix[:12])
        # The receipt ran one binary; its pin manifest describes another (a stale or hand-edited pin).
        control = self.client_receipt(root, self.client_base, verdict=False, pin_binary_sha='a' * 64,
                                      assertions={'initial_drag_keeps_server_cursor': False},
                                      pin_override={'repo': 'client', 'kind': 'client', 'head': self.client_base,
                                                    'dirty': False, 'binary_sha256': 'b' * 64})
        with self.assertRaisesRegex(ledger.Refused, 'does not describe the client that ran'):
            self.promote(root, self.runner(), client_control_receipt=control,
                         client_receipt=self.client_receipt(root, self.client_fix), dry_run=True)

    def test_dirty_client_pin_is_refused(self):
        self.client_repo()
        root = self.client_row(client_sha=self.client_fix[:12])
        control = self.client_receipt(root, self.client_base, verdict=False, dirty=True,
                                      assertions={'initial_drag_keeps_server_cursor': False})
        with self.assertRaisesRegex(ledger.Refused, 'is dirty'):
            self.promote(root, self.runner(), client_control_receipt=control,
                         client_receipt=self.client_receipt(root, self.client_fix), dry_run=True)

    def test_a_timed_out_control_is_not_a_red_observation(self):
        self.client_repo()
        root = self.client_row(client_sha=self.client_fix[:12])
        # status=timeout never observed the bug: only a failed run with a failed assertion is red.
        control = self.client_receipt(root, self.client_base, status='timeout',
                                      assertions={'native-v2-ledger-painting': False})
        with self.assertRaisesRegex(ledger.Refused, 'not red on any of the row'):
            self.promote(root, self.runner(), client_control_receipt=control,
                         client_receipt=self.client_receipt(root, self.client_fix), dry_run=True)

    def test_a_failed_run_without_a_failed_assertion_is_not_red(self):
        self.client_repo()
        root = self.client_row(client_sha=self.client_fix[:12])
        control = self.client_receipt(root, self.client_base, verdict=False,
                                      assertions={'native-v2-ledger-painting': True})
        with self.assertRaisesRegex(ledger.Refused, 'not red on any of the row'):
            self.promote(root, self.runner(), client_control_receipt=control,
                         client_receipt=self.client_receipt(root, self.client_fix), dry_run=True)

    def test_client_fix_commit_needs_both_receipts_and_a_graphical_ref(self):
        self.client_repo()
        root = self.client_row(client_sha=self.client_fix[:12])
        with self.assertRaisesRegex(ledger.Refused, 'needs the negative control'):
            self.promote(root, self.runner(), client_receipt=self.client_receipt(root, self.client_fix), dry_run=True)
        with self.assertRaisesRegex(ledger.Refused, 'needs the restore'):
            self.promote(root, self.runner(),
                         client_control_receipt=self.client_receipt(root, self.client_base, verdict=False, assertions={'native-v2-ledger-painting': False}), dry_run=True)
        plain = self.client_row(client_sha=self.client_fix[:12], ref='cargo:mc-core:mod::t')
        with self.assertRaisesRegex(ledger.Refused, 'real-client harness ref'):
            self.promote(plain, self.runner(), client_control_receipt=self.client_receipt(root, self.client_base, verdict=False, assertions={'native-v2-ledger-painting': False}),
                         client_receipt=self.client_receipt(root, self.client_fix), dry_run=True)

    def test_client_control_supplied_to_a_server_fix_commit_is_refused(self):
        root = self.scratch(area='client', ref='harness:entities:native-v2-craft-drag')
        with self.assertRaisesRegex(ledger.Refused, 'only for a fix_commit of the form client'):
            self.promote(root, self.runner(), client_control_receipt=Path('/nonexistent.json'), dry_run=True)

    def test_a_later_regression_on_head_refuses_promotion(self):
        root = self.scratch()
        (root / 'src/engine.rs').write_text('value = buggy again\n')
        self.git(root, 'commit', '-qam', 'regress')
        # The fix hunk no longer applies at HEAD, so the control falls back to the fix commit...
        with self.assertRaisesRegex(ledger.Refused, r'not green on the tree being promoted'):
            self.promote(root, self.runner())
        # ...but the restore is always on HEAD, where the check is red.
        self.assertEqual(self.git(root, 'worktree', 'list').count('\n'), 0)

    def test_control_falls_back_to_the_fix_commit_when_hunks_do_not_apply_at_head(self):
        root = self.scratch()
        (root / 'src/engine.rs').write_text('value = fixed and refactored\n')
        self.git(root, 'commit', '-qam', 'refactor')
        runner = self.runner()
        receipt = self.promote(root, runner, dry_run=True)
        self.assertEqual(receipt['control_tree'], 'fix_commit')
        self.assertEqual(runner.seen, [('value = buggy\n', True), ('value = fixed and refactored\n', True)])

    def test_uncommitted_candidate_is_checked_exactly_without_a_commit(self):
        root = self.scratch()
        (root / 'src/util.rs').write_text('u2\n')
        (root / 'src/new.rs').write_text('n1\n')
        (root / 'docs/bugs/note.md').write_text('docs/bugs edits are not part of the candidate\n')
        seen = []
        def runner(ctx, refs, tree, client):
            buggy = 'buggy' in (tree / 'src/engine.rs').read_text()
            seen.append(((tree / 'src/util.rs').read_text(), (tree / 'src/new.rs').read_text(), buggy))
            return {ref: ({'state': state, 'detail': '', 'assertion': {'file': 'crates/mc-core/src/tests.rs', 'line': 1, 'error': 'panic', 'class': 'assertion'}} if state == ledger.RED else {'state': state, 'detail': ''}) for ref in refs for state in [ledger.RED if buggy else ledger.GREEN]}
        receipt = self.promote(root, runner)
        # Control and restore both run on HEAD + the uncommitted diff + the untracked file.
        self.assertEqual(seen, [('u2\n', 'n1\n', True), ('u2\n', 'n1\n', False)])
        self.assertTrue(receipt['tree_dirty'])
        self.assertEqual(receipt['tree_sha256'], ledger.tree_fingerprint(root)['sha256'])
        self.assertIn('status = "fixed"', ledger.document(root))
        self.assertEqual(self.git(root, 'rev-parse', 'HEAD'), self.head)

    def test_uncommitted_regression_in_the_candidate_refuses_promotion(self):
        root = self.scratch()
        (root / 'src/engine.rs').write_text('value = buggy locally\n')
        with self.assertRaisesRegex(ledger.Refused, 'not green on the tree being promoted'):
            self.promote(root, self.runner())

    def test_negative_control_that_stays_green_is_refused(self):
        root = self.scratch()
        before = ledger.document(root)
        with self.assertRaisesRegex(ledger.Refused, 'negative control .* did not go red'):
            self.promote(root, self.runner(red_when_buggy=False))
        self.assertEqual(ledger.document(root), before)

    def test_restored_check_that_is_not_green_is_refused(self):
        root = self.scratch()
        def always_red(ctx, refs, tree, client):
            return {ref: {'state': ledger.RED, 'detail': 'boom', 'assertion': {'file': 'crates/mc-core/src/tests.rs', 'line': 1, 'error': 'panic', 'class': 'assertion'}} for ref in refs}
        with self.assertRaisesRegex(ledger.Refused, 'not green on the tree being promoted'):
            self.promote(root, always_red)

    def test_promotion_succeeds_and_dry_run_writes_nothing(self):
        root = self.scratch()
        before = ledger.document(root)
        runner = self.runner()
        receipt = self.promote(root, runner, dry_run=True)
        self.assertEqual(receipt['reversed_files'], ['src/engine.rs'])
        self.assertEqual(receipt['kept_test_files'], ['tests/new_case.rs'])
        # Control: only the non-test hunks reversed (test file kept); restore: the fix itself.
        self.assertEqual(runner.seen, [('value = buggy\n', True), ('value = fixed\n', True)])
        self.assertEqual((receipt['control_tree'], receipt['tree_head']), ('head', self.head))
        self.assertEqual(ledger.document(root), before)
        self.assertFalse((root / 'docs/bugs/promotions').exists())
        self.assertEqual(self.git(root, 'status', '--porcelain'), '')
        self.assertEqual(self.git(root, 'worktree', 'list').count('\n'), 0)
        self.promote(root, self.runner())
        text = ledger.document(root)
        self.assertIn('status = "fixed"', text)
        self.assertIn('promotion = "docs/bugs/promotions/BUG.json"', text)
        bug = tomllib.loads(text)['bug'][0]
        with patch.object(regressions, 'REPO_ROOT', root):
            self.assertEqual(regressions._promotion_errors('BUG', bug), [])
        with self.assertRaisesRegex(ledger.Refused, 'already fixed'):
            self.promote(root, self.runner())

    def test_inline_test_module_hunks_are_not_reversed(self):
        root = self.scratch()
        filler = ''.join(f'// filler {n}\n' for n in range(12))
        source = f'fn value() -> u8 {{\n    1\n}}\n{filler}\n#[cfg(test)]\nmod tests {{\n    #[test]\n    fn it() {{}}\n}}\n'
        (root / 'src/lib.rs').write_text(source)
        self.git(root, 'add', '-A')
        self.git(root, 'commit', '-qm', 'lib')
        (root / 'src/lib.rs').write_text(source.replace('    1\n', '    2\n').replace('fn it() {}', 'fn it() { assert_eq!(value(), 2); }'))
        self.git(root, 'commit', '-qam', 'fix lib')
        patch_text, reversed_files, test_files = ledger.non_test_patch(root, self.git(root, 'rev-parse', 'HEAD'))
        self.assertEqual(reversed_files, ['src/lib.rs'])
        self.assertIn('-    1\n+    2\n', patch_text)
        self.assertNotIn('assert_eq', patch_text)

    def test_inline_test_module_before_later_production(self):
        root = self.scratch()
        filler = ''.join(f'// filler {n}\n' for n in range(12))
        source = ('#[cfg(test)]\nmod tests {\n'
                  '    const BRACES: &str = r#"} { /*"#;\n'
                  '    /* { nested /* } */ { */\n'
                  '    fn test_it() { assert!(true); }\n}\n'
                  + filler + 'fn later() -> u8 {\n    1\n}\n')
        (root / 'src/lib.rs').write_text(source)
        self.git(root, 'add', '-A')
        self.git(root, 'commit', '-qm', 'test module before production')
        changed = source.replace('assert!(true)', 'assert!(false)').replace('    1\n', '    2\n')
        (root / 'src/lib.rs').write_text(changed)
        self.git(root, 'commit', '-qam', 'change test and production')
        patch_text, reversed_files, _ = ledger.non_test_patch(root, self.git(root, 'rev-parse', 'HEAD'))
        self.assertEqual(reversed_files, ['src/lib.rs'])
        self.assertIn('-    1\n+    2\n', patch_text)
        self.assertNotIn('assert!(false)', patch_text)

    def test_deletion_before_test_module_refuses_ambiguous_boundary(self):
        root = self.scratch()
        source = 'fn removed_production() {}\n#[cfg(test)]\nmod tests {\n    fn it() {}\n}\n'
        (root / 'src/lib.rs').write_text(source)
        self.git(root, 'add', '-A')
        self.git(root, 'commit', '-qm', 'production before test')
        (root / 'src/lib.rs').write_text(source.replace('fn removed_production() {}\n', ''))
        self.git(root, 'commit', '-qam', 'delete production')
        with self.assertRaisesRegex(ledger.Refused, 'deletion.*module boundary'):
            ledger.non_test_patch(root, self.git(root, 'rev-parse', 'HEAD'))

    def test_hunk_straddling_the_test_module_is_refused(self):
        root = self.scratch()
        source = 'fn value() -> u8 {\n    1\n}\n#[cfg(test)]\nmod tests {\n    fn it() {}\n}\n'
        (root / 'src/lib.rs').write_text(source)
        self.git(root, 'add', '-A')
        self.git(root, 'commit', '-qm', 'lib')
        (root / 'src/lib.rs').write_text(source.replace('    1\n', '    2\n').replace('fn it() {}', 'fn it() { value(); }'))
        self.git(root, 'commit', '-qam', 'fix lib')
        with self.assertRaisesRegex(ledger.Refused, 'straddles'):
            ledger.non_test_patch(root, self.git(root, 'rev-parse', 'HEAD'))

    def test_added_file_with_inline_test_module_is_reversed_whole(self):
        root = self.scratch()
        (root / 'src/other.rs').write_text('fn other() {}\n')
        self.git(root, 'add', '-A')
        self.git(root, 'commit', '-qm', 'base')
        (root / 'src/fresh.rs').write_text('pub fn value() -> u8 {\n    2\n}\n#[cfg(test)]\nmod tests {\n    fn it() {}\n}\n')
        self.git(root, 'add', '-A')
        self.git(root, 'commit', '-qm', 'add fresh')
        patch_text, reversed_files, _ = ledger.non_test_patch(root, self.git(root, 'rev-parse', 'HEAD'))
        self.assertEqual(reversed_files, ['src/fresh.rs'])
        self.assertIn('+mod tests', patch_text)

    def test_fixed_row_needs_a_matching_promotion_receipt(self):
        root = self.scratch()
        bug = {'id': 'BUG', 'status': 'fixed', 'fix_commit': self.fix}
        with patch.object(regressions, 'REPO_ROOT', root):
            self.assertRegex(regressions._promotion_errors('BUG', bug)[0], 'fixed without a promotion receipt')
            with patch.object(regressions, 'BASE_FIXED', frozenset({'BUG'})):
                self.assertEqual(regressions._promotion_errors('BUG', bug), [])
            bug['promotion'] = 'docs/bugs/promotions/BUG.json'
            self.assertRegex(regressions._promotion_errors('BUG', bug)[0], 'missing or not JSON')
            (root / 'docs/bugs/promotions').mkdir()
            (root / bug['promotion']).write_text(json.dumps({'id': 'BUG', 'verdict': 'promoted', 'fix_commit': 'deadbeef'}))
            self.assertRegex(regressions._promotion_errors('BUG', bug)[0], 'does not match')
            (root / bug['promotion']).write_text(json.dumps({'id': 'BUG', 'verdict': 'promoted', 'fix_commit': self.fix}))
            self.assertEqual(regressions._promotion_errors('BUG', bug), [])
            bug['status'] = 'open'
            self.assertRegex(regressions._promotion_errors('BUG', bug)[0], 'not fixed')

    def test_hand_flipped_row_fails_the_ledger_run(self):
        root = self.scratch()
        text = ledger.document(root).replace('status = "open"', 'status = "fixed"')
        ledger.write_rows(root, text)
        with patch.object(regressions, 'REPO_ROOT', root), patch.object(regressions, 'BASE_FIXED', frozenset()), patch.object(profiles, '_run_command', side_effect=AssertionError('must reject before executing')):
            with self.assertRaisesRegex(ValueError, 'BUG: fixed without a promotion receipt'):
                regressions.run(profiles.ProfileContext(self.artifacts))

    def test_reopened_base_fixed_row_must_leave_the_list(self):
        root = self.scratch()
        with patch.object(regressions, 'REPO_ROOT', root), patch.object(regressions, 'BASE_FIXED', frozenset({'BUG'})):
            with self.assertRaisesRegex(ValueError, 'BUG: reopened row is still in BASE_FIXED'):
                regressions.run(profiles.ProfileContext(self.artifacts))

    def test_probe_classifies_rows_and_never_skips_silently(self):
        root = self.scratch(ref='harness:entities:native-v2-craft-drag')
        bugs = ledger.load_rows(root)
        bugs.append({'id': 'EMPTY', 'status': 'open', 'regression': []})
        bugs.append({'id': 'BAD', 'status': 'open', 'regression': ['cargo:mc-core:mod::t', 'nonsense']})
        ctx = profiles.ProfileContext(self.artifacts)
        def runner(ctx, refs, tree, client):
            out = ledger.run_refs(ctx, [ref for ref in refs if ref.startswith('harness:') or ref == 'nonsense'], tree, client)
            out['cargo:mc-core:mod::t'] = {'state': ledger.RED, 'detail': 'FAIL', 'assertion': {'file': 'crates/mc-core/src/tests.rs', 'line': 1, 'error': 'panic', 'class': 'assertion'}}
            return out
        rows = ledger.probe(bugs, ['BUG', 'EMPTY', 'BAD'], client=False, ctx=ctx, runner=runner)
        self.assertEqual(rows['BUG']['verdict'], ledger.NEEDS_CLIENT)
        self.assertEqual(rows['EMPTY']['verdict'], ledger.UNRUNNABLE)
        self.assertEqual(ledger.format_row('EMPTY', rows['EMPTY']), 'EMPTY: unrunnable(no registered check)')
        self.assertEqual(rows['BAD']['verdict'], ledger.RED)
        self.assertIn('FAIL', rows['BAD']['reason'])
        with self.assertRaisesRegex(ledger.Refused, 'unknown ledger id'):
            ledger.probe(bugs, ['NOPE'], client=False, ctx=ctx, runner=runner)

    def test_probe_receipt_binds_dirty_tree_and_refuses_changed_tree(self):
        for change_during_probe in (False, True):
            with self.subTest(change_during_probe=change_during_probe):
                root = self.scratch()
                (root / 'src/engine.rs').write_text('dirty candidate\n')
                expected = ledger.tree_fingerprint(root)
                output = self.artifacts / 'out.json'
                ctx = profiles.ProfileContext(self.artifacts)

                def run(bugs, ids, *, client, ctx):
                    if change_during_probe:
                        (root / 'src/engine.rs').write_text('changed during probe\n')
                    return {'BUG': {'verdict': ledger.GREEN, 'reason': '', 'status': 'open',
                                    'refs': {'cargo:mc-core:mod::t': {'state': ledger.GREEN, 'detail': ''}}}}

                with patch.object(ledger, 'REPO_ROOT', root), patch.object(ledger, '_artifact_ctx', return_value=ctx), \
                        patch.object(ledger, 'probe', side_effect=run), patch('builtins.print'):
                    status = ledger.main(['probe', 'BUG', '--out', str(output)])
                receipt = json.loads(output.read_text())
                self.assertEqual(receipt['tree_start'], expected)
                self.assertEqual(receipt['tree_end'], ledger.tree_fingerprint(root))
                self.assertEqual(receipt['tree_stable'], not change_during_probe)
                self.assertEqual(status, 2 if change_during_probe else 0)
                self.assertEqual(receipt['rows']['BUG']['verdict'],
                                 ledger.UNRUNNABLE if change_during_probe else ledger.GREEN)
                self.assertEqual(receipt['rows']['BUG']['refs']['cargo:mc-core:mod::t']['state'], ledger.GREEN)

    def test_cargo_results_are_parsed_per_test(self):
        listing = {'rust-suites': {'lib': {'package-name': 'mc-core', 'testcases': {
            name: {'filter-match': {'status': 'matches'}} for name in
            ('a::ok', 'a::bad', 'a::timeout', 'a::abort', 'a::tmpass')
        }}}}
        listing['rust-suites']['lib']['testcases']['a::ignored'] = {
            'filter-match': {'status': 'matches'}, 'ignored': True}
        def run_command(argv, ctx, name, cwd=None, env=None):
            text = json.dumps(listing) if argv[2] == 'list' else (
                '        PASS [   0.010s] (1/5) mc-core a::ok\n'
                '        FAIL [   0.020s] (2/5) mc-core a::bad\n'
                '     TIMEOUT [   2.001s] (3/5) mc-core a::timeout\n'
                '       ABORT [   0.001s] (4/5) mc-core a::abort\n'
                '      TMPASS [   2.001s] (5/5) mc-core a::tmpass\n'
                '        SLOW [  90.001s] mc-core a::ok\n')
            (ctx.artifact_dir / name).write_text(text)
            ctx.logs.append(name)
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        ctx = profiles.ProfileContext(Path(directory.name))
        tests = {('mc-core', f'a::{name}') for name in
                 ('ok', 'bad', 'timeout', 'abort', 'tmpass', 'ignored', 'missing')}
        with patch.object(profiles, '_run_command', run_command), patch.object(regressions, 'MEM_RUN', Path('/nonexistent/mem-run.sh')):
            results = ledger._run_cargo(ctx, tests, Path('.'))
        self.assertEqual(results[('mc-core', 'a::ok')][0], ledger.GREEN)
        self.assertEqual(results[('mc-core', 'a::bad')][:2], (ledger.RED, 'FAIL'))
        self.assertEqual(results[('mc-core', 'a::timeout')][:2], (ledger.UNRUNNABLE, 'TIMEOUT; see log'))
        self.assertEqual(results[('mc-core', 'a::abort')][:2], (ledger.UNRUNNABLE, 'ABORT; see log'))
        self.assertEqual(results[('mc-core', 'a::tmpass')][:2], (ledger.UNRUNNABLE, 'TMPASS; see log'))
        self.assertEqual(results[('mc-core', 'a::ignored')][0], ledger.UNRUNNABLE)
        self.assertEqual(results[('mc-core', 'a::missing')][0], ledger.UNRUNNABLE)

    def test_cargo_timeout_cannot_promote_as_a_negative_control(self):
        root = self.scratch()
        before = ledger.document(root)
        listing = {'rust-suites': {'lib': {'package-name': 'mc-core', 'testcases': {
            'mod::t': {'filter-match': {'status': 'matches'}}}}}}
        runs = []

        def run_command(argv, ctx, name, cwd=None, env=None):
            runs.append(argv[2])
            if argv[2] == 'list':
                (ctx.artifact_dir / name).write_text(json.dumps(listing))
                ctx.logs.append(name)
                return
            (ctx.artifact_dir / name).write_text('     TIMEOUT [   2.001s] mc-core mod::t\n')
            ctx.logs.append(name)
            raise RuntimeError('test timed out')

        with patch.object(profiles, '_run_command', run_command), \
                patch.object(regressions, 'MEM_RUN', Path('/nonexistent/mem-run.sh')):
            with self.assertRaisesRegex(ledger.Refused, 'negative control .* did not go red'):
                self.promote(root, ledger.run_refs, dry_run=True)
        self.assertEqual(runs, ['list', 'run'])
        self.assertEqual(ledger.document(root), before)

    def test_control_worktree_gets_a_private_copy_of_the_vanilla_cache(self):
        root = self.scratch()
        (root / '.gitignore').write_text('/data/vanilla\n')
        self.git(root, 'add', '.gitignore')
        self.git(root, 'commit', '-qm', 'ignore')
        (root / 'data/vanilla').mkdir(parents=True)
        (root / 'data/vanilla/res.json').write_text('{}')
        seen = []
        def runner(ctx, refs, tree, client):
            seen.append((tree / 'data/vanilla/res.json').exists() and not (tree / 'data').is_symlink() and not (tree / 'data/vanilla').is_symlink())
            (tree / 'data/vanilla/res.json').write_text('{"scribbled": true}')
            buggy = 'buggy' in (tree / 'src/engine.rs').read_text()
            return {ref: ({'state': state, 'detail': '', 'assertion': {'file': 'crates/mc-core/src/tests.rs', 'line': 1, 'error': 'panic', 'class': 'assertion'}} if state == ledger.RED else {'state': state, 'detail': ''}) for ref in refs for state in [ledger.RED if buggy else ledger.GREEN]}
        self.promote(root, runner, dry_run=True)
        self.assertEqual(seen, [True, True])
        self.assertEqual((root / 'data/vanilla/res.json').read_text(), '{}')

    def test_control_worktree_gets_the_translation_caches_a_java_client_joins_through(self):
        root = self.scratch()
        (root / '.gitignore').write_text('/data/vanilla*\n')
        self.git(root, 'add', '.gitignore')
        self.git(root, 'commit', '-qm', 'ignore')
        (root / 'data/vanilla/steps/777-776/upper').mkdir(parents=True)
        (root / 'data/vanilla/steps/777-776/upper/version.json').write_text('{}')
        (root / 'data/vanilla-26.1.2').mkdir()
        (root / 'data/vanilla-26.1.2/res.json').write_text('{}')
        os.symlink('vanilla/steps', root / 'data/vanilla-steps')
        seen = []
        def runner(ctx, refs, tree, client):
            data = tree / 'data'
            seen.append((data / 'vanilla-steps/777-776/upper/version.json').is_file()
                        and os.readlink(data / 'vanilla-steps') == 'vanilla/steps'
                        and (data / 'vanilla-26.1.2/res.json').is_file()
                        and not (data / 'vanilla-26.1.2').is_symlink())
            buggy = 'buggy' in (tree / 'src/engine.rs').read_text()
            return {ref: ({'state': state, 'detail': '', 'assertion': {'file': 'crates/mc-core/src/tests.rs', 'line': 1, 'error': 'panic', 'class': 'assertion'}} if state == ledger.RED else {'state': state, 'detail': ''}) for ref in refs for state in [ledger.RED if buggy else ledger.GREEN]}
        self.promote(root, runner, dry_run=True)
        self.assertEqual(seen, [True, True])

    def test_promote_without_a_held_build_token_is_refused_before_the_control_lock(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name) / 'wt' / 'Promoter'
        root.mkdir(parents=True)
        with patch.dict(os.environ, {}, clear=False):
            os.environ.pop('KRASNOKAMEN_BUILD_TOKEN', None)
            with self.assertRaisesRegex(ledger.Refused, 'must run under mem-run'):
                with ledger._control_lock(root):
                    pass
        self.assertFalse((root.parent / 'ledger-control.lock').exists())
        with patch.dict(os.environ, {'KRASNOKAMEN_BUILD_TOKEN': '2'}):
            with ledger._control_lock(root):
                self.assertTrue((root.parent / 'ledger-control.lock').exists())

    def test_client_refs_are_classified_from_their_receipts(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        receipts = {}
        def run_command(argv, ctx, name, cwd=None, env=None):
            receipt = root / f'receipt-{len(ctx.logs)}'
            receipt.mkdir()
            (receipt / 'result.json').write_text(json.dumps(receipts['next']))
            (ctx.artifact_dir / name).write_text(f'[harness] artifact={receipt}\n')
            ctx.logs.append(name)
            if receipts['next']['status'] != 'passed':
                raise RuntimeError('exited 1')
        def release_server(ctx, profiles_module, tree=None):
            ctx.client_evidence['regressions_server'] = {'sha256': 'abc'}
            return root / 'server'
        ref = 'harness:entities:native-v2-craft-drag'
        ctx = profiles.ProfileContext(root)
        cases = [({'status': 'passed', 'details': {'x': 'abc'}}, ledger.GREEN),
                 # A failure without a negative verdict (missing root, never reached play, timeout) is
                 # not evidence about the check, in a probe or in a promotion control.
                 ({'status': 'failed', 'error': 'native client root is missing'}, ledger.UNRUNNABLE),
                 ({'status': 'failed', 'error': 'village-qa-settlements assertion failed: in_play'}, ledger.UNRUNNABLE),
                 # The same scenario id disconnecting mid-run is a flake, not a red.
                 ({'status': 'failed', 'error': 'native-v2-craft-drag assertion failed: in_play',
                   'details': {'named_scenario': {'id': 'native-v2-craft-drag'}}}, ledger.UNRUNNABLE),
                 ({'status': 'failed', 'error': 'timed out waiting for the client'}, ledger.UNRUNNABLE),
                 ({'status': 'prepared'}, ledger.UNRUNNABLE),
                 ({'status': 'passed', 'details': {}}, ledger.UNRUNNABLE)]
        registered = []
        with patch.object(profiles, '_run_command', run_command), patch.object(regressions, '_release_server', release_server), \
                patch.object(ledger, 'tree_fingerprint', return_value={'head': 'h', 'sha256': 's'}), \
                patch.object(ledger, '_register_server', lambda sha, tree: registered.append((sha, tree))):
            for receipt, state in cases:
                receipts['next'] = receipt
                self.assertEqual(ledger.run_refs(ctx, [ref], root, True)[ref]['state'], state, receipt)
            # A failed profile run naming an explicitly failing verdict is red.
            profile_ref = 'harness:village-qa:door_walk'
            receipts['next'] = {'status': 'failed', 'details': {'scenario': {'settlement_acceptance': {'behaviour': {'door_walk': 'FAIL'}}}}}
            self.assertEqual(ledger.run_refs(ctx, [profile_ref], root, True)[profile_ref]['state'], ledger.RED)
            receipts['next'] = {'status': 'failed', 'error': 'no client'}
            self.assertEqual(ledger.run_refs(ctx, [profile_ref], root, True)[profile_ref]['state'], ledger.UNRUNNABLE)
        # Every server binary the tooling builds is registered with the tree it was built from.
        self.assertEqual(registered[0], ('abc', {'head': 'h', 'sha256': 's'}))
        self.assertEqual(ledger.run_refs(ctx, [ref], root, False)[ref]['state'], ledger.NEEDS_CLIENT)
        with patch.object(regressions, '_release_server', side_effect=RuntimeError('no build')):
            self.assertEqual(ledger.run_refs(ctx, [ref], root, True)[ref]['state'], ledger.UNRUNNABLE)

    def test_client_receipts_of_a_removed_control_tree_stay_readable(self):
        # ledger promote runs its control and restore in a temporary worktree and removes it; the
        # receipt each ref records must still be there afterwards, red or green.
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        base = Path(directory.name)
        tree = base / 'ledger-control-candidate'
        receipts = {}
        def run_command(argv, ctx, name, cwd=None, env=None):
            artifact = tree / '.analysis/validation' / f'run-{len(ctx.logs)}'
            (artifact / 'world/region').mkdir(parents=True)
            (artifact / 'world/region/r.0.0.mca').write_bytes(b'region')
            (artifact / 'screenshots').mkdir()
            (artifact / 'screenshots/site.png').write_bytes(b'png')
            (artifact / 'result.json').write_text(json.dumps(receipts['next']))
            (ctx.artifact_dir / name).write_text(f'[harness] artifact={artifact}\n')
            ctx.logs.append(name)
            if receipts['next']['status'] != 'passed':
                raise RuntimeError('exited 1')
        def release_server(ctx, profiles_module, tree=None):
            ctx.client_evidence['regressions_server'] = {'sha256': 'abc'}
            return base / 'server'
        ref = 'harness:village-qa:door_walk'
        (base / 'promote').mkdir()
        ctx = profiles.ProfileContext(base / 'promote')
        passing = {'settlement_acceptance': {'behaviour': {'door_walk': 'PASS'}}}
        failing = {'settlement_acceptance': {'behaviour': {'door_walk': 'FAIL'}}}
        cases = [({'status': 'passed', 'details': {'server_binary_sha256': 'abc', 'scenario': passing}}, ledger.GREEN),
                 ({'status': 'failed', 'details': {'scenario': failing}}, ledger.RED)]
        with patch.object(profiles, '_run_command', run_command), patch.object(regressions, '_release_server', release_server), \
                patch.object(ledger, 'tree_fingerprint', return_value={'head': 'h', 'sha256': 's'}), \
                patch.object(ledger, '_register_server', lambda sha, tree: None):
            for receipt, state in cases:
                tree.mkdir()
                receipts['next'] = receipt
                entry = ledger.run_refs(ctx, [ref], tree, True)[ref]
                shutil.rmtree(tree)
                self.assertEqual(entry['state'], state)
                kept = Path(entry['receipt'])
                self.assertTrue(kept.is_relative_to(ctx.artifact_dir), kept)
                self.assertEqual(json.loads(kept.read_text())['status'], receipt['status'])
                self.assertTrue((kept.parent / 'screenshots/site.png').is_file())
                self.assertFalse((kept.parent / 'world').exists())

    def test_each_client_run_gets_its_own_client_settings(self):
        # Windowed gates inherit the caller's XDG root; a probe or promote runs many in a row and
        # must not carry client-settings.json or caches from one into the next.
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        base = Path(directory.name)
        caller = base / 'caller-xdg/krasnokamen-client'
        (caller / 'facts').mkdir(parents=True)
        (caller / 'client-settings.json').write_text('{"fov": 110}')
        seen = []
        def run_command(argv, ctx, name, cwd=None, env=None):
            client = Path(env['XDG_DATA_HOME']) / 'krasnokamen-client'
            seen.append((client, (client / 'facts').resolve(), (client / 'client-settings.json').exists()))
            (client / 'client-settings.json').write_text('{"fov": 30}')
            artifact = base / f'run-{len(ctx.logs)}'
            artifact.mkdir()
            (artifact / 'result.json').write_text(json.dumps({'status': 'passed', 'details': {
                'server_binary_sha256': 'abc', 'scenario': {'settlement_acceptance': {'behaviour': {'door_walk': 'PASS'}}}}}))
            (ctx.artifact_dir / name).write_text(f'[harness] artifact={artifact}\n')
            ctx.logs.append(name)
        def release_server(ctx, profiles_module, tree=None):
            ctx.client_evidence['regressions_server'] = {'sha256': 'abc'}
            return base / 'server'
        (base / 'promote').mkdir()
        ctx = profiles.ProfileContext(base / 'promote')
        with patch.dict(os.environ, {'XDG_DATA_HOME': str(caller.parent)}), \
                patch.object(profiles, '_run_command', run_command), patch.object(regressions, '_release_server', release_server), \
                patch.object(ledger, 'tree_fingerprint', return_value={'head': 'h', 'sha256': 's'}), \
                patch.object(ledger, '_register_server', lambda sha, tree: None):
            for _ in range(2):
                self.assertEqual(ledger.run_refs(ctx, ['harness:village-qa:door_walk'], base, True)['harness:village-qa:door_walk']['state'], ledger.GREEN)
        self.assertNotEqual(seen[0][0], seen[1][0])
        self.assertEqual([facts for _, facts, _ in seen], [(caller / 'facts').resolve()] * 2)
        self.assertEqual([settings for _, _, settings in seen], [False, False])
        self.assertEqual((caller / 'client-settings.json').read_text(), '{"fov": 110}')

    def test_a_caller_root_without_content_links_the_client_template(self):
        # A worktree's own XDG root starts empty; the run must still get the client content.
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        base = Path(directory.name)
        template = base / 'template'
        for name in ('facts', 'resources/assets', 'data/vanilla'):
            (template / name).mkdir(parents=True)
        (base / 'empty-xdg').mkdir()
        (base / 'promote').mkdir()
        ctx = profiles.ProfileContext(base / 'promote')
        env = {'XDG_DATA_HOME': str(base / 'empty-xdg'), 'KRASNOKAMEN_CLIENT_DATA_DIR': str(template / 'data/vanilla')}
        with patch.dict(os.environ, env):
            root = regressions._fresh_client_data(ctx, env, 'regressions-client-0.log')
        self.assertEqual((root / 'krasnokamen-client/facts').resolve(), (template / 'facts').resolve())

    def uncommitted_fix(self):
        """HEAD re-broken, then an uncommitted fix (its patch saved to a file) with an untracked test."""
        root = self.scratch(with_fix_commit=False)
        (root / 'src/engine.rs').write_text('value = buggy\n')
        self.git(root, 'commit', '-qam', 'regress')
        (root / 'src/engine.rs').write_text('value = fixed again\n')
        (root / 'tests/extra.rs').write_text('extra check\n')
        patch = Path(self.artifacts) / 'fix.patch'
        patch.write_text(self.git(root, 'diff', 'HEAD', '--', 'src/engine.rs') + '\n')
        return root, patch

    def record(self, root, value):
        ledger.edit(root, 'BUG', [f'fix_commit = "{value}"'])
        return ledger.row_path(root, 'BUG')

    def test_uncommitted_fix_patch_is_promoted_without_a_commit(self):
        root, fix_file = self.uncommitted_fix()
        # Unrelated uncommitted work stays in the candidate and is not reversed by the control.
        (root / 'src/util.rs').write_text('u2\n')
        self.record(root, 'patch:' + '0' * 64)
        with self.assertRaisesRegex(ledger.Refused, 'supply the patch with --fix-patch'):
            self.promote(root, self.runner())
        with self.assertRaisesRegex(ledger.Refused, r"non-test hunks are (patch:[0-9a-f]{64}), not the recorded") as refusal:
            self.promote(root, self.runner(), fix_patch=fix_file)
        recorded = re.search(r'(patch:[0-9a-f]{64}), not', str(refusal.exception)).group(1)
        ledger_file = self.record(root, recorded)
        seen = []
        def runner(ctx, refs, tree, client):
            buggy = 'buggy' in (tree / 'src/engine.rs').read_text()
            seen.append(((tree / 'src/engine.rs').read_text(), (tree / 'src/util.rs').read_text(), (tree / 'tests/extra.rs').exists()))
            return {ref: ({'state': state, 'detail': '', 'assertion': {'file': 'crates/mc-core/src/tests.rs', 'line': 1, 'error': 'panic', 'class': 'assertion'}} if state == ledger.RED else {'state': state, 'detail': ''}) for ref in refs for state in [ledger.RED if buggy else ledger.GREEN]}
        receipt = self.promote(root, runner, fix_patch=fix_file)
        self.assertEqual(seen, [('value = buggy\n', 'u2\n', True), ('value = fixed again\n', 'u2\n', True)])
        self.assertEqual((receipt['control_tree'], receipt['reversed_files'], receipt['kept_test_files']),
                         ('head', ['src/engine.rs'], []))
        self.assertEqual(receipt['fix_patch_sha256'], recorded[len('patch:'):])
        self.assertTrue(receipt['tree_dirty'])
        self.assertIn('status = "fixed"', ledger_file.read_text())
        # A patch-mode row must carry its committed patch (fatal otherwise); a re-audit reverses it.
        (root / 'docs/bugs/fix-patches').mkdir(parents=True, exist_ok=True)
        (root / 'docs/bugs/fix-patches/BUG.patch').write_text(fix_file.read_text())
        bug = tomllib.loads(ledger_file.read_text())['bug'][0]
        with patch.object(regressions, 'REPO_ROOT', root):
            self.assertEqual(regressions._promotion_errors('BUG', bug), [])

    def test_uncommitted_fix_with_a_later_uncommitted_re_break_is_refused(self):
        root, fix_file = self.uncommitted_fix()
        self.record(root, 'patch:' + '0' * 64)
        with self.assertRaisesRegex(ledger.Refused, r'(patch:[0-9a-f]{64}), not the recorded') as refusal:
            self.promote(root, self.runner(), fix_patch=fix_file)
        recorded = re.search(r'(patch:[0-9a-f]{64}), not', str(refusal.exception)).group(1)
        self.record(root, recorded)
        (root / 'src/engine.rs').write_text('value = buggy again\n')
        with self.assertRaisesRegex(ledger.Refused, 'does not apply in reverse on the candidate'):
            self.promote(root, self.runner(), fix_patch=fix_file)

    def test_a_changed_fix_patch_no_longer_matches_the_recorded_identity(self):
        root, fix_file = self.uncommitted_fix()
        self.record(root, 'patch:' + '1' * 64)
        with self.assertRaisesRegex(ledger.Refused, 'not the recorded'):
            self.promote(root, self.runner(), fix_patch=fix_file)

    def test_fix_patch_is_only_for_patch_identities(self):
        root = self.scratch()
        fix_file = Path(self.artifacts) / 'x.patch'
        fix_file.write_text('')
        with self.assertRaisesRegex(ledger.Refused, 'only for a fix_commit of the form patch'):
            self.promote(root, self.runner(), fix_patch=fix_file)

    def test_control_worktrees_have_stable_paths_and_are_removed(self):
        root = self.scratch(under_wt=True)
        trees = []
        def runner(ctx, refs, tree, client):
            trees.append(tree)
            buggy = 'buggy' in (tree / 'src/engine.rs').read_text()
            return {ref: ({'state': state, 'detail': '', 'assertion': {'file': 'crates/mc-core/src/tests.rs', 'line': 1, 'error': 'panic', 'class': 'assertion'}} if state == ledger.RED else {'state': state, 'detail': ''}) for ref in refs for state in [ledger.RED if buggy else ledger.GREEN]}
        with patch.dict(os.environ, {'KRASNOKAMEN_BUILD_TOKEN': '1'}):  # under wt/ a promote needs a held token
            self.promote(root, runner, dry_run=True)
            self.promote(root, runner, dry_run=True)
        # Same path every run: cargo reuses the build artifacts instead of growing the target dir.
        self.assertEqual(len(set(trees)), 1)
        self.assertEqual(trees[0], root.parent / 'ledger-control-candidate')
        self.assertFalse(trees[0].exists())
        self.assertEqual(self.git(root, 'worktree', 'list').count('\n'), 0)

    def test_unrunnable_control_is_not_a_red_control(self):
        root = self.scratch()
        def never_reached_play(ctx, refs, tree, client):
            return {ref: {'state': ledger.UNRUNNABLE, 'detail': 'assertion failed: in_play'} for ref in refs}
        with self.assertRaisesRegex(ledger.Refused, r'negative control \(head\) did not go red.*unrunnable'):
            self.promote(root, never_reached_play)

    def test_builds_for_another_tree_get_their_own_slot_through_mem_run(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        base = Path(directory.name)
        mem_run = base / 'mem-run.sh'
        mem_run.write_text('#!/bin/sh\n')
        tree_a, tree_b = base / 'wt/A', base / 'wt/B'
        with patch.object(regressions, 'MEM_RUN', mem_run), patch.object(regressions, 'REPO_ROOT', tree_a), \
                patch.dict('os.environ', {'CARGO_TARGET_DIR': str(base / 'targets/slot-a')}):
            argv, env = regressions.tree_build_command(tree_b, ['cargo', 'build'])
            self.assertEqual(argv, [str(mem_run), '-m', '12G', 'cargo', 'build'])
            self.assertNotIn('CARGO_TARGET_DIR', env)  # never the caller's slot, marked for tree A
            argv, env = regressions.tree_build_command(tree_a, ['cargo', 'build'])
            self.assertEqual((argv, env['CARGO_TARGET_DIR']), (['cargo', 'build'], str(base / 'targets/slot-a')))

class ReleasePinTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name) / 'tree'
        self.root.mkdir()
        for argv in (['init', '-q'], ['add', '.'], ['commit', '-qm', 'base']):
            if argv[0] == 'add':
                (self.root / 'source').write_text('candidate\n')
            subprocess.run(['git', '-C', str(self.root), *argv], check=True,
                           capture_output=True, env={**os.environ, **GIT_ENV})
        self.fingerprint = ledger.tree_fingerprint(self.root)
        self.pins = self.root.parent / 'pins'
        self.pins.mkdir()
        self.ctx = profiles.ProfileContext(self.root.parent / 'artifacts')
        self.addCleanup(patch.stopall)
        patch.object(regressions, 'REPO_ROOT', self.root).start()
        patch.object(regressions, 'PINS', self.pins).start()
        patch.object(regressions, '_host_target', return_value='x86_64-unknown-linux-gnu').start()
        patch.dict(os.environ, {'KRASNOKAMEN_SERVER_BINARY': '', 'KRASNOKAMEN_LEDGER_ROOT': ''}).start()
        self.build = patch.object(profiles, '_run_command',
                                  side_effect=AssertionError('must never build a release server')).start()

    def pin(self, *, register=True, name='candidate-server', binary_name='krasnokamen',
            target='x86_64-unknown-linux-gnu', drop=()):
        directory = self.pins / name
        directory.mkdir()
        binary = directory / binary_name
        binary.write_bytes(f'candidate {target}'.encode())
        binary.chmod(0o555)
        digest = hashlib.sha256(binary.read_bytes()).hexdigest()
        manifest = {
            'repo': 'core', 'kind': 'server', 'head': self.fingerprint['head'], 'target': target,
            'diff_sha256': 'e' * 64, 'inputs_sha256': 'i' * 64, 'env_deps': {},
            'binary': str(binary), 'binary_sha256': digest,
        }
        for key in drop:
            del manifest[key]
        (directory / 'manifest.json').write_text(json.dumps(manifest))
        if register:
            registry = self.root / '.analysis/ledger-builds'
            registry.mkdir(parents=True, exist_ok=True)
            (registry / f'{digest}.json').write_text(json.dumps({
                'sha256': digest,
                'tree': {key: self.fingerprint[key] for key in ('head', 'sha256')},
            }))
        return binary

    def test_pin_is_accepted_for_its_registered_tree_without_building(self):
        binary = self.pin()
        self.assertEqual(regressions._release_server(self.ctx, profiles), binary)
        self.build.assert_not_called()
        evidence = self.ctx.client_evidence['regressions_server']
        self.assertEqual(evidence['sha256'], hashlib.sha256(b'candidate x86_64-unknown-linux-gnu').hexdigest())
        self.assertEqual(evidence['tree'], self.fingerprint)

    def test_explicit_pin_is_accepted_for_its_registered_tree(self):
        binary = self.pin()
        with patch.dict(os.environ, {'KRASNOKAMEN_SERVER_BINARY': str(binary)}):
            self.assertEqual(regressions._release_server(self.ctx, profiles), binary)
        self.build.assert_not_called()

    def test_pin_for_another_target_of_the_same_tree_is_never_selected(self):
        # Sorted discovery meets the Windows pin of the same tree first ('pc' < 'unknown').
        self.pin(name='a-server-x86_64-pc-windows-gnu', binary_name='krasnokamen.exe', target='x86_64-pc-windows-gnu')
        linux = self.pin(name='b-server-x86_64-unknown-linux-gnu')
        self.assertEqual(regressions._release_server(self.ctx, profiles), linux)

    def test_explicit_pin_for_another_target_is_refused(self):
        windows = self.pin(binary_name='krasnokamen.exe', target='x86_64-pc-windows-gnu')
        with patch.dict(os.environ, {'KRASNOKAMEN_SERVER_BINARY': str(windows)}):
            with self.assertRaisesRegex(RuntimeError, "target 'x86_64-pc-windows-gnu', not this host"):
                regressions._release_server(self.ctx, profiles)
        self.build.assert_not_called()

    def test_pin_without_build_provenance_is_refused(self):
        for key in ('diff_sha256', 'inputs_sha256', 'env_deps'):
            with self.subTest(missing=key):
                binary = self.pin(name=f'pin-without-{key}', drop=(key,))
                with patch.dict(os.environ, {'KRASNOKAMEN_SERVER_BINARY': str(binary)}):
                    with self.assertRaisesRegex(RuntimeError, f'records no build provenance \\({key}\\)'):
                        regressions._release_server(self.ctx, profiles)
        self.build.assert_not_called()

    def test_relabelled_pin_is_refused(self):
        for name, origin in (('missing-source', None), ('other-tree-source', {'head': 'f' * 40, 'diff_sha256': 'e' * 64})):
            with self.subTest(source=name):
                if origin is not None:
                    (self.pins / name).mkdir()
                    (self.pins / name / 'manifest.json').write_text(json.dumps(origin))
                binary = self.pin(name=f'relabelled-from-{name}')
                manifest = binary.parent / 'manifest.json'
                manifest.write_text(json.dumps({**json.loads(manifest.read_text()), 'reused_from': name}))
                with patch.dict(os.environ, {'KRASNOKAMEN_SERVER_BINARY': str(binary)}):
                    with self.assertRaisesRegex(RuntimeError, f'relabelled pin of {name}|{name}/manifest.json'):
                        regressions._release_server(self.ctx, profiles)
        self.build.assert_not_called()

    def test_pin_reused_from_the_same_working_state_is_accepted(self):
        (self.pins / 'source-pin').mkdir()
        (self.pins / 'source-pin' / 'manifest.json').write_text(json.dumps(
            {'head': self.fingerprint['head'], 'diff_sha256': 'e' * 64}))
        binary = self.pin()
        manifest = binary.parent / 'manifest.json'
        manifest.write_text(json.dumps({**json.loads(manifest.read_text()), 'reused_from': 'source-pin'}))
        self.assertEqual(regressions._release_server(self.ctx, profiles), binary)

    def test_pin_of_another_tree_is_refused(self):
        binary = self.pin()
        (self.root / 'source').write_text('reverted control\n')
        with patch.dict(os.environ, {'KRASNOKAMEN_SERVER_BINARY': str(binary)}):
            with self.assertRaisesRegex(RuntimeError, 'different tree.*pin-release'):
                regressions._release_server(self.ctx, profiles)
        self.build.assert_not_called()

    def test_missing_pin_is_refused_and_names_exact_pin_release_command(self):
        with self.assertRaisesRegex(RuntimeError, 'no registered pin-release server') as failure:
            regressions._release_server(self.ctx, profiles)
        self.assertIn(f'`{regressions.MEM_RUN.with_name("pin-release")} {self.root}`', str(failure.exception))
        self.assertIn('negative control needs its own reverted-tree pin', str(failure.exception))
        self.build.assert_not_called()

    def test_explicit_unregistered_pin_is_refused_without_building(self):
        binary = self.pin(register=False)
        with patch.dict(os.environ, {'KRASNOKAMEN_SERVER_BINARY': str(binary)}):
            with self.assertRaisesRegex(RuntimeError, 'cannot use KRASNOKAMEN_SERVER_BINARY.*pin-release'):
                regressions._release_server(self.ctx, profiles)
        self.build.assert_not_called()

    def test_another_tree_cannot_use_the_callers_pin(self):
        binary = self.pin()
        other = self.root.parent / 'control'
        subprocess.run(['git', 'clone', '-q', str(self.root), str(other)], check=True, capture_output=True)
        (other / 'source').write_text('control\n')
        with patch.dict(os.environ, {
                'KRASNOKAMEN_SERVER_BINARY': str(binary), 'KRASNOKAMEN_LEDGER_ROOT': str(self.root)}):
            with self.assertRaisesRegex(RuntimeError, 'different tree.*pin-release') as failure:
                regressions._release_server(self.ctx, profiles, other)
        self.assertIn(str(other), str(failure.exception))
        self.build.assert_not_called()

    def test_negative_control_accepts_only_its_own_registered_pin(self):
        other = self.root.parent / 'control'
        subprocess.run(['git', 'clone', '-q', str(self.root), str(other)], check=True, capture_output=True)
        (other / 'source').write_text('reverted control\n')
        self.root = other
        self.fingerprint = ledger.tree_fingerprint(other)
        binary = self.pin()
        self.assertEqual(regressions._release_server(self.ctx, profiles, other), binary)
        self.build.assert_not_called()

    def test_changed_pin_bytes_are_refused(self):
        binary = self.pin()
        binary.chmod(0o755)
        binary.write_bytes(b'not the pinned server')
        with patch.dict(os.environ, {'KRASNOKAMEN_SERVER_BINARY': str(binary)}):
            with self.assertRaisesRegex(RuntimeError, 'checksum mismatch.*pin-release'):
                regressions._release_server(self.ctx, profiles)
        self.build.assert_not_called()
