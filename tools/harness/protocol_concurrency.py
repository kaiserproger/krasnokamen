"""Concurrent protocol inputs fenced by the core's opt-in consumed-input log."""
from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
import json
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
