"""Session adapters. Full fixtures/references: scenarios/domains/session.yaml.

Vanilla player persistence: https://minecraft.wiki/w/Player.dat_format .
No command fence is treated as successful execution or a gameplay oracle.
"""
from __future__ import annotations

import hashlib
import json
import math
import re
import socket
import struct
import tomllib
import uuid
from functools import partial
from pathlib import Path

from tools.harness import wire
from tools.harness.matrix_api import Adapter, Check, MatrixContext, MissingCapability


def require_method(owner, name: str, capability: str):
    method = getattr(owner, name, None)
    if not callable(method):
        raise MissingCapability(capability)
    return method


def required(snapshot: dict, key: str):
    if key not in snapshot or snapshot[key] is None:
        raise MissingCapability('observe:' + key)
    return snapshot[key]


def command(ctx: MatrixContext, text: str) -> str:
    """Administrative fixture in the row's dimension, without changing actor mode."""
    dimension = ctx.factors['dimension']
    return ctx.server.command(f'execute in minecraft:{dimension} run {text}')


def data(ctx: MatrixContext, target: str, path: str = '') -> str:
    capability = 'data_get_entity' if target.startswith('entity ') else 'data_get_block'
    query = f'data get {target} {path}'.rstrip()
    output = ctx.server.query(f'execute in minecraft:{ctx.factors["dimension"]} run {query}')
    if re.search(r'(?i)(unknown|not implemented|unsupported|incomplete command|command blocks only)', output):
        raise MissingCapability(capability)
    # A command echo/fence alone is never a value. Retain the complete SNBT suffix.
    match = re.search(r'(?:has the following (?:entity|block) data:|following data:)\s*(.+)', output)
    if not match:
        raise MissingCapability(capability + ':decoded_feedback')
    return match.group(1).strip()


def player_data(ctx: MatrixContext, fields: tuple[str, ...]) -> dict[str, str]:
    return {field: data(ctx, 'entity ' + ctx.clients[0].name, field) for field in fields}


def inventory_count(snapshot: dict, item: str) -> int:
    return sum(stack['count'] for stack in required(snapshot, 'inventory')
               if stack.get('item_id') == item)


def unavailable(ctx: MatrixContext, *, capability: str):
    """Fail closed for a named absent shared-driver surface, before mutation."""
    raise MissingCapability(capability)


def unsupported(capability: str) -> Adapter:
    refusal = partial(unavailable, capability=capability)
    return Adapter(refusal, refusal, refusal)


_REJOIN_FIELDS = ('Inventory', 'Health', 'foodLevel', 'XpLevel', 'XpTotal',
                  'XpP', 'active_effects', 'playerGameType', 'Pos', 'Rotation')


def _rejoin_setup(ctx: MatrixContext) -> None:
    actor = ctx.clients[0]
    # Query support before modifying the fixture. Baseline is not an oracle.
    player_data(ctx, _REJOIN_FIELDS)
    require_method(actor, 'disconnect_snapshot', 'observe:leave_snapshot')
    command(ctx, f'clear {actor.name}')
    command(ctx, f'give {actor.name} minecraft:diamond 7')
    command(ctx, f'experience set {actor.name} 5 levels')
    command(ctx, f'effect give {actor.name} minecraft:speed infinite 0')
    command(ctx, 'gamerule naturalRegeneration false')
    command(ctx, 'fill 160 79 158 800 79 162 minecraft:stone')
    command(ctx, 'fill 160 80 158 800 84 162 minecraft:air')
    command(ctx, 'fill 160 85 158 162 89 162 minecraft:air')
    command(ctx, f'spawnpoint {actor.name} 161 80 160')
    actor.teleport_to(161.5, 88, 160.5)
    ctx.wait_until(lambda: required(actor.snapshot(), 'health') < 20,
                   description='received non-default health after actual fall')
    actor.look_at(800, 81.62, 160.5)
    # Shared physics input must really simulate the run; protocol currently
    # raises client:move_physics rather than teleporting or inventing hunger.
    actor.move(forward=1, ticks=1200, sprint=True)
    ctx.wait_until(lambda: required(actor.snapshot(), 'food') < 20,
                   description='received hunger depletion from real sprinting',
                   timeout_seconds=90)
    ctx.state['before'] = player_data(ctx, _REJOIN_FIELDS)


