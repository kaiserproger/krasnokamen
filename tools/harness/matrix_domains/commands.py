"""Vanilla 26.1.2 command probes. No command acknowledgement is a state oracle.

Reference: https://minecraft.wiki/w/Commands and the versioned command facts
in scenarios/domains/evidence/DomCommands. Solaris extensions are labelled
explicitly; they must never be counted as vanilla parity.
"""
from __future__ import annotations

import re
from functools import partial
from dataclasses import asdict

from ..matrix_api import Adapter, Check, MissingCapability


def actor(ctx):
    if not ctx.clients:
        raise MissingCapability('client:actor')
    return ctx.clients[0]


def permission_level(ctx):
    permission = ctx.factors['permission']
    return 4 if permission == 'console' else int(permission.rsplit('_', 1)[-1])


def required_level(subject):
    if subject in {'help', 'list', 'me', 'msg', 'tell', 'w', 'teammsg', 'tm', 'settlement', 'trigger'}:
        return 0
    if subject in {'stop', 'save-all'}:
        return 4
    if subject in {'ban', 'ban-ip', 'banlist', 'co', 'tick', 'kick', 'mute', 'unmute',
                   'warn', 'transfer', 'pardon', 'pardon-ip', 'op', 'deop', 'whitelist'}:
        return 3
    return 2


def field(client, key):
    value = client.snapshot().get(key)
    if value is None:
        raise MissingCapability('cumulative_chat_history' if key == 'events' else 'snapshot:' + key)
    return value


def require_event_history(ctx):
    # Native MCP provides only the newest 20 rendered lines, without identity
    # or generation. An index into it is not an input/receipt fence.
    if ctx.factors.get('client_kind', '').startswith('native_'):
        raise MissingCapability('native_cumulative_received_chat')

def dimension_command(ctx, command):
    dimension = ctx.factors['dimension']
    if dimension not in {'overworld', 'minecraft:overworld'} and command.split()[0] in {
            'fill', 'setblock', 'summon', 'kill', 'tp', 'teleport', 'execute', 'data', 'forceload'}:
        identifier = dimension if ':' in dimension else 'minecraft:' + dimension
        return f'execute in {identifier} run {command}'
    return command



def fixture(ctx, command):
    return ctx.server.command(dimension_command(ctx, command))


def query(ctx, command):
    return ctx.server.query(dimension_command(ctx, command))


def issue(ctx, command):
    if ctx.factors['permission'] == 'console':
        ctx.state['command_reply'] = fixture(ctx, command)
    else:
        require_event_history(ctx)
        client = actor(ctx)
        before = len(field(client, 'events'))
        client.command(command)
        if command.startswith('gamerule ') and not ctx.state.get('permission_denied'):
            words = command.split()
            if len(words) == 3:
                ctx.wait_until(lambda: re.search(r'(?<![\w.-])' + re.escape(words[2]) + r'(?![\w.-])',
                                                query(ctx, ' '.join(words[:2]))) is not None,
                               description='authoritative gamerule mutation acknowledged')
        else:
            received = ctx.wait_until(
                lambda: [str(e) for e in field(client, 'events')[before:]
                         if 'MatrixConsoleFence' not in str(e)],
                description='received command feedback fence (not success oracle)')
            ctx.state['command_reply'] = '\n'.join(received)
    if not ctx.state.get('action_boundary_called'):
        ctx.state['action_boundary_called'] = True
        ctx.boundary()


def ticks(ctx, count):
    stats = ctx.server.dashboard_stats()
    if not isinstance(stats.get('simulation_tick'), int):
        raise MissingCapability('dashboard_stats:simulation_tick')
    start = stats['simulation_tick']
    ctx.wait_until(lambda: ctx.server.dashboard_stats()['simulation_tick'] >= start + count,
                   description=f'dashboard advances {count} ticks')


def check_block(ctx, position, block):
    marker = 'MatrixBlock' + ''.join(str(p).replace('-', 'm') for p in position)
    reply = query(ctx, f'execute if block {" ".join(map(str, position))} {block} run say {marker}')
    return Check('authoritative_block:' + str(position), marker in reply,
                 {'expected': block, 'reply': reply})


def check_entity(ctx, selector, present=True):
    marker = 'MatrixEntityOracle'
    condition = 'if' if present else 'unless'
    reply = query(ctx, f'execute {condition} entity {selector} run say {marker}')
    return Check('authoritative_entity:' + selector, marker in reply,
                 {'present': present, 'reply': reply})


def check_score(ctx, holder, objective, value):
    marker = 'MatrixScoreOracle'
    reply = query(ctx, f'execute if score {holder} {objective} matches {value} run say {marker}')
    return Check('authoritative_score:' + holder, marker in reply,
                 {'expected': value, 'reply': reply})


def inventory_count(client, item):
    return sum(s['count'] for s in field(client, 'inventory') if s['item_id'] == 'minecraft:' + item)


