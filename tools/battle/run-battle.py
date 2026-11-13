#!/usr/bin/env python3
"""Run one isolated battle with a pinned server and headless packet recorder."""
import argparse
import ctypes
import hashlib
import json
import math
import os
import platform
import queue
import re
import select
import shutil
import signal
import subprocess
import sys
import threading
import urllib.parse
import urllib.request
import uuid
from pathlib import Path

TICKS_PER_SECOND = 20
DEFAULT_CHECKPOINT_EVERY_TICKS = 1200
DEFAULT_STATUS_EVERY_TICKS = 100
# Take mode keeps the periodic checkpoint cadence this much longer than the take.
TAKE_CHECKPOINT_MARGIN_SECONDS = 120
# A tick whose owner_post_us reaches this is treated as a checkpoint capture. The capture runs on
# the tick thread inside owner_post (journal.rs/tick_loop.rs); in the integrated 20k ladder
# (receipts/BattleBaseline/integrated-ladder-2) captures measured 100.8-247.5 ms there and no other
# tick exceeded 23.8 ms, so 50 ms separates them. Evidence, not proof: the phase log is off in a take.
CHECKPOINT_SPIKE_US = 50_000
STATUS_FIELDS = ('tick', 'side1', 'side2', 'resident', 'active', 'nonzero_velocity', 'horses', 'regiments', 'dead')
STATUS_PATTERN = re.compile(r"Battle (?:paused|running) tick=(\d+) side1=(\d+) side2=(\d+) resident=(\d+) active=(\d+)"
                            r"(?: nonzero_velocity=(\d+))? horses=(\d+)(?: regiments=(\d+))?(?: dead=(\d+))?")
PLAYER_NAME = re.compile(r'[A-Za-z0-9_]{1,16}')
SIDES = {'red': 'red', '1': 'red', 'blue': 'blue', '2': 'blue'}
RECORDER_NAME = 'BattleRecorder'
DEFAULT_LIVE_TIMEOUT = 900