def _rejoin_act(ctx: MatrixContext) -> None:
    actor = ctx.clients[0]
    capture = require_method(actor, 'disconnect_snapshot', 'observe:leave_snapshot')
    ctx.state['leave'] = capture(fields=_REJOIN_FIELDS)
    actor.reconnect()
    ctx.boundary()
    ctx.state['after'] = player_data(ctx, _REJOIN_FIELDS)
    ctx.state['rejoin_diamonds'] = inventory_count(
        ctx.wait_until(lambda: actor.snapshot() if 'inventory' in actor.snapshot() else None,
                       description='received restored inventory'), 'minecraft:diamond')


def _rejoin_oracle(ctx: MatrixContext) -> list[Check]:
    before = ctx.state['leave']
    after = ctx.state['after']
    if not isinstance(before, dict) or any(field not in before for field in _REJOIN_FIELDS):
        raise MissingCapability('observe:leave_snapshot')
    checks = [Check('rejoin.' + field, after[field] == before[field],
                    {'before': before[field], 'after': after[field]}) for field in _REJOIN_FIELDS]
    checks.append(Check('rejoin.diamond_custody', ctx.state['rejoin_diamonds'] == 7,
                        ctx.state['rejoin_diamonds']))
    return checks


def _offline_setup(ctx: MatrixContext) -> None:
    # Read the actual canonical fixture config, not an invented client flag.
    mode = required(required(_server_config(ctx), 'auth'), 'online_mode')
    if mode is not False:
        raise MissingCapability('offline_mode_fixture')
    ctx.state['uuid_before'] = data(ctx, 'entity ' + ctx.clients[0].name, 'UUID')
    command(ctx, f'clear {ctx.clients[0].name}')
    command(ctx, f'give {ctx.clients[0].name} minecraft:diamond 7')
    ctx.state['inventory_before'] = data(ctx, 'entity ' + ctx.clients[0].name, 'Inventory')


def _offline_act(ctx: MatrixContext) -> None:
    ctx.clients[0].disconnect()
    ctx.clients[0].reconnect()
    ctx.boundary()
    ctx.state['uuid_after'] = data(ctx, 'entity ' + ctx.clients[0].name, 'UUID')
    ctx.state['inventory_after'] = data(ctx, 'entity ' + ctx.clients[0].name, 'Inventory')
    ctx.state['offline_diamonds'] = inventory_count(
        ctx.wait_until(lambda: ctx.clients[0].snapshot() if 'inventory' in ctx.clients[0].snapshot() else None,
                       description='received offline restored inventory'), 'minecraft:diamond')


def _offline_oracle(ctx: MatrixContext) -> list[Check]:
    digest = hashlib.md5(('OfflinePlayer:' + ctx.clients[0].name).encode(), usedforsecurity=False).digest()
    expected = uuid.UUID(bytes=digest, version=3)
    raw = ctx.state['uuid_after']
    match = re.fullmatch(r'\[I;\s*(-?\d+),\s*(-?\d+),\s*(-?\d+),\s*(-?\d+)\s*\]', raw)
    if not match:
        raise MissingCapability('data_get_entity:UUID_int_array')
    actual = uuid.UUID(bytes=b''.join((int(n) & 0xffffffff).to_bytes(4, 'big') for n in match.groups()))
    return [Check('offline.name_based_uuid', actual == expected, {'actual': str(actual), 'expected': str(expected)}),
            Check('offline.uuid_stable', raw == ctx.state['uuid_before']),
            Check('offline.inventory_restored', ctx.state['inventory_after'] == ctx.state['inventory_before']),
            Check('offline.diamond_custody', ctx.state['offline_diamonds'] == 7, ctx.state['offline_diamonds'])]