def arena(ctx):
    fixture(ctx, f'tp {actor(ctx).name} 0.5 90 0.5')
    ctx.server.ensure_loaded(((-8, 79, -8), (8, 94, 8)))
    fixture(ctx, 'fill -8 79 -8 8 79 8 stone')
    fixture(ctx, 'fill -8 80 -8 8 85 8 air')
    fixture(ctx, f'tp {actor(ctx).name} 0.5 80 0.5')
    ctx.wait_until(lambda: all(abs(a - b) < .01 for a, b in
                              zip(field(actor(ctx), 'position'), (.5, 80, .5))),
                   description='actor receives arena teleport')


def _setup(ctx, subject):
    name = actor(ctx).name
    ctx.state['subject'] = subject
    ctx.state['permission_denied'] = permission_level(ctx) < required_level(subject)
    if ctx.state['permission_denied']:
        ctx.state['events_before'] = [len(field(c, 'events')) for c in ctx.clients]
        return
    if subject in {'setblock', 'fill', 'execute', 'teleport', 'tp', 'spawnpoint', 'setworldspawn', 'summon', 'kill'}:
        arena(ctx)
    if subject in {'give', 'clear', 'enchant'}:
        fixture(ctx, f'clear {name}')
    if subject == 'banlist':
        fixture(ctx, 'ban MatrixOffline MatrixBanlist')
    if subject == 'clear':
        fixture(ctx, f'give {name} stone 7')
        ctx.wait_until(lambda: inventory_count(actor(ctx), 'stone') == 7, description='seven stone fixture')
    if subject in {'scoreboard', 'trigger'}:
        kind = 'trigger' if subject == 'trigger' else 'dummy'
        fixture(ctx, f'scoreboard objectives add matrix_cmd {kind}')
        fixture(ctx, f'scoreboard players set {name} matrix_cmd 0')
        if subject == 'trigger':
            fixture(ctx, f'scoreboard players enable {name} matrix_cmd')
    if subject == 'kill':
        fixture(ctx, 'summon pig 3 80 0 {NoAI:1b,CustomName:"MatrixKill"}')
    if subject == 'time':
        fixture(ctx, 'gamerule minecraft:advance_time false')
    if subject in {'experience', 'xp'}:
        fixture(ctx, f'experience set {name} 0 levels')
    if subject == 'gamerule':
        fixture(ctx, 'gamerule minecraft:keep_inventory false')
    if subject in {'function', 'schedule', 'reload', 'datapack'}:
        # A real fixture pack must be installed by the canonical lifecycle driver.
        packs = field(actor(ctx), 'fixture_datapacks')
        if 'matrix:command_probe' not in packs:
            raise MissingCapability('fixture_datapack:matrix:command_probe')
        fixture(ctx, 'scoreboard objectives add matrix_fn dummy')
        fixture(ctx, 'scoreboard players set result matrix_fn 0')
    if subject in {'msg', 'tell', 'w', 'teammsg', 'tm', 'kick', 'ban', 'ban-ip', 'pardon', 'pardon-ip', 'op', 'deop', 'transfer', 'mute', 'unmute', 'warn'} and len(ctx.clients) < 2:
        raise MissingCapability('multi_client_2')
    if subject in {'teammsg', 'tm'}:
        fixture(ctx, 'team add matrix_cmd')
        for client in ctx.clients[:2]:
            fixture(ctx, f'team join matrix_cmd {client.name}')
    if subject == 'bossbar':
        fixture(ctx, 'bossbar add matrix:test "Matrix"')
    if subject == 'team':
        fixture(ctx, 'team add matrix_cmd')
    if subject == 'enchant':
        fixture(ctx, f'give {name} diamond_sword')
        ctx.wait_until(lambda: inventory_count(actor(ctx), 'diamond_sword') == 1, description='sword fixture')
        slots = [s['slot'] for s in field(actor(ctx), 'inventory') if s['item_id'] == 'minecraft:diamond_sword' and 36 <= s['slot'] <= 44]
        if not slots:
            raise MissingCapability('fixture_hotbar_item:diamond_sword')
        actor(ctx).select_hotbar(slots[0] - 36)
        ctx.state['enchanted_slot'] = slots[0]
    if subject == 'advancement':
        fixture(ctx, f'advancement revoke {name} only minecraft:story/root')
    if subject in {'pardon', 'pardon-ip'}:
        target = ctx.clients[1].name
        fixture(ctx, ('ban ' if subject == 'pardon' else 'ban-ip ') + target + ' MatrixFixture')
    if subject == 'unmute':
        fixture(ctx, 'mute ' + ctx.clients[1].name)
    if subject == 'deop':
        fixture(ctx, 'op ' + ctx.clients[1].name)
    if subject in {'me', 'say', 'tellraw', 'msg', 'tell', 'w', 'teammsg', 'tm', 'mute', 'unmute', 'warn'}:
        require_event_history(ctx)
    ctx.state['events_before'] = [len(field(c, 'events')) for c in ctx.clients]


