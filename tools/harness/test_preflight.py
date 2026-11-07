"""Regression checks for pre-launch refusal and failure receipts."""
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from . import __main__ as cli, profiles, runtime


class PreflightTests(unittest.TestCase):
    def test_all_profiles_declare_prerequisites(self):
        self.assertEqual(set(profiles.PROFILES), set(profiles.PROFILE_PREREQUISITES))
        for name, spec in profiles.PROFILES.items():
            with self.subTest(profile=name):
                self.assertEqual(spec['prerequisites'], profiles.PROFILE_PREREQUISITES[name])
                self.assertEqual(len(spec['prerequisites']), 4)
        with self.assertRaises(KeyError):
            profiles.prerequisites({}, profiles.ProfileContext(Path('/tmp')))

    def test_protocol_only_and_building_profiles_do_not_require_client(self):
        for name in ('pause-empty', 'item-merge', 'graves'):
            ctx = profiles.ProfileContext(Path('/tmp'), client='java')
            needs = profiles.prerequisites(profiles.PROFILES[name], ctx)
            self.assertNotIn('client_binary', needs)
            self.assertNotIn('client_data', needs)
            self.assertNotIn('loader', needs)
        ctx = profiles.ProfileContext(Path('/tmp'))
        self.assertNotIn('server_binary', profiles.prerequisites(profiles.PROFILES['regressions-client'], ctx))

    def test_regression_preparation_does_not_require_a_client(self):
        for mode in ('--prepare', '--validate-run'):
            ctx = profiles.ProfileContext(Path('/tmp'), extra_args=[mode])
            self.assertNotIn('loader', profiles.prerequisites(profiles.PROFILES['regression'], ctx))
        ctx = profiles.ProfileContext(Path('/tmp'), extra_args=['--check'])
        self.assertEqual(profiles.prerequisites(profiles.PROFILES['regression'], ctx), {'loader'})
        ctx = profiles.ProfileContext(Path('/tmp'))
        self.assertIn('loader', profiles.prerequisites(profiles.PROFILES['regression'], ctx))

    def test_invalid_arguments_are_rejected_before_launch(self):
        from . import preflight
        for name in preflight._ARGUMENT_VALIDATORS:
            with self.subTest(profile=name), tempfile.TemporaryDirectory() as tmp:
                ctx = profiles.ProfileContext(Path(tmp), client='native', windowed=True,
                                              timeout_seconds=-1)
                with mock.patch('subprocess.Popen') as launch:
                    with self.assertRaises((ValueError, RuntimeError)):
                        preflight.check(name, ctx, profiles.PROFILES[name])
                    launch.assert_not_called()

    def test_subprocess_backend_parsers_reject_invalid_inputs_in_preflight(self):
        from . import preflight
        cases = {
            'catalog-load': ['--clients', '0'],
            'quirk-parity': ['--fuzz-style', 'unknown'],
            'seed-review': ['--seed', 'not-an-integer'],
            'seed-contact-sheet': [],
        }
        for name, arguments in cases.items():
            with self.subTest(profile=name), tempfile.TemporaryDirectory() as tmp:
                ctx = profiles.ProfileContext(Path(tmp), extra_args=arguments)
                with mock.patch('subprocess.Popen') as launch:
                    with self.assertRaises(ValueError):
                        preflight.check(name, ctx, profiles.PROFILES[name])
                    launch.assert_not_called()

    def test_seed_review_rejects_invalid_numeric_arguments_before_launch(self):
        from . import preflight
        for arguments in (['--timeout-seconds', '0'], ['--timeout-seconds', 'nan'],
                          ['--vanilla-village', '0', 'inf', '0']):
            with self.subTest(arguments=arguments), tempfile.TemporaryDirectory() as tmp:
                ctx = profiles.ProfileContext(Path(tmp), extra_args=arguments)
                with mock.patch('subprocess.Popen') as launch, self.assertRaisesRegex(ValueError, 'invalid seed-review arguments'):
                    preflight.check('seed-review', ctx, profiles.PROFILES['seed-review'])
                launch.assert_not_called()

    def test_loader_and_map_invalid_arguments_fail_in_preflight(self):
        from . import preflight
        with tempfile.TemporaryDirectory() as tmp:
            for name, ctx in (
                ('loader-live', profiles.ProfileContext(Path(tmp), platform='invalid')),
                ('settlement-map-qa', profiles.ProfileContext(
                    Path(tmp), client='native', windowed=True, timeout_seconds=-1)),
            ):
                with self.subTest(profile=name), mock.patch('subprocess.Popen') as launch:
                    with self.assertRaises((ValueError, RuntimeError)):
                        preflight.check(name, ctx, profiles.PROFILES[name])
                    launch.assert_not_called()

    def test_regression_profiles_reject_fractional_timeouts_before_launch(self):
        from . import preflight
        with tempfile.TemporaryDirectory() as tmp:
            for name in ('regression', 'playable', 'replay'):
                ctx = profiles.ProfileContext(Path(tmp), timeout_seconds=1.5)
                with self.subTest(profile=name), mock.patch('subprocess.Popen') as launch:
                    with self.assertRaises(ValueError):
                        preflight.check(name, ctx, profiles.PROFILES[name])
                    launch.assert_not_called()

    def test_singleplayer_zero_timeout_fails_before_launch(self):
        from . import preflight
        with tempfile.TemporaryDirectory() as tmp:
            ctx = profiles.ProfileContext(Path(tmp), client='native', windowed=True,
                                          scenario_id='native-world-first-start-interrupted',
                                          timeout_seconds=0)
            with mock.patch('subprocess.Popen') as launch:
                with self.assertRaises(ValueError):
                    preflight.check('singleplayer-regressions', ctx,
                                    profiles.PROFILES['singleplayer-regressions'])
                launch.assert_not_called()

    def test_missing_native_data_refuses_before_profile(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(cli, 'ARTIFACT_ROOT', Path(tmp)):
            env = {'SOLARIS_CLIENT_DATA_DIR': str(Path(tmp) / 'missing'),
                   'SOLARIS_SERVER_BINARY': '/bin/true', 'SOLARIS_CLIENT_BINARY': '/bin/true'}
            with mock.patch.dict(os.environ, env, clear=True), mock.patch.dict(profiles.PROFILES['survival'], {'run': mock.Mock()}):
                self.assertEqual(cli.cmd_run(['survival', '--client', 'native']), 1)
                profiles.PROFILES['survival']['run'].assert_not_called()
            receipt = json.loads(next(Path(tmp).glob('*/result.json')).read_text())
            self.assertIn('SOLARIS_CLIENT_DATA_DIR', receipt['error'])
            self.assertEqual(receipt['details']['failure']['phase'], 'preflight')

    def test_invalid_scene_step_refused_before_profile(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(cli, 'ARTIFACT_ROOT', Path(tmp)):
            scenario = Path(tmp) / 'scene.json'
            scenario.write_text(json.dumps({'steps': [{'tp': [1, 2]}]}))
            with mock.patch.dict(profiles.PROFILES['scene-shot'], {'run': mock.Mock(return_value={})}):
                self.assertEqual(cli.cmd_run(['scene-shot', '--client', 'native', '--windowed', str(scenario)]), 1)
                profiles.PROFILES['scene-shot']['run'].assert_not_called()
            receipt = json.loads(next(Path(tmp).glob('*/result.json')).read_text())
            self.assertIn('invalid scene step', receipt['error'])
            self.assertEqual(receipt['details']['failure']['phase'], 'preflight')

    def test_pooled_binary_is_refused_even_through_symlink(self):
        from . import preflight
        with tempfile.TemporaryDirectory() as tmp:
            pool = Path(tmp) / 'targets'
            binary = pool / 'slot-a' / 'binary'
            binary.parent.mkdir(parents=True)
            binary.write_text('#!/bin/sh\n')
            binary.chmod(0o755)
            link = Path(tmp) / 'pinned'
            link.symlink_to(binary)
            with mock.patch.object(preflight, '_POOL', pool):
                with self.assertRaisesRegex(ValueError, 'pooled target'):
                    preflight._binary('SOLARIS_SERVER_BINARY', str(link), {}, 'server_binary')

    def test_headless_root_links_template_and_restores_environment(self):
        from . import preflight
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / 'template' / 'data' / 'vanilla'
            data.mkdir(parents=True)
            (root / 'template' / 'facts').mkdir()
            (root / 'template' / 'resources' / 'assets').mkdir(parents=True)
            ctx = profiles.ProfileContext(root, client='native')
            with mock.patch.dict(os.environ, {'SOLARIS_CLIENT_DATA_DIR': str(data),
                                             'XDG_DATA_HOME': '/original'}):
                with preflight.client_data_root(ctx):
                    xdg = Path(os.environ['XDG_DATA_HOME'])
                    self.assertTrue((xdg / 'solaris-client' / 'facts').is_symlink())
                    self.assertEqual((xdg / 'solaris-client' / 'resources').resolve(),
                                     (root / 'template' / 'resources').resolve())
                self.assertFalse(xdg.exists())
                self.assertEqual(os.environ['XDG_DATA_HOME'], '/original')

    def test_explicit_nested_phase_is_preserved_in_receipt(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(cli, 'ARTIFACT_ROOT', Path(tmp)):
            def run(ctx):
                with runtime.phase('scenario'):
                    with runtime.phase('assertion'):
                        raise RuntimeError('assertion failed')
            with mock.patch.dict(profiles.PROFILES['code-health'], {'run': run}):
                self.assertEqual(cli.cmd_run(['code-health']), 1)
            receipt = json.loads(next(Path(tmp).glob('*/result.json')).read_text())
            self.assertEqual(receipt['details']['failure']['phase'], 'assertion')

    def test_server_exit_is_recorded(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(cli, 'ARTIFACT_ROOT', Path(tmp)):
            def run(ctx):
                import subprocess
                child = subprocess.Popen(['/bin/sh', '-c', 'exit 23'])
                runtime.wait_port(1, 5, child, log_path=Path(tmp) / 'server.log')
            with mock.patch.dict(profiles.PROFILES['code-health'], {'run': run}):
                self.assertEqual(cli.cmd_run(['code-health']), 1)
            receipt = json.loads(next(Path(tmp).glob('*/result.json')).read_text())
            self.assertEqual(receipt['details']['failure']['phase'], 'server_start')
            self.assertEqual(receipt['details']['failure']['exit_code'], 23)
