"""World-domain helpers; plain shared Matrix API, never a second runner.

Krasnokamen source facts (deploy-final): command_context.rs:215-288 implements
block/entity/score conditions; scoreboard.rs:757-767 reports numeric scores.
command_blocks.rs:245-288 supports data get ONLY for command blocks, NOT entity
NBT or arbitrary containers. Rich observations therefore fail closed.
"""
from __future__ import annotations
import inspect
import json

import re
from typing import Any

from ..matrix_api import Adapter, Check, MatrixContext, MissingCapability

Y = 100


def command(ctx: MatrixContext, text: str, dimension: str | None = None) -> str:
    dim = dimension or ctx.factors['dimension']
    response = ctx.server.command(f'execute in minecraft:{dim} run {text}')
    if 'Position is not loaded' in response or 'Condition position is not loaded' in response:
        raise MissingCapability('fixture_chunks_loaded')
    return response


def setup(ctx: MatrixContext) -> None:
    ctx.state['world_checks'] = []
    ctx.state['query_index'] = 0
    # Do not change driver-applied mode, dimension, difficulty or permissions.
    ctx.clients[0].teleport_to(24.5, 100, 16.5)
    ctx.server.ensure_loaded(((0, 96, 0), (47, 115, 31)))
    command(ctx, 'fill 0 99 0 47 99 31 minecraft:stone')
    command(ctx, 'fill 0 100 0 47 115 31 minecraft:air')
    command(ctx, f'tp {ctx.clients[0].name} 2.5 100 2.5')
    ctx.server.command('scoreboard objectives add dw_probe dummy')


def oracle(ctx: MatrixContext) -> list[Check]:
    return [Check(**entry) for entry in ctx.state['world_checks']]


def adapter(act) -> Adapter:
    def action(ctx):
        ctx.state['world_act'] = True
        act(ctx)
    return Adapter(setup, action, oracle)


def check(ctx: MatrixContext, name: str, passed: bool, actual: Any,
          expected: Any, source: str) -> None:
    ctx.state['world_checks'].append({
        'name': name, 'passed': bool(passed), 'detail': {
            'actual': actual, 'expected': expected, 'vanilla': '26.1.2',
            'source': 'https://minecraft.wiki/w/' + source,
        },
    })


def condition(ctx: MatrixContext, predicate: str, dimension: str | None = None) -> bool:
    ctx.state['query_index'] += 1
    holder = 'dw' + str(ctx.state['query_index'])
    ctx.server.command(f'scoreboard players set {holder} dw_probe 0')
    command(ctx, f'execute if {predicate} run scoreboard players set {holder} dw_probe 1', dimension)
    response = ctx.server.query(f'scoreboard players get {holder} dw_probe')
    match = re.search(rf'\b{holder} has ([01]) \[dw_probe\]', response)
    if match is None:
        raise MissingCapability('console_scoreboard_numeric_feedback')
    return match[1] == '1'


def block(ctx, pos, expected, *, source, name=None, dimension=None):
    present = condition(ctx, f'block {pos[0]} {pos[1]} {pos[2]} minecraft:{expected}', dimension)
    check(ctx, name or f'block:{pos}:{expected}', present, present, True, source)
    return present


def exists(ctx, selector, *, source, expected=True, dimension=None):
    actual = condition(ctx, f'entity {selector}', dimension)
    check(ctx, f'entity:{selector}', actual == expected, actual, expected, source)
    return actual


def tick(ctx) -> int:
    value = ctx.server.dashboard_stats().get('simulation_tick')
    if not isinstance(value, int):
        raise MissingCapability('dashboard_simulation_tick')
    return value


def boundary(ctx):
    if ctx.state.get('world_act') and not ctx.state.get('world_boundary'):
        ctx.state['world_boundary'] = True
        ctx.boundary()


def ticks(ctx, count):
    boundary(ctx)
    target = tick(ctx) + count
    return ctx.wait_until(lambda: tick(ctx) >= target,
                          description=f'simulation_tick >= {target}',
                          timeout_seconds=max(30.0, count / 10.0))


def block_ack(ctx, sequence):
    if type(sequence) is not int:
        raise MissingCapability('block_ack_sequence')
    def received():
        snapshot = ctx.clients[0].snapshot()
        if 'block_ack_sequence' not in snapshot:
            raise MissingCapability('block_ack_sequence')
        value = snapshot['block_ack_sequence']
        return type(value) is int and value >= sequence
    ctx.wait_until(received, description=f'block action acknowledgement >= {sequence}')
    boundary(ctx)


