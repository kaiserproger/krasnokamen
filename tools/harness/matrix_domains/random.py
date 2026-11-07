"""Seeded multiplayer operations over the existing MatrixClient contract.

Fixtures are finite, declared sources. Every operation is followed by independent
live server/client custody checks; packet submission alone never awards a pass.
"""
from __future__ import annotations

from collections import Counter
import json
import copy
import re
import struct
import zlib

from .. import runtime
from ..matrix_api import Adapter, Check, MissingCapability
from ..backends.village_autonomy_qa import _container_totals, _snbt_fields, _snbt_parts
from .session import data

CHEST = (34, 80, 32)
WEIGHT = {'minecraft:dirt': ('earth', 1), 'minecraft:oak_log': ('wood', 4),
          'minecraft:oak_planks': ('wood', 1)}


def stock(stacks):
    result = Counter()
    for stack in stacks:
        resource, weight = WEIGHT.get(stack['item_id'], (stack['item_id'], 1))
        result[resource] += weight * stack['count']
    return result


def invariant_checks(observed):
    """Pure oracle boundary, also used for harness-level negative controls."""
    checks = [Check('no_server_error', observed['running'] and not observed['errors'], observed['errors']),
              Check('conservation', observed['resources'] == observed['sources'], observed),
              Check('legal_action', not observed['refusals'] and observed['progress'], observed['refusals']),
              Check('observer_authority', observed['observer'] == observed['authority'],
                    {'observer': observed['observer'], 'authority': observed['authority']})]
    if observed.get('recovery') is not None:
        checks.append(Check('recovery_atomic_durable', observed['recovery_ok'], observed['recovery']))
    return checks


def mutate_observation(observed, fault):
    """Explicit harness-only negative control; never modifies server state."""
    observed = copy.deepcopy(observed)
    if fault == 'no_server_error':
        observed['errors'].append('ERROR planted observation')
    elif fault == 'conservation':
        observed['resources']['earth'] = observed['resources'].get('earth', 0) + 1
    elif fault == 'legal_action':
        observed['progress'] = False
    elif fault == 'observer_authority':
        observed['observer']['container']['minecraft:dirt'] = -1
    elif fault == 'recovery_atomic_durable':
        recovery = observed.get('recovery')
        if recovery is not None:
            recovered = recovery['recovered_exact_image']
            recovered['inventories'][recovery['actor_index']]['planted_slot'] = {'id': '"minecraft:dirt"'}
            observed['recovery_ok'] = (
                recovery['cursor_recovery_ok']
                and recovery['expected_exact_image'] == recovered)
            recovery['exact_post_image'] = observed['recovery_ok']
    elif fault is not None:
        raise ValueError('unknown observation fault: ' + fault)
    return observed


def _count(client, item):
    return sum(s['count'] for s in client.snapshot()['inventory'] if s['item_id'] == item)


def _supply(ctx, actor, item, count):
    missing = max(0, count - _count(actor, item))
    if missing:
        before = _count(actor, item)
        ctx.server.command(f'give {actor.name} {item} {missing}')
        actor.wait_until(lambda: _count(actor, item) == before + missing,
                         description='acknowledged finite source')
        resource, weight = WEIGHT[item]
        ctx.state['sources'][resource] += missing * weight
        ctx.state['source_events'].append({'actor': actor.name, 'item': item, 'count': missing})


def _inventory(ctx, actor):
    text = data(ctx, 'entity ' + actor.name, 'Inventory')
    return _container_totals('{Items:' + text + '}')


def _chest(ctx):
    return _container_totals(data(ctx, 'block ' + ' '.join(map(str, CHEST))))


def _exact_slots(items):
    items = items.strip()
    if not (items.startswith('[') and items.endswith(']')):
        raise ValueError('expected item list')
    slots = {}
    for item in _snbt_parts(items[1:-1]):
        fields = _snbt_fields(item)
        slot = fields.get('Slot')
        if slot is None or slot in slots:
            raise ValueError('missing or duplicate item slot')
        slots[slot] = fields
    return slots


