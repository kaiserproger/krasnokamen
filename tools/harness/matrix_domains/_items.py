"""Small observation helpers for DomItems' plain-Python matrix adapters.

No runner or declarative interpreter: matrix_api owns execution and receipts.
Inventory slots use the vanilla window-0 wire indices, not core storage indices.
"""
from __future__ import annotations

import json
import math
import re

from ..matrix_api import Adapter, Check, MissingCapability

WIKI = 'https://minecraft.wiki/w/'


def client(ctx):
    return ctx.clients[0]


def verb(ctx, name, *args, **kwargs):
    method = getattr(client(ctx), name, None)
    if not callable(method):
        raise MissingCapability('client:' + name)
    result = method(*args, **kwargs)
    if name in ('use_item_on', 'break_block'):
        # Admission ACK also covers refused/no-op actions. It is not proof of
        # gameplay success: callers must still inspect their world/item oracle.
        if type(result) is not int:
            raise MissingCapability('block_ack_sequence')
        def acknowledged():
            snapshot = client(ctx).snapshot()
            if 'block_ack_sequence' not in snapshot:
                raise MissingCapability('block_ack_sequence')
            value = snapshot['block_ack_sequence']
            return type(value) is int and value >= result
        ctx.wait_until(acknowledged, description=f'block admission sequence {result}')
    else:
        acknowledge(ctx)
    return result


def acknowledge(ctx):
    # Two exact, self-directed command replies fence the input tick and its
    # complete outbound batch. This is synchronization, NEVER a success oracle;
    # every caller still checks the resulting authoritative inventory/world.
    # A packet write or arbitrary snapshot revision would not fence a no-op.
    # /msg is owned by the deployed essentials plugin; /tell is the core alias.
    for _ in range(2):
        # Core tick/input/social.rs adds 20 to a 200-point spam budget;
        # tick.rs drains one per simulation tick. Do not disable that rule.
        ticks(ctx, 20)
        sequence = ctx.state.get('items_ack_sequence', 0) + 1
        ctx.state['items_ack_sequence'] = sequence
        marker = f'DomItemsInputFence{sequence:08d}'
        client(ctx).command(f'tell {client(ctx).name} {marker}')
        ctx.wait_until(lambda: any(marker in str(event) for event in client(ctx).snapshot().get('events', [])),
                       description='received ordered client input fence ' + marker)


def require_verbs(ctx, *names):
    for name in names:
        if not callable(getattr(client(ctx), name, None)):
            raise MissingCapability('client:' + name)


def observe(ctx, field, observer=0):
    snapshot = ctx.clients[observer].snapshot()
    value = snapshot.get(field)
    if value is None:
        raise MissingCapability('snapshot:' + field)
    if field == 'effects' and isinstance(value, dict):
        names = snapshot.get('registries', {}).get('minecraft:mob_effect', {})
        resolved = []
        for effect_id, entry in value.items():
            name = names.get(str(effect_id))
            if name is None:
                raise MissingCapability('snapshot:registries:minecraft:mob_effect:' + str(effect_id))
            resolved.append(dict(entry, id=name))
        return resolved
    return value


def command(ctx, text):
    # Console's default dimension is not necessarily the actor's dimension.
    dim = ctx.factors['dimension']
    return ctx.server.command(f'execute in minecraft:{dim} run {text}')


def query(ctx, text):
    dim = ctx.factors['dimension']
    reply = ctx.server.query(f'execute in minecraft:{dim} run {text}')
    if any(s in reply.lower() for s in ('not implemented', 'not supported', 'supports command blocks only', 'supported: data get|merge block')):
        capability = ('data_get_entity' if text.startswith('data get entity') else
                      'data_get_block' if text.startswith('data get block') else
                      'execute_store' if 'store ' in text else 'server_query:' + text.split()[0])
        raise MissingCapability(capability)
    return reply


