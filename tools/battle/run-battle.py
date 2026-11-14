#!/usr/bin/env python3
"""Run one isolated battle with a pinned server and headless packet recorder."""
import argparse
import ctypes
import hashlib
import json
import os
import platform
import queue
import re
import select
import shutil
import subprocess
import threading
import urllib.parse
import urllib.request
import uuid
from pathlib import Path


def arguments():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--pin', required=True, type=Path, help='server pin directory containing manifest.json')
    p.add_argument('--client', required=True, type=Path, help='recorder-enabled krasnokamen-client binary')
    p.add_argument('--client-data', required=True, type=Path)
    p.add_argument('--cache', required=True, type=Path, help='server content cache')
    p.add_argument('--run-dir', required=True, type=Path, help='new directory; never overwritten')
    p.add_argument('--world', type=Path, help='stopped native world to copy; omitted creates a fresh generated world')
    p.add_argument('--seed', type=int, default=42)
    p.add_argument('--world-seed', type=int, help='seed of the chosen world; defaults to battle seed')
    p.add_argument('--scenario', choices=['field-100', 'field-1k', 'field-10k'], default='field-10k')
    p.add_argument('--count', type=int, choices=range(1, 10001), metavar='1..10000')
    p.add_argument('--duration', type=int, default=180)
    p.add_argument('--paused-ticks', type=int, default=0, help='measure at least this many paused ticks after the roster barrier')
    p.add_argument('--minimum-battle-ticks', type=int, default=0, help='minimum continuously recorded battle ticks')
    p.add_argument('--port', type=int, default=25565)
    p.add_argument('--dashboard-port', type=int, default=8080)
    p.add_argument('--dry-run', action='store_true')
    a = p.parse_args()
    if a.world_seed is None:
        a.world_seed = a.seed
    if not -(2**63) <= a.world_seed <= 2**63 - 1:
        p.error('world seed must fit a signed 64-bit value')
    if not 0 <= a.seed <= 2**63 - 1 or a.duration < 1 or a.paused_ticks < 0 or a.minimum_battle_ticks < 0 or not (1 <= a.port <= 65535 and 1 <= a.dashboard_port <= 65535):
        p.error('invalid seed, duration, tick count or port')
    return a


def command_list(a):
    setup = f'battle setup {a.seed} {a.scenario}' + (f' {a.count}' if a.count is not None else '')
    return [setup, 'battle start', 'battle pause', 'stop']


