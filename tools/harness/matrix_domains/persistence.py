"""Persistence contracts; unavailable lifecycle/atomic readbacks fail closed.

References: https://minecraft.wiki/w/Commands/save and Player.dat_format.
Hot backup and WAL are Solaris durability guarantees, not vanilla file formats.
"""
from __future__ import annotations

import math
import re
from pathlib import Path

from tools.harness import runtime
from tools.harness.matrix_api import Adapter, Check, MatrixContext, MissingCapability
from tools.harness.matrix_domains.session import _cohort_chat, command, inventory_count, player_data, required, require_method, unsupported


def _save_setup(ctx: MatrixContext) -> None:
    ctx.state['negative'] = ctx.factors['permission'] not in {'console', 'op_4'}
    if ctx.state['negative']:
        ctx.state['command_roots'] = required(ctx.clients[0].snapshot(), 'command_roots')
    log_path = getattr(ctx.server, 'log_path', None)
    if log_path is None:
        raise MissingCapability('server:command_result_log')
    ctx.state['log_path'] = str(log_path)
    position = required(ctx.clients[0].snapshot(), 'position')
    x, y, z = (math.floor(value) for value in position)
    ctx.state['marker'] = (x + 2, y, z)
    command(ctx, f'setblock {x + 2} {y} {z} minecraft:gold_block')
    command(ctx, 'scoreboard objectives add lc_save dummy')
    command(ctx, 'scoreboard players set sentinel lc_save 17')
    ctx.state['before_tick'] = required(ctx.server.dashboard_stats(), 'simulation_tick')
    token = 'matrix-save-chat-' + ctx.row['id']
    ctx.state['chat_token'] = token
    # The source driver's event list must be received chat, not local sent text.
    required(ctx.clients[0].snapshot(), 'events')


def _save_act(ctx: MatrixContext) -> None:
    actor = ctx.clients[0]
    log_path = Path(ctx.state['log_path'])
    offset = len(log_path.read_text(errors='replace'))
    event_offset = len(required(actor.snapshot(), 'events'))
    if ctx.factors['permission'] == 'console':
        console = require_method(ctx.server, 'console', 'server:console_result')
        ctx.state['save_output'] = console('save-all flush', 'Saved the game')
    else:
        actor.command('save-all flush')
        if ctx.state['negative']:
            def refused():
                events = required(actor.snapshot(), 'events')[event_offset:]
                return events if any(re.search(r'(?i)(unknown or incomplete command|command.unknown)', str(event)) for event in events) else None
            ctx.state['refusal_events'] = ctx.wait_until(refused, description='actor save-all permission refusal')
            ctx.state['save_output'] = log_path.read_text(errors='replace')[offset:]
        else:
            def saved():
                fresh = log_path.read_text(errors='replace')[offset:]
                return 'Saved the game' in fresh, fresh
            ctx.state['save_output'] = runtime.wait_for_path_condition(log_path, 30, saved)
    ctx.boundary()
    x, y, z = ctx.state['marker']
    token = 'matrix-save-block-' + ctx.row['id']
    ctx.state['block_token'] = token
    ctx.state['block_output'] = command(ctx, f'execute if block {x} {y} {z} minecraft:gold_block run say {token}')
    ctx.state['score_output'] = ctx.server.query('scoreboard players get sentinel lc_save')
    def progressed():
        stats = ctx.server.dashboard_stats()
        return stats if required(stats, 'simulation_tick') > ctx.state['before_tick'] else None
    ctx.state['after_stats'] = ctx.wait_until(progressed, description='ticks advance after checkpoint')
    ctx.state['chat_events'] = _cohort_chat(ctx, ctx.state['chat_token'])


