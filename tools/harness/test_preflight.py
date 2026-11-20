"""Regression checks for pre-launch refusal and failure receipts."""
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from . import __main__ as cli, profiles, runtime


class PreflightTests(unittest.TestCase):
    def test_missing_native_data_refuses_before_profile(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(cli, 'ARTIFACT_ROOT', Path(tmp)):
            env = {'KRASNOKAMEN_CLIENT_DATA_DIR': str(Path(tmp) / 'missing'),
                   'KRASNOKAMEN_SERVER_BINARY': '/bin/true', 'KRASNOKAMEN_CLIENT_BINARY': '/bin/true'}
            with mock.patch.dict(os.environ, env, clear=True), mock.patch.dict(profiles.PROFILES['survival'], {'run': mock.Mock()}):
                self.assertEqual(cli.cmd_run(['survival', '--client', 'native']), 1)
                profiles.PROFILES['survival']['run'].assert_not_called()
            receipt = json.loads(next(Path(tmp).glob('*/result.json')).read_text())
            self.assertIn('KRASNOKAMEN_CLIENT_DATA_DIR', receipt['error'])
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
                    preflight._binary('KRASNOKAMEN_SERVER_BINARY', str(link), {}, 'server_binary')

    def test_headless_root_links_template_and_restores_environment(self):
        from . import preflight
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / 'template' / 'data' / 'vanilla'
            data.mkdir(parents=True)
            (root / 'template' / 'facts').mkdir()
            (root / 'template' / 'resources' / 'assets').mkdir(parents=True)
            ctx = profiles.ProfileContext(root, client='native')
            with mock.patch.dict(os.environ, {'KRASNOKAMEN_CLIENT_DATA_DIR': str(data),
                                             'XDG_DATA_HOME': '/original'}):
                with preflight.client_data_root(ctx):
                    xdg = Path(os.environ['XDG_DATA_HOME'])
                    self.assertTrue((xdg / 'krasnokamen-client' / 'facts').is_symlink())
                    self.assertEqual((xdg / 'krasnokamen-client' / 'resources').resolve(),
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