def _dashboard_setup(ctx: MatrixContext) -> None:
    ctx.state['before_stats'] = require_method(ctx.server, 'dashboard_full_stats', 'server:dashboard_full_stats')()
    required(ctx.state['before_stats'], 'simulation_tick')


def _dashboard_act(ctx: MatrixContext) -> None:
    ctx.state['dashboard_chat'] = _cohort_chat(ctx, 'matrix-dashboard-' + ctx.row['id'])
    before = require_method(ctx.server, 'dashboard_full_stats', 'server:dashboard_full_stats')()['simulation_tick']
    ctx.state['chat_tick'] = before
    ctx.boundary()
    def advanced():
        stats = require_method(ctx.server, 'dashboard_full_stats', 'server:dashboard_full_stats')()
        return stats if required(stats, 'simulation_tick') > before else None
    ctx.state['after_stats'] = ctx.wait_until(advanced, description='dashboard simulation tick advances')


def _dashboard_oracle(ctx: MatrixContext) -> list[Check]:
    stats = ctx.state['after_stats']
    players = required(stats, 'players')
    # Player/tick keys are StatsSnapshot fields; RSS needs a driver process probe.
    count = required(players, 'count')
    tps = required(stats, 'tps')
    latency = required(stats, 'tick')
    p99 = required(latency, 'p99_us')
    p50 = required(latency, 'p50_us')
    process_stats = require_method(ctx.server, 'process_stats', 'observe:server_rss')()
    rss = required(process_stats, 'rss_bytes')
    return [Check('dashboard.online', count == len(ctx.clients), {'actual': count, 'expected': len(ctx.clients)}),
            Check('dashboard.tps_finite', isinstance(tps, (int, float)) and math.isfinite(tps) and 0 <= tps <= 20.1, tps),
            Check('dashboard.p99_nonnegative', isinstance(p99, (int, float)) and math.isfinite(p99) and p99 >= 0, p99),
            Check('dashboard.rss_positive', isinstance(rss, int) and rss > 0, rss),
            Check('dashboard.mspt_nonnegative', isinstance(p50, (int, float)) and math.isfinite(p50) and p50 >= 0,
                  {'median_microseconds': p50}),
            Check('dashboard.chat_echo', set(ctx.state['dashboard_chat']) == {c.name for c in ctx.clients}
                  and all(ctx.state['dashboard_chat'].values()), ctx.state['dashboard_chat']),
            Check('dashboard.tick_advanced', stats['simulation_tick'] > ctx.state['chat_tick'])]

def _status_setup(ctx: MatrixContext) -> None:
    port = getattr(ctx.server, 'port', None)
    if not isinstance(port, int):
        raise MissingCapability('server:protocol_endpoint')
    ctx.state['endpoint'] = ('127.0.0.1', port)
    ctx.state['status_stats'] = ctx.server.dashboard_stats()


def _status_act(ctx: MatrixContext) -> None:
    ctx.state['full_stats'] = require_method(ctx.server, 'dashboard_full_stats', 'server:dashboard_full_stats')()
    ctx.state['status'] = wire.probe_status(ctx.state['endpoint'])
    ctx.boundary()


def _status_oracle(ctx: MatrixContext) -> list[Check]:
    status = ctx.state['status']
    stats = ctx.state['full_stats']
    players = required(status, 'players')
    roster = required(stats, 'players')
    return [
        Check('status.version', status.get('version') == {'name': '26.1.2', 'protocol': 775}, status.get('version')),
        Check('status.online', players.get('online') == len(ctx.clients), players),
        Check('status.max', players.get('max') == required(roster, 'max'), players),
        Check('status.motd', status.get('description') == {'text': required(required(stats, 'world'), 'motd')}, status.get('description')),
        Check('status.sample_schema', isinstance(players.get('sample'), list) and all(isinstance(p, dict) and 'id' in p and 'name' in p for p in players['sample'])),
        # probe_status rejects a wrong packet id or any unequal payload bytes.
        Check('status.pong_echo', True, {'validated_by': 'wire.probe_status', 'payload': 0x102030405060708}),
    ]