def _exact_image(ctx, blocks):
    inventories = [_exact_slots(data(ctx, 'entity ' + client.name, 'Inventory'))
                   for client in ctx.clients]
    chest = _snbt_fields(data(ctx, 'block ' + ' '.join(map(str, CHEST))))
    return {'inventories': inventories,
            'container': _exact_slots(chest.get('Items', '[]')),
            'blocks': copy.deepcopy(blocks)}


def _items(values):
    return [{'item_id': name, 'count': count} for name, count in sorted(values.items())]


def _close(actor):
    actor.close_container()
    actor.wait_until(lambda: actor.snapshot().get('menu', {}).get('id') == 0,
                     description='closed menu custody')


def _fence(ctx):
    # The query crosses the console owner after the preceding received ACK.
    ctx.server.command('time query gametime')
    return ctx.server.dashboard_stats()['simulation_tick']


def _published(ctx, through):
    directory = ctx.server.world_dir / 'journal'
    def ready():
        for path in directory.glob('batch-*-*.bin'):
            raw = path.read_bytes()
            if len(raw) < 32:
                raise AssertionError('truncated published WAL header')
            magic, version, low, high, size, crc = struct.unpack_from('<IIQQII', raw)
            if magic != 0x534F4C4A or version != 1 or len(raw) != size + 32 or zlib.crc32(raw[32:]) != crc:
                raise AssertionError('invalid published WAL batch')
            if high >= through:
                return True, {'path': str(path), 'low': low, 'high': high}
        return False, None
    return runtime.wait_for_path_condition(directory / '.fence', 30, ready)


def observe(ctx):
    clients = ctx.clients
    observer = clients[-1]
    authoritative = {'inventories': [_inventory(ctx, client) for client in clients],
                     'container': _chest(ctx), 'blocks': {}}
    received = {'inventories': [], 'container': {}, 'blocks': {}}
    for client in clients:
        snapshot = client.snapshot()
        if 'inventory' not in snapshot:
            raise MissingCapability('random:received_inventory')
        totals = Counter()
        for stack in snapshot['inventory']:
            # Crafting result is a preview, not physical custody.
            if stack['slot'] != 0:
                totals[stack['item_id']] += stack['count']
        received['inventories'].append(dict(totals))
    observer.teleport_to(33.5, 80, 32.5)
    observer.open_container(CHEST)
    snapshot = observer.snapshot()
    if snapshot.get('cursor'):
        raise AssertionError('observer acquired item custody')
    chest = Counter()
    for stack in snapshot['menu']['slots']:
        if stack['slot'] < 27:
            chest[stack['item_id']] += stack['count']
    received['container'] = dict(chest)
    _close(observer)
    for position in ctx.state['touched']:
        key = ','.join(map(str, position))
        state = observer.read_block_state(position)
        block = state['block_id']
        received['blocks'][key] = block
        marker = 'RandomBlock' + key.replace(',', '_')
        feedback = ctx.server.query(f'execute if block {" ".join(map(str, position))} {block} run say {marker}')
        authoritative['blocks'][key] = block if marker in feedback else 'MISMATCH'
    resources = Counter()
    for inventory in authoritative['inventories']:
        resources.update(stock(_items(inventory)))
    resources.update(stock(_items(authoritative['container'])))
    for block in authoritative['blocks'].values():
        if block in WEIGHT:
            resources.update(stock([{'item_id': block, 'count': 1}]))
    entities = observer.snapshot()
    if 'entities' not in entities:
        raise MissingCapability('random:received_item_entities:' +
                                str(entities.get('observation_errors', {}).get('entities', 'unknown')))
    for entity in entities['entities']:
        if 'item' in entity:
            resources.update(stock([entity['item']]))
    errors = re.findall(r'^.*(?:\bERROR\b|panicked at|panic:).*$',
                        ctx.server.log_path.read_text(errors='replace'), re.M)
    refusals = [str(client.observer.error) for client in clients if client.observer.error]
    return {'running': ctx.server.running, 'errors': errors, 'resources': dict(resources),
            'sources': {key: value for key, value in ctx.state['sources'].items() if value},
            'refusals': refusals, 'progress': True, 'observer': received,
            'authority': authoritative, 'recovery_ok': ctx.state.get('recovery_ok', True),
            'recovery': ctx.state.get('recovery')}


