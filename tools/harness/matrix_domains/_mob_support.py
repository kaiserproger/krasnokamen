"""Received-state and authoritative-query helpers for mob-domain adapters.

No lifecycle or runner here: MatrixContext owns both. Query capability failures
remain blocked; a syntactically accepted fixture is never a success oracle.
"""
from __future__ import annotations

import math
import re
from ..matrix_api import MissingCapability


def require_verb(client, name):
    method = getattr(client, name, None)
    if not callable(method):
        raise MissingCapability('client:' + name)
    return method


def field(client, name):
    value = client.snapshot().get(name)
    if value is None:
        raise MissingCapability('snapshot:' + name)
    return value


def tick(ctx):
    value = ctx.server.dashboard_stats().get('simulation_tick')
    if type(value) is not int:
        raise MissingCapability('dashboard:simulation_tick')
    return value


def ticks(ctx, count, sample=None):
    start = tick(ctx)
    def reached():
        if sample is not None:
            sample()
        current = tick(ctx)
        if current < start:
            raise RuntimeError('simulation tick moved backwards during observation')
        return current - start >= count
    ctx.wait_until(reached, description=f'authoritative simulation advances {count} ticks',
                   timeout_seconds=max(30, count / 10 + 30))


def exists(ctx, selector):
    # A false condition produces no vanilla feedback, so both outcomes emit an
    # explicit, mutually exclusive marker. Missing query support cannot be false.
    yes, no = 'DomMobsPresent', 'DomMobsAbsent'
    a = ctx.server.query(f'execute if entity {selector} run say {yes}')
    if yes in a:
        return True
    b = ctx.server.query(f'execute unless entity {selector} run say {no}')
    if no in b:
        return False
    raise MissingCapability('execute_if_entity')


def number(ctx, target, path, *, block=False, optional=False):
    kind = 'block' if block else 'entity'
    text = ctx.server.query(f'data get {kind} {target} {path} 1000')
    if 'Supported: data' in text or 'command blocks only' in text or 'Unknown or incomplete' in text:
        raise MissingCapability('data_get_' + kind)
    match = re.search(r'after scale factor of 1000\.00 is (-?\d+)\s*$', text, re.MULTILINE)
    if match is None:
        if 'No elements' in text or 'Nothing found' in text or 'Found no elements' in text:
            if optional:
                return None
            raise AssertionError('Missing required authoritative data path: '+path)
        raise RuntimeError(f'Numeric data query did not return its value: {text!r}')
    return int(match.group(1)) / 1000


def string(ctx, target, path, *, block=False):
    kind='block' if block else 'entity'
    text = ctx.server.query(f'data get {kind} {target} {path}')
    if 'Supported: data' in text or 'command blocks only' in text or 'Unknown or incomplete' in text:
        raise MissingCapability('data_get_'+kind)
    match = re.search(r'has the following (?:entity|block) data: (.*)$', text, re.MULTILINE)
    if match is None:
        if 'No elements' in text or 'Nothing found' in text or 'Found no elements' in text:
            return None
        raise RuntimeError('Data string query did not return its value: '+text)
    # Registry IDs never contain escaped quotation marks.
    return match.group(1).strip().strip('\"\'')


def entity(ctx, uuid):
    for row in field(ctx.clients[0], 'entities'):
        if row.get('uuid') == uuid:
            for name in ('id', 'type', 'position'):
                if row.get(name) is None:
                    raise MissingCapability('snapshot:entities.' + name)
            return row
    return None


def inventory(client):
    result = {}
    for stack in field(client, 'inventory'):
        result[stack['item_id']] = result.get(stack['item_id'], 0) + stack['count']
    return result


def select_item(ctx, item):
    slots = [s['slot'] - 36 for s in field(ctx.clients[0], 'inventory')
             if s['item_id'] == item and 36 <= s['slot'] <= 44]
    if not slots:
        raise MissingCapability('fixture_hotbar_item:' + item)
    ctx.clients[0].select_hotbar(slots[0])


def arena(ctx, *, floor='stone'):
    # Keep full-set shop auto-open fixtures (spawn Y60..100) out of the test.
    ctx.server.ensure_loaded(((-16, 199, -16), (16, 215, 16)))
    ctx.server.command(f'fill -16 199 -16 16 199 16 {floor}')
    ctx.server.command('fill -16 200 -16 16 215 16 air')
    for client in ctx.clients:
        ctx.server.command(f'tp {client.name} 0.5 200 0.5')
    ctx.wait_until(lambda: all(c.snapshot().get('position') is not None and
                   math.dist(c.snapshot()['position'][:3], [0.5, 200, 0.5]) < 1
                   for c in ctx.clients), description='received arena teleport')
    for client in ctx.clients:
        client.close_container()


def count(ctx, entity_type):
    # execute store is deliberately unnecessary: each selected entity adds one.
    ctx.server.command('scoreboard objectives add matrix_mobs dummy')
    ctx.server.command('scoreboard players set #count matrix_mobs 0')
    ctx.server.command(f'execute as @e[type=minecraft:{entity_type}] run scoreboard players add #count matrix_mobs 1')
    reply = ctx.server.query('scoreboard players get #count matrix_mobs')
    match = re.search(r'#count has (-?\d+) \[matrix_mobs\]', reply)
    if match is None:
        raise MissingCapability('scoreboard_count_query')
    return int(match.group(1))