def number(ctx, target, path):
    if target == client(ctx).name:
        snapshot = client(ctx).snapshot()
        xp_key = {'XpTotal': 'total', 'XpLevel': 'level', 'XpP': 'progress'}.get(path)
        if xp_key is not None and xp_key in snapshot.get('xp', {}):
            return float(snapshot['xp'][xp_key])
        if path == 'foodLevel' and snapshot.get('food') is not None:
            return float(snapshot['food'])
    reply = query(ctx, f'data get entity {target} {path}')
    # Vanilla feedback ends in one SNBT scalar; never extract a number from an
    # error, player name, selector or arbitrary compound.
    scalar = reply.rsplit(': ', 1)[-1].strip()
    if not re.fullmatch(r'[-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?[bBsSlLfFdD]?', scalar):
        raise MissingCapability('data_get_entity:' + path + ':scalar_feedback')
    return float(scalar.rstrip('bBsSlLfFdD'))


def health(ctx):
    value = client(ctx).snapshot().get('health')
    return float(value) if value is not None else number(ctx, client(ctx).name, 'Health')


def ticks(ctx, count):
    stats = ctx.server.dashboard_stats()
    # The shared driver must expose the real completed simulation tick counter.
    if 'simulation_tick' not in stats:
        raise MissingCapability('dashboard:simulation_tick')
    start = int(stats['simulation_tick'])
    ctx.wait_until(lambda: int(ctx.server.dashboard_stats()['simulation_tick']) >= start + count,
                   description=f'completed {count} simulation ticks from {start}',
                   timeout_seconds=max(30.0, count / 10.0 + 10.0))


