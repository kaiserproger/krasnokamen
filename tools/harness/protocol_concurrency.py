"""Concurrent protocol inputs fenced by the core's opt-in consumed-input log."""
from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
import json
import struct
import zlib
import threading

from . import runtime
from .matrix_api import MissingCapability


def _records(directory):
    files = list(directory.glob('*.jsonl'))
    if not files:
        return None
    path = max(files, key=lambda item: tuple(int(part) for part in item.stem.split('-')))
    raw = path.read_bytes()
    complete = raw.rfind(b'\n') + 1
    records = []
    offset = 0
    for line in raw[:complete].splitlines(keepends=True):
        offset += len(line)
        row = json.loads(line)
        if 'Gap' in row:
            raise MissingCapability('input_log:gap')
        if 'Tick' in row:
            records.append((offset, row['Tick']))
    return path, complete, records


def match_click_and_break(records, *, connection, window, state_id, slot, position):
    """Match one actor's exact click and dig-start inputs in the consumed log.

    A vanilla client breaks an instantly mined block with the start input alone.
    """
    expected_pos = dict(zip(('x', 'y', 'z'), position))
    clicks, starts = [], []
    for tick in records:
        for index, logged in enumerate(tick.get('inputs', [])):
            event = logged.get('Input', {})
            click = event.get('ContainerClick')
            if click is not None and all(click.get(key) == value for key, value in {
                'connection': connection, 'window': window, 'state_id': state_id,
                'slot': slot, 'button': 0, 'mode': 'Pickup',
            }.items()):
                clicks.append((tick['tick'], index, click))
            dig = event.get('Dig')
            if (dig is not None and dig.get('connection') == connection
                    and dig.get('position') == expected_pos and dig.get('action') == 'Start'):
                starts.append((tick['tick'], index, dig))
    if len(clicks) > 1 or len(starts) > 1:
        raise MissingCapability('input_log:ambiguous_destroy_inputs')
    if not clicks or not starts:
        raise MissingCapability('input_log:missing_destroy_inputs')
    click, start = clicks[0], starts[0]
    if not click[:2] < start[:2]:
        raise MissingCapability('input_log:destroy_input_order')
    return {'click': click, 'start': start}


def verify_destroy_tick_fence(inputs, removal_tick, container_tick):
    """Fail closed unless both committed records and the consumed inputs share one tick.

    The removal record in the start input's tick proves the break was not deferred.
    """
    ticks = [inputs['click'][0], inputs['start'][0], removal_tick, container_tick]
    if any(type(tick) is not int for tick in ticks):
        raise MissingCapability('destroy:same_tick_unprovable')
    if len(set(ticks)) != 1:
        raise MissingCapability('destroy:different_ticks')
    return ticks[0]


def submit_click_and_break(ctx, client, slot, position):
    """Submit one client's ordered batch and return its consumed-input receipts."""
    directory = ctx.server.world_dir / 'input-log'
    marker = 'DestroyFence' + str(ctx.state.get('concurrent_fence_serial', 0) + 1)
    ctx.state['concurrent_fence_serial'] = ctx.state.get('concurrent_fence_serial', 0) + 1
    ctx.server.command('say ' + marker)

    def ready():
        log = _records(directory)
        if log is None:
            return False, None
        connections = [join['connection'] for _, tick in log[2]
                       for item in tick.get('inputs', [])
                       if (join := item.get('Input', {}).get('Join')) and join.get('name') == client.name]
        fenced = any(marker in json.dumps(tick) for _, tick in log[2])
        return bool(connections) and fenced, (log, connections[-1])

    (path, baseline, _), connection = runtime.wait_for_path_condition(directory / '.input', 30, ready)
    result = client.submit_input_batch([
        {'kind': 'click_slot', 'slot': slot},
        {'kind': 'break_block', **dict(zip(('x', 'y', 'z'), position)), 'face': 'up'},
    ])
    # The client reports the container and state ids its click packet carried.
    click = (result.get('sent') or [None])[0]
    if (not isinstance(click, dict) or click.get('kind') != 'click_slot'
            or type(click.get('container_id')) is not int or type(click.get('state_id')) is not int):
        raise MissingCapability('client_batch:click_receipt')

    def consumed():
        log = _records(directory)
        if log is None or log[0] != path:
            raise MissingCapability('input_log:changed_during_destroy_round')
        ticks = [tick for end, tick in log[2] if end > baseline]
        try:
            matched = match_click_and_break(ticks, connection=connection, window=click['container_id'],
                                            state_id=click['state_id'], slot=slot, position=position)
        except MissingCapability as error:
            if str(error) == 'input_log:missing_destroy_inputs':
                return False, None
            raise
        return True, matched

    inputs = runtime.wait_for_path_condition(directory / '.input', 30, consumed)
    return {'inputs': inputs, 'client_batch': result}