def verb(ctx, name, *args, **kwargs):
    method = getattr(ctx.clients[0], name, None)
    if not callable(method):
        raise MissingCapability('client:' + name)
    if name == 'close_container':
        return method(*args, **kwargs)
    if name == 'break_block':
        sequence = method(*args, **kwargs)
        block_ack(ctx, sequence)
        return sequence
    arguments = dict(inspect.signature(method).bind(*args, **kwargs).arguments)
    receipts = ctx.concurrently([(ctx.clients[0], name, arguments)])
    boundary(ctx)
    return receipts


def observe(ctx, field):
    value = ctx.clients[0].snapshot()
    for part in field.split('.'):
        if not isinstance(value, dict) or part not in value or value[part] is None:
            raise MissingCapability('snapshot:' + field)
        value = value[part]
    return value


def inventory(ctx, item):
    stacks = observe(ctx, 'inventory')
    return sum(s['count'] for s in stacks if s['item_id'] == 'minecraft:' + item)


def give(ctx, item, count=1, *, clear=True):
    if clear:
        command(ctx, f'clear {ctx.clients[0].name}')
    command(ctx, f'give {ctx.clients[0].name} minecraft:{item} {count}')
    base = item.split('[', 1)[0]
    ctx.wait_until(lambda: inventory(ctx, base) >= count,
                   description='received fixture item ' + base)
    hotbar = [s for s in observe(ctx, 'inventory')
              if s['item_id'] == 'minecraft:' + base and 36 <= s['slot'] <= 44]
    if not hotbar:
        raise MissingCapability('fixture_hotbar_item:' + base)
    ctx.clients[0].select_hotbar(hotbar[0]['slot'] - 36)


def use(ctx, pos, item=None, face='up'):
    if item:
        give(ctx, item)
    ctx.clients[0].look_at(pos[0] + .5, pos[1] + .5, pos[2] + .5)
    sequence = ctx.clients[0].use_item_on(pos, face)
    block_ack(ctx, sequence)


def entity(ctx, kind):
    entities = observe(ctx, 'entities')
    found = [e for e in entities if e.get('type') == 'minecraft:' + kind]
    if not found:
        raise AssertionError('No observed entity: ' + kind)
    return found[0]


def data(ctx, target, field):
    response = command(ctx, f'data get entity {target} {field}')
    marker = 'has the following entity data: '
    if marker not in response:
        if 'command blocks only' in response or 'Unknown' in response:
            raise MissingCapability('data_get_entity')
        raise MissingCapability('data_path:' + field + ':' + response)
    text = response.split(marker, 1)[1].splitlines()[0].strip()
    # Only scalar/list paths are consumed; never invent compound NBT parsing.
    text = re.sub(r'(?<=\d)[bBsSlLfFdD](?=[,\]\s]|$)', '', text)
    text = re.sub(r'\[[BIL];', '[', text)
    try:
        return json.loads(text)
    except ValueError:
        raise MissingCapability('data_snbt_decode:' + field) from None


def data_size(ctx, target, field):
    ctx.server.command('scoreboard players set dw_size dw_probe -2147483648')
    response = command(ctx, f'execute store result score dw_size dw_probe run data get entity {target} {field}')
    if 'not implemented' in response or 'Unsupported' in response:
        raise MissingCapability('execute_store')
    if 'Nothing found at path' in response:
        raise MissingCapability('data_path:' + field)
    reply = ctx.server.query('scoreboard players get dw_size dw_probe')
    match = re.search(r'\bdw_size has (-?\d+) \[dw_probe\]', reply)
    if match is None or int(match[1]) == -2147483648:
        raise MissingCapability('data_get_entity')
    return int(match[1])


def entity_field(ctx, kind, field):
    observed_field = {'Pos': 'position', 'Health': 'health'}.get(field)
    if observed_field:
        observed = entity(ctx, kind)
        if observed_field not in observed or observed[observed_field] is None:
            raise MissingCapability('snapshot:entities.' + observed_field)
        return observed[observed_field]
    value = data(ctx, f'@e[type=minecraft:{kind},limit=1,sort=nearest]', field)
    if field in ('Sheared', 'Tame', 'AgeLocked'):
        return bool(value)
    return value


def interact(ctx, kind, item=None):
    if item:
        give(ctx, item)
    verb(ctx, 'interact_entity', entity(ctx, kind)['id'])


def plant(ctx, support, plant_block, pos=(8, 100, 8)):
    x, y, z = pos
    command(ctx, f'setblock {x} {y-1} {z} minecraft:{support}')
    command(ctx, f'setblock {x} {y} {z} minecraft:{plant_block}')


def growth(ctx, predicate, count=400):
    # Tick bound is a sampling budget, not a claim of deterministic vanilla RNG.
    command(ctx, 'gamerule random_tick_speed 1000')
    ticks(ctx, count)
    result = condition(ctx, predicate)
    command(ctx, 'gamerule random_tick_speed 3')
    return result