def setup(ctx):
    if len(ctx.clients) < 2:
        raise ValueError('random scenarios require an independent observer')
    ctx.state.update(sources={'earth': 0, 'wood': 0}, source_events=[], sink_events=[], checks=[], steps=[], touched=[])
    for index, client in enumerate(ctx.clients):
        client.teleport_to(30.5 + index, 80, 30.5)
    ctx.server.ensure_loaded_callback = ctx.clients[-1].ensure_loaded
    ctx.server.ensure_loaded(((28, 79, 28), (40, 85, 40)))
    for command in ('gamerule random_tick_speed 0', 'gamerule spawn_mobs false',
                    'fill 28 79 28 40 79 40 stone', 'fill 28 80 28 40 85 40 air',
                    'setblock 34 80 32 chest'):
        ctx.server.command(command)


def _click(actor, slot, **kwargs):
    old = actor.snapshot()['menu']['state_id']
    actor.click_slot(slot, **kwargs)
    actor.wait_until(lambda: actor.snapshot()['menu']['state_id'] != old,
                     description='acknowledged inventory transition')


def _slot(actor, item):
    return next(stack['slot'] for stack in actor.snapshot()['menu']['slots']
                if stack['item_id'] == item and stack['slot'] >= 9)


def recovery_matches(expected, recovered, actor_index, cursor):
    """Exact item identities/holders, not craft-equivalent weighted resources."""
    if expected['blocks'] != recovered['blocks'] or expected['container'] != recovered['container']:
        return False
    if len(expected['inventories']) != len(recovered['inventories']):
        return False
    for index, (before, after) in enumerate(zip(expected['inventories'], recovered['inventories'])):
        wanted = Counter(before)
        if index == actor_index and cursor:
            wanted[cursor['item_id']] += cursor['count']
        if dict(wanted) != after:
            return False
    return True


def _recover(ctx, actor):
    # A real finite transfer guarantees this is never an empty recovery probe.
    _supply(ctx, actor, 'minecraft:dirt', 8)
    actor.open_container(CHEST)
    _click(actor, _slot(actor, 'minecraft:dirt'), kind='quick_move')
    _close(actor)
    before = observe(ctx)
    actor.open_container(CHEST)
    source = next(s for s in actor.snapshot()['menu']['slots'] if s['slot'] < 27)
    _click(actor, source['slot'])
    cursor = actor.snapshot().get('cursor')
    if cursor is None or cursor['item_id'] != source['item_id'] or cursor['count'] != source['count']:
        raise AssertionError('received transfer cursor differs from chest source')
    # Query exact post-images WITHOUT closing the actor's live cursor/menu.
    expected = copy.deepcopy(before['authority'])
    expected['inventories'] = [_inventory(ctx, client) for client in ctx.clients]
    expected['container'] = _chest(ctx)
    last_ack = _fence(ctx)
    published = _published(ctx, last_ack)
    killed = ctx.server.kill9_restart()
    for client in ctx.clients:
        client.disconnect()
        client.reconnect()
    after = observe(ctx)
    cursor_ok = recovery_matches(expected, after['authority'], ctx.clients.index(actor), cursor)
    # Also crash a fully settled transfer: this permits comparing exact slots and
    # components, without guessing where disconnect returns a carried stack.
    expected_exact = _exact_image(ctx, after['authority']['blocks'])
    exact_tick = _fence(ctx)
    exact_published = _published(ctx, exact_tick)
    exact_kill = ctx.server.kill9_restart()
    for client in ctx.clients:
        client.disconnect()
        client.reconnect()
    settled = observe(ctx)
    recovered_exact = _exact_image(ctx, settled['authority']['blocks'])
    exact = cursor_ok and expected_exact == recovered_exact
    ctx.state['recovery_ok'] = exact
    ctx.state['recovery'] = {'published_after_ack': published, 'last_ack': last_ack,
                             'expected_committed_image': expected, 'cursor': cursor,
                             'actor_index': ctx.clients.index(actor),
                             'recovered_image': after['authority'], 'cursor_recovery_ok': cursor_ok,
                             'expected_exact_image': expected_exact,
                             'recovered_exact_image': recovered_exact, 'exact_post_image': exact,
                             'exact_tick': exact_tick, 'exact_published': exact_published,
                             'kill': killed, 'exact_kill': exact_kill}