def committed_destroy_records(ctx, tick, actor, position):
    """Read the two attributed records from the published, CRC-checked tick WAL."""
    directory = ctx.server.world_dir / 'journal'

    def collect(entries):
        for entry in entries:
            if 'Dimension' in entry:
                yield from collect(entry['Dimension']['entries'])
            elif 'BlockLog' in entry:
                yield from entry['BlockLog']['value'].get('records', [])

    def ready():
        found = []
        covered = False
        for path in directory.glob('batch-*-*.bin'):
            raw = path.read_bytes()
            if len(raw) < 32:
                raise RuntimeError(f'truncated published WAL: {path}')
            magic, version, low, high, size, crc = struct.unpack_from('<IIQQII', raw)
            if magic != 0x534F4C4A or version != 1 or len(raw) != size + 32 or zlib.crc32(raw[32:]) != crc:
                raise RuntimeError(f'invalid published WAL: {path}')
            if low <= tick <= high:
                covered = True
                entries = [json.loads(line) for line in raw[32:].splitlines()]
                found.extend(record for record in collect(entries)
                             if record.get('actor') == actor and record.get('pos') == list(position))
        if not covered:
            return False, None
        blocks = [r for r in found if 'Block' in r.get('change', {})]
        containers = [r for r in found if 'Container' in r.get('change', {})]
        if len(blocks) != 1 or len(containers) != 1:
            raise MissingCapability('block_log:missing_or_ambiguous_destroy_records')
        return True, {'block': blocks[0], 'container': containers[0]}

    return runtime.wait_for_path_condition(directory / '.fence', 30, ready)


def concurrent_clicks(ctx, actions, *, same_tick=False):
    """Send one click per actor; match each to its consumed tick, not wall time."""
    if not actions or any(verb != 'click_slot' for _, verb, _ in actions):
        raise MissingCapability('concurrent:click_slot_only')
    if len({client.name for client, _, _ in actions}) != len(actions):
        raise ValueError('concurrent click actors must be distinct')
    directory = ctx.server.world_dir / 'input-log'
    serial = ctx.state.get('concurrent_fence_serial', 0) + 1
    ctx.state['concurrent_fence_serial'] = serial
    marker = f'RandomConcurrentFence{serial:08d}'
    ctx.server.command('say ' + marker)
    names = {client.name for client, _, _ in actions}

    def ready():
        log = _records(directory)
        if log is None:
            return False, None
        connections = {}
        for _, tick in log[2]:
            for logged in tick.get('inputs', []):
                join = logged.get('Input', {}).get('Join')
                if join and join['name'] in names:
                    connections[join['name']] = join['connection']
        fenced = any(marker in json.dumps(tick) for _, tick in log[2])
        return names == set(connections) and fenced, (log, connections)

    (path, baseline, _), connections = runtime.wait_for_path_condition(directory / '.input', 30, ready)
    expected = []
    for client, verb, params in actions:
        if set(params) - {'slot', 'kind', 'button'} or params.get('kind', 'pickup') != 'quick_move':
            raise MissingCapability('concurrent:quick_move_only')
        menu = client.snapshot()['menu']
        expected.append({'connection': connections[client.name], 'window': menu['id'],
                         'state_id': menu['state_id'], 'slot': params['slot'],
                         'button': params.get('button', 0), 'mode': 'QuickMove'})
    gate = threading.Barrier(len(actions))
    def submit(action):
        client, verb, params = action
        gate.wait(timeout=30)
        return getattr(client, verb)(**params)
    with ThreadPoolExecutor(max_workers=len(actions)) as executor:
        futures = [executor.submit(submit, action) for action in actions]
        for future in futures:
            future.result()

    def consumed():
        log = _records(directory)
        if log is None or log[0] != path:
            raise MissingCapability('input_log:changed_during_concurrent_round')
        matches = [[] for _ in actions]
        for end, tick in log[2]:
            if end <= baseline:
                continue
            for logged in tick.get('inputs', []):
                click = logged.get('Input', {}).get('ContainerClick')
                if click is None:
                    continue
                for index, wanted in enumerate(expected):
                    if all(click.get(key) == value for key, value in wanted.items()):
                        matches[index].append((tick['tick'], click))
        if any(len(items) > 1 for items in matches):
            raise MissingCapability('input_log:ambiguous_click')
        if not all(matches):
            return False, None
        receipts = [{'client': client.name, 'verb': verb, 'tick': matches[index][0][0],
                     'input': matches[index][0][1]} for index, (client, verb, _) in enumerate(actions)]
        return True, receipts

    return runtime.wait_for_path_condition(directory / '.input', 30, consumed)
