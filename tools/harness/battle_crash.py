"""Battle SIGKILL custody campaign, driven through the pinned battle runner."""
from __future__ import annotations

import argparse
import ctypes
import hashlib
import math
import json
import os
import re
import signal
import select
import subprocess
import struct
import sys
import uuid
import time
from pathlib import Path

from tools.battle.crash_snapshot import verified_copy, verify_tree
from . import runtime
from .backends.protocol_lifecycle import reserve_matrix_port
from .crash_wal import classify_observation, read_published
from .matrix_execution import offline_uuid
from .protocol_matrix_client import ProtocolMatrixClient

ROOT = runtime.REPO_ROOT
RUNNER = ROOT / 'tools/battle/run-battle.py'
MOMENTS = ('before-checkpoint', 'during-append', 'after-publish', 'after-checkpoint')
CHECKPOINT = re.compile(r'checkpoint committed tick=(\d+)')


def fields(text):
    """Split one console SNBT compound at its top level, preserving exact values."""
    if not text.startswith('{') or not text.endswith('}'):
        raise ValueError('not an SNBT compound')
    result, start, depth, quote, escape = {}, 1, 0, False, False
    def insert(part):
        part = part.strip()
        if part.startswith('"'):
            key, end = json.JSONDecoder().raw_decode(part)
            if not part[end:].lstrip().startswith(':'):
                raise ValueError('missing SNBT field separator')
            value = part[end:].lstrip()[1:]
        else:
            key, value = part.split(':', 1)
            key = key.strip()
        if key in result:
            raise ValueError(f'duplicate SNBT field {key!r}')
        result[key] = value.strip()

    for index in range(1, len(text)):
        char = text[index]
        if quote:
            if escape:
                escape = False
            elif char == '\\':
                escape = True
            elif char == '"':
                quote = False
        elif char == '"':
            quote = True
        elif char in '[{':
            depth += 1
        elif char in ']}':
            if char == '}' and depth == 0:
                part = text[start:index].strip()
                if part:
                    insert(part)
                return result
            depth -= 1
        elif char == ',' and depth == 0:
            insert(text[start:index])
            start = index + 1
    raise ValueError('unterminated SNBT')


def census(text, dimension=None, *, allow_empty=False):
    result = {}
    for line in text.splitlines():
        marker = ' has the following entity data: '
        if marker not in line:
            continue
        label, data = line.split(marker, 1)
        values = fields(data)
        ident = values.get('UUID')
        if not ident or ident in result:
            raise ValueError(f'entity missing or duplicate UUID: {ident}')
        result[ident] = {'type': label, 'dimension': dimension, 'holders': {key: values[key] for key in
                          ('Inventory', 'EnderItems', 'Items', 'HandItems', 'ArmorItems',
                           'Item', 'item', 'FireworksItem', 'equipment')
                          if key in values},
                         'age': values.get('Age'), 'life': values.get('life'), 'pos': values.get('Pos'),
                         'in_ground': values.get('inGround'), 'tags': values.get('Tags', '[]')}
    if not result and not allow_empty:
        raise ValueError('empty census')
    return result


def dimension_census(console):
    """Spatial selector scopes @e to each dimension; check against the global set."""
    result = {}
    for dimension in ('minecraft:overworld', 'minecraft:the_nether', 'minecraft:the_end'):
        text = console.command(f'execute in {dimension} run execute as @e[distance=0..] run data get entity @s')
        part = census(text, dimension, allow_empty=True)
        if result.keys() & part.keys():
            raise ValueError('duplicate UUID in dimension census')
        result.update(part)
    global_entities = census(console.command('execute as @e run data get entity @s'), allow_empty=True)
    if not result or result.keys() != global_entities.keys():
        raise ValueError('dimension census does not cover global entity census')
    return result


def visit(entry):
    if not isinstance(entry, dict):
        return
    if 'Dimension' in entry:
        for child in entry['Dimension']['entries']:
            yield from visit(child)
    if 'Player' in entry:
        yield entry['Player']


def grant_images(wal, player_uuid, item_id):
    found = []
    for frame in wal['frames']:
        for entry in frame['entries']:
            for player in visit(entry):
                if player['uuid'] == str(player_uuid) and any(
                    stack.get('item_id') == item_id and stack.get('count') == 5
                    for stack in player.get('inventory', [])):
                    found.append({'file': frame['file'], 'lo': frame['lo'], 'hi': frame['hi']})
    return found


def uuid_snbt(value):
    number = uuid.UUID(value).int
    return '[I; ' + ', '.join(str(n if n < 2**31 else n - 2**32) for n in
        ((number >> 96) & 0xffffffff, (number >> 64) & 0xffffffff,
         (number >> 32) & 0xffffffff, number & 0xffffffff)) + ']'


def projectile_lifetimes(wal, through_tick):
    """Use the projectile's own validated WAL post-image, not a global constant."""
    result = {}
    def entities(entry):
        if isinstance(entry, dict) and 'Dimension' in entry:
            for child in entry['Dimension']['entries']:
                yield from entities(child)
        elif isinstance(entry, dict) and 'Entity' in entry:
            yield entry['Entity']
    for frame in wal['frames']:
        if frame['hi'] > through_tick:
            continue
        for entry in frame['entries']:
            for entity in entities(entry):
                projectile = entity.get('projectile')
                if isinstance(projectile, dict) and projectile.get('kind') in ('Arrow', 'SpectralArrow'):
                    lifetime = projectile.get('ground_lifetime')
                    if type(lifetime) is int and lifetime > 0:
                        result[uuid_snbt(entity['uuid'])] = {
                            'lifetime': lifetime, 'file': frame['file'], 'tick_hi': frame['hi']}
    return result


