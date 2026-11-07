"""Selector membership, ordering and rejection probes (Java 26.1.2).
Reference: https://minecraft.wiki/w/Target_selectors . Random order is tested
for cardinality/membership, never for a prescribed random outcome.
"""
from functools import partial

from ..matrix_api import Adapter, Check, MissingCapability
from .commands import actor, arena, check_score, field, fixture, issue, permission_level


_SELECTORS = {
    'at_s': '@s', 'at_p': '@p', 'at_a': '@a', 'at_r': '@r', 'at_e': '@e',
    'player_name': '{actor}', 'opt_xyz': '@e[type=pig,x=4,y=80,z=0,distance=..1]',
    'opt_dxdydz': '@e[type=pig,x=0,y=80,z=0,dx=4,dy=1,dz=1]',
    'opt_type': '@e[type=pig]', 'opt_name': '@e[name=MatrixNear]',
    'opt_distance': '@e[type=pig,x=0,y=80,z=0,distance=2..5]',
    'opt_limit': '@e[type=pig,sort=nearest,limit=2]',
    'sort_nearest': '@e[type=pig,sort=nearest,limit=1]',
    'sort_furthest': '@e[type=pig,sort=furthest,limit=1]',
    'sort_random': '@e[type=pig,sort=random,limit=1]',
    'sort_arbitrary': '@e[type=pig,sort=arbitrary,limit=2]',
}


def setup(ctx, subject):
    arena(ctx)
    fixture(ctx, 'scoreboard objectives add matrix_sel dummy')
    for name, x in [('MatrixNear', 1), ('MatrixMid', 4), ('MatrixFar', 7)]:
        fixture(ctx, f'summon pig {x} 80 0 {{NoAI:1b,CustomName:"{name}"}}')
    fixture(ctx, 'summon cow 2 80 0 {NoAI:1b,CustomName:"MatrixCow"}')
    for index, client in enumerate(ctx.clients):
        fixture(ctx, f'tp {client.name} {index * 3} 80 2')
    names = [c.name for c in ctx.clients] + ['MatrixNear', 'MatrixMid', 'MatrixFar', 'MatrixCow']
    ctx.state['names'] = names
    for name in names:
        fixture(ctx, f'scoreboard players set {name} matrix_sel 0')
    fixture(ctx, 'scoreboard players set MatrixCount matrix_sel 0')
    ctx.state['event_start'] = len(field(actor(ctx), 'events'))
    ctx.state['recipient_starts'] = [len(field(c, 'events')) for c in ctx.clients]


def act(ctx, subject):
    if subject == 'invalid_selector':
        ctx.state['rejections'] = []
        for selector in ('@x', '@e[foo=1]', '@e[limit=1,limit=2]', '@e[limit=0]', '@e[distance=-1]'):
            issue(ctx, f'execute as {selector} run scoreboard players add MatrixCount matrix_sel 1')
            ctx.state['rejections'].append(ctx.state['command_reply'])
        return
    if subject == 'nonop_selector_msg':
        issue(ctx, 'msg @a MatrixForbiddenSelector')
        return
    selector = _SELECTORS[subject].format(actor=actor(ctx).name)
    # Explicitly anchor at the actor even for a console matrix row: console @s
    # otherwise has no entity and is not a meaningful executor test.
    command = f'execute as {actor(ctx).name} at @s run tp {selector} 0 80 6'
    issue(ctx, command)