def _mismatch_act(ctx: MatrixContext) -> None:
    replies = []
    for index, version in enumerate((774, 776)):
        with socket.create_connection(ctx.state['endpoint'], timeout=10) as connection:
            connection.settimeout(10)
            connection.sendall(wire.frame(0, wire.handshake('localhost', ctx.state['endpoint'][1], 2, version)))
            packet, body = wire.read_frame(connection)
            replies.append({'protocol': version, 'packet': packet, 'reason': json.loads(wire.read_mc_string(body))})
        if index == 0:
            ctx.boundary()
    ctx.state['mismatch_replies'] = replies
    ctx.state['after_roster'] = required(ctx.server.dashboard_stats(), 'players')


def _mismatch_oracle(ctx: MatrixContext) -> list[Check]:
    checks = []
    for reply in ctx.state['mismatch_replies']:
        checks.append(Check('protocol_mismatch.' + str(reply['protocol']),
                            reply['packet'] == wire.LOGIN_CLIENTBOUND_DISCONNECT
                            and reply['reason'].get('translate') == 'multiplayer.disconnect.incompatible'
                            and reply['reason'].get('with') == ['26.1.2'], reply))
    checks.append(Check('protocol_mismatch.no_admitted_player',
                        ctx.state['after_roster'] == ctx.state['status_stats']['players'],
                        ctx.state['after_roster']))
    return checks


def _server_config(ctx: MatrixContext) -> dict:
    path = getattr(ctx.server, 'config_path', None)
    if path is None:
        raise MissingCapability('server:effective_config')
    return tomllib.loads(Path(path).read_text())


def _admin_setup(ctx: MatrixContext, service: str) -> None:
    """Enable one listener on the same canonical fixture, never a second server."""
    port = getattr(ctx.server, 'port', None)
    if not isinstance(port, int):
        raise MissingCapability('server:protocol_endpoint')
    reconfigure = require_method(ctx.server, 'reconfigure', 'server:reconfigure')
    settings = {'enabled': True, 'port': port + 2}
    if service == 'rcon':
        settings.update(bind_address='127.0.0.1', password='MatrixRconLocal42')
    for client in ctx.clients:
        client.disconnect()
    reconfigure({service: settings})
    for client in ctx.clients:
        client.reconnect()
    ctx.state['admin_endpoint'] = ['127.0.0.1', port + 2]
    ctx.state['admin_stats'] = require_method(ctx.server, 'dashboard_full_stats', 'server:dashboard_full_stats')()
    ctx.state['admin_config'] = _server_config(ctx)


def _recv_exact(connection, length: int) -> bytes:
    chunks = bytearray()
    while len(chunks) < length:
        part = connection.recv(length - len(chunks))
        if not part:
            raise EOFError('administrative response ended before its declared length')
        chunks.extend(part)
    return bytes(chunks)


def _rcon_frame(request_id: int, kind: int, text: str) -> bytes:
    # Local 26.1.2 javap fact: mc-edge/src/rcon.rs:1-2,35-64.
    body = struct.pack('<ii', request_id, kind) + text.encode() + b'\0\0'
    return struct.pack('<i', len(body)) + body


def _rcon_send(connection, request_id: int, kind: int, text: str) -> None:
    connection.sendall(_rcon_frame(request_id, kind, text))


def _rcon_reply(connection) -> dict:
    length, = struct.unpack('<i', _recv_exact(connection, 4))
    if not 10 <= length <= 4106:
        raise ValueError('invalid RCON response length')
    body = _recv_exact(connection, length)
    if body[-2:] != b'\0\0':
        raise ValueError('invalid RCON response terminators')
    request_id, kind = struct.unpack('<ii', body[:8])
    return {'id': request_id, 'kind': kind, 'text': body[8:-2].decode()}


def _rcon_setup(ctx: MatrixContext) -> None:
    _admin_setup(ctx, 'rcon')
    ctx.server.command('scoreboard objectives add lc_rcon dummy')
    ctx.server.command('scoreboard players set sentinel lc_rcon 3')


