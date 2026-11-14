"""Shared ordinary Python operations for the seven block-domain adapters.

No scenario interpreter. Optional observations/verbs fail closed. In particular
Krasnokamen /data only supports command blocks (command_blocks.rs:245-284), so these
adapters never pretend /data entity or arbitrary block NBT is implemented.
"""
import re
import inspect
from ..matrix_api import Adapter, Check, MissingCapability

POS = (1,144,0)


def actor(ctx):
    if not ctx.clients:
        raise MissingCapability('client:connected')
    return ctx.clients[0]


def call(client, verb, *args, **kwargs):
    method = getattr(client, verb, None)
    if not callable(method):
        raise MissingCapability('client:' + verb)
    signature = inspect.signature(method)
    if not any(p.kind == inspect.Parameter.VAR_KEYWORD for p in signature.parameters.values()):
        for key in kwargs:
            if key not in signature.parameters:
                raise MissingCapability('client:' + verb + ':' + key)
    return method(*args, **kwargs)


def observed(ctx, key, client=None):
    value = (client or actor(ctx)).snapshot().get(key)
    if value is None:
        raise MissingCapability('snapshot:' + key)
    return value


def command(ctx, text):
    # Console is not the player; explicitly route fixture/query coordinates.
    dimension = ctx.factors['dimension']
    return ctx.server.command(f'execute in minecraft:{dimension} run {text}')


def query(ctx, text):
    dimension = ctx.factors['dimension']
    return ctx.server.query(f'execute in minecraft:{dimension} run {text}')


def require_modes(ctx, *modes):
    mode = ctx.factors['game_mode']
    if mode not in modes:
        raise MissingCapability('context_oracle:' + ctx.row['subject'] + ':' + mode)


def check(ctx, name, passed, detail=None):
    ctx.state['block_checks'].append({'name': name, 'passed': bool(passed), 'detail': detail})


def block_is(ctx, pos, state):
    ctx.state['query_serial'] = ctx.state.get('query_serial', 0) + 1
    marker = 'DomBlocksPredicate' + str(ctx.state['query_serial'])
    xyz = ' '.join(map(str, pos))
    reply = query(ctx, f'execute if block {xyz} {state} run say {marker}')
    return marker in reply


def block(ctx, pos, state, name=None):
    passed = block_is(ctx, pos, state)
    check(ctx, name or 'block:' + str(pos) + ':' + state, passed,
          {'position': pos, 'expected': state})
    if passed:
        for client in ctx.clients[1:]:
            wait_block(client, pos, state, 'observer received action result:' + state)
            check(ctx, 'observer:' + client.name + ':' + str(pos) + ':' + state, True,
                  {'position': pos, 'expected': state, 'observation': 'received block state'})


def wait_block(client, pos, state, description):
    name, _, properties = state.partition('[')
    name = name if ':' in name else 'minecraft:' + name
    expected = dict(pair.split('=', 1) for pair in properties.rstrip(']').split(',') if pair)
    def received():
        try:
            value = call(client, 'read_block_state', pos)
        except MissingCapability as error:
            if str(error).startswith('snapshot:block_state_not_received:'):
                return False
            raise
        return value['block_id'] == name and all(
            value.get('properties', {}).get(key) == wanted for key, wanted in expected.items())
    call(client, 'wait_until', received, description=description)


def setblock(ctx, state, pos=POS):
    command(ctx, 'setblock ' + ' '.join(map(str, pos)) + ' ' + state)
    if not block_is(ctx, pos, state):
        raise RuntimeError('fixture setblock not observed: ' + state)
    wait_block(actor(ctx), pos, state, 'received fixture block before action:' + state)


def count(ctx, item, client=None):
    item = item if ':' in item else 'minecraft:' + item
    return sum(s['count'] for s in observed(ctx, 'inventory', client) if s['item_id'] == item)


def equip(ctx, item=None, amount=1, client=None):
    client = client or actor(ctx)
    command(ctx, f'clear {client.name}')
    ctx.wait_until(lambda: observed(ctx, 'inventory', client) == [],
                   description='empty inventory fixture received before grant')
    if item is None:
        client.select_hotbar(0)
        return
    command(ctx, f'give {client.name} {item} {amount}')
    base = item.split('[', 1)[0]
    ctx.wait_until(lambda: count(ctx, base, client) == amount,
                   description='finite item fixture received:' + item)
    name = base if ':' in base else 'minecraft:' + base
    slots = [s['slot'] for s in observed(ctx, 'inventory', client)
             if s['item_id'] == name and 36 <= s['slot'] <= 44]
    if not slots:
        raise MissingCapability('fixture_hotbar_item:' + name)
    client.select_hotbar(slots[0] - 36)


def wait_action_ack(ctx, token):
    if not isinstance(token, int) or isinstance(token, bool):
        raise MissingCapability('client:submitted_block_action_sequence')
    def acknowledged():
        snapshot = actor(ctx).snapshot()
        if 'block_ack_sequence' not in snapshot:
            raise MissingCapability('snapshot:block_ack_sequence')
        value = snapshot['block_ack_sequence']
        return value is not None and value >= token
    ctx.wait_until(acknowledged,
                   description='received BlockChangedAck for submitted action ' + str(token))


def use(ctx, pos=POS, face='up', cursor=None):
    client = actor(ctx)
    client.look_at(*(v + .5 for v in pos))
    if cursor is None:
        token = client.use_item_on(pos, face=face)
    else:
        token = call(client, 'use_item_on', pos, face=face, cursor=cursor)
    wait_action_ack(ctx, token)
    boundary(ctx)