def arguments():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--pin', required=True, type=Path, help='server pin directory containing manifest.json')
    p.add_argument('--client', required=True, type=Path, help='recorder-enabled solaris-client binary')
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
    p.add_argument('--take', action='store_true', help='take mode: a quiet shot window. Phase log off, no status polling after start, checkpoint cadence longer than the take with one forced checkpoint right before start, tick gaps recorded; writes take.json')
    p.add_argument('--checkpoint-every-ticks', type=int, metavar='TICKS', help='[data] checkpoint cadence written to server.toml; default 1200, in take mode longer than the whole take')
    p.add_argument('--phase-log', action=argparse.BooleanOptionalAction, default=None, help='MC_CORE_PHASE_LOG for the server; default on, off in take mode')
    p.add_argument('--status-every', type=int, metavar='TICKS', help='after start, request battle status about every TICKS recorded ticks (0 = never); default 100, 0 in take mode. The pre-start population barrier always polls')
    p.add_argument('--view-distance', type=int, default=12, metavar='CHUNKS', help='server and recorder view distance, 2..32')
    p.add_argument('--allow-marker-gaps', action='store_true', help='record recorder marker and telemetry tick gaps instead of failing the run; implied by --take')
    p.add_argument('--server-scope', nargs='?', const='', default=None, metavar='SLICE', help='run the server in its own transient systemd user scope (optionally under SLICE) so the cgroup memory/CPU telemetry is the server alone')
    p.add_argument('--live-player', action='append', default=[], metavar='NAME:SIDE', help='network player (offline-mode name) who takes a front-rank infantry slot of SIDE (red|blue) near the centre with its kit before start; repeatable. The runner waits until every live name is online before it launches the recorder')
    p.add_argument('--live-spectator', action='append', default=[], metavar='NAME[:TARGET]', help='network player put in spectator mode behind TARGET (default: the first --live-player) before start; repeatable')
    p.add_argument('--live-timeout', type=int, default=DEFAULT_LIVE_TIMEOUT, metavar='SECONDS', help=f'how long the online barrier waits for every live player and spectator (default {DEFAULT_LIVE_TIMEOUT})')
    p.add_argument('--console-fifo', action='store_true', help='create RUN_DIR/console.fifo: each line written to it is sent to the server console (recorded in commands.txt), e.g. `echo "kill PovRed" > RUN_DIR/console.fifo`')
    p.add_argument('--dry-run', action='store_true')
    a = p.parse_args()
    if a.world_seed is None:
        a.world_seed = a.seed
    if not -(2**63) <= a.world_seed <= 2**63 - 1:
        p.error('world seed must fit a signed 64-bit value')
    if not 0 <= a.seed <= 2**63 - 1 or a.duration < 1 or a.paused_ticks < 0 or a.minimum_battle_ticks < 0 or not (1 <= a.port <= 65535 and 1 <= a.dashboard_port <= 65535):
        p.error('invalid seed, duration, tick count or port')
    if not 2 <= a.view_distance <= 32:
        p.error('view distance must be 2..32')
    if a.status_every is not None and a.status_every < 0:
        p.error('status interval must not be negative')
    if a.checkpoint_every_ticks is not None and a.checkpoint_every_ticks < 1:
        p.error('checkpoint cadence must be at least one tick')
    if a.phase_log is None:
        a.phase_log = not a.take
    if a.status_every is None:
        a.status_every = 0 if a.take else DEFAULT_STATUS_EVERY_TICKS
    if a.take:
        a.allow_marker_gaps = True
    if a.checkpoint_every_ticks is None:
        a.checkpoint_every_ticks = take_checkpoint_ticks(a.duration) if a.take else DEFAULT_CHECKPOINT_EVERY_TICKS
    elif a.take and a.checkpoint_every_ticks <= a.duration * TICKS_PER_SECOND:
        p.error(f'take mode needs a checkpoint cadence above the {a.duration * TICKS_PER_SECOND} ticks of the take, or a checkpoint lands in the shot window')
    if a.live_timeout < 1:
        p.error('live timeout must be at least one second')
    try:
        a.live_players, a.live_spectators = live_roles(a.live_player, a.live_spectator)
    except ValueError as error:
        p.error(str(error))
    return a


def live_roles(players, spectators):
    """(name, side) live players and (name, target) spectators from the flag values."""
    roles = []
    for value in players:
        name, _, side = value.partition(':')
        if not PLAYER_NAME.fullmatch(name) or side not in SIDES:
            raise ValueError(f'--live-player {value!r}: expected NAME:red|blue with a 1..16 character [A-Za-z0-9_] name')
        roles.append((name, SIDES[side]))
    watchers = []
    for value in spectators:
        name, _, target = value.partition(':')
        if not target:
            if not roles:
                raise ValueError(f'--live-spectator {value!r} needs a TARGET or a --live-player to follow')
            target = roles[0][0]
        if not PLAYER_NAME.fullmatch(name) or not PLAYER_NAME.fullmatch(target):
            raise ValueError(f'--live-spectator {value!r}: expected NAME[:TARGET] with 1..16 character [A-Za-z0-9_] names')
        watchers.append((name, target))
    names = [name for name, _ in roles] + [name for name, _ in watchers]
    if len(names) != len(set(names)):
        raise ValueError('live player and spectator names must be distinct')
    if RECORDER_NAME in names:
        raise ValueError(f'{RECORDER_NAME} is the recorder; pick another live name')
    if len(names) > 8:
        raise ValueError('at most 8 live players and spectators')
    for name, target in watchers:
        if target not in names:
            raise ValueError(f'spectator {name} follows {target}, who is neither a live player nor a spectator')
    return roles, watchers


def live_names(a):
    return [name for name, _ in a.live_players] + [name for name, _ in a.live_spectators]


def live_commands(a):
    """Console commands, in order: the watch that makes the server log joins (before the barrier), then
    each placement (after the population barrier, before start)."""
    names = live_names(a)
    if not names:
        return None, []
    placements = [f'battle live {name} {side}' for name, side in a.live_players]
    placements += [f'battle spectate {name} {target}' for name, target in a.live_spectators]
    return f'battle watch {" ".join(names)}', placements