def evidence(value):
    if isinstance(value, dict):
        return {(key if isinstance(key, str) else json.dumps(key, sort_keys=True)): evidence(item)
                for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [evidence(item) for item in value]
    return value


def check(ctx, name, actual, expected, *, source, tolerance=0):
    passed = (math.isclose(actual, expected, rel_tol=0, abs_tol=tolerance)
              if isinstance(actual, (int, float)) and isinstance(expected, (int, float))
              else actual == expected)
    ctx.state.setdefault('checks', []).append({
        'name': name, 'passed': passed, 'detail': {
            'actual': evidence(actual), 'expected': evidence(expected),
            'source': source, 'version': '26.1.2'}})
    boundary(ctx)


def condition(ctx, name, passed, detail, *, source):
    ctx.state.setdefault('checks', []).append({
        'name': name, 'passed': bool(passed), 'detail': {
            'observation': evidence(detail), 'source': source, 'version': '26.1.2'}})
    boundary(ctx)


def boundary(ctx):
    if not ctx.state.get('items_boundary'):
        ctx.state['items_boundary'] = True
        ctx.boundary()


def oracle(ctx):
    checks = ctx.state.get('checks')
    if not checks:
        raise RuntimeError('No behavioral oracle was exercised')
    return [Check(**value) for value in checks]


def setup(ctx):
    ctx.state['checks'] = []
    require_verbs(ctx, 'close_container')
    client(ctx).teleport_to(0.5, 180, 0.5)
    ensure_loaded = getattr(ctx.server, 'ensure_loaded', None)
    if not callable(ensure_loaded):
        raise MissingCapability('server:ensure_loaded')
    ensure_loaded(((-8, 179, -8), (16, 210, 16)))
    command(ctx, 'fill -8 179 -8 16 179 16 stone')
    command(ctx, 'fill -8 180 -8 16 188 16 air')
    # A deployed market may open a menu on entering its zone. Setup starts from
    # the real player inventory, never from a plugin-owned shop window.
    verb(ctx, 'close_container')
    command(ctx, f'clear {client(ctx).name}')
    command(ctx, f'effect clear {client(ctx).name}')
    ctx.wait_until(lambda: count(ctx) == 0, description='empty inventory fixture received')


def survival(ctx):
    if ctx.factors['game_mode'] not in ('survival', 'adventure'):
        raise MissingCapability('scenario_variant:' + ctx.row['subject'] + ':' + ctx.factors['game_mode'])


def stacks(ctx):
    return observe(ctx, 'inventory')


def count(ctx, item=None):
    if item and ':' not in item:
        item = 'minecraft:' + item
    return sum(s['count'] for s in stacks(ctx) if item is None or s['item_id'] == item)


def cursor(ctx):
    snap = client(ctx).snapshot()
    if 'cursor' not in snap:
        raise MissingCapability('snapshot:cursor')
    return snap['cursor'] or {'item_id': 'minecraft:air', 'count': 0}


def menu(ctx):
    return observe(ctx, 'menu')


def menu_slot(ctx, slot):
    values = menu(ctx)['slots']
    return next((s for s in values if s and s.get('slot') == slot), None) or {'count': 0}


def inventory_slot(ctx, item):
    item = item if ':' in item else 'minecraft:' + item
    return next(s['slot'] for s in stacks(ctx) if s['item_id'] == item)


def give(ctx, item, amount=1):
    base = item.split('[', 1)[0]
    before = count(ctx, base)
    command(ctx, f'give {client(ctx).name} {item} {amount}')
    ctx.wait_until(lambda: count(ctx, base) == before + amount,
                   description=f'received {amount} {base}')


def select(ctx, item):
    slot = inventory_slot(ctx, item)
    if not 36 <= slot <= 44:
        raise MissingCapability('fixture_hotbar_item:' + item)
    verb(ctx, 'select_hotbar', slot - 36)


def open_menu(ctx, block='chest'):
    command(ctx, f'setblock 2 180 0 {block}')
    client(ctx).look_at(2.5, 180.5, 0.5)
    verb(ctx, 'open_container', (2, 180, 0))
    ctx.wait_until(lambda: client(ctx).snapshot().get('menu'), description=block + ' menu received')


def player_menu_slot(ctx, inventory_index):
    # All workstation/container windows end with 27 main + 9 hotbar slots.
    size = menu(ctx).get('slot_count')
    if not isinstance(size, int):
        raise MissingCapability('snapshot:menu:slot_count')
    if size < 36:
        raise MissingCapability('snapshot:menu:complete_slots')
    if 9 <= inventory_index <= 35:
        return size - 36 + inventory_index - 9
    if 36 <= inventory_index <= 44:
        return size - 9 + inventory_index - 36
    raise MissingCapability('container_slot_mapping:' + str(inventory_index))


def put(ctx, item, destination, amount=None):
    src = player_menu_slot(ctx, inventory_slot(ctx, item))
    verb(ctx, 'click_slot', src)
    if amount is None:
        verb(ctx, 'click_slot', destination)
    else:
        for _ in range(amount):
            verb(ctx, 'click_slot', destination, button=1)
        verb(ctx, 'click_slot', src)


def entity_exists(ctx, selector):
    marker = 'DomItemsEntityOracle'
    return marker in query(ctx, f'execute if entity {selector} run say {marker}')


def block_is(ctx, pos, block):
    marker = 'DomItemsBlockOracle'
    xyz = ' '.join(map(str, pos))
    return marker in query(ctx, f'execute if block {xyz} {block} run say {marker}')


def entities(ctx, kind):
    return [e for e in observe(ctx, 'entities') if e['type'] == 'minecraft:' + kind]


def summon(ctx, kind, *, nbt='', pos='4.5 180 0.5'):
    command(ctx, f'summon {kind} {pos} {nbt}')
    ctx.wait_until(lambda: entities(ctx, kind), description=kind + ' spawn received')
    return entities(ctx, kind)[-1]['id']


def consume(ctx, item, duration=32):
    require_verbs(ctx, 'use_item', 'release_use_item')
    select(ctx, item)
    verb(ctx, 'use_item', hand='main')
    ticks(ctx, duration + 2)
    verb(ctx, 'release_use_item')


def adapter(action, prepare=setup):
    return Adapter(prepare, action, oracle)
