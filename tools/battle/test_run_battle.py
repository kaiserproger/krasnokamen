import hashlib
import importlib.util
import json
import queue
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path


class BattleRunnerTests(unittest.TestCase):
    def test_dry_run_shows_commands_without_creating_run(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            script = Path(__file__).with_name('run-battle.sh')
            run = root / 'run'
            result = subprocess.run([str(script), '--dry-run', '--pin', str(root / 'pin'),
                                     '--client', str(root / 'client'), '--client-data', str(root / 'client-data'),
                                     '--cache', str(root / 'cache'), '--run-dir', str(run),
                                     '--seed', '17', '--scenario', 'field-1k', '--count', '250',
                                     '--paused-ticks', '1200', '--minimum-battle-ticks', '1200'],
                                    text=True, capture_output=True, check=True)
            self.assertIn('battle setup 17 field-1k 250', result.stdout)
            self.assertIn('battle start', result.stdout)
            self.assertIn('battle pause', result.stdout)
            self.assertIn('telemetry.jsonl', result.stdout)
            self.assertIn('recorder record', result.stdout)
            self.assertIn('battle roster', result.stdout)
            self.assertIn('--roster-file', result.stdout)
            self.assertIn('--roster-ready-file', result.stdout)
            self.assertIn('--roster-timeout 900', result.stdout)
            self.assertIn('wait at least 1200 paused ticks', result.stdout)
            self.assertIn('require at least 1200 continuously recorded battle ticks', result.stdout)
            self.assertLess(result.stdout.index('recorder record'), result.stdout.index('battle setup'))
            self.assertLess(result.stdout.index('battle roster'), result.stdout.index('battle start'))
            self.assertFalse(run.exists())

    def test_invalid_count_fails_without_starting(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            script = Path(__file__).with_name('run-battle.sh')
            result = subprocess.run([str(script), '--dry-run', '--pin', str(root / 'pin'),
                                     '--client', str(root / 'client'), '--client-data', str(root / 'client-data'),
                                     '--cache', str(root / 'cache'), '--run-dir', str(root / 'run'),
                                     '--count', '10001'], text=True, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((root / 'run').exists())

    def test_early_server_exit_writes_failed_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pin = root / 'pin'
            pin.mkdir()
            binary = pin / 'server'
            binary.write_text('#!/bin/sh\nexit 1\n')
            binary.chmod(0o755)
            (pin / 'manifest.json').write_text(json.dumps({
                'kind': 'server', 'binary': str(binary),
                'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'head': 'test'
            }))
            client = root / 'client'
            client.write_text('test')
            for name in ('client-data', 'cache'):
                (root / name).mkdir()
            run = root / 'run'
            result = subprocess.run([str(Path(__file__).with_name('run-battle.sh')),
                                     '--pin', str(pin), '--client', str(client),
                                     '--client-data', str(root / 'client-data'), '--cache', str(root / 'cache'),
                                     '--run-dir', str(run)], text=True, capture_output=True, timeout=30)
            self.assertNotEqual(result.returncode, 0)
            receipt = json.loads((run / 'result.json').read_text())
            self.assertEqual(receipt['status'], 'failed')
            self.assertIn('server exited', receipt['error'])

    def test_paused_ladder_waits_for_tick_span(self):
        spec = importlib.util.spec_from_file_location('runner', Path(__file__).with_name('run-battle.py'))
        runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runner)
        drained = threading.Event()
        class TickQueue(queue.Queue):
            def get_nowait(self):
                try:
                    return super().get_nowait()
                except queue.Empty:
                    drained.set()
                    raise
        class Recorder:
            def poll(self):
                return None
        ticks = TickQueue()
        def publish():
            drained.wait()
            for tick in (100, 200, 600, 1400):
                ticks.put(tick)
        producer = threading.Thread(target=publish)
        producer.start()
        self.assertEqual(runner.wait_paused_ticks(ticks, queue.Queue(), Recorder(), 1200, 150), (200, 1400))
        producer.join()

    def test_population_barrier_uses_fresh_fully_active_status(self):
        spec = importlib.util.spec_from_file_location('runner', Path(__file__).with_name('run-battle.py'))
        runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runner)
        class Recorder:
            def poll(self):
                return None
        messages = queue.Queue()
        messages.put('Battle paused tick=99 side1=0 side2=0 resident=0 active=0 nonzero_velocity=0 horses=0')
        messages.put('Battle paused tick=101 side1=100 side2=100 resident=200 active=199 nonzero_velocity=0 horses=20')
        messages.put('Battle paused tick=102 side1=100 side2=100 resident=200 active=200 nonzero_velocity=0 horses=20')
        self.assertEqual(runner.await_population(messages, 100, 100, Recorder()), 102)

    def test_population_barrier_refuses_a_missing_horse_or_inactive_soldier_before_start(self):
        spec = importlib.util.spec_from_file_location('runner', Path(__file__).with_name('run-battle.py'))
        runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runner)
        class Recorder:
            def poll(self):
                return None
        messages = queue.Queue()
        messages.put('Battle paused tick=200 side1=100 side2=100 resident=200 active=200 nonzero_velocity=0 horses=19')
        with self.assertRaisesRegex(RuntimeError, 'horses=19'):
            runner.await_population(messages, 100, 100, Recorder())
        messages.put('Battle paused tick=1400 side1=100 side2=100 resident=199 active=199 nonzero_velocity=0 horses=20')
        with self.assertRaisesRegex(RuntimeError, 'left the active area'):
            runner.await_population(messages, 100, 1300, Recorder(), wait=False)

    def test_telemetry_flush_reaches_last_recorded_tick(self):
        spec = importlib.util.spec_from_file_location('runner', Path(__file__).with_name('run-battle.py'))
        runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runner)
        ticks = queue.Queue()
        ticks.put(100)
        ticks.put(200)
        ticks.put(300)
        runner.wait_telemetry_through(ticks, queue.Queue(), 250)
        self.assertTrue(ticks.empty())


if __name__ == '__main__':
    unittest.main()