def _save_oracle(ctx: MatrixContext) -> list[Check]:
    output = ctx.state['save_output']
    if ctx.state['negative']:
        checks = [
            Check('save.permission_root_hidden', 'save-all' not in ctx.state['command_roots']),
            Check('save.permission_refused', bool(ctx.state['refusal_events']), ctx.state['refusal_events']),
            Check('save.no_authorized_save', not re.search(r'(Saving the game|Saved the game)', output), output),
        ]
    else:
        checks = [Check('save.explicit_completion',
                        bool(re.search(r'(?i)(saved the game|saved game|commands\.save\.success)', output)), output)]
    checks.extend([
        Check('save.block_preserved', bool(re.search(r'(?m)^(?:chat.type.announcement: Server, |\[Server\] )' + re.escape(ctx.state['block_token']) + r'\s*$', ctx.state['block_output'])), ctx.state['block_output']),
        Check('save.score_preserved', bool(re.search(r'\b17\b', ctx.state['score_output'])), ctx.state['score_output']),
        Check('save.ticks_continue', ctx.state['after_stats']['simulation_tick'] > ctx.state['before_tick']),
        Check('save.client_progress', set(ctx.state['chat_events']) == {c.name for c in ctx.clients}
              and all(ctx.state['chat_events'].values()), ctx.state['chat_events']),
    ])
    return checks


_PLAYER_FIELDS = ('Inventory', 'EnderItems', 'XpLevel', 'XpTotal', 'XpP',
                  'active_effects', 'respawn', 'playerGameType')


def _player_setup(ctx: MatrixContext) -> None:
    require_method(ctx.server, 'clean_restart', 'server:clean_restart')
    require_method(ctx.clients[0], 'close_container', 'client:close_container')
    if ctx.factors['server_lifecycle'] == 'hot_backup_during_action':
        raise MissingCapability('server:restored_player_data')
    if ctx.factors['game_mode'] == 'spectator':
        raise MissingCapability('fixture:preexisting_ender_inventory_for_spectator')
    actor = ctx.clients[0]
    # Probe implemented NBT before constructing the fixture.
    player_data(ctx, ('Inventory',))
    x, y, z = (math.floor(value) for value in required(actor.snapshot(), 'position'))
    ctx.state['ender'] = (x + 2, y, z)
    command(ctx, f'setblock {x + 2} {y} {z} minecraft:ender_chest')
    command(ctx, f'clear {actor.name}')
    command(ctx, f'give {actor.name} minecraft:diamond 7')
    command(ctx, f'experience set {actor.name} 5 levels')
    command(ctx, f'effect give {actor.name} minecraft:speed infinite 0')
    command(ctx, f'spawnpoint {actor.name} {x} {y} {z}')
    if ctx.factors['server_lifecycle'] == 'crash_kill9_wal_replay':
        ctx.wait_until(lambda: inventory_count(actor.snapshot(), 'minecraft:diamond') == 7,
                       description='received pre-experiment custody baseline')
        ctx.state['player_baseline'] = player_data(ctx, _PLAYER_FIELDS)
        if ctx.state['player_baseline']['EnderItems'] != '[]':
            raise RuntimeError('crash custody experiment requires an empty baseline ender chest')
        # Seal only the fixture baseline. Never wait for publication of the
        # subsequent transfer/grant before the mid-action SIGKILL boundary.
        ctx.state['baseline_save'] = ctx.server.save_all()


def _player_act(ctx: MatrixContext) -> None:
    actor = ctx.clients[0]
    actor.open_container(ctx.state['ender'])
    menu = required(actor.snapshot(), 'menu')
    slots = required(menu, 'slots')
    candidates = [stack['slot'] for stack in slots
                  if stack.get('item_id') == 'minecraft:diamond' and stack.get('slot', -1) >= 27]
    if len(candidates) != 1:
        raise RuntimeError('expected one seven-diamond player stack in ender menu')
    actor.click_slot(candidates[0], kind='quick_move')
    def stored():
        current = required(required(actor.snapshot(), 'menu'), 'slots')
        return sum(s['count'] for s in current if s.get('slot', 27) < 27 and s.get('item_id') == 'minecraft:diamond') == 7
    ctx.wait_until(stored, description='seven diamonds authoritatively received in ender chest')
    require_method(actor, 'close_container', 'client:close_container')()
    command(ctx, f'give {actor.name} minecraft:emerald 3')
    if ctx.factors['server_lifecycle'] != 'crash_kill9_wal_replay':
        ctx.state['player_before'] = player_data(ctx, _PLAYER_FIELDS)
    ctx.boundary()
    if ctx.factors['server_lifecycle'] != 'crash_kill9_wal_replay':
        ctx.server.save_all()
        for client in ctx.clients:
            client.disconnect()
        ctx.state['restart'] = require_method(ctx.server, 'clean_restart', 'server:clean_restart')()
        for client in ctx.clients:
            client.reconnect()
    ctx.state['player_after'] = player_data(ctx, _PLAYER_FIELDS)
    actor.open_container(ctx.state['ender'])
    ctx.state['restored_ender_menu'] = required(actor.snapshot(), 'menu')
    if ctx.factors['server_lifecycle'] == 'crash_kill9_wal_replay':
        snapshot = actor.snapshot()
        if 'cursor' not in snapshot:
            raise MissingCapability('observe:cursor')
        ctx.state['restored_cursor'] = snapshot['cursor']