# Each entry is one concrete state transition, not a shell or scenario language.
_ACTIONS = {
    'advancement': 'advancement grant {p} only minecraft:story/root',
    'ban': 'ban {q} MatrixBan', 'ban-ip': 'ban-ip {q} MatrixBanIp',
    'banlist': 'banlist players', 'bossbar': 'bossbar set matrix:test value 37',
    'clear': 'clear {p} stone 3', 'co': 'co inspect',
    'data': 'data get entity {p} Inventory', 'datapack': 'datapack disable "file/matrix"',
    'help': 'help give', 'list': 'list', 'me': 'me MatrixEmote',
    'msg': 'msg {q} MatrixPrivate', 'tell': 'tell {q} MatrixPrivate', 'w': 'w {q} MatrixPrivate',
    'teammsg': 'teammsg MatrixTeam', 'tm': 'tm MatrixTeam',
    'settlement': 'settlement guide', 'trigger': 'trigger matrix_cmd add 3',
    'gamemode': 'gamemode creative {p}', 'teleport': 'teleport {p} 4.5 80 4.5',
    'tp': 'tp {p} 4.5 80 4.5', 'execute': 'execute positioned 2 80 2 run setblock ~ ~ ~ gold_block',
    'give': 'give {p} stone 7', 'kill': 'kill @e[type=pig,name=MatrixKill]',
    'time': 'time set 6000', 'weather': 'weather rain 6000t', 'difficulty': 'difficulty easy',
    'gamerule': 'gamerule minecraft:keep_inventory true',
    'summon': 'summon pig 3 80 0 {{NoAI:1b,CustomName:"MatrixSummon"}}',
    'worldborder': 'worldborder set 128', 'setblock': 'setblock 2 80 2 gold_block',
    'fill': 'fill 2 80 2 3 80 3 gold_block', 'experience': 'experience set {p} 7 levels',
    'xp': 'xp set {p} 7 levels', 'spawnpoint': 'spawnpoint {p} 4 80 4',
    'setworldspawn': 'setworldspawn 4 80 4', 'forceload': 'forceload add 0 0',
    'say': 'say MatrixBroadcast', 'seed': 'seed', 'enchant': 'enchant {p} sharpness 2',
    'effect': 'effect give {p} speed 30 1 true', 'debug': 'debug start',
    'team': 'team join matrix_cmd {p}', 'tps': 'tps', 'mspt': 'mspt',
    'scoreboard': 'scoreboard players set {p} matrix_cmd 37',
    'locate': 'locate biome minecraft:plains', 'tellraw': 'tellraw {p} {{"text":"MatrixRaw"}}',
    'title': 'title {p} title {{"text":"MatrixTitle"}}', 'function': 'function matrix:command_probe',
    'schedule': 'schedule function matrix:command_probe 5t', 'reload': 'reload',
    'tick': 'tick rate 10', 'kick': 'kick {q} MatrixKick', 'mute': 'mute {q}',
    'unmute': 'unmute {q}', 'warn': 'warn {q} MatrixWarn',
    'transfer': 'transfer 127.0.0.1 26479 {q}', 'pardon': 'pardon {q}',
    'pardon-ip': 'pardon-ip {q}', 'op': 'op {q}', 'deop': 'deop {q}',
    'whitelist': 'whitelist add MatrixOffline', 'stop': 'stop', 'save-all': 'save-all flush',
}


def _act(ctx, subject):
    q = ctx.clients[1].name if len(ctx.clients) > 1 else actor(ctx).name
    issue(ctx, _ACTIONS[subject].format(p=actor(ctx).name, q=q))
    if ctx.state.get('permission_denied'):
        return
    if subject in {'mute', 'unmute'}:
        ctx.clients[1].chat('MatrixMuteProbe')
    if subject in {'pardon', 'pardon-ip'}:
        ctx.clients[1].reconnect()
    if subject == 'schedule':
        ticks(ctx, 6)