def _rcon_act(ctx: MatrixContext) -> None:
    endpoint = tuple(ctx.state['admin_endpoint'])
    with socket.create_connection(endpoint, timeout=10) as connection:
        # Pipeline atomically: a failed auth must never authorize the mutation.
        connection.sendall(_rcon_frame(41, 3, 'wrong-password')
                           + _rcon_frame(42, 2, 'scoreboard players set sentinel lc_rcon 999'))
        ctx.state['wrong_auth'] = _rcon_reply(connection)
    with socket.create_connection(endpoint, timeout=10) as connection:
        _rcon_send(connection, 43, 2, 'scoreboard players set sentinel lc_rcon 999')
        ctx.state['unauth_command'] = _rcon_reply(connection)
    ctx.state['unauth_score'] = ctx.server.query('scoreboard players get sentinel lc_rcon')
    ctx.boundary()
    with socket.create_connection(endpoint, timeout=10) as connection:
        _rcon_send(connection, 44, 3, 'MatrixRconLocal42')
        ctx.state['valid_auth'] = _rcon_reply(connection)
        _rcon_send(connection, 45, 2, 'scoreboard players set sentinel lc_rcon 17')
        ctx.state['rcon_set'] = _rcon_reply(connection)
        _rcon_send(connection, 46, 2, 'scoreboard players get sentinel lc_rcon')
        ctx.state['rcon_get'] = _rcon_reply(connection)
    ctx.state['rcon_score'] = ctx.server.query('scoreboard players get sentinel lc_rcon')


def _rcon_oracle(ctx: MatrixContext) -> list[Check]:
    state = ctx.state
    return [
        Check('rcon.wrong_password_rejected', state['wrong_auth']['id'] == -1
              and state['wrong_auth']['kind'] == 2, state['wrong_auth']),
        Check('rcon.unauth_command_rejected', state['unauth_command']['id'] == -1,
              state['unauth_command']),
        Check('rcon.unauth_no_mutation', bool(re.search(r'\b3\b', state['unauth_score']))
              and not re.search(r'\b999\b', state['unauth_score']), state['unauth_score']),
        Check('rcon.correct_auth', state['valid_auth'] == {'id': 44, 'kind': 2, 'text': ''},
              state['valid_auth']),
        Check('rcon.command_response', state['rcon_get']['id'] == 46
              and state['rcon_get']['kind'] == 0
              and bool(re.search(r'\b17\b', state['rcon_get']['text'])), state['rcon_get']),
        Check('rcon.authoritative_score', bool(re.search(r'\b17\b', state['rcon_score'])),
              state['rcon_score']),
    ]


def _query_stat(connection, *, full: bool, session_id: bytes) -> dict:
    # Local 26.1.2 javap fact: mc-edge/src/query.rs:1,55-80,97-135.
    connection.send(b'\xfe\xfd\x09' + session_id)
    challenge = connection.recv(65507)
    if challenge[:5] != b'\x09' + session_id or not challenge.endswith(b'\0'):
        raise ValueError('GS4 challenge identity/framing mismatch')
    token = int(challenge[5:-1])
    connection.send(b'\xfe\xfd\x00' + session_id + struct.pack('>i', token)
                    + (b'\0\0\0\0' if full else b''))
    reply = connection.recv(65507)
    if reply[:5] != b'\x00' + session_id:
        raise ValueError('GS4 stat identity mismatch')
    payload = reply[5:]
    if full:
        if not payload.startswith(b'splitnum\0\x80\0'):
            raise ValueError('GS4 full-stat framing mismatch')
        values, names = payload[11:].split(b'\0\x01player_\0\0', 1)
        fields = values.rstrip(b'\0').split(b'\0')
        if len(fields) % 2:
            raise ValueError('GS4 full-stat key/value mismatch')
        result = {fields[i].decode(): fields[i + 1].decode() for i in range(0, len(fields), 2)}
        result['players'] = [name.decode() for name in names.rstrip(b'\0').split(b'\0') if name]
        return result
    motd, gametype, world, online, maximum, rest = payload.split(b'\0', 5)
    if len(rest) < 3 or not rest.endswith(b'\0'):
        raise ValueError('GS4 basic-stat endpoint framing mismatch')
    return {'hostname': motd.decode(), 'gametype': gametype.decode(), 'map': world.decode(),
            'numplayers': online.decode(), 'maxplayers': maximum.decode(),
            'hostport': str(struct.unpack('<H', rest[:2])[0]), 'hostip': rest[2:-1].decode()}