def sha256_file(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def quote(value):
    return json.dumps(str(value))


def read_pin(pin):
    manifest = json.loads((pin / 'manifest.json').read_text())
    if manifest.get('kind') != 'server':
        raise ValueError('pin manifest is not a server pin')
    binary = Path(manifest['binary']).resolve(strict=True)
    digest = sha256_file(binary)
    if digest != manifest['binary_sha256']:
        raise ValueError('server pin checksum mismatch')
    return binary, manifest


def watch_ready(path, process, timeout=180):
    """Wait on a directory event or child exit, not a timer polling loop."""
    libc = ctypes.CDLL(None, use_errno=True)
    fd = libc.inotify_init1(os.O_CLOEXEC)
    if fd < 0:
        raise OSError(ctypes.get_errno(), 'inotify_init1')
    try:
        if libc.inotify_add_watch(fd, os.fsencode(path.parent), 0x00000080 | 0x00000100) < 0:
            raise OSError(ctypes.get_errno(), 'inotify_add_watch')
        pidfd = os.pidfd_open(process.pid)
        try:
            if path.exists():
                return
            import time
            deadline = time.monotonic() + timeout
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError(f'recorder did not create {path}')
                ready, _, _ = select.select([fd, pidfd], [], [], remaining)
                if fd in ready:
                    os.read(fd, 65536)
                    if path.exists():
                        return
                if pidfd in ready:
                    raise RuntimeError(f'recorder exited before readiness: {process.wait()}')
        finally:
            os.close(pidfd)
    finally:
        os.close(fd)


def telemetry_get(port, after=None):
    params = {'wait': 'true', 'min_samples': '1' if after is None else '100'}
    if after is not None:
        params['after_tick'] = str(after)
    url = f'http://127.0.0.1:{port}/telemetry.jsonl?' + urllib.parse.urlencode(params)
    with urllib.request.urlopen(url, timeout=15) as response:
        return response.read().decode('utf-8')


def telemetry_worker(port, output, stop, failures, sample_status, ticks):
    last = None
    try:
        with output.open('x') as target:
            while not stop.is_set():
                text = telemetry_get(port, last)
                rows = [json.loads(line) for line in text.splitlines() if line]
                for row in rows:
                    tick = row['tick']
                    if last is not None and tick != last + 1:
                        raise ValueError(f'telemetry tick gap or reversal: {last} to {tick}')
                    target.write(json.dumps(row, separators=(',', ':')) + '\n')
                    last = tick
                target.flush()
                if rows and not stop.is_set():
                    ticks.put(last)
                    sample_status()
    except Exception as exc:
        failures.put(exc)
        ticks.put(None)


def stream_log(pipe, file, messages):
    with file.open('x') as output:
        for line in pipe:
            output.write(line)
            output.flush()
            if any(marker in line for marker in ('Krasnokamen is listening (edge v2)', 'Battle staged:', 'Battle roster:', 'Battle started', 'Battle paused', 'Command failed:', 'Battle error:')):
                messages.put(line)
    messages.put(None)


def expect(messages, phrase, timeout=600):
    import time
    deadline = time.monotonic() + timeout
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError(f'server did not report {phrase!r}')
        line = messages.get(timeout=remaining)
        if line is None:
            raise RuntimeError(f'server exited before {phrase!r}')
        if phrase in line:
            return line
        if 'Command failed:' in line or 'Battle error:' in line:
            raise RuntimeError(line.strip())


def await_population(messages, per_side, roster_tick, recorder):
    pattern = re.compile(r"Battle paused tick=(\d+) side1=(\d+) side2=(\d+) resident=(\d+) active=(\d+)")
    while True:
        line = expect(messages, 'Battle paused tick=')
        match = pattern.search(line)
        if match is None:
            continue
        tick, side1, side2, resident, active = map(int, match.groups())
        if tick <= roster_tick:
            continue
        if recorder.poll() is not None:
            raise RuntimeError('recorder exited while waiting for active population')
        if side1 != per_side or side2 != per_side:
            raise RuntimeError(f'battle population changed before start: {side1}+{side2}')
        if resident == 2 * per_side and active == 2 * per_side:
            return tick


def stop_child(process, timeout):
    if process is None or process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def next_tick(ticks, failures, recorder):
    tick = ticks.get(timeout=180)
    if tick is None:
        raise RuntimeError(f'telemetry exporter failed: {failures.get()}')
    if recorder.poll() is not None:
        raise RuntimeError('recorder exited during paused measurement')
    return tick


def wait_paused_ticks(ticks, failures, recorder, count, roster_tick):
    if count == 0:
        return None, None
    while True:
        try:
            ticks.get_nowait()
        except queue.Empty:
            break
    if not failures.empty():
        raise RuntimeError(f'telemetry exporter failed: {failures.get()}')
    start = next_tick(ticks, failures, recorder)
    while start <= roster_tick:
        start = next_tick(ticks, failures, recorder)
    end = start
    while end - start < count:
        end = next_tick(ticks, failures, recorder)
    return start, end


def wait_telemetry_through(ticks, failures, required):
    while True:
        tick = ticks.get(timeout=180)
        if tick is None:
            raise RuntimeError(f'telemetry exporter failed: {failures.get()}')
        if tick >= required:
            return


def run(a):
    commands = command_list(a)
    if a.dry_run:
        print('\n'.join([f'server from {a.pin}/manifest.json --config {a.run_dir}/server.toml',
                         f'recorder record --server 127.0.0.1:{a.port} --username BattleRecorder --output {a.run_dir}/take.solrec --header {a.run_dir}/header.json --duration {a.duration} --ready-file {a.run_dir}/take.ready --roster-file {a.run_dir}/roster.txt --roster-ready-file {a.run_dir}/roster.ready --roster-timeout 900 --data-dir {a.client_data}',
                         'wait for recorder initial readiness',
                         f'export GET http://127.0.0.1:{a.dashboard_port}/telemetry.jsonl; sample battle status every 100 ticks',
                         commands[0], 'battle roster',
                         'publish exact UUID roster and wait for recorder roster readiness',
                         'wait for fresh exact side counts and fully resident, active soldiers',
                         f'wait at least {a.paused_ticks} paused ticks', commands[1],
                         f'require at least {a.minimum_battle_ticks} continuously recorded battle ticks', *commands[2:]]))
        return
    binary, manifest = read_pin(a.pin.resolve())
    client, cache, client_data = (p.resolve(strict=True) for p in (a.client, a.cache, a.client_data))
    root = a.run_dir.resolve()
    root.mkdir(parents=True, exist_ok=False)
    a.created_dir = root
    world = root / 'world'
    if a.world:
        shutil.copytree(a.world.resolve(strict=True), world, symlinks=False)
    server_name = f'battle-{uuid.uuid4().hex}'
    config = root / 'server.toml'
    config.write_text(f'''[server]\nname = {quote(server_name)}\nview_distance = 12\nsimulation_distance = 12\npause_when_empty_seconds = 0\n[network]\nbind_address = "127.0.0.1"\nport = {a.port}\n[auth]\nonline_mode = false\n[dashboard]\nenabled = true\nbind_address = "127.0.0.1"\nport = {a.dashboard_port}\n[recorder]\nplayers = ["BattleRecorder"]\nworld_queue_bytes = 16777216\n[data]\nworld_dir = {quote(world)}\nvanilla_data_dir = {quote(cache)}\nseed = {a.world_seed}\n''')
    (root / 'commands.txt').touch(exist_ok=False)
    recorder_argv = [str(client), 'record', '--server', f'127.0.0.1:{a.port}', '--username', 'BattleRecorder', '--output', str(root / 'take.solrec'), '--header', str(root / 'header.json'), '--duration', str(a.duration), '--ready-file', str(root / 'take.ready'), '--roster-file', str(root / 'roster.txt'), '--roster-ready-file', str(root / 'roster.ready'), '--roster-timeout', '900', '--data-dir', str(client_data)]
    (root / 'manifest.json').write_text(json.dumps({'server_pin': manifest, 'client_sha256': sha256_file(client), 'seed': a.seed, 'world_seed': a.world_seed, 'scenario': a.scenario, 'count_per_side_override': a.count, 'world_source': str(a.world) if a.world else None, 'platform': platform.platform(), 'uname': list(platform.uname()), 'server_argv': [str(binary), '--config', str(config)], 'recorder_argv': recorder_argv, 'server_environment_overrides': {'MC_CORE_PHASE_LOG': '1'}}, indent=2) + '\n')
    with (root / 'host-commands.txt').open('x') as host:
        for cmd in (['uname', '-a'], ['lscpu']):
            host.write('$ ' + ' '.join(cmd) + '\n')
            host.flush()
            subprocess.run(cmd, stdout=host, stderr=subprocess.STDOUT, check=True, text=True)
    messages, failures, ticks, stop = queue.Queue(), queue.Queue(), queue.Queue(), threading.Event()
    server = None
    recorder = None
    telemetry = None
    log_thread = None
    try:
        environment = dict(os.environ, MC_CORE_PHASE_LOG='1')
        server = subprocess.Popen([str(binary), '--config', str(config)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1, env=environment, cwd=root)
        command_lock = threading.Lock()
        def send(command):
            with command_lock:
                with (root / 'commands.txt').open('a') as record:
                    record.write(command + '\n')
                server.stdin.write(command + '\n')
                server.stdin.flush()
        log_thread = threading.Thread(target=stream_log, args=(server.stdout, root / 'server.log', messages), daemon=True)
        log_thread.start()
        expect(messages, 'Krasnokamen is listening (edge v2)')
        header = root / 'header.json'
        header.write_text(json.dumps({'protocol_version': 777, 'server_build': manifest['head'], 'seed': a.seed, 'start_tick': 0, 'wall_time': 0}) + '\n')
        with (root / 'recorder.log').open('x') as recorder_log:
            recorder = subprocess.Popen(recorder_argv, stdout=recorder_log, stderr=subprocess.STDOUT, cwd=root)
            watch_ready(root / 'take.ready', recorder)
            initial = json.loads((root / 'take.ready').read_text())
            if initial.get('phase') != 'play' or not initial.get('pose_known') or not initial.get('tick_marker_received'):
                raise RuntimeError('recorder initial readiness lacks Play, pose or tick marker')
            if recorder.poll() is not None:
                raise RuntimeError('recorder exited before setup')
            with urllib.request.urlopen(f'http://127.0.0.1:{a.dashboard_port}/stats', timeout=15) as response:
                dashboard = json.load(response)
            if dashboard.get('world', {}).get('name') != server_name:
                raise RuntimeError('dashboard belongs to another server')
            (root / 'dashboard-identity.json').write_text(json.dumps(dashboard, indent=2) + '\n')
            telemetry = threading.Thread(target=telemetry_worker, args=(a.dashboard_port, root / 'telemetry.jsonl', stop, failures, lambda: send('battle status'), ticks), daemon=True)
            telemetry.start()
            send(commands[0])
            expect(messages, 'Battle staged:')
            send('battle roster')
            roster_line = expect(messages, 'Battle roster:')
            names = roster_line.split('Battle roster:', 1)[1].strip().split(',')
            ids = [str(uuid.UUID(name)) for name in names]
            per_side = a.count or {'field-100': 100, 'field-1k': 1000, 'field-10k': 10000}[a.scenario]
            if len(ids) < 2 * per_side or len(ids) != len(set(ids)):
                raise RuntimeError('battle roster is incomplete or contains duplicate UUIDs')
            temporary = root / 'roster.tmp'
            temporary.write_text('\n'.join(ids) + '\n')
            temporary.replace(root / 'roster.txt')
            watch_ready(root / 'roster.ready', recorder, timeout=930)
            ready = json.loads((root / 'roster.ready').read_text())
            if (ready.get('phase') != 'play' or not ready.get('pose_known')
                    or not ready.get('tick_marker_received')
                    or ready.get('required_entities') != len(ids)
                    or ready.get('received_entities', 0) < len(ids)
                    or type(ready.get('tick')) is not int):
                raise RuntimeError('recorder readiness receipt does not cover the battle roster')
            if recorder.poll() is not None:
                raise RuntimeError('recorder exited before battle start')
            send('battle status')
            population_tick = await_population(messages, per_side, ready['tick'], recorder)
            paused_start, paused_end = wait_paused_ticks(ticks, failures, recorder, a.paused_ticks, population_tick)
            send(commands[1])
            start_line = expect(messages, 'Battle started tick=')
            battle_start = int(start_line.split('Battle started tick=', 1)[1].strip().split()[0])
            rc = recorder.wait(timeout=a.duration + 180)
            if rc != 0:
                raise RuntimeError(f'recorder failed with exit {rc}')
        if not (root / 'take.solrec').is_file() or (root / 'take.solrec').stat().st_size == 0:
            raise RuntimeError('recorder returned success without a recording')
        record_receipt = json.loads((root / 'take.solrec.result.json').read_text())
        first_tick, last_tick = record_receipt.get('first_tick'), record_receipt.get('last_tick')
        gaps = record_receipt.get('marker_gaps')
        if (record_receipt.get('clean_completion') is not True or type(first_tick) is not int
                or type(last_tick) is not int or type(gaps) is not int or gaps != 0
                or first_tick > battle_start or last_tick <= battle_start):
            raise RuntimeError('recorder receipt lacks a continuous battle marker range')
        send(commands[2])
        pause_line = expect(messages, 'Battle paused tick=')
        battle_end = int(pause_line.split('Battle paused tick=', 1)[1].strip().split()[0])
        wait_telemetry_through(ticks, failures, last_tick)
        (root / 'phase-boundaries.json').write_text(json.dumps({
            'roster_ready_tick': ready['tick'], 'fully_active_population_tick': population_tick,
            'paused_sample_start_tick': paused_start, 'paused_sample_end_tick': paused_end,
            'battle_command_tick': battle_start, 'battle_sample_start_tick': battle_start + 1,
            'server_pause_tick': battle_end, 'recorded_last_tick': last_tick,
        }, indent=2) + '\n')
        stop.set(); telemetry.join(timeout=20)
        if telemetry.is_alive():
            raise RuntimeError('telemetry exporter did not stop')
        if not failures.empty():
            raise RuntimeError(f'telemetry exporter failed: {failures.get()}')
        if not (root / 'telemetry.jsonl').is_file() or (root / 'telemetry.jsonl').stat().st_size == 0:
            raise RuntimeError('telemetry export produced no samples')
        send(commands[3])
        if server.wait(timeout=180) != 0:
            raise RuntimeError('server did not shut down cleanly')
        recorded_ticks = last_tick - battle_start
        if recorded_ticks < a.minimum_battle_ticks:
            raise RuntimeError(f'recorder captured {recorded_ticks} battle ticks, fewer than requested {a.minimum_battle_ticks}')
        (root / 'result.json').write_text(json.dumps({'status': 'complete', 'recording': 'take.solrec', 'telemetry': 'telemetry.jsonl', 'server_log': 'server.log'}, indent=2) + '\n')
    finally:
        stop.set()
        stop_child(recorder, 10)
        stop_child(server, 30)
        if telemetry is not None:
            telemetry.join(timeout=20)
        if log_thread is not None:
            log_thread.join(timeout=5)


if __name__ == '__main__':
    args = arguments()
    try:
        run(args)
    except Exception as error:
        if hasattr(args, 'created_dir'):
            (args.created_dir / 'result.json').write_text(json.dumps({'status': 'failed', 'error': str(error)}, indent=2) + '\n')
        raise
