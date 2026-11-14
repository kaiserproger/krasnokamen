import argparse
import hashlib
import importlib.util
import json
import os
import queue
import socket
import subprocess
import tempfile
import threading
import unittest
import unittest.mock
from pathlib import Path


def load_runner():
    spec = importlib.util.spec_from_file_location('runner', Path(__file__).with_name('run-battle.py'))
    runner = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(runner)
    return runner


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


# A stand-in for the server: console commands on stdin, the dashboard on HTTP, and a tick thread that
# models the core's checkpoint cadence (tick_loop.rs: a counter that `save-all` and a capture reset;
# a capture is an owner_post_us spike).
FAKE_SERVER = r'''#!/usr/bin/env python3
import http.server, json, os, re, sys, threading, time, urllib.parse, uuid
from pathlib import Path
text = Path(sys.argv[sys.argv.index('--config') + 1]).read_text()
name = json.loads(re.search(r'^name = (.+)$', text, re.M).group(1))
port = int(re.search(r'\[dashboard\].*?^port = (\d+)', text, re.M | re.S).group(1))
cadence = int(re.search(r'^checkpoint_every_ticks = (\d+)$', text, re.M).group(1))
Path('env.txt').write_text(os.environ.get('MC_CORE_PHASE_LOG', 'unset'))
cond = threading.Condition()
state = {'tick': 0, 'since': 0, 'rows': [], 'start': None, 'forced': False, 'status_calls': 0, 'per_side': 0}

def ticker():
    while True:
        time.sleep(0.002)
        with cond:
            state['tick'] += 1
            tick = state['tick']
            state['since'] += 1
            spike = 0
            if state['forced'] or state['since'] >= cadence:
                spike, state['since'], state['forced'] = 150_000, 0, False
                print(f'2026-11-11T00:00:00Z  INFO mc_core::core::journal: checkpoint committed tick={tick} total_ms=1', flush=True)
            state['rows'].append({'tick': tick, 'wall_start_unix_ns': 10**12 + tick * 50_000_000, 'wall_us': 20_000 + tick % 7,
                                  'total_us': 19_000, 'phase_us': {'owner_post_us': 300 + spike}, 'host': None})
            cond.notify_all()

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass
    def reply(self, body, kind):
        data = body.encode()
        self.send_response(200)
        self.send_header('Content-Type', kind)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)
    def do_GET(self):
        url = urllib.parse.urlparse(self.path)
        query = urllib.parse.parse_qs(url.query)
        if url.path == '/stats':
            self.reply(json.dumps({'world': {'name': name}}), 'application/json')
        elif url.path == '/_fake':
            with cond:
                self.reply(json.dumps({'tick': state['tick'], 'start': state['start']}), 'application/json')
        else:
            after = int(query.get('after_tick', ['0'])[0])
            wanted = int(query['min_samples'][0])
            with cond:
                cond.wait_for(lambda: state['tick'] - after >= wanted, timeout=10)
                rows = [row for row in state['rows'] if row['tick'] > after]
            self.reply(''.join(json.dumps(row) + '\n' for row in rows), 'application/x-ndjson')

server = http.server.ThreadingHTTPServer(('127.0.0.1', port), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
threading.Thread(target=ticker, daemon=True).start()
if os.environ.get('FAKE_IGNORE_KEY'):
    print('2026-11-11T00:00:00Z  WARN mc_edge::config: ignoring unknown config key origin=/r/server.toml key=data.checkpoint_every_ticks', flush=True)
print('Krasnokamen is listening (edge v2)', flush=True)
for line in sys.stdin:
    words = line.split()
    with cond:
        tick = state['tick'] + 1
    if words[:2] == ['battle', 'setup']:
        state['per_side'] = int(words[4])
        print(f'Battle staged: {state["per_side"]} per side, seed {words[2]}', flush=True)
    elif words[:2] == ['battle', 'roster']:
        count = 2 * state['per_side'] + 2 * (state['per_side'] // 10)
        roster = 'Battle roster: ' + ','.join(str(uuid.uuid4()) for _ in range(count))
        sys.stdout.write(roster[:len(roster) // 2])
        sys.stdout.flush()
        print('tick outer ms: tick=1 pre=0.10 run=1.00', file=sys.stderr, flush=True)  # the phase log, mid roster line
        time.sleep(0.05)
        sys.stdout.write(roster[len(roster) // 2:] + '\n')
        sys.stdout.flush()
    elif words[:2] == ['battle', 'status']:
        state['status_calls'] += 1
        per = state['per_side']
        active = 2 * per - 1 if state['status_calls'] == 1 else 2 * per
        label = 'running' if state['start'] else 'paused'
        print(f'Battle {label} tick={tick} side1={per} side2={per} resident={2 * per} active={active} nonzero_velocity=0 horses={2 * (per // 10)} regiments=4 dead=0', flush=True)
    elif words == ['save-all']:
        with cond:
            state['forced'] = True
        time.sleep(0.01)
        print('Saved the game', flush=True)
    elif words[:2] == ['battle', 'start']:
        state['start'] = tick
        print(f'Battle started tick={tick}', flush=True)
    elif words[:2] == ['battle', 'pause']:
        print(f'Battle paused tick={tick}', flush=True)
    elif words == ['stop']:
        break
'''