def _query_setup(ctx: MatrixContext) -> None:
    _admin_setup(ctx, 'query')


def _query_act(ctx: MatrixContext) -> None:
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as connection:
        connection.settimeout(10)
        connection.connect(tuple(ctx.state['admin_endpoint']))
        ctx.state['query_basic'] = _query_stat(connection, full=False, session_id=b'\x01\x02\x03\x04')
        ctx.state['query_full'] = _query_stat(connection, full=True, session_id=b'\x01\x02\x03\x05')
        ctx.boundary()
        connection.send(b'\xfe\xfd\x00')  # Truncated, unauthenticated request.
        ctx.state['query_after_malformed'] = _query_stat(connection, full=False, session_id=b'\x01\x02\x03\x06')


def _query_oracle(ctx: MatrixContext) -> list[Check]:
    state = ctx.state
    config = state['admin_config']
    stats = state['admin_stats']
    expected = {'hostname': required(required(stats, 'world'), 'motd'), 'gametype': 'SMP',
                'map': Path(config['data']['world_dir']).name, 'numplayers': str(len(ctx.clients)),
                'maxplayers': str(required(required(stats, 'players'), 'max')),
                'hostport': str(config['network']['port']), 'hostip': config['network']['bind_address']}
    checks = [Check('query.' + name + '.' + key, state[name].get(key) == value,
                    {'expected': value, 'actual': state[name].get(key)})
              for name in ('query_basic', 'query_full', 'query_after_malformed')
              for key, value in expected.items()]
    checks.extend([
        Check('query.full_version', state['query_full'].get('version') == '26.1.2', state['query_full']),
        Check('query.full_game_id', state['query_full'].get('game_id') == 'MINECRAFT'),
        Check('query.full_roster', sorted(state['query_full']['players']) == sorted(c.name for c in ctx.clients),
              state['query_full']['players']),
    ])
    return checks


def _admission_setup(ctx: MatrixContext) -> None:
    _status_setup(ctx)
    if required(required(_server_config(ctx), 'auth'), 'online_mode') is not False:
        raise MissingCapability('offline_mode_fixture')


def _login_probe(ctx: MatrixContext, name: str) -> dict:
    """Read actual Login-state rejection without constructing a fake Play client."""
    from tools.harness.matrix_execution import offline_uuid
    endpoint = tuple(ctx.state['endpoint'])
    with socket.create_connection(endpoint, timeout=10) as connection:
        connection.sendall(wire.frame(0, wire.handshake('localhost', endpoint[1], 2, 775))
                           + wire.frame(wire.LOGIN_SERVERBOUND_START, wire.mc_string(name) + offline_uuid(name).bytes))
        packet, body = wire.read_frame(connection)
        if packet == wire.LOGIN_CLIENTBOUND_SET_COMPRESSION:
            packet, body = wire.read_compressed_frame(connection)
        reason = json.loads(wire.read_mc_string(body)) if packet == wire.LOGIN_CLIENTBOUND_DISCONNECT else None
        return {'packet': packet, 'reason': reason}


def _cohort_chat(ctx: MatrixContext, token: str) -> dict:
    offsets = {client.name: len(required(client.snapshot(), 'events')) for client in ctx.clients}
    ctx.clients[0].chat(token)
    received = {}
    for client in ctx.clients:
        received[client.name] = client.wait_until(
            lambda client=client: [event for event in required(client.snapshot(), 'events')[offsets[client.name]:]
                                   if token in str(event)],
            description='received admission-result chat on ' + client.name)
    return received