def utc_now():
    import datetime
    return datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='milliseconds')


class LivePresence:
    """Online state of watched names from the server's `Battle player online|offline: NAME` lines
    (battle/live_players.rs), fed by the log reader thread, plus every death, respawn and placement line
    for the receipt. Waiting is on a condition the log reader notifies, never a timer."""
    PATTERN = re.compile(r'Battle (player online|player offline|live placed|live respawned|live died|live rearmed|spectator placed): (\w+)')

    def __init__(self, names):
        self.names = set(names)
        self.online = {}
        self.events = []
        self.closed = False
        self.barrier_passed = None
        self.condition = threading.Condition()

    def feed(self, line):
        match = self.PATTERN.search(line)
        if match is None or match.group(2) not in self.names:
            return
        kind, name = match.groups()
        with self.condition:
            self.events.append({'event': kind.replace(' ', '_'), 'name': name, 'utc': utc_now(), 'line': line.strip()})
            if kind == 'player online':
                self.online[name] = True
            elif kind == 'player offline':
                self.online[name] = False
            self.condition.notify_all()

    def close(self):
        with self.condition:
            self.closed = True
            self.condition.notify_all()

    def wait_online(self, names, timeout):
        with self.condition:
            done = self.condition.wait_for(lambda: self.closed or all(self.online.get(name) for name in names), timeout)
            if self.closed and not all(self.online.get(name) for name in names):
                raise RuntimeError('server exited before every live player was online')
            if not done:
                missing = [name for name in names if not self.online.get(name)]
                raise TimeoutError(f'live players not online after {timeout} s: {", ".join(missing)}')

    def first_online(self):
        with self.condition:
            first = {}
            for event in self.events:
                if event['event'] == 'player_online':
                    first.setdefault(event['name'], event['utc'])
            return first

    def receipt(self):
        with self.condition:
            return {'names': sorted(self.names), 'barrier_passed': self.barrier_passed, 'events': list(self.events)}


def command_list(a):
    setup = f'battle setup {a.seed} {a.scenario}' + (f' {a.count}' if a.count is not None else '')
    return [setup, 'battle start', 'battle pause', 'stop']


def take_checkpoint_ticks(duration):
    """Cadence for a take. The runner forces one checkpoint right before `battle start`, which restarts
    the server's tick counter for the cadence (tick_loop.rs); the recorder stops `duration` seconds
    after the roster barrier, so no periodic checkpoint is due before the take ends."""
    return (duration + TAKE_CHECKPOINT_MARGIN_SECONDS) * TICKS_PER_SECOND


def server_config_text(a, server_name, world, cache):
    return f'''[server]\nname = {quote(server_name)}\nview_distance = {a.view_distance}\nsimulation_distance = 12\npause_when_empty_seconds = 0\n[network]\nbind_address = "127.0.0.1"\nport = {a.port}\n[auth]\nonline_mode = false\n[dashboard]\nenabled = true\nbind_address = "127.0.0.1"\nport = {a.dashboard_port}\n[recorder]\nplayers = ["BattleRecorder"]\nworld_queue_bytes = 16777216\n[data]\nworld_dir = {quote(world)}\nvanilla_data_dir = {quote(cache)}\nseed = {a.world_seed}\ncheckpoint_every_ticks = {a.checkpoint_every_ticks}\n'''


def server_environment(a, base):
    """MC_CORE_PHASE_LOG is read by presence, so off must remove it even if the caller exported it."""
    environment = dict(base)
    if a.phase_log:
        environment['MC_CORE_PHASE_LOG'] = '1'
    else:
        environment.pop('MC_CORE_PHASE_LOG', None)
    return environment


def server_argv(a, binary, config, unit):
    argv = [str(binary), '--config', str(config)]
    if a.server_scope is None:
        return argv
    scope = ['systemd-run', '--user', '--scope', '--quiet', '--collect', '--unit', unit]
    if a.server_scope:
        scope += ['--slice', a.server_scope]
    return scope + ['--'] + argv


