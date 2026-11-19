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

    def test_owner_order_d8_control_runs_the_unchanged_parallel_gate(self):
        # The control changes MAX_PROJECTS on its own tree/pin, not this argv.
        command = regressions._profile_command('village-owner-order-qa', 'two_projects_advance')
        self.assertEqual(command, regressions._profile_command('village-owner-order-qa', 'all'))

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

    def test_new_kinds_and_level_three_verdicts_are_registrable(self):
        for verdict in ('gate.L1_built', 'gate.L2_upgrade', 'gate.L3_upgrade', 'wall.L3_upgrade',
                        'watch_tower.L3_upgrade', 'barracks.L3_upgrade', 'armorer.L2_upgrade',
                        'weaponsmith.L1_built'):
            command = regressions._profile_command('buildings-qa', verdict)
            self.assertEqual(command[:2], ['env', f"BUILDINGS_QA_SELECT={verdict.split('.')[0]}"])
        command = regressions._profile_command('buildings-qa', 'house.broken_cell_replaced')
        self.assertEqual(command[:2], ['env', 'BUILDINGS_QA_SELECT=house'])
        for verdict in ('armorer.L3_upgrade', 'house.L4_upgrade', 'storage.L3_upgrade',
                        'wall.broken_cell_replaced'):
            with self.assertRaises(ValueError):
                regressions._profile_command('buildings-qa', verdict)

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

    def test_nextest_listing_accepts_compiler_output(self):
        result = regressions._nextest_listing('Compiling example\\n{"rust-suites": {}}\\n')
        self.assertEqual(result['rust-suites'], {})

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

    def test_assertion_failure_is_attributed_to_its_bug(self):
        cargo = {'mc-core': {'module::broken': {'BROKEN'}, 'module::other': {'OTHER'}}}
        log = '        FAIL [   0.120s] (1/2) mc-core module::broken\n'
        self.assertEqual(regressions._failed_ids(log, cargo), {'BROKEN'})

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
        self.assertEqual(result[('client', 'unit::test')], (ledger.GREEN, ''))
        self.assertEqual(calls[-1], ['cargo', 'test', '-p', 'client', '--lib', '--', 'unit::test'])

    def test_client_bin_only_package_uses_named_binary_target(self):
        package = {'name': 'krasnokamen-launcher', 'targets': [
            {'name': 'krasnokamen-launcher', 'kind': ['bin']}]}
        result, calls = self.run_client_packages([package], [('krasnokamen-launcher', 'bootstrap::tests')])
        self.assertEqual(result[('krasnokamen-launcher', 'bootstrap::tests')], (ledger.GREEN, ''))
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
                state, detail = result[('launcher', 'test')]
                self.assertEqual(state, ledger.UNRUNNABLE)
                self.assertIn('launcher', detail)
                self.assertEqual(len(calls), 1)  # metadata only, never guessed cargo test

    def test_client_bin_ref_cannot_pass_by_selecting_no_test(self):
        package = {'name': 'launcher', 'targets': [{'name': 'launcher', 'kind': ['bin']}]}
        result, _ = self.run_client_packages([package], [('launcher', 'missing')],
                                             summary='test result: ok. 0 passed; 0 failed')
        self.assertEqual(result[('launcher', 'missing')], (ledger.UNRUNNABLE, 'ran no matching test'))


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
            return {ref: {'state': state, 'detail': ''} for ref in refs}
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
            return {ref: {'state': ledger.RED if buggy else ledger.GREEN, 'detail': ''} for ref in refs}
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
            return {ref: {'state': ledger.RED, 'detail': 'boom'} for ref in refs}
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
            out['cargo:mc-core:mod::t'] = {'state': ledger.RED, 'detail': 'FAIL'}
            return out
        rows = ledger.probe(bugs, ['BUG', 'EMPTY', 'BAD'], client=False, ctx=ctx, runner=runner)
        self.assertEqual(rows['BUG']['verdict'], ledger.NEEDS_CLIENT)
        self.assertEqual(rows['EMPTY']['verdict'], ledger.UNRUNNABLE)
        self.assertEqual(ledger.format_row('EMPTY', rows['EMPTY']), 'EMPTY: unrunnable(no registered check)')
        self.assertEqual(rows['BAD']['verdict'], ledger.RED)
        self.assertIn('FAIL', rows['BAD']['reason'])
        with self.assertRaisesRegex(ledger.Refused, 'unknown ledger id'):
            ledger.probe(bugs, ['NOPE'], client=False, ctx=ctx, runner=runner)

    def test_cargo_results_are_parsed_per_test(self):
        listing = {'rust-suites': {'lib': {'package-name': 'mc-core', 'testcases': {
            'a::ok': {'filter-match': {'status': 'matches'}}, 'a::bad': {'filter-match': {'status': 'matches'}},
            'a::ignored': {'filter-match': {'status': 'matches'}, 'ignored': True}}}}}
        def run_command(argv, ctx, name, cwd=None, env=None):
            text = json.dumps(listing) if argv[2] == 'list' else (
                '        PASS [   0.010s] (1/2) mc-core a::ok\n        FAIL [   0.020s] (2/2) mc-core a::bad\n')
            (ctx.artifact_dir / name).write_text(text)
            ctx.logs.append(name)
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        ctx = profiles.ProfileContext(Path(directory.name))
        tests = {('mc-core', 'a::ok'), ('mc-core', 'a::bad'), ('mc-core', 'a::ignored'), ('mc-core', 'a::missing')}
        with patch.object(profiles, '_run_command', run_command), patch.object(regressions, 'MEM_RUN', Path('/nonexistent/mem-run.sh')):
            results = ledger._run_cargo(ctx, tests, Path('.'))
        self.assertEqual(results[('mc-core', 'a::ok')][0], ledger.GREEN)
        self.assertEqual(results[('mc-core', 'a::bad')], (ledger.RED, 'FAIL'))
        self.assertEqual(results[('mc-core', 'a::ignored')][0], ledger.UNRUNNABLE)
        self.assertEqual(results[('mc-core', 'a::missing')][0], ledger.UNRUNNABLE)

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
            return {ref: {'state': ledger.RED if buggy else ledger.GREEN, 'detail': ''} for ref in refs}
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
            return {ref: {'state': ledger.RED if buggy else ledger.GREEN, 'detail': ''} for ref in refs}
        self.promote(root, runner, dry_run=True)
        self.assertEqual(seen, [True, True])

    def test_control_worktrees_live_beside_the_other_worktrees(self):
        self.assertEqual(ledger._worktree_parent(Path('/work/wt/LedgerTriage')), Path('/work/wt'))
        self.assertIsNone(ledger._worktree_parent(Path('/home/user/solaris')))

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
            return {ref: {'state': ledger.RED if buggy else ledger.GREEN, 'detail': ''} for ref in refs}
        receipt = self.promote(root, runner, fix_patch=fix_file)
        self.assertEqual(seen, [('value = buggy\n', 'u2\n', True), ('value = fixed again\n', 'u2\n', True)])
        self.assertEqual((receipt['control_tree'], receipt['reversed_files'], receipt['kept_test_files']),
                         ('head', ['src/engine.rs'], []))
        self.assertEqual(receipt['fix_patch_sha256'], recorded[len('patch:'):])
        self.assertTrue(receipt['tree_dirty'])
        self.assertIn('status = "fixed"', ledger_file.read_text())
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
            return {ref: {'state': ledger.RED if buggy else ledger.GREEN, 'detail': ''} for ref in refs}
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