def _mine(actor, position):
    actor.look_at(position[0] + .5, position[1] + .5, position[2] + .5)
    sequence = actor.break_block(position, phase='start')
    if type(sequence) is not int:
        raise MissingCapability('block_ack_sequence')
    def acknowledged():
        snapshot = actor.snapshot()
        if 'block_ack_sequence' not in snapshot:
            raise MissingCapability('snapshot:block_ack_sequence')
        value = snapshot['block_ack_sequence']
        return type(value) is int and value >= sequence
    actor.wait_until(acknowledged,
                     description='mining start admitted')
    # SetTime is periodic. Its cached value can predate StartDestroyBlock by
    # almost one interval, so first cross a fresh packet before measuring hold.
    actor.wait_ticks(1)
    actor.wait_ticks(20)  # dirt by hand: 0.5 hardness, 15 survival ticks
    actor.break_block(position, phase='stop')


def _concurrent_chest(ctx):
    actors = ctx.clients[:2]
    if len(ctx.clients) < 3:
        raise MissingCapability('concurrent:two_actors_and_observer')
    actor = actors[0]
    _supply(ctx, actor, 'minecraft:dirt', 8)
    actor.teleport_to(33.5, 80, 32.5)
    actor.open_container(CHEST)
    _click(actor, _slot(actor, 'minecraft:dirt'), kind='quick_move')
    _close(actor)
    for player in actors:
        player.teleport_to(33.5, 80, 32.5)
        player.open_container(CHEST)
    slots = [stack['slot'] for stack in actors[0].snapshot()['menu']['slots']
             if stack['slot'] < 27 and stack['item_id'] == 'minecraft:dirt']
    if not slots:
        raise AssertionError('concurrent chest fixture did not receive dirt')
    slot = slots[0]
    actors[1].wait_until(lambda: any(stack['slot'] == slot and stack['item_id'] == 'minecraft:dirt'
                                    for stack in actors[1].snapshot()['menu']['slots']),
                         description='second actor received conflicting chest stack')
    ctx.concurrently([(player, 'click_slot', {'slot': slot, 'kind': 'quick_move'})
                      for player in actors])
    for player in actors:
        _close(player)