def _oracle(ctx, subject):
    p = actor(ctx)
    if subject in {'setblock', 'execute'}:
        return [check_block(ctx, (2, 80, 2), 'gold_block')]
    if subject == 'fill':
        return [check_block(ctx, (x, 80, z), 'gold_block') for x in (2, 3) for z in (2, 3)]
    if subject in {'give', 'clear'}:
        expected = 4 if subject == 'clear' else 7
        checks = [Check('inventory_count', inventory_count(p, 'stone') == expected,
                        {'expected': expected, 'actual': inventory_count(p, 'stone')})]
        if subject == 'clear':
            reply = ctx.state['command_reply']
            checks.append(Check('removed_count_reported', re.search(r'(?i)removed\s+3\b', reply) is not None, reply))
        return checks
    if subject == 'enchant':
        actual = held_enchantments(ctx)
        return [Check('held_sharpness_level', actual == {'minecraft:sharpness': 2}, actual)]
    if subject == 'tick':
        reply = query(ctx, 'tick query')
        return [Check('tick_rate_query', re.search(r'(?is)(?:query.rate.running|tick rate).*?\b10\.0\b', reply) is not None, reply)]
    if subject == 'debug':
        reply = ctx.state['command_reply']
        if 'Unknown or incomplete' in reply:
            return [Check('vanilla_debug_profiler_available', False, reply)]
        return [Check('vanilla_debug_profiler_started', field(p, 'debug_profiler_running') is True)]
    if subject in {'teleport', 'tp'}:
        pos = field(p, 'position')
        return [Check('received_position', all(abs(a-b) < .01 for a, b in zip(pos, (4.5, 80, 4.5))), pos)]
    if subject == 'gamemode':
        return [Check('received_gamemode', field(p, 'game_mode') == 'creative', field(p, 'game_mode'))]
    if subject in {'scoreboard', 'trigger'}:
        checks = [check_score(ctx, p.name, 'matrix_cmd', 3 if subject == 'trigger' else 37)]
        if subject == 'trigger':
            issue(ctx, 'trigger matrix_cmd add 3')
            checks.append(check_score(ctx, p.name, 'matrix_cmd', 3))
        return checks
    if subject in {'summon', 'kill'}:
        checks = [check_entity(ctx, '@e[type=pig,name=Matrix' + ('Summon]' if subject == 'summon' else 'Kill]'), subject == 'summon')]
        if subject == 'summon':
            checks.append(check_entity(ctx, '@e[type=pig,name=MatrixSummon,x=3,y=80,z=0,distance=..0.01]'))
        return checks
    if subject in {'time', 'experience', 'xp', 'difficulty', 'gamerule', 'worldborder', 'bossbar', 'forceload'}:
        command, pattern = {
            'time': ('time query daytime', r'(?<!\d)6000(?!\d)'),
            'experience': (f'experience query {p.name} levels', r'(?<!\d)7(?!\d)'),
            'xp': (f'experience query {p.name} levels', r'(?<!\d)7(?!\d)'),
            'difficulty': ('difficulty', r'(?i)\beasy\b'),
            'gamerule': ('gamerule minecraft:keep_inventory', r'\btrue\b'),
            'worldborder': ('worldborder get', r'(?<!\d)128(?:\.0)?(?!\d)'),
            'bossbar': ('bossbar get matrix:test value', r'(?<!\d)37(?!\d)'),
            'forceload': ('forceload query 0 0', r'(?i)(?:is marked|is force|is marked for force|forceloaded)'),
        }[subject]
        reply = query(ctx, command)
        return [Check('independent_query', re.search(pattern, reply) is not None, reply)]
    if subject in {'function', 'schedule', 'reload'}:
        if subject == 'reload':
            issue(ctx, 'function matrix:command_probe')
        return [check_score(ctx, 'result', 'matrix_fn', 37)]
    if subject in {'me', 'say', 'tellraw', 'msg', 'tell', 'w', 'teammsg', 'tm', 'mute', 'unmute', 'warn'}:
        marker = {'me': 'MatrixEmote', 'say': 'MatrixBroadcast', 'tellraw': 'MatrixRaw',
                  'msg': 'MatrixPrivate', 'tell': 'MatrixPrivate', 'w': 'MatrixPrivate',
                  'teammsg': 'MatrixTeam', 'tm': 'MatrixTeam', 'mute': 'MatrixMuteProbe',
                  'unmute': 'MatrixMuteProbe', 'warn': 'MatrixWarn'}[subject]
        checks = []
        for index, client in enumerate(ctx.clients):
            events = field(client, 'events')[ctx.state['events_before'][index]:]
            expected = subject != 'mute'
            if subject in {'msg', 'tell', 'w', 'teammsg', 'tm'}:
                expected = index < 2
            if subject == 'tellraw':
                expected = index == 0
            if subject == 'warn':
                expected = index == 1
            matching = [str(e) for e in events if marker in str(e)]
            checks.append(Check('message_recipient:' + client.name,
                                len(matching) == int(expected), events))
            if subject == 'me' and expected:
                checks.append(Check('emote_format:' + client.name,
                                    any(f'* {p.name} {marker}' in e or
                                        ('chat.type.emote' in e and p.name in e) for e in matching), events))
            if subject == 'say' and expected:
                sender = 'Server' if ctx.factors['permission'] == 'console' else p.name
                checks.append(Check('broadcast_format:' + client.name,
                                    any(f'[{sender}] {marker}' in e or
                                        ('chat.type.announcement' in e and sender in e) for e in matching), events))
            if subject == 'msg' and expected and ctx.factors.get('plugin_profile') == 'live':
                prefix = f'[to {ctx.clients[1].name}]' if index == 0 else f'[from {p.name}]'
                checks.append(Check('essentials_private_format:' + client.name,
                                    any(prefix in str(e) and marker in str(e) for e in events), events))
        return checks
    if subject in {'kick', 'ban', 'ban-ip'}:
        target = ctx.clients[1]
        reason = field(target, 'disconnect_reason')
        return [Check('target_disconnected', field(target, 'connected') is False, reason),
                Check('disconnect_reason', 'Matrix' in reason, reason)]
    if subject in {'pardon', 'pardon-ip'}:
        return [Check('rejoin_after_pardon', field(ctx.clients[1], 'connected') is True)]
    if subject in {'op', 'deop'}:
        roots = field(ctx.clients[1], 'command_roots')
        return [Check('command_tree_permission', ('give' in roots) == (subject == 'op'), roots)]
    if subject == 'whitelist':
        reply = ctx.server.query('whitelist list')
        return [Check('whitelist_membership', 'MatrixOffline' in reply, reply)]
    if subject == 'team':
        reply = query(ctx, 'team list matrix_cmd')
        return [Check('team_membership', p.name in reply, reply)]
    if subject == 'data':
        reply = ctx.state.get('command_reply')
        if reply is None:
            reply = query(ctx, f'data get entity {p.name} Inventory')
        if 'Supported:' in reply or 'Unknown or incomplete' in reply:
            raise MissingCapability('data_get_entity')
        return [Check('vanilla_entity_inventory_query',
                      'has the following entity data:' in reply and '[' in reply, reply)]
    if subject in {'help', 'list', 'seed', 'banlist', 'tps', 'mspt', 'settlement', 'co'}:
        reply = ctx.state.get('command_reply')
        if reply is None:
            reply = '\n'.join(str(event) for event in field(p, 'events')[ctx.state['events_before'][0]:])
        if subject == 'list':
            match = re.search(r'There are (\d+) of a max of \d+ players online: ([^\x00]*)', reply)
            names = {c.name for c in ctx.clients}
            return [Check('listed_online_exact',
                          match is not None and int(match[1]) == len(names)
                          and set(match[2].strip().split(', ')) == names, reply)]
        if subject == 'seed':
            expected_seed = field(p, 'world_seed')
            return [Check('configured_world_seed', str(expected_seed) in reply, reply)]
        if subject == 'co':
            return [Check('inspection_mode_enabled', field(p, 'block_log_inspection_mode') is True)]
        if subject in {'tps', 'mspt'}:
            match = re.search(r'(?i)\b' + subject + r'\b[^0-9+-]*([+-]?[0-9]+(?:\.[0-9]+)?)', reply)
            value = float(match[1]) if match else None
            return [Check('numeric_report:' + subject,
                          value is not None and value >= 0 and (subject != 'tps' or value <= 20), reply)]
        patterns = {'help': r'give.*targets', 'banlist': r'MatrixOffline',
                    'settlement': r'(?i)settlement'}
        return [Check('query_output:' + subject, re.search(patterns[subject], reply) is not None, reply)]
    key, expected = {
        'weather': ('weather', 'rain'), 'spawnpoint': ('respawn_position', [4, 80, 4]),
        'setworldspawn': ('world_spawn', [4, 80, 4]),
        'effect': ('effects', {'minecraft:speed': {'amplifier': 1}}),
        'advancement': ('advancements', 'minecraft:story/root'), 'title': ('title', 'MatrixTitle'),
        'locate': ('located_biome', 'minecraft:plains'),
        'transfer': ('transfer', {'host': '127.0.0.1', 'port': 26479}),
        'datapack': ('enabled_datapacks', 'file/matrix'), 'stop': ('server_exit_code', 0),
        'save-all': ('checkpoint_flushed', True),
    }[subject]
    observer = ctx.clients[1] if subject == 'transfer' else p
    actual = field(observer, key)
    passed = actual == expected
    if subject == 'advancement':
        passed = expected in actual
    elif subject == 'datapack':
        passed = expected not in actual
    return [Check('observed:' + key, passed, {'expected': expected, 'actual': actual})]