def compare_entities(before, after, lifetimes=None, elapsed_upper=None):
    """Classify possibly expired arrows as unverified, never as preserved."""
    lifetimes = lifetimes or {}
    missing, unverified = [], {}
    for ident in sorted(before.keys() - after.keys()):
        entity = before[ident]
        life = entity.get('life')
        proof = lifetimes.get(ident)
        if (entity['type'] == 'Arrow' and entity.get('in_ground') == '1b'
                and isinstance(life, str) and re.fullmatch(r'\d+s', life)
                and proof and elapsed_upper is not None
                and int(life[:-1]) + elapsed_upper >= proof['lifetime']):
            unverified[ident] = {'life': int(life[:-1]), 'elapsed_upper': elapsed_upper,
                                 'lifetime_evidence': proof}
        else:
            missing.append(ident)
    changed = sorted(ident for ident in before.keys() & after.keys()
                     if (before[ident]['type'], before[ident]['holders'])
                     != (after[ident]['type'], after[ident]['holders']))
    return {'missing': missing, 'new': sorted(after.keys() - before.keys()),
            'changed': changed, 'unverified': unverified}


def cut_verdict(comparison, *, valid, custody_ok, probe_ok):
    if (not valid or not custody_ok or not probe_ok
            or any(comparison[key] for key in ('missing', 'new', 'changed'))):
        return 'failed'
    return 'incomplete' if comparison['unverified'] else 'passed'


def verify_witnesses(run, snapshot, baseline, grant_inventory):
    for name, expected in (('before-census.json', baseline),
                           ('acknowledged-inventory.json', grant_inventory)):
        witness = snapshot / 'witness' / name
        if witness.read_bytes() != (run / name).read_bytes() or json.loads(witness.read_text()) != expected:
            raise ValueError(f'pre-recovery witness mismatch: {name}')


def project_world(run, copied_world, name):
    """Project a verified disposable world without starting the tick owner."""
    command = json.loads((run / 'server-command.json').read_text())
    receipt = (run / f'{name}.json').resolve()
    argv = [command['binary'], '--config', command['config'], 'world', 'crash-census',
            '--copy', str(copied_world), '--receipt', str(receipt)]
    result = subprocess.run(argv, cwd=run, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            timeout=300, check=False)
    (run / f'{name}.log').write_bytes(result.stdout)
    if result.returncode:
        raise RuntimeError(f'{name} failed ({result.returncode}): {run / f"{name}.log"}')
    projected = json.loads(receipt.read_text())
    return projected


def projected_entities(projected):
    return {uuid_snbt(ident): {**row, 'dimension': dimension}
            for dimension, entities in projected['entities'].items()
            for ident, row in entities.items()}


def durable_census(run, snapshot, wal):
    """Project holders from the verified inspection copy, never the primary evidence."""
    verify_tree(snapshot, json.loads((run / 'crash-handoff.json').read_text()))
    projected = project_world(run, (snapshot / 'world').resolve(), 'durable-census')
    if (projected['checkpoint_tick'], projected['durable_tick']) != (
            wal['checkpoint_tick'], wal['published_through']):
        raise ValueError('durable census checkpoint/WAL fence mismatch')
    return projected_entities(projected)


def compare_wire_ids(expected, actual):
    """Players reconnect with new wire IDs; compare only persisted non-player IDs."""
    expected = {ident: row for ident, row in expected.items() if row['kind'] != 'player'}
    actual = {ident: row for ident, row in actual.items() if row['kind'] != 'player'}
    missing = sorted(expected.keys() - actual.keys())
    new = sorted(actual.keys() - expected.keys())
    changed = sorted(ident for ident in expected.keys() & actual.keys()
                     if not isinstance(expected[ident].get('wire_id'), int)
                     or (expected[ident]['dimension'], expected[ident]['wire_id'])
                     != (actual[ident]['dimension'], actual[ident].get('wire_id')))
    return {'missing': missing, 'new': new, 'changed': changed}


def apply_wire_comparison(comparison, wire):
    comparison['changed'] = sorted(set(comparison['changed']) | set(wire['changed']) | set(wire['new']))
    for ident in wire['missing']:
        if ident not in comparison['missing']:
            comparison['unverified'][ident] = {'reason': 'wire spawn not observed'}


