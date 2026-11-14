"""Platform adapter helpers; observations are never inferred from command admission."""
from __future__ import annotations

import math
import re
from collections import Counter
from typing import Any

from tools.harness.matrix_api import Check, MatrixContext, MissingCapability


def client(ctx: MatrixContext, index: int = 0):
    if len(ctx.clients) <= index:
        raise MissingCapability(f'multi_client_{index + 1}')
    return ctx.clients[index]


def observe(ctx: MatrixContext, key: str, index: int = 0) -> Any:
    value = client(ctx, index).snapshot()
    for part in key.split('.'):
        if not isinstance(value, dict) or part not in value or value[part] is None:
            raise MissingCapability('snapshot:' + key)
        value = value[part]
    return value


def method(owner: Any, name: str, prefix: str = 'server'):
    value = getattr(owner, name, None)
    if not callable(value):
        raise MissingCapability(prefix + ':' + name)
    return value


def fixture(ctx: MatrixContext, name: str, **values: Any):
    # Driver-owned fixture setup, before connections where configuration requires it.
    # A driver must explicitly implement this; no silent mock/config fallback.
    fn = getattr(ctx.server, 'fixture', None)
    if not callable(fn):
        raise MissingCapability('fixture:' + name)
    return fn(name, **values)


def admin(ctx: MatrixContext, text: str) -> str:
    reply = ctx.server.command(text)
    if re.search(r'Unknown or incomplete command|Unknown command|not implemented', reply, re.I):
        raise MissingCapability('server_command:' + text.split()[0])
    return reply


def setup(ctx: MatrixContext) -> None:
    if ctx.row.get('origin') == 'triple:dupe:two_players_take_same_tick':
        raise MissingCapability('same_tick_admission_fence')
    if any('clients repeat the action concurrently' in step for step in ctx.row.get('steps', [])):
        raise MissingCapability('concurrent_subject_actors')
    ctx.state['checks'] = []
    ctx.state['baseline'] = client(ctx).snapshot()
    ctx.state['platform_boundary'] = False


def acknowledged(ctx: MatrixContext) -> None:
    if not ctx.state['platform_boundary']:
        ctx.state['platform_boundary'] = True
        ctx.boundary()


def evidence(value: Any) -> Any:
    if isinstance(value, (bytes, bytearray)):
        return {'bytes_hex': value.hex()}
    if isinstance(value, dict):
        return {key: evidence(item) for key, item in value.items()}
    if isinstance(value, (tuple, list)):
        return [evidence(item) for item in value]
    return value


def check(ctx: MatrixContext, name: str, actual: Any, expected: Any) -> None:
    test(ctx, name, actual == expected, {'actual': actual, 'expected': expected})


def test(ctx: MatrixContext, name: str, passed: bool, detail: Any) -> None:
    ctx.state['checks'].append({'name': name, 'passed': bool(passed), 'detail': evidence(detail)})
    acknowledged(ctx)


def oracle(ctx: MatrixContext) -> list[Check]:
    checks = ctx.state['checks']
    if not checks:
        raise MissingCapability('oracle_observation:' + ctx.row['subject'])
    return [Check(**record) for record in checks]


def response(ctx: MatrixContext, text: str, pattern: str, index: int = 0) -> str:
    if ctx.factors.get('permission') == 'console':
        line = ctx.server.command(text)
        test(ctx, text.split()[0] + ' response', bool(re.search(pattern, line)), line)
        return line
    before = Counter(observe(ctx, 'events', index))
    client(ctx, index).command(text)
    def received():
        fresh = Counter(observe(ctx, 'events', index)) - before
        return next((line for line in fresh if isinstance(line, str)
                     and re.search(pattern, line)), None)
    line = ctx.wait_until(received, description=f'{text.split()[0]} response matching {pattern}')
    test(ctx, text.split()[0] + ' response', bool(re.search(pattern, line)), line)
    return line


def query(ctx: MatrixContext, text: str, pattern: str) -> str:
    try:
        reply = ctx.server.query(text)
    except MissingCapability:
        for prefix, capability in (('data get entity', 'data_get_entity'),
                                   ('data get block', 'data_get_block'),
                                   ('execute store', 'execute_store')):
            if text.startswith(prefix):
                raise MissingCapability(capability) from None
        raise
    if re.search(r'Unknown or incomplete command|Unknown command', reply, re.I):
        capability = {'data get entity': 'data_get_entity', 'data get block': 'data_get_block',
                      'execute store': 'execute_store'}
        raise MissingCapability(next((v for k, v in capability.items() if text.startswith(k)),
                                     'server_query:' + text.split()[0]))
    test(ctx, text, bool(re.search(pattern, reply)), reply)
    return reply


def position(ctx: MatrixContext, index: int = 0) -> list[float]:
    return list(observe(ctx, 'position', index))


def at(ctx: MatrixContext, expected, index: int = 0) -> None:
    actual = ctx.wait_until(lambda: (p if math.dist(p, expected) <= .25 else None)
                            if (p := position(ctx, index)) else None,
                            description=f'player {index} position {expected}')
    test(ctx, 'position', math.dist(actual, expected) <= .25,
         {'actual': actual, 'expected': expected, 'tolerance': .25})


def ticks(ctx: MatrixContext, count: int) -> None:
    def tick():
        stats = ctx.server.dashboard_stats()
        if 'simulation_tick' not in stats:
            raise MissingCapability('dashboard_tick_counter')
        return int(stats['simulation_tick'])
    start = tick()
    ctx.wait_until(lambda: tick() >= start + count,
                   description=f'dashboard advances {count} ticks',
                   timeout_seconds=max(30., count / 10.))