def _whitelist_setup(ctx: MatrixContext) -> None:
    _admission_setup(ctx)
    for client in ctx.clients[1:]:
        ctx.server.command('whitelist add ' + client.name)
    ctx.server.command('whitelist remove ' + ctx.clients[0].name)
    ctx.server.command('whitelist on')


def _whitelist_act(ctx: MatrixContext) -> None:
    actor = ctx.clients[0]
    actor.disconnect()
    ctx.state['whitelist_refusal'] = _login_probe(ctx, actor.name)
    ctx.server.command('whitelist add ' + actor.name)
    actor.reconnect()
    ctx.state['whitelist_play'] = required(actor.snapshot(), 'position')
    ctx.boundary()
    ctx.state['whitelist_chat'] = _cohort_chat(ctx, 'matrix-whitelist-' + ctx.row['id'])
    ctx.state['whitelist_roster'] = required(ctx.server.dashboard_stats(), 'players')


def _whitelist_oracle(ctx: MatrixContext) -> list[Check]:
    refusal = ctx.state['whitelist_refusal']
    return [
        Check('whitelist.unlisted_rejected', refusal['packet'] == wire.LOGIN_CLIENTBOUND_DISCONNECT
              and (refusal['reason'] or {}).get('translate') == 'multiplayer.disconnect.not_whitelisted', refusal),
        Check('whitelist.listed_reaches_play', ctx.state['whitelist_play'] is not None),
        Check('whitelist.roster', ctx.state['whitelist_roster'] == len(ctx.clients), ctx.state['whitelist_roster']),
        Check('whitelist.received_cohort', set(ctx.state['whitelist_chat']) == {c.name for c in ctx.clients}
              and all(ctx.state['whitelist_chat'].values()), ctx.state['whitelist_chat']),
    ]


def _full_setup(ctx: MatrixContext) -> None:
    _admission_setup(ctx)
    if any(client.name == 'MatrixOverflow' for client in ctx.clients):
        raise MissingCapability('fixture:unused_overflow_identity')
    for client in ctx.clients:
        client.disconnect()
    require_method(ctx.server, 'reconfigure', 'server:reconfigure')({'server': {'max_players': len(ctx.clients)}})
    for client in ctx.clients:
        client.reconnect()
    ctx.state['full_limit'] = required(required(
        require_method(ctx.server, 'dashboard_full_stats', 'server:dashboard_full_stats')(), 'players'), 'max')


def _full_act(ctx: MatrixContext) -> None:
    ctx.state['full_refusal'] = _login_probe(ctx, 'MatrixOverflow')
    ctx.boundary()
    ctx.state['full_chat'] = _cohort_chat(ctx, 'matrix-full-' + ctx.row['id'])
    ctx.state['full_roster'] = required(ctx.server.dashboard_stats(), 'players')


def _full_oracle(ctx: MatrixContext) -> list[Check]:
    refusal = ctx.state['full_refusal']
    return [
        Check('full.configured_limit', ctx.state['full_limit'] == len(ctx.clients), ctx.state['full_limit']),
        Check('full.extra_login_rejected', refusal['packet'] == wire.LOGIN_CLIENTBOUND_DISCONNECT
              and (refusal['reason'] or {}).get('translate') == 'multiplayer.disconnect.server_full', refusal),
        Check('full.existing_roster', ctx.state['full_roster'] == len(ctx.clients), ctx.state['full_roster']),
        Check('full.existing_clients_progress', set(ctx.state['full_chat']) == {c.name for c in ctx.clients}
              and all(ctx.state['full_chat'].values()), ctx.state['full_chat']),
    ]


def _ban_setup(ctx: MatrixContext) -> None:
    _admission_setup(ctx)
    if ctx.factors['server_lifecycle'] == 'hot_backup_during_action':
        raise MissingCapability('server:restored_player_data')
    actor = ctx.clients[0]
    command(ctx, f'clear {actor.name}')
    command(ctx, f'give {actor.name} minecraft:diamond 7')
    ctx.wait_until(lambda: inventory_count(actor.snapshot(), 'minecraft:diamond') == 7,
                   description='received seven-diamond ban fixture')