def observe_wire_ids(console, run, address, cache, expected, early):
    """Observe persisted non-players through real spawn packets, grouped by dimension/chunk."""
    groups = {}
    for ident, saved in expected.items():
        if saved['kind'] == 'player' or ident not in early:
            continue
        raw = early[ident].get('pos')
        if not isinstance(raw, str) or not raw.startswith('[') or not raw.endswith(']'):
            raise ValueError(f'missing census position for {ident}')
        values = [float(value.strip().removesuffix('d').removesuffix('f'))
                  for value in raw[1:-1].split(',')]
        if len(values) != 3 or not all(math.isfinite(value) for value in values):
            raise ValueError(f'invalid census position for {ident}')
        dimension = early[ident]['dimension']
        groups.setdefault((dimension, math.floor(values[0] / 16), math.floor(values[2] / 16)),
                          []).append((ident, values))
    actor = ProtocolMatrixClient('WireWitness', address, run, cache, console.command,
                                 account_uuid=offline_uuid('WireWitness'), timeout_seconds=120)
    observed, visits = {}, []
    try:
        reply = console.command('gamemode spectator WireWitness')
        if 'spectator' not in reply.lower():
            raise RuntimeError(f'could not set wire witness spectator: {reply[-500:]}')
        for (dimension, chunk_x, chunk_z), members in sorted(groups.items()):
            pos = members[0][1]
            teleports = actor.observer.teleports
            console.command(f'execute in {dimension} run tp WireWitness {pos[0]} {pos[1]} {pos[2]}')
            actor.wait_until(lambda: actor.observer.teleports > teleports,
                             description='wire witness teleport', timeout_seconds=120)
            actor.wait_until(lambda: (chunk_x, chunk_z) in actor.observer.chunks,
                             description='wire witness destination chunk', timeout_seconds=120)
            dimension_reply = console.command('data get entity WireWitness Dimension')
            if dimension not in dimension_reply:
                raise RuntimeError(f'wire witness dimension mismatch: {dimension_reply[-500:]}')
            wanted = {ident for ident, _ in members}
            def seen_members():
                snapshot = actor.snapshot()
                return wanted <= {uuid_snbt(entity['uuid']) for entity in snapshot.get('entities', [])}
            try:
                actor.wait_until(seen_members, description='wire witness entity spawns', timeout_seconds=15)
            except TimeoutError:
                pass  # Missing packets remain unverified; they can never be counted as a pass.
            snapshot = actor.snapshot()
            errors = snapshot.get('observation_errors', {})
            if errors.get('entities') or errors.get('entity_lifecycle'):
                raise RuntimeError(f'wire entity observer incomplete: {errors}')
            matching = []
            for entity in snapshot.get('entities', []):
                ident = uuid_snbt(entity['uuid'])
                if ident not in wanted:
                    continue
                row = {'kind': expected[ident]['kind'], 'dimension': dimension, 'wire_id': entity['id']}
                if ident in observed and observed[ident] != row:
                    raise ValueError(f'conflicting wire identities for {ident}')
                observed[ident] = row
                matching.append(entity)
            visits.append({'dimension': dimension, 'chunk': [chunk_x, chunk_z],
                           'expected': sorted(wanted), 'observed': matching})
    finally:
        actor.disconnect()
    receipt = {'observed': observed, 'visits': visits}
    save_json(run / 'wire-identity.json', receipt)
    return observed


def compare_durable(expected, actual):
    """Compare durable entity/player holders against the frozen recovery census."""
    missing = sorted(expected.keys() - actual.keys())
    new = sorted(actual.keys() - expected.keys())
    changed = []
    for ident in sorted(expected.keys() & actual.keys()):
        saved, live = expected[ident], actual[ident]
        kind = saved['kind'].split(':', 1)[-1].replace('_', '').lower()
        label = live['type'].replace(' ', '').lower()
        if (not saved.get('dimension') or saved['dimension'] != live.get('dimension')
                or (kind != 'player' and kind != label) or saved['holders'] != live['holders']):
            changed.append(ident)
    return {'missing': missing, 'new': new, 'changed': changed, 'unverified': {}}


def published_removal(wal, ident, before_lo, before_hi):
    """Prove UUID-to-wire-id continuity and a later dimension-scoped tombstone."""
    def visit_entries(entry, dimension='minecraft:overworld'):
        if 'Dimension' in entry:
            child = entry['Dimension']
            for nested in child['entries']:
                yield from visit_entries(nested, child['dimension'])
        else:
            yield dimension, entry

    events = []
    for frame in wal['frames']:
        for entry in frame['entries']:
            for dimension, item in visit_entries(entry):
                if image := item.get('Entity'):
                    events.append(('image', image['tick'], dimension, image['id'],
                                   uuid_snbt(image['uuid']), frame))
                if removal := item.get('RemoveEntity'):
                    events.append(('remove', removal['tick'], dimension, removal['id'], None, frame))
    prior = [index for index, event in enumerate(events)
             if event[0] == 'image' and event[4] == ident and event[1] <= before_lo]
    if not prior:
        return None
    image = events[prior[-1]]
    relevant = [event for event in events[prior[-1] + 1:]
                if (event[2], event[3]) == (image[2], image[3]) or event[4] == ident]
    if not relevant or relevant[-1][0] != 'remove' or not before_hi < relevant[-1][1] <= wal['published_through']:
        return None
    if any(event[0] != 'image' or (event[2], event[3], event[4]) !=
           (image[2], image[3], ident) for event in relevant[:-1]):
        return None
    expected = image[5]['lo']
    for frame in wal['frames']:
        if frame['hi'] < expected:
            continue
        if frame['lo'] != expected:
            return None
        expected = frame['hi'] + 1
    if expected <= wal['published_through']:
        return None
    return {'dimension': image[2], 'id': image[3], 'image_frame': image[5]['file'],
            'removal_frame': relevant[-1][5]['file'], 'removal_sha256': relevant[-1][5]['sha256']}