def oracle(ctx, subject):
    names = ctx.state['names']
    if subject == 'invalid_selector':
        expected = ('unknown selector', 'unknown option', "isn't applicable",
                    'must not be less than 1', 'distance cannot be negative')
        replies = ctx.state['rejections']
        checks = [check_score(ctx, 'MatrixCount', 'matrix_sel', 0)]
        checks.extend(Check('invalid_selector_error:' + str(index),
                            token in reply.lower(), reply)
                      for index, (token, reply) in enumerate(zip(expected, replies)))
        checks.append(Check('all_invalid_cases_observed', len(replies) == len(expected), replies))
        return checks
    if subject == 'nonop_selector_msg':
        if permission_level(ctx) >= 2:
            raise MissingCapability('nonop_client:selector_permission_probe')
        reply = ctx.state['command_reply']
        if ctx.factors.get('plugin_profile') == 'live':
            # Essentials resolves an exact online-player name, not selectors.
            passed = 'Online player not found or ambiguous.' in reply
        else:
            passed = 'Selector not allowed' in reply or 'not allowed to use' in reply
        checks = [Check('nonop_selector_rejected', passed, reply)]
        for client, start in zip(ctx.clients, ctx.state['recipient_starts']):
            events = field(client, 'events')[start:]
            checks.append(Check('no_selector_delivery:' + client.name,
                                not any('MatrixForbiddenSelector' in str(e) for e in events), events))
        return checks
    # Sample the one actual selection via moved positions. Custom names are
    # not entity scoreboard identities (vanilla uses UUIDs for nonplayers).
    for name in names:
        fixture(ctx, f'execute if entity @e[name={name},x=0,y=80,z=6,distance=..0.25] run scoreboard players set {name} matrix_sel 1')
    players = {c.name for c in ctx.clients}
    expected = {
        'at_s': {actor(ctx).name}, 'at_p': {actor(ctx).name}, 'at_a': players,
        'at_e': set(names), 'player_name': {actor(ctx).name},
        'opt_xyz': {'MatrixMid'}, 'opt_dxdydz': {'MatrixNear', 'MatrixMid'},
        'opt_type': {'MatrixNear', 'MatrixMid', 'MatrixFar'}, 'opt_name': {'MatrixNear'},
        'opt_distance': {'MatrixMid'}, 'opt_limit': {'MatrixNear', 'MatrixMid'},
        'sort_nearest': {'MatrixNear'}, 'sort_furthest': {'MatrixFar'},
    }
    if subject in expected:
        checks = [check_score(ctx, name, 'matrix_sel', int(name in expected[subject])) for name in names]
        if subject in {'opt_distance', 'opt_limit'}:
            selector = '@e[distance=-1]' if subject == 'opt_distance' else '@e[limit=0]'
            issue(ctx, f'execute as {selector} run scoreboard players add MatrixCount matrix_sel 1')
            token = 'distance cannot be negative' if subject == 'opt_distance' else 'must not be less than 1'
            checks.append(Check('invalid_bound_rejected', token in ctx.state['command_reply'].lower(),
                                ctx.state['command_reply']))
            checks.append(check_score(ctx, 'MatrixCount', 'matrix_sel', 0))
        if subject == 'opt_type':
            checks.extend(extra_membership(ctx, '@e[type=!pig]', players | {'MatrixCow'}))
        if subject == 'opt_name':
            checks.extend(extra_membership(ctx, '@e[name=!MatrixNear]', set(names) - {'MatrixNear'}))
        if subject == 'opt_dxdydz':
            checks.extend(extra_membership(ctx, '@e[type=pig,x=4,y=80,z=0,dx=-4,dy=1,dz=1]',
                                           {'MatrixNear', 'MatrixMid'}))
            # Adult pig width .9 intersects x=0 despite its origin being -.2.
            checks.extend(extra_membership(ctx, '@e[type=pig,x=0,y=80,z=0,dx=0,dy=1,dz=0]',
                                           {'MatrixNear'}, near_x=-.2))
        return checks
    # Count selected known candidates without depending on unspecified ordering.
    fixture(ctx, 'scoreboard players set MatrixSelected matrix_sel 0')
    candidates = players if subject == 'at_r' else {'MatrixNear', 'MatrixMid', 'MatrixFar'}
    for name in candidates:
        fixture(ctx, f'execute if score {name} matrix_sel matches 1 run scoreboard players add MatrixSelected matrix_sel 1')
    count = 2 if subject == 'sort_arbitrary' else 1
    return [check_score(ctx, 'MatrixSelected', 'matrix_sel', count)] + [
        check_score(ctx, name, 'matrix_sel', 0) for name in names if name not in candidates]


def extra_membership(ctx, selector, expected, near_x=1):
    """Reset actual fixtures, then independently query one additional selection."""
    for name, x in [('MatrixNear', near_x), ('MatrixMid', 4), ('MatrixFar', 7), ('MatrixCow', 2)]:
        fixture(ctx, f'tp @e[name={name}] {x} 80 0')
    for index, client in enumerate(ctx.clients):
        fixture(ctx, f'tp {client.name} {index * 3} 80 2')
    for name in ctx.state['names']:
        fixture(ctx, f'scoreboard players set {name} matrix_sel 0')
    issue(ctx, f'execute as {actor(ctx).name} at @s run tp {selector} 0 80 6')
    for name in ctx.state['names']:
        fixture(ctx, f'execute if entity @e[name={name},x=0,y=80,z=6,distance=..0.25] run scoreboard players set {name} matrix_sel 1')
    return [check_score(ctx, name, 'matrix_sel', int(name in expected)) for name in ctx.state['names']]


ADAPTERS = {'selectors.' + subject: Adapter(partial(setup, subject=subject),
            partial(act, subject=subject), partial(oracle, subject=subject))
            for subject in (*_SELECTORS, 'invalid_selector', 'nonop_selector_msg')}