def mine(ctx, pos=POS, **kwargs):
    client = actor(ctx)
    client.look_at(*(v + .5 for v in pos))
    token = call(client, 'break_block', pos, **kwargs)
    wait_action_ack(ctx, token)
    boundary(ctx)
    return token


def tick(ctx):
    stats = ctx.server.dashboard_stats()
    value = stats.get('simulation_tick')
    if not isinstance(value, int):
        raise MissingCapability('dashboard:simulation_tick')
    return value


def ticks(ctx, amount):
    target = tick(ctx) + amount
    ctx.wait_until(lambda: tick(ctx) >= target,
                   description=f'dashboard simulation_tick >= {target}',
                   timeout_seconds=max(30.0, amount / 10.0))


def entities(ctx, kind):
    name = kind if ':' in kind else 'minecraft:' + kind
    # Only received fields are available; metadata omissions remain explicit.
    return [e for e in observed(ctx, 'entities') if e['type'] == name]


def drops(ctx, item):
    name = item if ':' in item else 'minecraft:' + item
    result = 0
    for entity in entities(ctx, 'item'):
        stack = entity.get('item')
        if stack is None:
            raise MissingCapability('client_observation:entities.item')
        if stack['item_id'] == name:
            result += stack['count']
    return result


def field(obj, key, capability):
    if key not in obj or obj[key] is None:
        raise MissingCapability(capability + ':' + key)
    return obj[key]


def experience(ctx):
    return field(observed(ctx, 'xp'), 'total', 'snapshot:xp')


def orb_experience(ctx):
    return sum(field(entity, 'xp_value', 'snapshot:entities')
               for entity in entities(ctx, 'experience_orb'))


def entity_health(ctx, kind):
    matches = entities(ctx, kind)
    if len(matches) != 1:
        raise RuntimeError('health fixture requires exactly one ' + kind)
    return field(matches[0], 'health', 'snapshot:entities')


def effects(ctx):
    registry = field(observed(ctx, 'registries'), 'minecraft:mob_effect', 'snapshot:registries')
    result = {}
    for effect_id, effect in observed(ctx, 'effects').items():
        if effect_id not in registry:
            raise MissingCapability('registry:mob_effect:' + effect_id)
        result[registry[effect_id]] = effect
    return result


def block_entity(ctx, pos=POS):
    for entry in observed(ctx, 'block_entities'):
        if tuple(entry['position']) == tuple(pos):
            return field(entry, 'nbt', 'client_observation:block_entities')
    raise MissingCapability('client_observation:block_entity:' + str(pos))


def fixture(ctx):
    ctx.state['block_checks'] = []
    ctx.state['query_serial'] = 0
    subject = ctx.row['subject']
    observer_subject = subject.startswith('block_place.') or subject in {
        'block_interact.door_trapdoor_gate_toggle', 'block_interact.lever_button',
        'block_interact.note_block_tune', 'block_interact.repeater_comparator_daylight',
        'block_interact.axe_strip_scrape_unwax', 'block_interact.shovel_path',
        'block_interact.hoe_till',
    }
    composite = ctx.row.get('origin', '').startswith('triple:dupe:') or ctx.row.get('origin') == 'triple:multiplayer_x_same_block'
    if len(ctx.clients)>1 and subject!='block_interact.ender_chest_personal':
        if len(ctx.clients)!=2 or composite or not observer_subject:
            raise MissingCapability('scenario_concurrent_actor_schedule:' + subject)
    for client in ctx.clients:
        call(client, 'teleport_to', .5, 160, 2.5)
    call(ctx.server, 'ensure_loaded', ((-8,143,-8),(8,159,8)))
    command(ctx, 'fill -8 143 -8 8 143 8 stone')
    command(ctx, 'fill -8 144 -8 8 159 8 air')
    command(ctx, 'kill @e[type=!minecraft:player,x=-8,y=143,z=-8,dx=16,dy=20,dz=16]')
    for client in ctx.clients:
        call(client, 'teleport_to', .5, 144, 2.5)
        call(client, 'close_container')
    if not block_is(ctx, (0,143,0), 'stone'):
        raise RuntimeError('fixture floor not observed')


def oracle(ctx):
    checks = ctx.state['block_checks']
    if not checks:
        raise RuntimeError('block scenario produced no observations')
    return [Check(**entry) for entry in checks]


def boundary(ctx):
    if not ctx.state.get('block_boundary'):
        ctx.state['block_boundary'] = True
        ctx.boundary()



def data(ctx, target, path=''):
    kind = target.split()[0]
    reply = query(ctx, f'data get {target}' + (' ' + path if path else ''))
    marker = ' has the following ' + kind + ' data: '
    if marker not in reply:
        if any(text in reply for text in ('not implemented', 'Unknown or incomplete',
                                          'command blocks only', 'currently supports')):
            raise MissingCapability('data_get_' + kind)
        raise RuntimeError('data query failed: ' + reply)
    return next(line.split(marker, 1)[1].strip() for line in reply.splitlines() if marker in line)


def scalar(ctx, target, path, scale=1):
    reply = query(ctx, f'data get {target} {path} {scale}')
    matched = re.search(r' after scale factor of [-\d.]+ is (-?\d+)[ \t]*$', reply, re.MULTILINE)
    if not matched:
        if any(text in reply for text in ('not implemented', 'Unknown or incomplete',
                                          'command blocks only', 'currently supports')):
            raise MissingCapability('data_get_' + target.split()[0])
        raise RuntimeError('numeric data query failed: ' + reply)
    return int(matched.group(1))

def adapter(run):
    return Adapter(fixture, run, oracle)