def simulation_tick(console):
    reply = console.command('time query gametime')
    match = re.search(r'(?:The time is\s+|commands\.time\.query[^0-9]*)(\d+)', reply)
    if match is None:
        raise ValueError(f'missing authoritative game tick: {reply!r}')
    return int(match.group(1))


def recipient_custody(granted, inventory, cursor):
    """Allow the exact acknowledged placement, or an empty unpublished grant."""
    return inventory == granted, inventory in ([], granted) and cursor is None


def combine_censuses(early, after, player_ids, recipient):
    combined = {key: value for key, value in early.items() if key not in player_ids}
    combined.update({key: value for key, value in after.items() if key in player_ids and key != recipient})
    return combined


def wait_path(path, predicate, process, timeout=240):
    def ready():
        if process.poll() is not None:
            raise RuntimeError(f'process exited {process.returncode} while waiting on {path}')
        return predicate()
    return runtime.wait_for_path_condition(path, timeout, ready)


def kill_on_append(journal, pid, runner, timeout=60):
    """Kill on a real temporary WAL creation event, then classify from the copy."""
    libc = ctypes.CDLL(None, use_errno=True)
    fd = libc.inotify_init1(os.O_CLOEXEC | os.O_NONBLOCK)
    if fd < 0:
        raise OSError(ctypes.get_errno(), 'inotify_init1')
    try:
        if libc.inotify_add_watch(fd, os.fsencode(journal), 0x100) < 0:
            raise OSError(ctypes.get_errno(), 'inotify_add_watch')
        pidfd = os.pidfd_open(runner.pid)
        try:
            deadline = time.monotonic() + timeout
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError('no WAL append creation event')
                ready, _, _ = select.select([fd, pidfd], [], [], remaining)
                if pidfd in ready:
                    raise RuntimeError('battle runner exited before WAL append')
                if fd in ready:
                    data = os.read(fd, 65536)
                    offset = 0
                    while offset < len(data):
                        _, mask, _, length = struct.unpack_from('iIII', data, offset)
                        name = os.fsdecode(data[offset + 16:offset + 16 + length].split(b'\0', 1)[0])
                        offset += 16 + length
                        if mask & 0x4000:
                            raise RuntimeError('WAL append inotify queue overflow')
                        if mask & 0x100 and re.fullmatch(r'batch-\d+-\d+\.bin\.tmp', name):
                            os.kill(pid, signal.SIGKILL)
                            return name
        finally:
            os.close(pidfd)
    finally:
        os.close(fd)


class Console:
    def __init__(self, log, process, send):
        self.log, self.process, self.send = log, process, send

    def command(self, text):
        if '\n' in text or '\r' in text:
            raise ValueError('one console command required')
        marker = 'CrashFence' + uuid.uuid4().hex
        fence = f'Scores for {marker}: {{}}'
        offset = len(self.log.read_text(errors='replace'))
        self.send(text + '\nscoreboard players list ' + marker + '\n')
        def arrived():
            fresh = self.log.read_text(errors='replace')[offset:]
            return (fence in fresh, fresh)
        fresh = wait_path(self.log, arrived, self.process)
        return fresh.split(fence, 1)[0]

    def wait_reply(self, text, wanted, failures=()):
        offset = len(self.log.read_text(errors='replace'))
        self.send(text + '\n')
        def arrived():
            fresh = self.log.read_text(errors='replace')[offset:]
            if any(item in fresh for item in failures):
                raise RuntimeError(f'{text} failed: {fresh[-1000:]}')
            return (wanted in fresh, fresh)
        return wait_path(self.log, arrived, self.process, 300)


def require(text, expected):
    if expected not in text:
        raise RuntimeError(f'missing {expected!r} in command reply: {text[-1000:]}')


def checkpoint(log):
    matches = CHECKPOINT.findall(log.read_text(errors='replace'))
    if not matches:
        raise RuntimeError('checkpoint commit not observed')
    return int(matches[-1])


def save_json(path, obj):
    path.write_text(json.dumps(obj, indent=2, default=str) + '\n')