class StatusPolicy:
    """Which telemetry batches request `battle status`. While the barrier is open (before the battle
    starts) every batch does: the population barrier needs a fresh census as chunks activate. After
    close_barrier only every `every` ticks, never when `every` is 0. Event driven: it is consulted
    once per telemetry batch, there is no timer."""

    def __init__(self, every):
        self.every = every
        self.barrier_open = True
        self.last = None

    def close_barrier(self):
        self.barrier_open = False

    def due(self, tick):
        if self.barrier_open:
            return True
        if self.every == 0:
            return False
        if self.last is None or tick - self.last >= self.every:
            self.last = tick
            return True
        return False


def parse_status(line):
    match = STATUS_PATTERN.search(line)
    if match is None:
        return None
    return {name: int(value) for name, value in zip(STATUS_FIELDS, match.groups()) if value is not None}


def percentile(sorted_values, q):
    """Nearest rank."""
    return sorted_values[max(1, math.ceil(len(sorted_values) * q / 100)) - 1]


def window_summary(rows, first, last):
    """MSPT/TPS/memory over telemetry ticks first..last; a checkpoint capture shows as an owner_post_us spike."""
    window = [row for row in rows if first <= row['tick'] <= last]
    summary = {'first_tick': first, 'last_tick': last, 'samples': len(window), 'missing_ticks': last - first + 1 - len(window)}
    if not window:
        return summary
    wall = sorted(row['wall_us'] for row in window)
    spikes = sorted(row['tick'] for row in window if row['phase_us']['owner_post_us'] >= CHECKPOINT_SPIKE_US)
    elapsed = (window[-1]['wall_start_unix_ns'] - window[0]['wall_start_unix_ns']) / 1e9
    ticks = window[-1]['tick'] - window[0]['tick']
    rss = [row['host']['process_rss_bytes'] for row in window if row.get('host') and row['host'].get('process_rss_bytes') is not None]
    summary.update({
        'wall_us': {'mean': round(sum(wall) / len(wall), 1), 'p50': percentile(wall, 50), 'p95': percentile(wall, 95), 'p99': percentile(wall, 99), 'max': wall[-1]},
        'tps': round(ticks / elapsed, 3) if elapsed > 0 and ticks > 0 else None,
        'owner_post_max_us': max(row['phase_us']['owner_post_us'] for row in window),
        'checkpoint_spike_ticks': spikes,
        'process_rss_max_bytes': max(rss) if rss else None,
        'host_samples': len(rss),
        'deferred_updates_sum_total': sum(row.get('recorders', {}).get('deferred_updates_sum', 0) for row in window),
    })
    return summary


def server_checkpoint_ticks(log_text):
    """Ticks of the server's INFO `checkpoint committed tick=N` lines (journal.rs). Unlike the phase log
    these exist in every run, so a take still has an independent record of each checkpoint."""
    return [int(tick) for tick in re.findall(r'checkpoint committed tick=(\d+)', log_text)]


def ignored_config_keys(log_text):
    """Config keys the server warned it does not know (`ignoring unknown config key ... key=data.x`): a pin
    older than a key silently runs on the default."""
    return re.findall(r'ignoring unknown config key .*?\bkey=(\S+)', log_text)