def arena(ctx: MatrixContext) -> tuple[int, int, int]:
    x, y, z = (math.floor(v) for v in position(ctx))
    if abs(x) <= 8 and abs(z) <= 8 and 60 <= y <= 100:
        x += 64
        z += 64
        client(ctx).teleport_to(x, y, z)
        method(client(ctx), 'close_container', 'client')()
    method(ctx.server, 'ensure_loaded')(((x-8, y-1, z-8), (x+8, y+6, z+8)))
    admin(ctx, f'fill {x-8} {y-1} {z-8} {x+8} {y-1} {z+8} minecraft:stone')
    admin(ctx, f'fill {x-8} {y} {z-8} {x+8} {y+6} {z+8} minecraft:air')
    ctx.state['origin'] = (x, y, z)
    return x, y, z


def block(ctx: MatrixContext, pos, index: int = 0) -> dict:
    state = method(client(ctx, index), 'read_block_state', 'client')(tuple(pos))
    return {'name': state['block_id'], 'properties': state['properties']}


def wait_block(ctx: MatrixContext, pos, name: str, index: int = 0) -> dict:
    def received():
        try:
            state = block(ctx, pos, index=index)
        except MissingCapability as error:
            if str(error).startswith('snapshot:block_state_not_received:'):
                return None
            raise
        return state if state['name'] == name else None
    return ctx.wait_until(received, description=f'client {index} received {name} at {pos}')


def block_is(ctx: MatrixContext, pos, name: str, **properties) -> None:
    state = name
    if properties:
        state += '[' + ','.join(f'{k}={v}' for k, v in properties.items()) + ']'
    serial = ctx.state.get('block_query_serial', 0) + 1
    ctx.state['block_query_serial'] = serial
    marker = f'MatrixPlatformBlock{serial:04d}'
    text = f'execute if block {pos[0]} {pos[1]} {pos[2]} {state} run say {marker}'
    observed = ctx.wait_until(lambda: (reply if marker in reply else None)
                             if (reply := ctx.server.query(text)) else None,
                             description=f'authoritative block {pos} == {state}')
    test(ctx, f'block {pos}', marker in observed, observed)


def restart(ctx: MatrixContext) -> None:
    ctx.server.save_all()
    for actor in ctx.clients:
        actor.disconnect()
    method(ctx.server, 'clean_restart')()
    for actor in ctx.clients:
        actor.reconnect()


def inventory(ctx: MatrixContext, item: str, index: int = 0) -> int:
    return sum(int(s['count']) for s in observe(ctx, 'inventory', index)
               if s['item_id'] == item)


def use(ctx: MatrixContext, pos, face: str = 'up', index: int = 0):
    client(ctx, index).look_at(*(v + .5 for v in pos))
    return client(ctx, index).use_item_on(tuple(pos), face)


def block_ack(ctx: MatrixContext, token, index: int = 0) -> None:
    if type(token) is not int:
        raise MissingCapability('block_ack_sequence')
    def consumed():
        snapshot = client(ctx, index).snapshot()
        if 'block_ack_sequence' not in snapshot:
            raise MissingCapability('block_ack_sequence')
        sequence = snapshot['block_ack_sequence']
        return sequence is not None and sequence >= token
    ctx.wait_until(consumed, description=f'actor {index} consumed block input {token}')


def rich_action(ctx: MatrixContext, name: str, *args, index: int = 0, **kwargs):
    actor = client(ctx, index)
    if name == 'select_inventory_item':
        item = args[0]
        entry = ctx.wait_until(
            lambda: next((s for s in observe(ctx, 'inventory', index)
                          if s['item_id'] == item and s['count'] > 0), None),
            description='received fixture item ' + item)
        source = entry['slot']
        if 36 <= source <= 44:
            actor.select_hotbar(source - 36)
        else:
            actor.click_slot(source, kind='swap', button=0)
            actor.select_hotbar(0)
        return None
    if name == 'read_blocks':
        low, high = args
        return [block(ctx, (x, y, z))
                for x in range(low[0], high[0]+1)
                for y in range(low[1], high[1]+1)
                for z in range(low[2], high[2]+1)]
    if name == 'item_entities_in_radius':
        center, radius = args
        return [entity for entity in observe(ctx, 'entities', index)
                if entity['type'] == 'minecraft:item'
                and math.dist(entity['position'], center) <= radius]
    if name in ('find_entity', 'entity_count', 'entity_exists', 'entity_state', 'attack_until_dead'):
        def entities():
            return observe(ctx, 'entities', index)
        if name == 'find_entity':
            return ctx.wait_until(lambda: next((e['id'] for e in entities() if e['type'] == args[0]), None),
                                  description='spawned entity ' + args[0])
        if name == 'entity_count':
            return sum(e['type'] == args[0] for e in entities())
        entity = next((e for e in entities() if e['id'] == args[0]), None)
        if name == 'entity_exists':
            return entity is not None
        if name == 'entity_state':
            if entity is None:
                raise AssertionError('entity despawned: ' + str(args[0]))
            return entity
        for _ in range(100):
            entity = next((e for e in entities() if e['id'] == args[0]), None)
            if entity is None or (entity.get('health') is not None and entity['health'] <= 0):
                return None
            actor.attack(args[0])
            ticks(ctx, 20)
        raise AssertionError('entity survived 100 acknowledged attacks')
    return method(actor, name, 'client')(*args, **kwargs)