def one(run, moment, pin, client_binary, client_data, cache, world, negative_control=False):
    port, dashboard = reserve_matrix_port(), None
    dashboard = reserve_matrix_port(exclude=port)
    run.parent.mkdir(parents=True, exist_ok=True)
    argv = [sys.executable, str(RUNNER), '--pin', str(pin), '--client', str(client_binary),
            '--client-data', str(client_data), '--cache', str(cache), '--run-dir', str(run),
            '--seed', '46', '--world-seed', '935', '--scenario', 'field-100', '--count', '40',
            '--duration', '1800', '--port', str(port), '--dashboard-port', str(dashboard),
            '--world', str(world), '--live-player', 'Proto0:red', '--console-fifo', '--crash-handoff',
            '--no-phase-log']
    record = {'moment': moment, 'command': argv, 'status': 'failed'}
    process = recovery = recovery_input = None
    actors = []
    try:
        with (run.parent / (run.name + '-driver.log')).open('wb') as log:
            process = subprocess.Popen(argv, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            wait_path(run.parent / '.event', lambda: (run.exists(), None), process)
            wait_path(run / 'server.log', lambda: ((run / 'server.log').exists() and
                'Krasnokamen is listening (edge v2)' in (run / 'server.log').read_text(errors='replace'), None), process)
            logfile, fifo = run / 'server.log', run / 'console.fifo'
            wait_path(fifo, lambda: (fifo.exists(), None), process)
            def send_fifo(text):
                with fifo.open('w') as out:
                    out.write(text)
            console = Console(logfile, process, send_fifo)
            for name in ('Proto0', 'Proto1'):
                actors.append(ProtocolMatrixClient(name, ('127.0.0.1', port), run, cache, console.command,
                                                  account_uuid=offline_uuid(name), timeout_seconds=120))
            wait_path(logfile, lambda: ('Battle started tick=' in logfile.read_text(errors='replace'), None), process, 900)
            require(console.command('battle status'), 'Battle running')
            console.wait_reply('backup', ' complete:', ('Backup failed:', 'Backups are disabled'))
            if not list((run / 'backups').glob('*/manifest.json')):
                raise RuntimeError('backup completion has no manifest')
            require(console.command('summon minecraft:arrow -152 110 145 '
                                   '{NoGravity:1b,Tags:["crashgive_probe"]}'), 'Summoned')
            if negative_control:
                # The control needs an actually observed old arrow with headroom.
                # Each fenced query waits on server events, not elapsed wall time.
                for _ in range(120):
                    observed = dimension_census(console)
                    if any(entity['type'] == 'Arrow' and entity.get('in_ground') == '1b'
                           and isinstance(entity.get('life'), str)
                           and re.fullmatch(r'\d+s', entity['life'])
                           and 70 <= int(entity['life'][:-1]) <= 85
                           for entity in observed.values()):
                        break
                else:
                    raise RuntimeError('no grounded old arrow reached control age')
            console.wait_reply('save-all', 'Saved the game', ('Failed to save the game',))
            baseline_checkpoint = checkpoint(logfile)
            before_tick_lo = simulation_tick(console)
            baseline = dimension_census(console)
            before_tick_hi = simulation_tick(console)
            save_json(run / 'before-census.json', baseline)
            roster = {line.strip() for line in (run / 'roster.txt').read_text().splitlines() if line.strip()}
            probes = [ident for ident, entity in baseline.items()
                      if entity['type'] == 'Arrow' and '"crashgive_probe"' in entity['tags']]
            if len(probes) != 1:
                raise RuntimeError(f'expected exactly one tagged projectile probe, got {len(probes)}')
            probe_id = probes[0]
            if not {uuid_snbt(value) for value in roster} <= baseline.keys():
                raise RuntimeError('pre-cut census does not cover the runner roster')
            witness_id = uuid_snbt(str(offline_uuid('Proto1')))
            if witness_id not in baseline:
                raise RuntimeError('pre-cut census lacks the recipient')
            actors[1].wait_until(lambda: {'inventory', 'cursor'} <= actors[1].snapshot().keys(),
                                 description='recipient inventory and cursor observed', timeout_seconds=60)
            initial_witness = actors[1].snapshot()
            if (initial_witness['inventory'] or initial_witness['cursor'] is not None
                    or baseline[witness_id]['holders'].get('Inventory') != '[]'):
                raise RuntimeError('recipient inventory or cursor was not initially empty')
            kit = next((entity for entity in baseline.values() if entity['type'] == 'Proto0'), None)
            if not kit or not any(value not in ('[]', '{}') for value in kit['holders'].values()):
                raise RuntimeError('battle kit was not observed in the live player census')
            require(console.command('battle status'), 'Battle running')
            grant_reply = console.command('give Proto1 minecraft:netherite_ingot 5')
            require(grant_reply, 'Gave 5')
            actors[1].wait_until(lambda: any(s['item_id'] == 'minecraft:netherite_ingot' and s['count'] == 5
                                           for s in actors[1].snapshot().get('inventory', [])),
                                 description='acknowledged give inventory', timeout_seconds=60)
            granted = actors[1].snapshot()
            if len(granted.get('inventory', [])) != 1 or granted.get('cursor', 'unobserved') is not None:
                raise RuntimeError('acknowledged grant did not leave one stack and an empty cursor')
            grant_inventory = granted['inventory']
            save_json(run / 'acknowledged-inventory.json', grant_inventory)
            item_id = next(number for number, name in actors[1].items.items() if name == 'minecraft:netherite_ingot')
            archive = run / 'wal-archive'
            if moment in ('after-publish', 'after-checkpoint'):
                def published():
                    evidence = read_published(archive, baseline_checkpoint)
                    images = [image for image in grant_images(evidence, offline_uuid('Proto1'), item_id)
                              if image['hi'] > baseline_checkpoint]
                    return (bool(images), images)
                record['grant_images_before_kill'] = wait_path(archive / '.event', published, process)
            if moment == 'after-checkpoint':
                console.wait_reply('save-all', 'Saved the game', ('Failed to save the game',))
            pid = json.loads((run / 'server-pid.json').read_text())['pid']
            if moment == 'during-append':
                record['append_create'] = kill_on_append(run / 'world/journal', pid, process)
            else:
                os.kill(pid, signal.SIGKILL)
            for actor in actors:
                actor.disconnect()
            actors.clear()
            code = process.wait(timeout=300)
            if code != 0:
                raise RuntimeError(f'battle handoff failed {code}; see driver log')
            handoff = json.loads((run / 'crash-handoff.json').read_text())
            record['pre_recovery_evidence'] = {
                name: {'path': str(run / name),
                       'sha256': hashlib.sha256((run / name).read_bytes()).hexdigest()}
                for name in ('before-census.json', 'acknowledged-inventory.json')}
            save_json(run / 'cut-evidence.json', record)
            if handoff['exit'] != -signal.SIGKILL:
                raise RuntimeError('missing SIGKILL proof')
            primary = Path(handoff['crash_copy'])
            snapshot = run / 'crash-inspection-copy'
            record['crash_verification'] = verified_copy(primary, snapshot, handoff)
            record['crash_copy'] = str(primary)
            verify_witnesses(run, snapshot, baseline, grant_inventory)
            save_json(run / 'cut-evidence.json', record)
            cp = checkpoint(snapshot / 'server.log')
            wal = read_published(snapshot / 'wal-archive', cp)
            retained = read_published(snapshot / 'world/journal', cp)
            tail = lambda value: [(frame['lo'], frame['hi'], frame['sha256'])
                                  for frame in value['frames'] if frame['hi'] > cp]
            if tail(wal) != tail(retained):
                raise ValueError('archived and retained published WAL tails differ')
            images = [image for image in grant_images(wal, offline_uuid('Proto1'), item_id)
                      if image['hi'] > baseline_checkpoint]
            tmp = [str(p.name) for p in (snapshot / 'world/journal').glob('*.tmp') if p.stat().st_size]
            durable = durable_census(run, snapshot, wal)
            image = images[-1] if images else None
            image_checkpointed = any(image['hi'] <= cp for image in images)
            valid = {'before-checkpoint': cp == baseline_checkpoint,
                     'during-append': record.get('append_create') in tmp,
                     'after-publish': bool(image and not image_checkpointed),
                     'after-checkpoint': image_checkpointed}[moment]
            record.update({'baseline_checkpoint': baseline_checkpoint, 'checkpoint_at_cut': cp,
                           'published_through': wal['published_through'], 'grant_images': images,
                           'unpublished_temporary_files': tmp, 'moment_observed': valid,
                           'crash_copy': str(primary), 'wal_archive': str(snapshot / 'wal-archive')})
            save_json(run / 'cut-evidence.json', record)
            recovered_log = run / 'recovery.log'
            with recovered_log.open('wb') as out:
                read_fd, write_fd = os.pipe()
                try:
                    recovery = subprocess.Popen([sys.executable, str(RUNNER), '--recover-from-crash',
                                                 str(run), '--start-frozen'],
                                                cwd=ROOT, stdin=read_fd, stdout=out,
                                                stderr=subprocess.STDOUT, start_new_session=True)
                except BaseException:
                    os.close(write_fd)
                    raise
                finally:
                    os.close(read_fd)
                recovery_input = os.fdopen(write_fd, 'wb', buffering=0)
                wait_path(recovered_log, lambda: ('Krasnokamen is listening (edge v2)' in
                    recovered_log.read_text(errors='replace'), None), recovery, 240)
                marker = re.search(r'recovery simulation frozen before first tick tick=(\d+)',
                                   recovered_log.read_text(errors='replace'))
                if marker is None or int(marker[1]) != wal['published_through']:
                    raise RuntimeError('recovery freeze barrier missing or at the wrong durable tick')
                recovery_text = recovered_log.read_text(errors='replace')
                if (any((run / 'empty-plugins').iterdir()) or
                        'plugin.runtime_ready:' in recovery_text):
                    raise RuntimeError('recovery fixture must have zero deployed plugins')
                def send_recovery(text):
                    recovery_input.write(text.encode())
                recovered_console = Console(recovered_log, recovery, send_recovery)
                # First input is the read-only census, before any client joins.
                early = dimension_census(recovered_console)
                recovery_tick_lo = simulation_tick(recovered_console)
                freeze_reply = recovered_console.command('tick query')
                if 'frozen' not in freeze_reply.lower():
                    raise RuntimeError(f'recovery did not report frozen ticks: {freeze_reply[-1000:]}')
                recovery_tick_hi = simulation_tick(recovered_console)
                if recovery_tick_lo != recovery_tick_hi:
                    raise RuntimeError('recovery census was not frozen')
                save_json(run / 'after-early-census.json', early)
                wire_observed = observe_wire_ids(recovered_console, run, ('127.0.0.1', port),
                                                 cache, durable, early)
                if simulation_tick(recovered_console) != recovery_tick_hi:
                    raise RuntimeError('simulation advanced during wire identity observation')
                wire_comparison = compare_wire_ids(durable, wire_observed)
                # Keep simulation frozen across player reconnects too.
                for name in ('Proto0', 'Proto1', 'BattleRecorder'):
                    actors.append(ProtocolMatrixClient(name, ('127.0.0.1', port), run, cache,
                        recovered_console.command, account_uuid=offline_uuid(name), timeout_seconds=120))
                after = dimension_census(recovered_console)
                after.pop(uuid_snbt(str(offline_uuid('WireWitness'))), None)
                if simulation_tick(recovered_console) != recovery_tick_hi:
                    raise RuntimeError('simulation advanced during frozen player reconnects')
                save_json(run / 'after-census.json', after)
                actors[1].wait_until(lambda: {'inventory', 'cursor'} <= actors[1].snapshot().keys(),
                                     description='recovered recipient inventory and cursor', timeout_seconds=60)
                witness = actors[1].snapshot()
                inventory = witness['inventory']
                survived, inventory_valid = recipient_custody(grant_inventory, inventory, witness['cursor'])
                classification = classify_observation(acknowledged=True, published_image=bool(image),
                    checkpoint_image=image_checkpointed, observed_after=survived)
                player_ids = {uuid_snbt(str(offline_uuid(name)))
                              for name in ('Proto0', 'Proto1', 'BattleRecorder')}
                after_holders = combine_censuses(early, after, player_ids, witness_id)
                if witness_id in after:
                    after_holders[witness_id] = after[witness_id]
                elapsed_upper = (recovery_tick_hi - before_tick_lo
                                 if recovery_tick_hi >= before_tick_lo else None)
                record.update({'census_tick_bounds': {'before': [before_tick_lo, before_tick_hi],
                                                      'recovery': [recovery_tick_lo, recovery_tick_hi]},
                               'maximum_elapsed_ticks': elapsed_upper, 'freeze_reply': freeze_reply})
                lifetimes = projectile_lifetimes(wal, before_tick_lo)
                probe = baseline[probe_id]
                probe_clock = probe.get('life') if probe.get('in_ground') == '1b' else probe.get('age')
                probe_age = int(probe_clock.rstrip('bsl')) if (
                    isinstance(probe_clock, str) and re.fullmatch(r'\d+[bsl]?', probe_clock)) else None
                probe_nonexpired = (probe_id in lifetimes and probe_age is not None and elapsed_upper is not None
                                    and probe_age + elapsed_upper < lifetimes[probe_id]['lifetime'])
                probe_compared = probe_id in durable and probe_id in early and probe_nonexpired
                fresh_control = compare_durable(durable,
                    {key: value for key, value in after_holders.items() if key != probe_id})
                negative_control_failed = probe_compared and probe_id in fresh_control['missing']
                old_candidates = [(int(entity['life'][:-1]), ident) for ident, entity in baseline.items()
                    if ident != probe_id and ident in durable and ident in early and entity['type'] == 'Arrow'
                    and entity.get('in_ground') == '1b' and isinstance(entity.get('life'), str)
                    and re.fullmatch(r'\d+s', entity['life']) and int(entity['life'][:-1]) >= 60
                    and ident in lifetimes and elapsed_upper is not None
                    and int(entity['life'][:-1]) + elapsed_upper < lifetimes[ident]['lifetime']]
                old_control_id = max(old_candidates)[1] if old_candidates else None
                old_control = compare_durable(durable,
                    {key: value for key, value in after_holders.items() if key != old_control_id})
                old_control_failed = old_control_id is not None and old_control_id in old_control['missing']
                if negative_control:
                    if not old_control_failed:
                        raise RuntimeError('no observed old nonexpired arrow for negative control')
                    del after_holders[old_control_id]
                    record['negative_control_removed'] = old_control_id
                comparison = compare_durable(durable, after_holders)
                apply_wire_comparison(comparison, wire_comparison)
                # An arrow witnessed before the cut but not in the durable image
                # needs an identity-scoped removal proof; expiry is not proof.
                removals = {}
                for ident, entity in baseline.items():
                    if entity['type'] == 'Arrow' and ident not in durable:
                        proof = published_removal(wal, ident, before_tick_lo, before_tick_hi)
                        if proof:
                            removals[ident] = proof
                        else:
                            comparison['unverified'][ident] = {'reason': 'no identity-scoped removal proof'}
                record['witness_removals'] = removals
                witness_expected = dict(durable.get(witness_id, {}).get('holders', {}))
                witness_actual = dict(after.get(witness_id, {}).get('holders', {}))
                witness_expected.pop('Inventory', None)
                witness_actual.pop('Inventory', None)
                witness_valid = (witness_id in durable and witness_id in after and inventory_valid
                                 and witness_expected == witness_actual)
                record.update({'give_classification': classification, 'recipient_inventory': inventory,
                               'recipient_cursor': witness['cursor'],
                               'recipient_other_holders_equal': witness_valid,
                               'census_tick_bounds': {'before': [before_tick_lo, before_tick_hi],
                                                      'recovery': [recovery_tick_lo, recovery_tick_hi]},
                               'maximum_elapsed_ticks': elapsed_upper, 'freeze_reply': freeze_reply,
                               'entity_comparison': comparison,
                               'wire_identity_comparison': wire_comparison,
                               'old_arrow_control': {'uuid': old_control_id,
                                                     'life': max(old_candidates)[0] if old_candidates else None,
                                                     'drop_negative_control_failed': old_control_failed},
                               'projectile_probe': {'uuid': probe_id, 'compared': probe_compared,
                                                    'nonexpired': probe_nonexpired, 'age': probe_age,
                                                    'in_ground': probe.get('in_ground'),
                                                    'lifetime_evidence': lifetimes.get(probe_id),
                                                    'drop_negative_control_failed': negative_control_failed},
                               'early_census': str(run / 'after-early-census.json')})
                record['status'] = cut_verdict(comparison, valid=valid,
                    custody_ok=classification != 'mismatch' and witness_valid,
                    probe_ok=probe_compared and negative_control_failed)
                if not negative_control and record['status'] == 'failed' and (
                        classification == 'mismatch' or not witness_valid or comparison['missing']
                        or comparison['new'] or comparison['changed'] or not probe_compared
                        or not negative_control_failed):
                    record['ledger_candidate'] = {
                        'repro': {'command': [sys.executable, '-m', 'tools.harness', 'run', 'battle-crash'],
                                  'environment': {'KRASNOKAMEN_SERVER_BINARY': str(pin / 'krasnokamen'),
                                                  'KRASNOKAMEN_CLIENT_BINARY': str(client_binary),
                                                  'KRASNOKAMEN_BATTLE_CLIENT_ROOT': str(client_data),
                                                  'KRASNOKAMEN_BATTLE_WORLD': str(world)}},
                        'runner_command': argv, 'moment': moment, 'crash_copy': str(primary),
                        'before_census': str(run / 'before-census.json'),
                        'after_census': str(run / 'after-census.json'),
                        'note': 'Investigate multi-tick census differences and published image before filing as a defect.',
                    }
    except Exception as error:
        record['error'] = f'{type(error).__name__}: {error}'
    finally:
        for actor in actors:
            actor.disconnect()
        if recovery is not None and recovery.poll() is None:
            runtime.stop_process(recovery)
        if recovery_input is not None:
            recovery_input.close()
        if process is not None and process.poll() is None:
            runtime.stop_process(process)
        port.release()
        dashboard.release()
        run.mkdir(parents=True, exist_ok=True)
        save_json(run / 'scenario-result.json', record)
    return record


def campaign(rows):
    table = {}
    for moment in MOMENTS:
        selected = [row for row in rows if row['moment'] == moment]
        observed = [row for row in selected if row.get('moment_observed')]
        table[moment] = {'attempts': len(selected), 'observed': len(observed),
                         'passed': sum(row['status'] == 'passed' for row in observed),
                         'failed': sum(row['status'] == 'failed' for row in observed),
                         'incomplete': sum(row['status'] == 'incomplete' for row in observed),
                         'unclassified': len(selected) - len(observed),
                         'evidence': [row.get('crash_copy') for row in selected]}
    failed = any(row.get('ledger_candidate') or row.get('error') for row in rows) or any(
        table[moment]['failed'] for moment in MOMENTS)
    complete = all(table[moment]['passed'] >= 5 for moment in MOMENTS)
    incomplete = any(table[moment]['incomplete'] for moment in MOMENTS)
    status = 'failed' if failed else 'passed' if complete else 'incomplete' if incomplete else 'failed'
    return {'status': status, 'moments': table, 'rows': rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--world', type=Path, default=os.environ.get('KRASNOKAMEN_BATTLE_WORLD'),
                        help='stopped seed-935 natural-site fixture (or KRASNOKAMEN_BATTLE_WORLD)')
    parser.add_argument('--negative-control', action='store_true',
                        help='run one real cut and remove an observed old arrow from its recovered comparison')
    args = parser.parse_args()
    if args.world is None:
        parser.error('--world or KRASNOKAMEN_BATTLE_WORLD is required')
    pin = Path(os.environ['KRASNOKAMEN_SERVER_BINARY']).resolve(strict=True).parent
    client = Path(os.environ['KRASNOKAMEN_CLIENT_BINARY']).resolve(strict=True)
    client_data = Path(os.environ['KRASNOKAMEN_BATTLE_CLIENT_ROOT']).resolve(strict=True)
    cache = (ROOT / 'data/vanilla').resolve(strict=True)
    args.out.mkdir(parents=True, exist_ok=False)
    if args.negative_control:
        row = one(args.out / 'before-checkpoint-1', MOMENTS[0], pin, client, client_data, cache,
                  args.world.resolve(strict=True), negative_control=True)
        expected = (not row.get('error') and row.get('moment_observed') and row['status'] == 'failed'
                    and row.get('projectile_probe', {}).get('drop_negative_control_failed')
                    and row.get('old_arrow_control', {}).get('drop_negative_control_failed')
                    and row.get('negative_control_removed') in row.get('entity_comparison', {}).get('missing', []))
        save_json(args.out / 'negative-control.json', {'detected': bool(expected), 'row': row})
        if not expected:
            raise SystemExit(1)
        return
    rows = []
    for moment in MOMENTS:
        for index in range(50 if moment == 'during-append' else 5):
            if sum(row.get('moment_observed', False) for row in rows if row['moment'] == moment) >= 5:
                break
            rows.append(one(args.out / f'{moment}-{index+1}', moment, pin, client, client_data, cache,
                            args.world.resolve(strict=True)))
            save_json(args.out / 'campaign.json', campaign(rows))
            if 'error' in rows[-1]:
                raise SystemExit(1)
    passed = campaign(rows)['status'] == 'passed'
    if not passed:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