def take_report(a, rows, census, receipt, first, last, telemetry_gaps, setup_command, server_log):
    """take.json: what was configured, what the window measured, and whether it is a clean take."""
    window = window_summary(rows, first, last)
    checkpoint_ticks = server_checkpoint_ticks(server_log)
    ignored = ignored_config_keys(server_log)
    inside = [tick for tick in checkpoint_ticks if first <= tick <= last]
    before = [tick for tick in checkpoint_ticks if tick < first]
    reasons = []
    if ignored:
        reasons.append(f'server ignored config keys: {", ".join(ignored)}')
    if window.get('checkpoint_spike_ticks'):
        reasons.append('checkpoint capture spike inside the shot window')
    if inside:
        reasons.append('server log records a checkpoint inside the shot window')
    if a.phase_log:
        reasons.append('phase log was on')
    if a.status_every:
        reasons.append('battle status polled after start')
    if receipt.get('marker_gaps'):
        reasons.append('recorder marker gaps')
    if telemetry_gaps:
        reasons.append('telemetry tick gaps')
    if window['missing_ticks']:
        reasons.append('shot window ticks missing from telemetry')
    return {
        'seed': a.seed, 'world_seed': a.world_seed, 'scenario': a.scenario, 'count_per_side_override': a.count,
        'setup_command': setup_command, 'census_before_start': census,
        'config': {'checkpoint_every_ticks': a.checkpoint_every_ticks, 'phase_log': a.phase_log, 'status_every': a.status_every,
                   'view_distance': a.view_distance, 'allow_marker_gaps': a.allow_marker_gaps, 'server_scope': a.server_scope,
                   'forced_checkpoint_before_start': a.take},
        'checkpoint_spike_threshold_us': CHECKPOINT_SPIKE_US,
        'server_checkpoints': {'ticks': checkpoint_ticks, 'in_window': inside, 'last_before_window': before[-1] if before else None,
                               'next_periodic_margin_ticks': a.checkpoint_every_ticks - (last - before[-1]) if before else None},
        'server_ignored_config_keys': ignored,
        'window': window,
        'recorder': {key: receipt.get(key) for key in ('first_tick', 'last_tick', 'marker_gaps')},
        'telemetry': {'rows': len(rows), 'first_tick': rows[0]['tick'] if rows else None, 'last_tick': rows[-1]['tick'] if rows else None,
                      'gaps': telemetry_gaps, 'host_samples': sum(1 for row in rows if row.get('host'))},
        'clean': not reasons, 'unclean_reasons': reasons,
    }


