"""World-border state and custody probes, Java 26.1.2.
Reference: https://minecraft.wiki/w/World_border (26.1.2 uses tick-based lerp).
The matrix's teleport subject name is misleading: /tp may cross a border.
"""
import re
from functools import partial

from ..matrix_api import Adapter, Check, MissingCapability
from .commands import actor, arena, check_block, field, fixture, inventory_count, query, ticks
from .chat import verb


def issue(ctx, command):
    # Border controls are operator world fixtures; actor permissions govern
    # interaction/pearl inputs, and are separately tested by commands.worldborder.
    ctx.state['command_reply'] = fixture(ctx, command)
    if not ctx.state.get('action_boundary_called'):
        ctx.state['action_boundary_called'] = True
        ctx.boundary()


def size(ctx):
    reply = query(ctx, 'worldborder get')
    match = re.search(r'(?:border is|border.*?size[^0-9]*)\s*([0-9]+(?:\.[0-9]+)?)', reply)
    if not match:
        raise MissingCapability('worldborder_get:parseable_result')
    return float(match.group(1))


def setup(ctx, subject):
    arena(ctx)
    fixture(ctx, 'worldborder center 0 0')
    fixture(ctx, 'worldborder set 100')
    if subject == 'center':
        fixture(ctx, 'worldborder set 20')
        fixture(ctx, 'worldborder damage buffer 0')
        fixture(ctx, f'tp {actor(ctx).name} -8 80 0')
    if subject == 'block_actions_outside':
        fixture(ctx, 'setblock 3 80 0 stone')
        fixture(ctx, 'worldborder set 4')
        fixture(ctx, f'clear {actor(ctx).name}')
        fixture(ctx, f'give {actor(ctx).name} dirt 8')
        ctx.wait_until(lambda: inventory_count(actor(ctx), 'dirt') == 8, description='received dirt fixture')
        slots = [s['slot'] for s in field(actor(ctx), 'inventory')
                 if s['item_id'] == 'minecraft:dirt' and 36 <= s['slot'] <= 44]
        if not slots:
            raise MissingCapability('fixture_hotbar_item:dirt')
        actor(ctx).select_hotbar(slots[0] - 36)
    ctx.state['health_before'] = field(actor(ctx), 'health')


def act(ctx, subject):
    p = actor(ctx)
    if subject == 'set_and_lerp':
        issue(ctx, 'worldborder set 50 200t')
        ticks(ctx, 100)
        ctx.state['middle_size'] = size(ctx)
        ticks(ctx, 110)
    elif subject == 'center':
        issue(ctx, 'worldborder center 8 9')
        ticks(ctx, 20)
    elif subject == 'damage_buffer_warning':
        for command in ('worldborder set 20', 'worldborder damage buffer 0',
                        'worldborder damage amount 1', 'worldborder warning distance 7',
                        'worldborder warning time 3'):
            issue(ctx, command)
        fixture(ctx, 'worldborder damage buffer 3')
        fixture(ctx, f'tp {p.name} 12 80 0')
        ctx.state['buffer_health_before'] = field(p, 'health')
        ticks(ctx, 20)
        ctx.state['buffer_health_after'] = field(p, 'health')
        fixture(ctx, 'worldborder damage buffer 0')
        ticks(ctx, 20)
    elif subject == 'block_actions_outside':
        p.look_at(3.5, 80.5, .5)
        token = verb(p, 'break_block', (3, 80, 0))
        if token is None:
            raise MissingCapability('block_ack_sequence')
        ctx.wait_until(lambda: field(p, 'block_ack_sequence') >= token,
                       description='outside-border break acknowledgement')
        ctx.boundary()
        token = p.use_item_on((3, 80, 0), face='up')
        if token is None:
            raise MissingCapability('block_ack_sequence')
        ctx.wait_until(lambda: field(p, 'block_ack_sequence') >= token,
                       description='outside-border placement acknowledgement')
        ticks(ctx, 2)
    elif subject == 'teleport_outside_clamped':
        issue(ctx, 'worldborder set 20')
        fixture(ctx, 'worldborder damage buffer 0')
        issue(ctx, f'tp {p.name} 15 80 0')
        ctx.state['tp_position'] = field(p, 'position')
        ticks(ctx, 20)
        ctx.state['tp_health_after'] = field(p, 'health')
        # Projectiles hit the boundary in 26.1.2; use_item is required to test
        # the separate pearl route, never infer it from /tp.
        fixture(ctx, f'tp {p.name} 8 80 0')
        fixture(ctx, f'give {p.name} ender_pearl 1')
        ctx.wait_until(lambda: inventory_count(p, 'ender_pearl') >= 1, description='received pearl fixture')
        items = field(p, 'inventory')
        slots = [s['slot'] for s in items if s['item_id'] == 'minecraft:ender_pearl' and 36 <= s['slot'] <= 44]
        if not slots:
            raise MissingCapability('fixture_hotbar_item:ender_pearl')
        p.select_hotbar(slots[0] - 36)
        p.look_at(15, 80, 0)
        verb(p, 'use_item')
        ticks(ctx, 40)
    elif subject == 'border_persistence':
        for command in ('worldborder center 8 9', 'worldborder set 50',
                        'worldborder warning distance 7', 'worldborder damage buffer 2'):
            issue(ctx, command)
        ctx.server.save_all()
        method = getattr(ctx.server, 'clean_restart', None)
        if method is None:
            raise MissingCapability('server_clean_restart')
        for client in ctx.clients:
            client.disconnect()
        method()
        for client in ctx.clients:
            client.reconnect()
        border = field(p, 'world_border')
        if 'damage_buffer' not in border:
            raise MissingCapability('snapshot:world_border.damage_buffer')
        ctx.state['static_border'] = border
        ctx.state['static_size'] = size(ctx)
        issue(ctx, 'worldborder set 100 2400t')
        ticks(ctx, 40)
        border = field(p, 'world_border')
        for key in ('target_size', 'remaining_ticks'):
            if key not in border:
                raise MissingCapability('snapshot:world_border.' + key)
        ctx.state['lerp_before_restart'] = border
        ctx.server.save_all()
        for client in ctx.clients:
            client.disconnect()
        method()
        for client in ctx.clients:
            client.reconnect()