def grammar_checks(ctx, subject, checks=None):
    """Exercise reachable advertised branches against independent state queries."""
    if checks is None:
        checks = []
    p = actor(ctx)
    if subject == 'time':
        for value, expected in [('day', 1000), ('noon', 6000), ('night', 13000),
                                ('midnight', 18000), ('1d', 0), ('2s', 40), ('17t', 17)]:
            issue(ctx, 'time set ' + value)
            reply = query(ctx, 'time query daytime')
            checks.append(Check('time_set:' + value, re.search(r'\b' + str(expected) + r'\b', reply) is not None, reply))
        issue(ctx, 'time add 3t')
        reply = query(ctx, 'time query daytime')
        checks.append(Check('time_add', re.search(r'\b20\b', reply) is not None, reply))
    elif subject in {'experience', 'xp'}:
        for unit in ('points', 'levels'):
            issue(ctx, f'{subject} set {p.name} 0 {unit}')
            issue(ctx, f'{subject} add {p.name} 3 {unit}')
            reply = query(ctx, f'experience query {p.name} {unit}')
            checks.append(Check('xp_add:' + unit, re.search(r'\b3\b', reply) is not None, reply))
    elif subject == 'difficulty':
        for value in ('normal', 'hard', 'easy'):
            issue(ctx, 'difficulty ' + value)
            reply = query(ctx, 'difficulty')
            checks.append(Check('difficulty:' + value, value in reply.lower(), reply))
        fixture(ctx, 'summon zombie 3 80 0 {NoAI:1b,CustomName:"MatrixPeaceful"}')
        checks.append(check_entity(ctx, '@e[name=MatrixPeaceful]'))
        issue(ctx, 'difficulty peaceful')
        ticks(ctx, 2)
        checks.append(check_entity(ctx, '@e[name=MatrixPeaceful]', False))
    elif subject == 'setblock':
        fixture(ctx, 'setblock 2 80 2 stone')
        issue(ctx, 'setblock 2 80 2 gold_block keep')
        checks.append(check_block(ctx, (2, 80, 2), 'stone'))
        issue(ctx, 'setblock 2 80 2 gold_block replace')
        checks.append(check_block(ctx, (2, 80, 2), 'gold_block'))
        fixture(ctx, 'gamerule minecraft:block_drops true')
        issue(ctx, 'setblock 2 80 2 stone destroy')
        checks.append(check_block(ctx, (2, 80, 2), 'stone'))
        checks.append(check_entity(ctx, '@e[type=item,x=2,y=80,z=2,distance=..2]'))
    elif subject == 'fill':
        for mode in ('outline', 'hollow'):
            fixture(ctx, 'fill 2 80 2 4 82 4 stone')
            issue(ctx, f'fill 2 80 2 4 82 4 gold_block {mode}')
            checks.append(check_block(ctx, (3, 81, 3), 'stone' if mode == 'outline' else 'air'))
            for x in range(2, 5):
                for y in range(80, 83):
                    for z in range(2, 5):
                        if (x, y, z) != (3, 81, 3):
                            checks.append(check_block(ctx, (x, y, z), 'gold_block'))
        issue(ctx, 'fill 2 80 2 4 82 4 stone keep')
        checks.append(check_block(ctx, (3, 81, 3), 'stone'))
        checks.append(check_block(ctx, (2, 80, 2), 'gold_block'))
        issue(ctx, 'fill 2 80 2 4 82 4 dirt replace stone')
        checks.append(check_block(ctx, (3, 81, 3), 'dirt'))
        checks.append(check_block(ctx, (2, 80, 2), 'gold_block'))
        fixture(ctx, 'gamerule minecraft:block_drops true')
        issue(ctx, 'fill 2 80 2 2 80 2 stone destroy')
        checks.append(check_block(ctx, (2, 80, 2), 'stone'))
        checks.append(check_entity(ctx, '@e[type=item,x=2,y=80,z=2,distance=..2]'))
        fixture(ctx, 'gamerule minecraft:max_block_modifications 1')
        issue(ctx, 'fill 2 80 2 3 80 2 diamond_block')
        checks.append(check_block(ctx, (2, 80, 2), 'stone'))
        checks.append(check_block(ctx, (3, 80, 2), 'gold_block'))
        fixture(ctx, 'gamerule minecraft:max_block_modifications 32768')
    elif subject == 'trigger':
        for suffix, expected in [('set 9', 9), ('', 10)]:
            fixture(ctx, f'scoreboard players enable {p.name} matrix_cmd')
            issue(ctx, f'trigger matrix_cmd {suffix}'.strip())
            checks.append(check_score(ctx, p.name, 'matrix_cmd', expected))
            issue(ctx, 'trigger matrix_cmd add 100')
            checks.append(check_score(ctx, p.name, 'matrix_cmd', expected))
    elif subject in {'teammsg', 'tm'}:
        fixture(ctx, 'team leave ' + p.name)
        issue(ctx, subject + ' MatrixNoTeam')
        reply = ctx.state['command_reply']
        checks.append(Check('teamless_message_rejected', 'team' in reply.lower() and
                            ('not' in reply.lower() or 'must' in reply.lower()), reply))
    elif subject == 'forceload':
        issue(ctx, 'forceload remove 0 0')
        reply = query(ctx, 'forceload query 0 0')
        checks.append(Check('forceload_removed', 'not' in reply.lower(), reply))
    elif subject == 'bossbar':
        issue(ctx, 'bossbar set matrix:test max 80')
        reply = query(ctx, 'bossbar get matrix:test max')
        checks.append(Check('bossbar_max', re.search(r'\b80\b', reply) is not None, reply))
        issue(ctx, 'bossbar remove matrix:test')
        reply = query(ctx, 'bossbar list')
        checks.append(Check('bossbar_removed', 'matrix:test' not in reply and 'Matrix' not in reply, reply))
    elif subject == 'scoreboard':
        issue(ctx, f'scoreboard players add {p.name} matrix_cmd 5')
        checks.append(check_score(ctx, p.name, 'matrix_cmd', 42))
        issue(ctx, f'scoreboard players remove {p.name} matrix_cmd 2')
        checks.append(check_score(ctx, p.name, 'matrix_cmd', 40))
        issue(ctx, 'scoreboard players set MatrixOperand matrix_cmd 3')
        issue(ctx, f'scoreboard players operation {p.name} matrix_cmd *= MatrixOperand matrix_cmd')
        checks.append(check_score(ctx, p.name, 'matrix_cmd', 120))
        issue(ctx, f'scoreboard players reset {p.name} matrix_cmd')
        reply = query(ctx, f'scoreboard players get {p.name} matrix_cmd')
        checks.append(Check('score_reset', 'not' in reply.lower() or 'no score' in reply.lower(), reply))
    elif subject == 'team':
        issue(ctx, 'team leave ' + p.name)
        reply = query(ctx, 'team list matrix_cmd')
        checks.append(Check('team_left', p.name not in reply, reply))
        issue(ctx, 'team join matrix_cmd ' + p.name)
        issue(ctx, 'team empty matrix_cmd')
        reply = query(ctx, 'team list matrix_cmd')
        checks.append(Check('team_empty', p.name not in reply, reply))
        issue(ctx, 'team remove matrix_cmd')
        reply = query(ctx, 'team list')
        checks.append(Check('team_removed', 'matrix_cmd' not in reply, reply))
    elif subject == 'banlist':
        fixture(ctx, 'ban-ip 192.0.2.1 MatrixOfflineIP')
        issue(ctx, 'banlist ips')
        checks.append(Check('ip_ban_listed', '192.0.2.1' in ctx.state['command_reply'], ctx.state['command_reply']))
    elif subject == 'clear':
        issue(ctx, f'clear {p.name} stone 0')
        checks.append(Check('clear_zero_is_count_only', inventory_count(p, 'stone') == 4, field(p, 'inventory')))
        issue(ctx, f'clear {p.name}')
        ctx.wait_until(lambda: len(field(p, 'inventory')) == 0, description='clear-all inventory update')
        checks.append(Check('clear_all', len(field(p, 'inventory')) == 0, field(p, 'inventory')))
    elif subject == 'give':
        fixture(ctx, f'clear {p.name}')
        issue(ctx, f'give {p.name} stone[minecraft:custom_name="MatrixNamed"] 1')
        ctx.wait_until(lambda: inventory_count(p, 'stone') == 1, description='named item received')
        items = field(p, 'inventory')
        named = [item for item in items if item['item_id'] == 'minecraft:stone']
        if not named or 'components' not in named[0]:
            raise MissingCapability('snapshot:inventory.components')
        component = named[0]['components'].get('minecraft:custom_name')
        checks.append(Check('given_component_preserved', 'MatrixNamed' in str(component), component))
        fixture(ctx, f'clear {p.name}')
        fixture(ctx, f'give {p.name} stone 2304')
        ctx.wait_until(lambda: inventory_count(p, 'stone') == 2304, description='full 36-stack inventory fixture')
        issue(ctx, f'give {p.name} dirt 1')
        checks.append(Check('overflow_does_not_replace_inventory', inventory_count(p, 'stone') == 2304, field(p, 'inventory')))
        checks.append(Check('overflow_not_inserted', inventory_count(p, 'dirt') == 0, field(p, 'inventory')))
        x, y, z = field(p, 'position')
        checks.append(check_entity(ctx, f'@e[type=item,x={x},y={y},z={z},distance=..4]'))
    elif subject == 'execute':
        fixture(ctx, 'scoreboard objectives add matrix_exec dummy')
        fixture(ctx, 'scoreboard players set result matrix_exec 0')
        for condition in ('if block 2 80 2 gold_block', 'unless block 2 80 2 stone',
                          f'if entity {p.name}', 'unless entity @e[name=MatrixAbsent]'):
            issue(ctx, f'execute {condition} run scoreboard players add result matrix_exec 1')
        checks.append(check_score(ctx, 'result', 'matrix_exec', 4))
        issue(ctx, 'execute if block 2 80 2 stone run scoreboard players add result matrix_exec 100')
        checks.append(check_score(ctx, 'result', 'matrix_exec', 4))
        issue(ctx, f'execute as {p.name} at @s run setblock ~1 ~ ~1 gold_block')
        checks.append(check_block(ctx, (1, 80, 1), 'gold_block'))
        issue(ctx, 'execute positioned 0 80 0 rotated 0 0 positioned ^ ^ ^2 run setblock ~ ~ ~ emerald_block')
        checks.append(check_block(ctx, (0, 80, 2), 'emerald_block'))
        issue(ctx, 'execute positioned 0 80 0 facing 0 80 10 positioned ^ ^ ^2 run setblock ~ ~ ~ diamond_block')
        checks.append(check_block(ctx, (0, 80, 2), 'diamond_block'))
        issue(ctx, 'execute positioned 2.8 80.6 2.8 align xyz run setblock ~0.5 ~0.5 ~0.5 copper_block')
        checks.append(check_block(ctx, (2, 80, 2), 'copper_block'))
        issue(ctx, f'execute as {p.name} at @s rotated 0 0 anchored eyes positioned ^ ^ ^1 run setblock ~ ~ ~ iron_block')
        checks.append(check_block(ctx, (0, 81, 1), 'iron_block'))
        issue(ctx, 'execute in minecraft:the_nether run setblock 4 80 4 obsidian')
        reply = query(ctx, 'execute in minecraft:the_nether if block 4 80 4 obsidian run say MatrixDimensionOracle')
        checks.append(Check('execute_dimension', 'MatrixDimensionOracle' in reply, reply))
        fixture(ctx, 'scoreboard players set MatrixStored matrix_exec 0')
        issue(ctx, 'execute store result score MatrixStored matrix_exec run scoreboard players get result matrix_exec')
        if 'not implemented' in ctx.state['command_reply'].lower():
            ctx.state.setdefault('grammar_missing_capabilities', []).append('execute_store')
        else:
            checks.append(check_score(ctx, 'MatrixStored', 'matrix_exec', 4))
    elif subject == 'enchant':
        for enchantment, level in [('silk_touch', 1), ('sharpness', 6), ('smite', 1)]:
            issue(ctx, f'enchant {p.name} {enchantment} {level}')
            actual = held_enchantments(ctx)
            checks.append(Check('incompatible_enchant_no_mutation:' + enchantment,
                                actual == {'minecraft:sharpness': 2}, actual))
            reply = ctx.state['command_reply']
            checks.append(Check('incompatible_enchant_reports_error:' + enchantment,
                                any(token in reply.lower() for token in ('cannot', 'incompatible', 'too high', 'failed')), reply))
        issue(ctx, f'enchant {p.name} sharpness 0')
        actual = held_enchantments(ctx)
        checks.append(Check('zero_enchant_does_not_downgrade', actual == {'minecraft:sharpness': 2}, actual))
    elif subject == 'tick':
        issue(ctx, 'tick freeze')
        reply = query(ctx, 'tick query')
        checks.append(Check('tick_frozen', 'frozen' in reply.lower(), reply))
        start = ctx.server.dashboard_stats()['simulation_tick']
        try:
            issue(ctx, 'tick step 5t')
            ctx.wait_until(lambda: ctx.server.dashboard_stats()['simulation_tick'] >= start + 5,
                           description='five explicitly stepped simulation ticks')
            after = ctx.server.dashboard_stats()['simulation_tick']
            checks.append(Check('exact_frozen_steps', after == start + 5, {'before': start, 'after': after}))
        finally:
            fixture(ctx, 'tick unfreeze')
        ticks(ctx, 2)
        start = ctx.server.dashboard_stats()['simulation_tick']
        issue(ctx, 'tick sprint 10t')
        ctx.wait_until(lambda: ctx.server.dashboard_stats()['simulation_tick'] >= start + 10,
                       description='requested sprint simulation ticks consumed')
        checks.append(Check('sprint_advances_requested_ticks',
                            ctx.server.dashboard_stats()['simulation_tick'] >= start + 10))
    return checks


