"""Lifecycle phase evidence from real shared helpers."""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

from . import profiles, runtime
from .backends import buildings_qa, seed_review
from .backends.probe_steps import ProbeSteps


class LifecyclePhases(unittest.TestCase):
    def test_unlaunchable_server_is_server_start(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(FileNotFoundError) as caught:
                runtime.start_server([str(Path(directory) / 'missing')])
            self.assertEqual(runtime.observed_phase(caught.exception), 'server_start')

    def test_default_bucket_run_uses_private_display(self):
        with tempfile.TemporaryDirectory() as directory:
            ctx = profiles.ProfileContext(Path(directory))
            display_log = Mock(name='display_log')
            display_log.name = str(Path(directory) / 'xvfb.log')
            with patch.object(profiles, 'start_xvfb', return_value=(object(), ':456', display_log)) as start, \
                 patch.object(profiles, '_run_command', return_value={}), \
                 patch.object(profiles, 'stop_process'):
                profiles.run_bucket_resync(ctx)
            start.assert_called_once_with(ctx.artifact_dir)

    def test_seed_review_propagates_nested_failure_phase(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'seed-review').mkdir()
            (root / 'seed-review' / 'result.json').write_text('{"failure_phase":"join"}')
            failure = runtime.ChildExited('backend failed', 1, root / 'seed-review.log')
            with patch.object(profiles, '_run_command', side_effect=failure):
                with self.assertRaises(runtime.ChildExited) as caught:
                    profiles.run_seed_review(profiles.ProfileContext(root))
            self.assertEqual(runtime.observed_phase(caught.exception), 'join')

    def test_seed_review_records_failed_display_start(self):
        with tempfile.TemporaryDirectory(dir=runtime.REPO_ROOT) as directory:
            root = Path(directory)
            config = root / 'config.toml'
            config.write_text('')
            with patch.dict('os.environ', {'SOLARIS_VALIDATION_RUN_DIR': directory,
                                          'SOLARIS_SERVER_BINARY': '/bin/true'}), \
                 patch.object(seed_review, 'write_config', return_value=(config, root / 'world')), \
                 patch.object(seed_review, 'prepare_operator_fixture'), \
                 patch.object(runtime, 'start_xvfb', side_effect=RuntimeError('display failed')):
                with self.assertRaisesRegex(RuntimeError, 'display failed'):
                    seed_review.run(10)
            import json
            result = json.loads((root / 'seed-review' / 'result.json').read_text())
            self.assertEqual(result['failure_phase'], 'client_start')

    def test_seed_review_records_teardown_failure(self):
        with tempfile.TemporaryDirectory(dir=runtime.REPO_ROOT) as directory:
            root = Path(directory)
            config = root / 'config.toml'
            config.write_text('')
            with patch.dict('os.environ', {'SOLARIS_VALIDATION_RUN_DIR': directory,
                                          'SOLARIS_SERVER_BINARY': '/bin/true'}), \
                 patch.object(seed_review, 'write_config', return_value=(config, root / 'world')), \
                 patch.object(seed_review, 'prepare_operator_fixture'), \
                 patch.object(runtime, 'start_xvfb', side_effect=RuntimeError('display failed')), \
                 patch.object(runtime, 'stop_process', side_effect=RuntimeError('stop failed')):
                with self.assertRaisesRegex(RuntimeError, 'stop failed'):
                    seed_review.run(10)
            import json
            result = json.loads((root / 'seed-review' / 'result.json').read_text())
            self.assertEqual(result['failure_phase'], 'teardown')
            self.assertFalse(result['passed'])

    def test_server_and_client_readiness_exits(self):
        with tempfile.TemporaryDirectory() as directory:
            for phase in ('server_start', 'client_start'):
                with self.subTest(phase=phase):
                    child = subprocess.Popen(['/bin/sh', '-c', 'exit 23'])
                    with self.assertRaises(runtime.ChildExited) as caught:
                        runtime.wait_port(1, 5, child, log_path=Path(directory) / 'child.log', phase_name=phase)
                    self.assertEqual(runtime.observed_phase(caught.exception), phase)
                    self.assertEqual(runtime.child_exit(caught.exception).returncode, 23)

    def test_java_client_exit_during_start_is_client_start(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            wrapper = root / 'gradlew'
            wrapper.write_text('#!/bin/sh\nexit 37\n')
            wrapper.chmod(0o755)
            with patch.dict(os.environ, {'SOLARIS_LOADER_ROOT': str(root)}), \
                    patch('tools.harness.audio.private_audio_preflight'):
                with self.assertRaises(runtime.ChildExited) as caught:
                    runtime.start_client(game_dir=root / 'game', log_path=root / 'java.log',
                                         token='test', mcp_port=runtime.reserve_port(),
                                         username='Test', display=':999')
            self.assertEqual(runtime.observed_phase(caught.exception), 'client_start')
            self.assertEqual(caught.exception.returncode, 37)

    def test_stop_failure_is_teardown(self):
        process = SimpleNamespace(pid=1, poll=lambda: None)
        with patch.object(runtime.os, 'killpg', side_effect=OSError('cannot stop')):
            with self.assertRaises(OSError) as caught:
                runtime.stop_process(process)
        self.assertEqual(runtime.observed_phase(caught.exception), 'teardown')

    def test_scenario_assertion_and_join(self):
        class Client:
            def call_tool(self, name, args):
                raise RuntimeError('failed to join')
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(RuntimeError) as caught:
                with ProbeSteps(Client(), Path(directory), 5, 'test') as probe:
                    probe.check('condition', False)
            self.assertEqual(runtime.observed_phase(caught.exception), 'assertion')
            with self.assertRaises(RuntimeError) as caught:
                with ProbeSteps(Client(), Path(directory), 5, 'test') as probe:
                    probe.call('minecraft_wait_for_play', {})
            self.assertEqual(runtime.observed_phase(caught.exception), 'join')

    def test_building_retry_splits_long_wait_into_valid_requests(self):
        class Probe:
            def __init__(self):
                self.ticks = []
            def call(self, name, args):
                self.ticks.append(args['ticks'])
        probe = Probe()
        with patch.object(buildings_qa, '_send', return_value={}), patch.object(buildings_qa, '_chat_lines', return_value=[]):
            self.assertFalse(buildings_qa._order_echo_retry(probe, {}, 'tag', 'bp', 1, 'order', False, SimpleNamespace(windowed=False)))
        self.assertEqual(probe.ticks, [20, 255, 255, 255, 255, 180] * 3)