def sha256_file(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def quote(value):
    return json.dumps(str(value))


def read_pin(pin):
    manifest = json.loads((pin / 'manifest.json').read_text())
    if manifest.get('kind') != 'server':
        raise ValueError('pin manifest is not a server pin')
    if manifest.get('incremental'):
        raise ValueError('server pin is an incremental build; a battle measurement needs pin-release --no-incremental')
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


def telemetry_worker(port, output, stop, failures, on_batch, ticks, gaps=None):
    """Export /telemetry.jsonl. `gaps` (a list) records skipped ticks as [last, next]; None makes them fatal."""
    last = None
    try:
        with output.open('x') as target:
            while not stop.is_set():
                text = telemetry_get(port, last)
                rows = [json.loads(line) for line in text.splitlines() if line]
                for row in rows:
                    tick = row['tick']
                    if last is not None and (tick <= last or (tick != last + 1 and gaps is None)):
                        raise ValueError(f'telemetry tick gap or reversal: {last} to {tick}')
                    if last is not None and tick != last + 1:
                        gaps.append([last, tick])
                    target.write(json.dumps(row, separators=(',', ':')) + '\n')
                    last = tick
                target.flush()
                if rows and not stop.is_set():
                    ticks.put(last)
                    on_batch(last)
    except Exception as exc:
        failures.put(exc)
        ticks.put(None)


def console_fifo_worker(path, send):
    """Forward each line written to the FIFO to the server console. Opening blocks until a writer
    appears and reading blocks until it writes: no timer. Each writer's EOF reopens the FIFO."""
    while True:
        with open(path, encoding='utf-8', errors='replace') as fifo:
            for line in fifo:
                command = line.strip()
                if command:
                    send(command)


def stream_log(pipe, file, messages, presence=None):
    try:
        with file.open('x') as output:
            for line in pipe:
                output.write(line)
                output.flush()
                if presence is not None:
                    presence.feed(line)
                if any(marker in line for marker in ('Solaris is listening (edge v2)', 'Battle staged:', 'Battle roster:', 'Battle started', 'Battle paused', 'Saved the game', 'Failed to save the game', 'Command failed:', 'Battle error:', 'Battle watching:', 'Battle live placed:', 'Battle spectator placed:')):
                    messages.put(line)
    finally:
        if presence is not None:
            presence.close()
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
        if 'Command failed:' in line or 'Battle error:' in line or 'Failed to save the game' in line:
            raise RuntimeError(line.strip())


def expected_horses(per_side):
    """staging.rs plan(): each side gets count / 10 cavalry, one horse each."""
    return 2 * (per_side // 10)


def await_population(messages, per_side, roster_tick, recorder, wait=True, census=None):
    """First status newer than roster_tick with the exact staged roster fully resident
    and active. wait=False demands it from that one status (the check right before
    start) instead of waiting for chunks to activate. `census`, if given, receives the
    parsed fields of the status that decided."""
    while True:
        line = expect(messages, 'Battle paused tick=')
        status = parse_status(line)
        if status is None:
            continue
        tick, side1, side2, resident, active, horses = (status[name] for name in ('tick', 'side1', 'side2', 'resident', 'active', 'horses'))
        if tick <= roster_tick:
            continue
        if census is not None:
            census.update(status)
        if recorder.poll() is not None:
            raise RuntimeError('recorder exited while waiting for active population')
        if side1 != per_side or side2 != per_side or horses != expected_horses(per_side):
            raise RuntimeError(f'battle population changed before start: {side1}+{side2}, horses={horses}')
        if resident == 2 * per_side and active == 2 * per_side:
            return tick
        if not wait:
            raise RuntimeError(f'battle population left the active area before start: resident={resident} active={active}')


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
    live_watch, live_placements = live_commands(a)
    if a.dry_run:
        shown_config = server_config_text(a, 'battle-<random>', a.run_dir / 'world', a.cache)
        status_after = 'never' if a.status_every == 0 else f'about every {a.status_every} ticks'
        print('\n'.join([f'server from {a.pin}/manifest.json --config {a.run_dir}/server.toml',
                         'server argv: ' + ' '.join(server_argv(a, f'{a.pin}/<binary>', a.run_dir / 'server.toml', 'battle-<random>')),
                         f'server environment: MC_CORE_PHASE_LOG={"1" if a.phase_log else "unset"}',
                         f'server.toml:\n{shown_config.rstrip()}',
                         *([live_watch, f'live barrier: wait up to {a.live_timeout} s until {", ".join(live_names(a))} are online ("Battle player online: NAME"); log each join time to live-players.json'] if live_watch else []),
                         f'recorder record --server 127.0.0.1:{a.port} --username BattleRecorder --output {a.run_dir}/take.solrec --header {a.run_dir}/header.json --duration {a.duration} --ready-file {a.run_dir}/take.ready --roster-file {a.run_dir}/roster.txt --roster-ready-file {a.run_dir}/roster.ready --roster-timeout 900 --data-dir {a.client_data}',
                         'wait for recorder initial readiness',
                         f'export GET http://127.0.0.1:{a.dashboard_port}/telemetry.jsonl; before start each telemetry batch requests battle status (population barrier), after start {status_after}',
                         commands[0], 'battle roster',
                         'publish exact UUID roster and wait for recorder roster readiness',
                         'wait for fresh exact side counts and fully resident, active soldiers',
                         f'wait at least {a.paused_ticks} paused ticks',
                         *[f'{placement} (wait for "Battle {"live" if placement.startswith("battle live") else "spectator"} placed: {placement.split()[2]}")' for placement in live_placements],
                         *(['recount: exact side counts, fully resident and active'] if live_placements else []),
                         *([f'save-all (forced checkpoint right before start, restarts the {a.checkpoint_every_ticks}-tick cadence); wait for "Saved the game"'] if a.take else []),
                         commands[1],
                         f'require at least {a.minimum_battle_ticks} continuously recorded battle ticks',
                         *(['write take.json: window MSPT/TPS/memory, checkpoint spikes in the shot window, clean verdict'] if a.take else []),
                         *commands[2:]]))
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
    config.write_text(server_config_text(a, server_name, world, cache))
    (root / 'commands.txt').touch(exist_ok=False)
    recorder_argv = [str(client), 'record', '--server', f'127.0.0.1:{a.port}', '--username', 'BattleRecorder', '--output', str(root / 'take.solrec'), '--header', str(root / 'header.json'), '--duration', str(a.duration), '--ready-file', str(root / 'take.ready'), '--roster-file', str(root / 'roster.txt'), '--roster-ready-file', str(root / 'roster.ready'), '--roster-timeout', '900', '--data-dir', str(client_data)]
    launch_argv = server_argv(a, binary, config, server_name)
    (root / 'manifest.json').write_text(json.dumps({'server_pin': manifest, 'client_sha256': sha256_file(client), 'seed': a.seed, 'world_seed': a.world_seed, 'scenario': a.scenario, 'count_per_side_override': a.count, 'world_source': str(a.world) if a.world else None, 'platform': platform.platform(), 'uname': list(platform.uname()), 'server_argv': launch_argv, 'recorder_argv': recorder_argv, 'server_environment_overrides': {'MC_CORE_PHASE_LOG': '1'} if a.phase_log else {}, 'server_environment_removed': [] if a.phase_log else ['MC_CORE_PHASE_LOG'], 'take': a.take, 'checkpoint_every_ticks': a.checkpoint_every_ticks, 'status_every': a.status_every, 'view_distance': a.view_distance, 'allow_marker_gaps': a.allow_marker_gaps, 'server_scope': a.server_scope}, indent=2) + '\n')
    with (root / 'host-commands.txt').open('x') as host:
        for cmd in (['uname', '-a'], ['lscpu']):
            host.write('$ ' + ' '.join(cmd) + '\n')
            host.flush()
            subprocess.run(cmd, stdout=host, stderr=subprocess.STDOUT, check=True, text=True)
    messages, failures, ticks, stop = queue.Queue(), queue.Queue(), queue.Queue(), threading.Event()
    status_policy = StatusPolicy(a.status_every)
    presence = LivePresence(live_names(a)) if live_watch else None
    telemetry_gaps = []
    server = None
    recorder = None
    telemetry = None
    log_thread = None
    try:
        environment = server_environment(a, os.environ)
        server = subprocess.Popen(launch_argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1, env=environment, cwd=root)
        command_lock = threading.Lock()
        def send(command):
            with command_lock:
                with (root / 'commands.txt').open('a') as record:
                    record.write(command + '\n')
                server.stdin.write(command + '\n')
                server.stdin.flush()
        log_thread = threading.Thread(target=stream_log, args=(server.stdout, root / 'server.log', messages, presence), daemon=True)
        log_thread.start()
        if a.console_fifo:
            os.mkfifo(root / 'console.fifo', 0o600)
            threading.Thread(target=console_fifo_worker, args=(root / 'console.fifo', send), daemon=True).start()
        expect(messages, 'Solaris is listening (edge v2)')
        ignored = ignored_config_keys((root / 'server.log').read_text(errors='replace'))
        if a.take and any(key.endswith('checkpoint_every_ticks') for key in ignored):
            raise RuntimeError('server pin ignores data.checkpoint_every_ticks (it predates the key, 95c4bc59): a take cannot control its checkpoint cadence; pin a newer server')
        if a.server_scope is not None:
            (root / 'server-scope.json').write_text(json.dumps({'unit': f'{server_name}.scope', 'pid': server.pid, 'cgroup': Path(f'/proc/{server.pid}/cgroup').read_text().strip()}, indent=2) + '\n')
        if presence is not None:
            # Before the recorder: its take duration and roster timeout start when it launches.
            send(live_watch)
            expect(messages, 'Battle watching:')
            print(f'{utc_now()} live barrier: waiting up to {a.live_timeout} s for {", ".join(live_names(a))}', flush=True)
            presence.wait_online(live_names(a), a.live_timeout)
            for name, at in presence.first_online().items():
                print(f'live barrier: {name} online at {at}', flush=True)
            presence.barrier_passed = utc_now()
            (root / 'live-players.json').write_text(json.dumps({'players': a.live_players, 'spectators': a.live_spectators, **presence.receipt()}, indent=2) + '\n')
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
            def request_status(tick):
                if status_policy.due(tick):
                    send('battle status')
            telemetry = threading.Thread(target=telemetry_worker, args=(a.dashboard_port, root / 'telemetry.jsonl', stop, failures, request_status, ticks, telemetry_gaps if a.allow_marker_gaps else None), daemon=True)
            telemetry.start()
            send(commands[0])
            expect(messages, 'Battle staged:')
            send('battle roster')
            roster_line = expect(messages, 'Battle roster:')
            names = roster_line.split('Battle roster:', 1)[1].strip().split(',')
            ids = [str(uuid.UUID(name)) for name in names]
            per_side = a.count or {'field-100': 100, 'field-1k': 1000, 'field-10k': 10000}[a.scenario]
            if len(ids) != 2 * per_side + expected_horses(per_side) or len(ids) != len(set(ids)):
                raise RuntimeError('battle roster is not the exact staged soldiers and horses, or contains duplicate UUIDs')
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
            # Live players enter the rank once every soldier stands on its ground: the slot's
            # soldier gives his ground and the file steps one rank back (battle/live_players.rs).
            for placement in live_placements:
                _, verb, name, other = placement.split()
                presence.wait_online([name] if verb == 'live' else [name, other], a.live_timeout)
                send(placement)
                expect(messages, f'Battle {"live" if verb == "live" else "spectator"} placed: {name} ')
            # The paused window can be long; census again right before start.
            send('battle status')
            census = {}
            await_population(messages, per_side, paused_end or population_tick, recorder, wait=False, census=census)
            status_policy.close_barrier()
            if a.take:
                # Restart the cadence counter outside the shot window: one capture now, none until after the take.
                send('save-all')
                expect(messages, 'Saved the game')
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
                or type(last_tick) is not int or type(gaps) is not int or (gaps != 0 and not a.allow_marker_gaps)
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
        result = {'status': 'complete', 'recording': 'take.solrec', 'telemetry': 'telemetry.jsonl', 'server_log': 'server.log'}
        if a.take:
            # After the server exited and its log is complete: the checkpoint lines are the independent record.
            if log_thread is not None:
                log_thread.join(timeout=10)
            rows = [json.loads(line) for line in (root / 'telemetry.jsonl').read_text().splitlines() if line]
            server_log = (root / 'server.log').read_text(errors='replace')
            report = take_report(a, rows, census, record_receipt, battle_start + 1, last_tick, telemetry_gaps, commands[0], server_log)
            (root / 'take.json').write_text(json.dumps(report, indent=2) + '\n')
            result.update({'take': 'take.json', 'take_clean': report['clean']})
            if not report['clean']:
                print('take is not clean: ' + '; '.join(report['unclean_reasons']), file=sys.stderr)
        (root / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    finally:
        stop.set()
        stop_child(recorder, 10)
        stop_child(server, 30)
        if telemetry is not None:
            telemetry.join(timeout=20)
        if log_thread is not None:
            log_thread.join(timeout=5)
        if presence is not None:
            # Every join, leave, placement, death and respawn of the live names, with wall time.
            (root / 'live-players.json').write_text(json.dumps({'players': a.live_players, 'spectators': a.live_spectators, **presence.receipt()}, indent=2) + '\n')


if __name__ == '__main__':
    args = arguments()
    # A stop request (a long trial ended by `kill`) unwinds through run()'s finally: the recorder and
    # server are stopped and the live-player receipt is written.
    signal.signal(signal.SIGTERM, lambda signum, frame: sys.exit(128 + signum))
    try:
        run(args)
    except Exception as error:
        if hasattr(args, 'created_dir'):
            (args.created_dir / 'result.json').write_text(json.dumps({'status': 'failed', 'error': str(error)}, indent=2) + '\n')
        raise