def _player_oracle(ctx: MatrixContext) -> list[Check]:
    if ctx.factors['server_lifecycle'] == 'crash_kill9_wal_replay':
        return _player_crash_oracle(ctx)
    checks = [Check('player_roundtrip.' + field,
                    ctx.state['player_before'][field] == ctx.state['player_after'][field],
                    {'before': ctx.state['player_before'][field], 'after': ctx.state['player_after'][field]})
              for field in _PLAYER_FIELDS]
    # The received menu already contains the player-inventory suffix; do not
    # double-count that suffix through snapshot.inventory.
    diamonds = sum(stack['count'] for stack in required(ctx.state['restored_ender_menu'], 'slots')
                   if stack.get('item_id') == 'minecraft:diamond')
    checks.append(Check('player_roundtrip.diamond_custody', diamonds == 7, diamonds))
    return checks


def _player_crash_oracle(ctx: MatrixContext) -> list[Check]:
    """Recovered ordered prefix, not latest-ACK durability.

    A durable-publication survival proof is separate. This mid-action proof
    accepts losing unpublished transfer/grant mutations, but never torn custody
    or a later grant recovered without its preceding transfer.
    """
    stable_fields = tuple(field for field in _PLAYER_FIELDS if field not in {'Inventory', 'EnderItems'})
    checks = [Check('player_crash.baseline.' + field,
                    ctx.state['player_after'][field] == ctx.state['player_baseline'][field],
                    {'baseline': ctx.state['player_baseline'][field], 'recovered': ctx.state['player_after'][field]})
              for field in stable_fields]
    stacks = required(ctx.state['restored_ender_menu'], 'slots')
    inventory = sum(stack['count'] for stack in stacks
                    if stack['slot'] >= 27 and stack.get('item_id') == 'minecraft:diamond')
    ender = sum(stack['count'] for stack in stacks
                if stack['slot'] < 27 and stack.get('item_id') == 'minecraft:diamond')
    emeralds = sum(stack['count'] for stack in stacks if stack.get('item_id') == 'minecraft:emerald')
    # Setup checkpoint -> one seven-diamond transfer -> one three-emerald grant.
    prefix = (inventory, ender, emeralds)
    checks.extend([
        Check('player_crash.atomic_ordered_prefix', prefix in {(7, 0, 0), (0, 7, 0), (0, 7, 3)},
              {'inventory_diamonds': inventory, 'ender_diamonds': ender, 'emeralds': emeralds}),
        Check('player_crash.diamond_custody', inventory + ender == 7, inventory + ender),
        Check('player_crash.cursor_empty', ctx.state['restored_cursor'] is None, ctx.state['restored_cursor']),
        Check('player_crash.no_unexpected_items',
              all(stack.get('item_id') in {'minecraft:diamond', 'minecraft:emerald'} for stack in stacks), stacks),
    ])
    return checks


ADAPTERS = {
    'persistence.player_data_roundtrip': Adapter(_player_setup, _player_act, _player_oracle),
    'persistence.block_entities_roundtrip': unsupported('data_get_block'),
    'persistence.entities_roundtrip': unsupported('data_get_entity'),
    'persistence.world_metadata_roundtrip': unsupported('observe:raid_dragon_metadata'),
    'persistence.corrupt_store_fail_closed': unsupported('server:corrupt_store_fixture'),
    'persistence.backup_restore': unsupported('observe:restored_initial_tick'),
    'persistence.save_all_flush': Adapter(_save_setup, _save_act, _save_oracle),
}