def held_enchantments(ctx):
    items = field(actor(ctx), 'inventory')
    stack = next((item for item in items if item['slot'] == ctx.state['enchanted_slot']), None)
    if stack is None or 'components' not in stack:
        raise MissingCapability('snapshot:inventory.components')
    registry = field(actor(ctx), 'registries').get('minecraft:enchantment')
    if registry is None:
        raise MissingCapability('snapshot:registries.minecraft:enchantment')
    result = {}
    for registry_id, level in stack['components'].get('minecraft:enchantments', []):
        name = registry.get(str(registry_id))
        if name is None:
            raise MissingCapability('snapshot:registries.minecraft:enchantment:' + str(registry_id))
        result[name] = level
    return result


def complete_oracle(ctx, subject):
    if ctx.state.get('permission_denied'):
        reply = ctx.state['command_reply']
        rejected = Check('low_permission_rejected', 'Unknown or incomplete command' in reply, reply)
        if not rejected.passed:
            return [rejected]
        roots = field(actor(ctx), 'command_roots')
        return [rejected, Check('low_permission_root_hidden', subject not in roots, roots)]
    missing = None
    try:
        checks = _oracle(ctx, subject)
    except MissingCapability as error:
        missing = error
        checks = []
    try:
        grammar_checks(ctx, subject, checks)
    except MissingCapability as error:
        missing = missing or error
    ctx.state['domain_checks'] = [asdict(check) for check in checks]
    # Preserve real product failures even when another clause lacks observation.
    if not all(check.passed for check in checks):
        return checks
    if missing is not None:
        raise missing
    if ctx.state.get('grammar_missing_capabilities'):
        raise MissingCapability(ctx.state['grammar_missing_capabilities'][0])
    if required_level(subject):
        raise MissingCapability(f'client:permission_probe:op_{required_level(subject) - 1}')
    return checks


ADAPTERS = {'commands.' + subject: Adapter(partial(_setup, subject=subject),
            partial(_act, subject=subject), partial(complete_oracle, subject=subject))
            for subject in _ACTIONS}