# A stand-in for the headless recorder: ready files, then it records until WINDOW ticks after battle start.
FAKE_RECORDER = r'''#!/usr/bin/env python3
import json, os, sys, time, urllib.request
from pathlib import Path
argv = sys.argv
def arg(name):
    return argv[argv.index(name) + 1]
def fake():
    return json.load(urllib.request.urlopen(f'http://127.0.0.1:{os.environ["FAKE_DASHBOARD_PORT"]}/_fake'))
def publish(path, body):
    temporary = Path(str(path) + '.tmp')
    temporary.write_text(json.dumps(body))
    temporary.replace(path)
publish(Path(arg('--ready-file')), {'phase': 'play', 'pose_known': True, 'tick_marker_received': True})
roster = Path(arg('--roster-file'))
while not roster.exists():
    time.sleep(0.005)
ids = roster.read_text().split()
publish(Path(arg('--roster-ready-file')), {'phase': 'play', 'pose_known': True, 'tick_marker_received': True,
                                         'required_entities': len(ids), 'received_entities': len(ids), 'tick': fake()['tick']})
while fake()['start'] is None:
    time.sleep(0.005)
start = fake()['start']
while fake()['tick'] < start + int(os.environ['FAKE_WINDOW_TICKS']):
    time.sleep(0.005)
output = Path(arg('--output'))
def frame(payload):
    return (0).to_bytes(4, 'big') + len(payload).to_bytes(4, 'big') + payload
def marker(tick):
    return frame(b'\x18\x0ckrasnokamen:tick' + tick.to_bytes(8, 'big') + bytes(8))
def chunk(x, z):
    return frame(b'\x2e' + x.to_bytes(4, 'big', signed=True) + z.to_bytes(4, 'big', signed=True) + b'data')
output.write_bytes(b'{"protocol_version":777}\n' + marker(start - 1) + chunk(3, -5) + marker(start + 1) + chunk(-9, 2) + chunk(3, -5))
Path(str(output) + '.result.json').write_text(json.dumps({'clean_completion': True, 'first_tick': 1, 'last_tick': fake()['tick'], 'marker_gaps': 0}))
'''


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

    def test_incremental_server_pin_is_refused(self):
        spec = importlib.util.spec_from_file_location('runner', Path(__file__).with_name('run-battle.py'))
        runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runner)
        with tempfile.TemporaryDirectory() as directory:
            pin = Path(directory)
            binary = pin / 'server'
            binary.write_text('test')
            manifest = {'kind': 'server', 'binary': str(binary),
                        'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'head': 'test'}
            for recorded, accepted in ((None, True), (False, True), (True, False)):
                with self.subTest(incremental=recorded):
                    (pin / 'manifest.json').write_text(json.dumps(
                        manifest if recorded is None else dict(manifest, incremental=recorded)))
                    if accepted:
                        self.assertEqual(runner.read_pin(pin)[0], binary.resolve())
                    else:
                        with self.assertRaisesRegex(ValueError, 'incremental'):
                            runner.read_pin(pin)

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

    def test_take_arguments_default_to_a_quiet_window(self):
        runner = load_runner()
        base = ['--pin', '/p', '--client', '/c', '--client-data', '/d', '--cache', '/k', '--run-dir', '/r', '--duration', '60']
        with unittest.mock.patch('sys.argv', ['run-battle.py', *base]):
            normal = runner.arguments()
        self.assertEqual((normal.phase_log, normal.status_every, normal.checkpoint_every_ticks, normal.allow_marker_gaps), (True, 100, 1200, False))
        with unittest.mock.patch('sys.argv', ['run-battle.py', *base, '--take']):
            take = runner.arguments()
        self.assertEqual((take.phase_log, take.status_every, take.allow_marker_gaps), (False, 0, True))
        self.assertEqual(take.checkpoint_every_ticks, (60 + 120) * 20)
        with unittest.mock.patch('sys.argv', ['run-battle.py', *base, '--take', '--phase-log', '--status-every', '50', '--checkpoint-every-ticks', '5000']):
            override = runner.arguments()
        self.assertEqual((override.phase_log, override.status_every, override.checkpoint_every_ticks), (True, 50, 5000))

    def test_take_refuses_a_cadence_inside_the_take(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result = subprocess.run([str(Path(__file__).with_name('run-battle.sh')), '--dry-run', '--take', '--duration', '60',
                                     '--checkpoint-every-ticks', '1200', '--pin', str(root / 'pin'), '--client', str(root / 'client'),
                                     '--client-data', str(root / 'client-data'), '--cache', str(root / 'cache'),
                                     '--run-dir', str(root / 'run')], text=True, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('checkpoint cadence', result.stderr)
            self.assertFalse((root / 'run').exists())

    def test_dry_run_prints_the_config_key_and_take_plan(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            common = ['--dry-run', '--pin', str(root / 'pin'), '--client', str(root / 'client'), '--client-data', str(root / 'client-data'),
                      '--cache', str(root / 'cache'), '--run-dir', str(root / 'run'), '--duration', '60', '--view-distance', '16']
            script = str(Path(__file__).with_name('run-battle.sh'))
            take = subprocess.run([script, *common, '--take', '--server-scope', 'solaris-heavy.slice'], text=True, capture_output=True, check=True).stdout
            self.assertIn('checkpoint_every_ticks = 3600', take)
            self.assertIn('view_distance = 16', take)
            self.assertIn('MC_CORE_PHASE_LOG=unset', take)
            self.assertIn('after start never', take)
            self.assertIn('systemd-run --user --scope', take)
            self.assertIn('--slice solaris-heavy.slice', take)
            self.assertLess(take.index('battle roster'), take.index('save-all'))
            self.assertLess(take.index('save-all'), take.index('battle start'))
            normal = subprocess.run([script, *common], text=True, capture_output=True, check=True).stdout
            self.assertIn('checkpoint_every_ticks = 1200', normal)
            self.assertIn('MC_CORE_PHASE_LOG=1', normal)
            self.assertIn('every 100 ticks', normal)
            self.assertNotIn('save-all', normal)
            self.assertNotIn('systemd-run', normal)

    def test_server_environment_removes_an_inherited_phase_log(self):
        runner = load_runner()
        inherited = {'MC_CORE_PHASE_LOG': '1', 'PATH': '/bin'}
        self.assertNotIn('MC_CORE_PHASE_LOG', runner.server_environment(argparse.Namespace(phase_log=False), inherited))
        self.assertEqual(runner.server_environment(argparse.Namespace(phase_log=True), {'PATH': '/bin'})['MC_CORE_PHASE_LOG'], '1')
        self.assertEqual(inherited['MC_CORE_PHASE_LOG'], '1')

    def test_status_policy_polls_only_while_the_barrier_is_open_or_on_request(self):
        runner = load_runner()
        quiet = runner.StatusPolicy(0)
        self.assertTrue(quiet.due(100) and quiet.due(200))
        held = runner.StatusPolicy(0)
        held.hold = True
        self.assertFalse(held.due(100))
        held.hold = False
        self.assertTrue(held.due(200))
        quiet.close_barrier()
        self.assertFalse(any(quiet.due(tick) for tick in range(300, 5000, 100)))
        paced = runner.StatusPolicy(250)
        paced.close_barrier()
        self.assertEqual([tick for tick in range(100, 1000, 100) if paced.due(tick)], [100, 400, 700])

    def test_parse_status_reads_running_and_extended_fields(self):
        runner = load_runner()
        line = 'Battle running tick=77 side1=9 side2=8 resident=20 active=19 nonzero_velocity=3 horses=2 regiments=14 dead=1'
        self.assertEqual(runner.parse_status(line), {'tick': 77, 'side1': 9, 'side2': 8, 'resident': 20, 'active': 19,
                                                      'nonzero_velocity': 3, 'horses': 2, 'regiments': 14, 'dead': 1})
        self.assertEqual(runner.parse_status('Battle paused tick=5 side1=1 side2=1 resident=2 active=2 horses=0')['horses'], 0)
        self.assertIsNone(runner.parse_status('Battle started tick=5'))

    def test_server_scope_wraps_only_the_server(self):
        runner = load_runner()
        plain = argparse.Namespace(server_scope=None)
        self.assertEqual(runner.server_argv(plain, '/bin/server', '/c.toml', 'u'), ['/bin/server', '--config', '/c.toml'])
        scoped = runner.server_argv(argparse.Namespace(server_scope=''), '/bin/server', '/c.toml', 'battle-1')
        self.assertEqual(scoped[:7], ['systemd-run', '--user', '--scope', '--quiet', '--collect', '--unit', 'battle-1'])
        self.assertEqual(scoped[7:], ['--', '/bin/server', '--config', '/c.toml'])
        self.assertIn('solaris-heavy.slice', runner.server_argv(argparse.Namespace(server_scope='solaris-heavy.slice'), '/s', '/c', 'u'))

    def test_telemetry_gaps_are_recorded_in_a_take_and_fatal_otherwise(self):
        runner = load_runner()
        def export(gaps):
            batches = [[{'tick': 1}, {'tick': 2}], [{'tick': 5}, {'tick': 6}]]
            stop, failures, ticks, polled = threading.Event(), queue.Queue(), queue.Queue(), []
            def get(port, after=None):
                rows = batches.pop(0)
                if not batches:
                    stop.set()
                return ''.join(json.dumps(row) + '\n' for row in rows)
            with tempfile.TemporaryDirectory() as directory, unittest.mock.patch.object(runner, 'telemetry_get', get):
                output = Path(directory) / 'telemetry.jsonl'
                runner.telemetry_worker(1, output, stop, failures, polled.append, ticks, gaps)
                written = [json.loads(line)['tick'] for line in output.read_text().splitlines()]
            return written, failures, polled
        gaps = []
        written, failures, polled = export(gaps)
        self.assertEqual((written, gaps, polled), ([1, 2, 5, 6], [[2, 5]], [2]))
        self.assertTrue(failures.empty())
        written, failures, _ = export(None)
        self.assertEqual(written, [1, 2])
        self.assertIn('gap or reversal', str(failures.get()))

    def test_window_summary_finds_a_checkpoint_spike_and_clean_windows(self):
        runner = load_runner()
        def row(tick, post=300, wall=20_000, rss=None):
            return {'tick': tick, 'wall_start_unix_ns': tick * 50_000_000, 'wall_us': wall, 'phase_us': {'owner_post_us': post},
                    'host': None if rss is None else {'process_rss_bytes': rss}, 'recorders': {'deferred_updates_sum': 2}}
        rows = [row(1, post=200_000), *(row(tick, rss=tick * 1000) for tick in range(2, 12)), row(12, wall=60_000)]
        summary = runner.window_summary(rows, 2, 12)
        self.assertEqual((summary['samples'], summary['missing_ticks'], summary['checkpoint_spike_ticks']), (11, 0, []))
        self.assertEqual((summary['wall_us']['max'], summary['wall_us']['p50'], summary['tps']), (60_000, 20_000, 20.0))
        self.assertEqual((summary['process_rss_max_bytes'], summary['host_samples'], summary['deferred_updates_sum_total']), (11_000, 10, 22))
        self.assertEqual(runner.window_summary(rows, 1, 12)['checkpoint_spike_ticks'], [1])
        self.assertEqual(runner.window_summary(rows, 20, 25)['missing_ticks'], 6)

    def test_take_report_is_unclean_for_a_spike_gap_phase_log_or_polling(self):
        runner = load_runner()
        rows = [{'tick': tick, 'wall_start_unix_ns': tick * 50_000_000, 'wall_us': 20_000, 'phase_us': {'owner_post_us': 300}, 'host': None} for tick in range(1, 11)]
        quiet = argparse.Namespace(seed=1, world_seed=1, scenario='field-100', count=None, checkpoint_every_ticks=4000, phase_log=False,
                                   status_every=0, view_distance=12, pre_stage_ticks=0, allow_marker_gaps=True, server_scope=None, take=True)
        receipt = {'first_tick': 1, 'last_tick': 10, 'marker_gaps': 0}
        committed = lambda *ticks: ''.join(f'2026-11-11T00:00:00Z  INFO mc_core::core::journal: checkpoint committed tick={tick} total_ms=1\n' for tick in ticks)
        report = runner.take_report(quiet, rows, {'tick': 3}, receipt, 2, 10, [], 'battle setup 1 field-100', committed(1))
        shallow = runner.take_report(quiet, rows, {}, receipt, 2, 10, [], 'x', '', {'distinct_chunks': 9, 'max_ring_estimate': 6})
        self.assertEqual(shallow['unclean_reasons'], ['recorded terrain reaches ring 6, below view distance 12 - 1 (still streaming when the recording ended; raise --pre-stage-ticks)'])
        self.assertEqual(runner.take_report(quiet, rows, {}, receipt, 2, 10, [], 'x', '', {'distinct_chunks': 0})['unclean_reasons'], ['no terrain chunks in the recording'])
        self.assertTrue(runner.take_report(quiet, rows, {}, receipt, 2, 10, [], 'x', '', {'distinct_chunks': 9, 'max_ring_estimate': 11})['clean'])
        self.assertEqual((report['clean'], report['unclean_reasons']), (True, []))
        self.assertEqual(report['census_before_start'], {'tick': 3})
        spiky = [dict(rows[4], phase_us={'owner_post_us': 150_000})] + rows[5:]
        report = runner.take_report(quiet, rows[:4] + spiky, {}, dict(receipt, marker_gaps=1), 2, 10, [[4, 5]], 'x', committed(1, 7))
        self.assertFalse(report['clean'])
        self.assertEqual(report['window']['checkpoint_spike_ticks'], [5])
        self.assertEqual(report['server_checkpoints']['in_window'], [7])
        for reason in ('checkpoint capture spike', 'server log records a checkpoint', 'recorder marker gaps', 'telemetry tick gaps'):
            self.assertTrue(any(reason in text for text in report['unclean_reasons']), reason)
        noisy = argparse.Namespace(**dict(vars(quiet), phase_log=True, status_every=100))
        reasons = runner.take_report(noisy, rows, {}, receipt, 2, 10, [], 'x', '')['unclean_reasons']
        self.assertEqual(reasons, ['phase log was on', 'battle status polled after start'])
        self.assertEqual(runner.take_report(quiet, rows, {}, receipt, 2, 10, [], 'x', committed(1))['server_checkpoints']['next_periodic_margin_ticks'], 4000 - 9)
        ignored = 'WARN mc_edge::config: ignoring unknown config key origin=/r/server.toml key=data.checkpoint_every_ticks\n'
        report = runner.take_report(quiet, rows, {}, receipt, 2, 10, [], 'x', ignored)
        self.assertEqual(report['server_ignored_config_keys'], ['data.checkpoint_every_ticks'])
        self.assertEqual(report['unclean_reasons'], ['server ignored config keys: data.checkpoint_every_ticks'])

    def test_server_checkpoint_ticks_come_from_the_info_log_not_the_phase_log(self):
        runner = load_runner()
        log = ('2026-11-11T11:01:31Z  INFO mc_core::core::journal: checkpoint committed tick=849 total_ms=688 chunk_ms=619\n'
               'checkpoint capture ms: tick=850 data=219.44\n'
               '2026-11-11T11:02:08Z  INFO mc_core::core::journal: checkpoint committed tick=1574 total_ms=1332\n')
        self.assertEqual(runner.server_checkpoint_ticks(log), [849, 1574])

    def run_fake_battle(self, root, *extra, expect_success=True, **fake_environment):
        server = root / 'server'
        server.write_text(FAKE_SERVER)
        server.chmod(0o755)
        recorder = root / 'recorder'
        recorder.write_text(FAKE_RECORDER)
        recorder.chmod(0o755)
        (root / 'pin').mkdir()
        (root / 'pin' / 'manifest.json').write_text(json.dumps({'kind': 'server', 'binary': str(server), 'head': 'fake',
                                                                  'binary_sha256': hashlib.sha256(server.read_bytes()).hexdigest()}))
        for name in ('client-data', 'cache'):
            (root / name).mkdir()
        dashboard = free_port()
        environment = dict(os.environ, FAKE_DASHBOARD_PORT=str(dashboard), FAKE_WINDOW_TICKS='300', MC_CORE_PHASE_LOG='1', **fake_environment)
        result = subprocess.run([str(Path(__file__).with_name('run-battle.sh')), '--pin', str(root / 'pin'), '--client', str(recorder),
                                 '--client-data', str(root / 'client-data'), '--cache', str(root / 'cache'), '--run-dir', str(root / 'run'),
                                 '--scenario', 'field-100', '--count', '10', '--seed', '9', '--port', str(free_port()),
                                 '--dashboard-port', str(dashboard), '--duration', '1', *extra],
                                text=True, capture_output=True, timeout=120, env=environment)
        if not expect_success:
            return result, root / 'run'
        self.assertEqual(result.returncode, 0, result.stderr)
        commands = (root / 'run' / 'commands.txt').read_text().splitlines()
        return commands, root / 'run'

    def test_arena_arguments_default_to_the_arena_world_seed_and_follow_a_receipt(self):
        runner = load_runner()
        base = ['--pin', '/p', '--client', '/c', '--client-data', '/d', '--cache', '/k', '--run-dir', '/r', '--seed', '7']
        def parse(*extra):
            with unittest.mock.patch('sys.argv', ['run-battle.py', *base, *extra]):
                return runner.arguments()
        self.assertEqual(parse().world_seed, 7)
        built = parse('--arena', 'grass')
        self.assertEqual((built.arena, built.world_seed, built.arena_seed), ('grass', 42, 42))
        self.assertEqual(parse('--arena', 'grass', '--world-seed', '5').world_seed, 5)
        with tempfile.TemporaryDirectory() as directory:
            world = Path(directory) / 'arena' / 'world'
            world.mkdir(parents=True)
            (world.parent / 'result.json').write_text(json.dumps({'status': 'complete', 'world_seed': 99}))
            reused = parse('--arena-world', str(world))
            self.assertEqual((reused.arena, reused.world_seed), ('grass', 99))
            with unittest.mock.patch('sys.stderr'), self.assertRaises(SystemExit):
                parse('--arena-world', str(world), '--world-seed', '5')
        with unittest.mock.patch('sys.stderr'), self.assertRaises(SystemExit):
            parse('--arena', 'grass', '--world', '/w')

    def test_stage_arena_builds_with_the_runner_pin_and_cache_or_reuses_a_world(self):
        runner = load_runner()
        self.assertTrue(callable(runner.arena_builder()))
        calls = []
        def build(run_dir, binary, cache, seed, world_seed):
            calls.append((run_dir, binary, cache, seed, world_seed))
            return {'world': '/arena/world', 'world_seed': world_seed}
        with unittest.mock.patch.object(runner, 'arena_builder', lambda: build):
            world, receipt = runner.stage_arena(argparse.Namespace(arena_world=None, arena_seed=3, world_seed=42), '/bin/server', '/cache', Path('/run'))
        self.assertEqual(calls, [(Path('/run/arena'), '/bin/server', '/cache', 3, 42)])
        self.assertEqual((world, receipt['world_seed']), (Path('/arena/world'), 42))
        with tempfile.TemporaryDirectory() as directory:
            prebuilt = Path(directory) / 'arena' / 'world'
            prebuilt.mkdir(parents=True)
            (prebuilt.parent / 'result.json').write_text(json.dumps({'status': 'complete', 'world_seed': 42, 'command_sha256': 'abc'}))
            world, receipt = runner.stage_arena(argparse.Namespace(arena_world=prebuilt), '/bin/server', '/cache', Path('/run'))
            self.assertEqual((world, receipt['command_sha256']), (prebuilt.resolve(), 'abc'))

    def test_dry_run_names_the_arena_step(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            common = [str(Path(__file__).with_name('run-battle.sh')), '--dry-run', '--pin', str(root / 'pin'), '--client', str(root / 'c'),
                      '--client-data', str(root / 'd'), '--cache', str(root / 'k'), '--run-dir', str(root / 'run')]
            built = subprocess.run([*common, '--arena', 'grass'], text=True, capture_output=True, check=True).stdout
            self.assertIn('arena: build grass arena (layout seed 42, world seed 42)', built)
            self.assertIn('seed = 42', built)
            reused = subprocess.run([*common, '--arena-world', str(root / 'w')], text=True, capture_output=True, check=True).stdout
            self.assertIn(f'arena: reuse prebuilt grass arena world {root / "w"}', reused)
            self.assertNotIn('arena:', subprocess.run(common, text=True, capture_output=True, check=True).stdout)

    def test_run_on_a_reused_arena_copies_it_and_records_where_it_came_from(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            world = root / 'arena' / 'world'
            world.mkdir(parents=True)
            (world / 'level.marker').write_text('arena')
            (world.parent / 'result.json').write_text(json.dumps({'status': 'complete', 'world_seed': 31, 'command_sha256': 'abc'}))
            commands, run = self.run_fake_battle(root, '--arena-world', str(world))
            self.assertEqual((run / 'world' / 'level.marker').read_text(), 'arena')
            self.assertIn('seed = 31', (run / 'server.toml').read_text())
            info = json.loads((run / 'arena.json').read_text())
            self.assertEqual((info['kind'], info['reused'], info['receipt']['command_sha256']), ('grass', True, 'abc'))
            self.assertEqual(json.loads((run / 'manifest.json').read_text())['world_source'], str(world.resolve()))

    def test_take_refuses_a_server_pin_that_ignores_the_cadence_key(self):
        with tempfile.TemporaryDirectory() as directory:
            result, run = self.run_fake_battle(Path(directory), '--take', '--checkpoint-every-ticks', '1000', expect_success=False, FAKE_IGNORE_KEY='1')
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('ignores data.checkpoint_every_ticks', json.loads((run / 'result.json').read_text())['error'])
            self.assertEqual((run / 'commands.txt').read_text(), '')

    def test_take_run_forces_one_checkpoint_before_start_and_stays_quiet_in_the_window(self):
        with tempfile.TemporaryDirectory() as directory:
            commands, run = self.run_fake_battle(Path(directory), '--take', '--checkpoint-every-ticks', '1000', '--view-distance', '6', '--pre-stage-ticks', '150')
            start, pause = commands.index('battle start'), commands.index('battle pause')
            self.assertEqual(commands[start - 1], 'save-all')
            self.assertEqual(commands.count('save-all'), 1)
            self.assertNotIn('battle status', commands[start:pause])
            self.assertGreaterEqual(commands[:start].count('battle status'), 2)
            self.assertEqual(commands[0], 'battle setup 9 field-100 10')
            self.assertEqual((run / 'env.txt').read_text(), 'unset')
            self.assertIn('checkpoint_every_ticks = 1000', (run / 'server.toml').read_text())
            take = json.loads((run / 'take.json').read_text())
            self.assertEqual((take['clean'], take['unclean_reasons']), (True, []))
            self.assertEqual(take['window']['checkpoint_spike_ticks'], [])
            checkpoints = take['server_checkpoints']
            self.assertEqual(checkpoints['in_window'], [])
            self.assertIsNotNone(checkpoints['last_before_window'])
            self.assertGreater(checkpoints['next_periodic_margin_ticks'], 0)
            self.assertGreaterEqual(take['window']['samples'], 300)
            self.assertEqual(take['census_before_start']['regiments'], 4)
            self.assertEqual(take['config']['pre_stage_ticks'], 150)
            self.assertEqual(take['recording_chunks'], {'distinct_chunks': 2, 'x_range': [-9, 3], 'z_range': [-5, 2], 'max_ring_from_origin': 9, 'max_ring_estimate': 6, 'first_sent_from_battle_start': 1})
            result = json.loads((run / 'result.json').read_text())
            self.assertEqual((result['status'], result['take_clean']), ('complete', True))
            rows = [json.loads(line) for line in (run / 'telemetry.jsonl').read_text().splitlines()]
            self.assertTrue(any(r['phase_us']['owner_post_us'] >= 50_000 for r in rows if r['tick'] < take['window']['first_tick']))

    def test_default_run_keeps_the_phase_log_and_polls_status_after_start(self):
        with tempfile.TemporaryDirectory() as directory:
            commands, run = self.run_fake_battle(Path(directory))
            start, pause = commands.index('battle start'), commands.index('battle pause')
            self.assertNotIn('save-all', commands)
            self.assertGreaterEqual(commands[start:pause].count('battle status'), 1)
            self.assertEqual((run / 'env.txt').read_text(), '1')
            self.assertIn('checkpoint_every_ticks = 1200', (run / 'server.toml').read_text())
            self.assertFalse((run / 'take.json').exists())


if __name__ == '__main__':
    unittest.main()