def _action(ctx, step):
    actor = ctx.clients[step['actor']]
    verb, count = step['verb'], step['count']
    if verb == 'concurrent':
        _concurrent_chest(ctx)
        return
    position = (32 + step['target'] % 4, 80, 34 + step['target'] // 4 % 3)
    actor.teleport_to(position[0] + .5, 80, position[2] - 1.5)
    if verb in {'place', 'break'}:
        if list(position) not in ctx.state['touched']:
            ctx.state['touched'].append(list(position))
        old = actor.read_block_state(position)['block_id']
        if old == 'minecraft:dirt':
            # Retain an existing source rather than overwriting its custody.
            _mine(actor, position)
            actor.wait_until(lambda: actor.read_block_state(position)['block_id'] == 'minecraft:air',
                             description='received prior block removal')
            actor.teleport_to(position[0] + .5, 80, position[2] + .5)
            actor.wait_ticks(2)
            old = 'minecraft:air'
        if verb == 'place':
            _supply(ctx, actor, 'minecraft:dirt', 1)
            actor.open_inventory()
            source = _slot(actor, 'minecraft:dirt')
            if source != 36:
                _click(actor, source, kind='swap', button=0)
            actor.select_hotbar(0)
            actor.use_item_on((position[0], 79, position[2]))
            actor.wait_until(lambda: actor.read_block_state(position)['block_id'] != old,
                             description='legal place made progress')
        else:
            ctx.server.command(f'setblock {position[0]} 80 {position[2]} dirt')
            ctx.state['sources']['earth'] += 1
            ctx.state['source_events'].append({'block': list(position), 'item': 'minecraft:dirt', 'count': 1})
            actor.wait_until(lambda: actor.read_block_state(position)['block_id'] == 'minecraft:dirt',
                             description='received declared block source')
            _mine(actor, position)
            actor.wait_until(lambda: actor.read_block_state(position)['block_id'] != 'minecraft:dirt',
                             description='legal break made progress')
            actor.teleport_to(position[0] + .5, 80, position[2] + .5)
            actor.wait_ticks(2)
    elif verb in {'click', 'drag', 'container'}:
        _supply(ctx, actor, 'minecraft:dirt', max(8, count))
        if verb == 'container':
            actor.teleport_to(33.5, 80, 32.5)
            actor.open_container(CHEST)
            _click(actor, _slot(actor, 'minecraft:dirt'), kind='quick_move')
        else:
            actor.open_inventory()
            _click(actor, _slot(actor, 'minecraft:dirt'))
            if verb == 'drag':
                # Quick-craft start/add/end are one acknowledged transaction.
                actor.click_slot(-999, kind='quick_craft', button=0)
                actor.click_slot(20, kind='quick_craft', button=1)
                actor.click_slot(21, kind='quick_craft', button=1)
                _click(actor, -999, kind='quick_craft', button=2)
            else:
                _click(actor, 20, button=1)
            if actor.snapshot().get('cursor'):
                _click(actor, 22)
        _close(actor)
    elif verb == 'craft':
        _supply(ctx, actor, 'minecraft:oak_log', 1)
        actor.open_inventory()
        _click(actor, _slot(actor, 'minecraft:oak_log'))
        _click(actor, 1, button=1)
        if actor.snapshot().get('cursor'):
            _click(actor, 23)
        actor.wait_until(lambda: any(s['slot'] == 0 for s in actor.snapshot()['menu']['slots']),
                         description='received crafting result')
        _click(actor, 0, kind='quick_move')
        _close(actor)
    elif verb == 'mount':
        ctx.server.command('summon oak_boat 32 80 32')
        actor.teleport_to(31.5, 80, 32.5)
        actor.wait_until(lambda: any(e['type'] == 'minecraft:oak_boat'
                                    for e in actor.snapshot().get('entities', [])),
                         description='received mount fixture')
        entity = next(e for e in actor.snapshot()['entities'] if e['type'] == 'minecraft:oak_boat')
        actor.interact_entity(entity['id'])
        actor.wait_until(lambda: actor.snapshot().get('entity_id') in
                         actor.snapshot()['passengers'].get(entity['id'], []),
                         description='legal mount acknowledged')
        actor.sneak(True)
        actor.wait_ticks(2)
        actor.sneak(False)
    elif verb == 'rejoin':
        actor.disconnect()
        actor.reconnect()
    elif verb == 'kill9':
        _recover(ctx, actor)
    else:
        raise MissingCapability('random:verb:' + verb)
    _close(actor)


def act(ctx):
    for index, step in enumerate(ctx.row['actions']):
        before_tick = _fence(ctx)
        _action(ctx, step)
        observation = observe(ctx)
        observation = mutate_observation(observation, ctx.row.get('fault'))
        for check in invariant_checks(observation):
            ctx.state['checks'].append(check.__dict__)
            if not check.passed:
                raise AssertionError(f'random:{check.name}:step={index}: {check.detail}')
        ctx.state['steps'].append({'index': index, 'action': step,
                                    'tick_before': before_tick, 'tick_after': _fence(ctx)})
        ctx.state['final'] = observation['authority']
        (ctx.artifact_dir / 'random-progress.json').write_text(json.dumps(ctx.state, indent=2) + '\n')


def oracle(ctx):
    return [Check(**check) for check in ctx.state['checks']]


ADAPTERS = {'random.seeded': Adapter(setup, act, oracle)}