def oracle(ctx, subject):
    p = actor(ctx)
    if subject == 'set_and_lerp':
        checks = [Check('intermediate_size', 70 <= ctx.state['middle_size'] <= 80, ctx.state['middle_size']),
                  Check('terminal_size', size(ctx) == 50, size(ctx))]
        if not all(check.passed for check in checks):
            return checks
        border = field(p, 'world_border')
        if 'target_size' not in border:
            raise MissingCapability('snapshot:world_border.target_size')
        checks.append(Check('border_lerp_packet_target', border['target_size'] == 50, border))
        return checks
    if subject == 'block_actions_outside':
        return [check_block(ctx, (3, 80, 0), 'stone'), check_block(ctx, (3, 81, 0), 'air'),
                Check('refused_placement_preserves_inventory', inventory_count(p, 'dirt') == 8, field(p, 'inventory'))]
    if subject == 'teleport_outside_clamped':
        position = field(p, 'position')
        return [Check('command_tp_not_border_clamped', abs(ctx.state['tp_position'][0] - 15) < .01, ctx.state['tp_position']),
                Check('command_tp_outside_takes_border_damage',
                      (ctx.state['tp_health_after'] < ctx.state['health_before']) ==
                      (ctx.factors['game_mode'] in {'survival', 'adventure'}), ctx.state['tp_health_after']),
                Check('pearl_stops_at_border', 8 < position[0] <= 10, position),
                Check('pearl_consumption_respects_mode',
                      inventory_count(p, 'ender_pearl') == int(ctx.factors['game_mode'] == 'creative'),
                      field(p, 'inventory'))]
    border = field(p, 'world_border')
    if subject == 'center':
        vulnerable = ctx.factors['game_mode'] in {'survival', 'adventure'}
        return [Check('border_center_packet', border['center'] == [8, 9], border),
                Check('center_shift_outside_damage',
                      (field(p, 'health') < ctx.state['health_before']) == vulnerable, field(p, 'health'))]
    if subject == 'damage_buffer_warning':
        vulnerable = ctx.factors['game_mode'] in {'survival', 'adventure'}
        checks = [Check('outside_damage_respects_mode',
                      (field(p, 'health') < ctx.state['health_before']) == vulnerable,
                      {'before': ctx.state['health_before'], 'after': field(p, 'health')}),
                Check('warning_distance_packet', border['warning_distance'] == 7, border),
                Check('warning_time_packet', border['warning_time'] == 3, border),
                Check('buffer_prevents_damage', ctx.state['buffer_health_before'] == ctx.state['buffer_health_after'],
                      {'before': ctx.state['buffer_health_before'], 'after': ctx.state['buffer_health_after']})]
        if all(check.passed for check in checks):
            checks.append(Check('red_border_warning_visible', field(p, 'border_warning_visible') is True))
        return checks
    before = ctx.state['static_border']
    lerp = ctx.state['lerp_before_restart']
    for key in ('target_size', 'remaining_ticks'):
        if key not in border:
            raise MissingCapability('snapshot:world_border.' + key)
    return [Check('persisted_size', ctx.state['static_size'] == 50, ctx.state['static_size']),
            Check('persisted_center', before['center'] == [8, 9], before),
            Check('persisted_warning', before['warning_distance'] == 7, before),
            Check('persisted_damage_buffer', before['damage_buffer'] == 2, before),
            Check('persisted_lerp_target', border['target_size'] == lerp['target_size'] == 100, border),
            Check('persisted_lerp_remaining', 0 < border['remaining_ticks'] <= lerp['remaining_ticks'], border)]


ADAPTERS = {'world_border.' + s: Adapter(partial(setup, subject=s), partial(act, subject=s), partial(oracle, subject=s))
            for s in ('set_and_lerp', 'center', 'damage_buffer_warning', 'block_actions_outside',
                      'teleport_outside_clamped', 'border_persistence')}