def _ban_act(ctx: MatrixContext) -> None:
    actor = ctx.clients[0]
    actor.disconnect()
    ctx.server.command('ban ' + actor.name + ' matrix-name-ban')
    ctx.state['name_ban'] = _login_probe(ctx, actor.name)
    ctx.server.command('pardon ' + actor.name)
    actor.reconnect()
    ctx.state['name_pardon_diamonds'] = inventory_count(actor.snapshot(), 'minecraft:diamond')
    ctx.boundary()
    # This is the isolated canonical fixture's loopback cohort, not a live
    # user-facing server. Restore every cohort member after the IP-ban trial.
    for client in ctx.clients:
        client.disconnect()
    ctx.server.command('ban-ip 127.0.0.1 matrix-ip-ban')
    ctx.state['ip_ban'] = _login_probe(ctx, actor.name)
    ctx.server.command('pardon-ip 127.0.0.1')
    for client in ctx.clients:
        client.reconnect()
    ctx.state['ip_pardon_diamonds'] = inventory_count(actor.snapshot(), 'minecraft:diamond')
    ctx.state['ban_chat'] = _cohort_chat(ctx, 'matrix-pardoned-' + ctx.row['id'])


def _ban_oracle(ctx: MatrixContext) -> list[Check]:
    checks = []
    for name, reason in (('name', 'matrix-name-ban'), ('ip', 'matrix-ip-ban')):
        reply = ctx.state[name + '_ban']
        checks.append(Check('ban.' + name + '_refused', reply['packet'] == wire.LOGIN_CLIENTBOUND_DISCONNECT
                            and reason in json.dumps(reply['reason']), reply))
        count = ctx.state[name + '_pardon_diamonds']
        checks.append(Check('ban.' + name + '_pardon_restores_inventory', count == 7, count))
    checks.append(Check('ban.pardoned_cohort_progress',
                        set(ctx.state['ban_chat']) == {c.name for c in ctx.clients}
                        and all(ctx.state['ban_chat'].values()), ctx.state['ban_chat']))
    return checks


_BLOCKED = {
    'join_fresh_spawn': 'snapshot:chunk_collision_shapes',
    'online_mode_auth': 'online_mode_auth',
    'kick': 'observe:disconnect_reason_and_leave_component',
    'mute_warn': 'observe:command_roots',
    'transfer': 'transfer_follow',
    'connection_limit': 'same_tick_admission_fence',
    'view_distance_negotiation': 'observe:cache_center_and_simulation_distance',
    'locale_presentation': 'observe:localized_dialog',
    'velocity_forwarding': 'velocity_proxy',
    'msmp_management': 'admin_protocol_client:msmp',
    'server_brand_f3': 'read_hud_ui',
}
ADAPTERS = {'session.' + name: unsupported(capability) for name, capability in _BLOCKED.items()}
ADAPTERS.update({
    'session.rejoin_restore_state': Adapter(_rejoin_setup, _rejoin_act, _rejoin_oracle),
    'session.offline_mode_uuid': Adapter(_offline_setup, _offline_act, _offline_oracle),
    'session.dashboard_metrics': Adapter(_dashboard_setup, _dashboard_act, _dashboard_oracle),
    'session.status_ping': Adapter(_status_setup, _status_act, _status_oracle),
    'session.protocol_mismatch': Adapter(_status_setup, _mismatch_act, _mismatch_oracle),
    'session.rcon': Adapter(_rcon_setup, _rcon_act, _rcon_oracle),
    'session.query_gs4': Adapter(_query_setup, _query_act, _query_oracle),
    'session.whitelist_gate': Adapter(_whitelist_setup, _whitelist_act, _whitelist_oracle),
    'session.max_players_full': Adapter(_full_setup, _full_act, _full_oracle),
    'session.ban_pardon': Adapter(_ban_setup, _ban_act, _ban_oracle),
})
