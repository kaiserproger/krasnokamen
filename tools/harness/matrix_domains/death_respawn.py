"""Vanilla and deployed-graves death lanes share the same death actions.

The canonical profile supplies enabled_plugins from the actual copied package
manifests. A grave lane never silently reuses the vanilla ground-drop oracle.
"""
from collections import Counter
import json
from functools import partial

from ..matrix_api import MissingCapability
from ._items import (
    adapter, block_is, check, client, command, condition, count, cursor, entities,
    give, health, inventory_slot, menu, menu_slot, number, observe, open_menu,
    player_menu_slot, put, query, require_verbs, select, setup, summon, survival,
    ticks, verb, WIKI,
)

GRAVES = 'local:scenarios/live-plugins/graves/README.md:11-28'


def grave_lane(ctx):
    plugins = ctx.state.get('enabled_plugins')
    if plugins is None:
        plugins = getattr(ctx.server, 'enabled_plugins', None)
    if plugins is None:
        raise MissingCapability('deployment:enabled_plugins')
    enabled = 'graves' in plugins
    profile = ctx.factors.get('plugin_profile')
    if profile in ('vanilla', 'live'):
        expected = profile == 'live'
        if enabled != expected:
            raise RuntimeError(f'{profile} death profile contradicts deployed graves={enabled}')
        return expected
    return enabled


def prepare(ctx):
    setup(ctx)
    require_verbs(ctx, 'respawn')
    command(ctx, 'gamerule immediate_respawn false')
    command(ctx, 'gamerule keep_inventory false')
    command(ctx, 'gamerule respawn_radius 0')
    command(ctx, 'execute in minecraft:overworld run fill -2 179 -2 2 179 2 stone')
    command(ctx, 'execute in minecraft:overworld run setworldspawn 0 180 0')
    command(ctx, f'tp {client(ctx).name} 4.5 180 4.5')


def die(ctx):
    command(ctx, f'kill {client(ctx).name}')
    ctx.wait_until(lambda: health(ctx) <= 0, description='authoritative death health')
    check(ctx, 'player_dead', health(ctx), 0, source=WIKI + 'Death')


def respawn(ctx):
    verb(ctx, 'respawn')
    ctx.wait_until(lambda: health(ctx) > 0, description='respawn health received')
    check(ctx, 'respawn_full_health', health(ctx), 20, source=WIKI + 'Spawn')


def identity(stack):
    if 'components' not in stack:
        raise MissingCapability('snapshot:stack:components')
    return json.dumps([stack['item_id'], stack['components']], sort_keys=True, separators=(',', ':'))


def custody(stacks):
    total = Counter()
    for stack in stacks:
        if stack and stack.get('count', 0):
            total[identity(stack)] += stack['count']
    return total


def fixture(ctx, with_cursor=False):
    require_verbs(ctx, 'use_item', 'swap_hands')
    give(ctx, 'diamond', 11)
    give(ctx, 'iron_ingot', 7)
    for item in ('iron_boots', 'iron_leggings', 'iron_chestplate', 'iron_helmet'):
        give(ctx, item)
        select(ctx, item)
        verb(ctx, 'use_item', hand='main')
    give(ctx, 'torch', 13)
    select(ctx, 'torch')
    verb(ctx, 'swap_hands')
    command(ctx, f'experience set {client(ctx).name} 10 levels')
    ctx.wait_until(lambda: number(ctx, client(ctx).name, 'XpLevel') == 10,
                   description='received death fixture experience level')
    if with_cursor:
        give(ctx, 'emerald', 5)
        # Keep the open chest near the death cell so the menu stays in reach.
        command(ctx, 'setblock 5 180 4 chest')
        verb(ctx, 'open_container', (5, 180, 4))
        slot = player_menu_slot(ctx, inventory_slot(ctx, 'emerald'))
        verb(ctx, 'click_slot', slot)
    all_stacks = list(observe(ctx, 'inventory'))
    if with_cursor and cursor(ctx)['count']:
        all_stacks.append(cursor(ctx))
    ctx.state['death_custody'] = custody(all_stacks)
    ctx.state['death_xp'] = number(ctx, client(ctx).name, 'XpTotal')
    ctx.state['death_level'] = number(ctx, client(ctx).name, 'XpLevel')


def assert_death_custody(ctx, keep=False):
    expected = ctx.state['death_custody']
    # Ambient terrain drops (e.g. kelp far below the arena) are not copies of
    # this player's fixture. Retain component-sensitive custody for every
    # matching item type so a renamed/damaged duplicate is still detected.
    fixture_items = {json.loads(key)[0] for key in expected}
    ground = [e['item'] for e in entities(ctx, 'item') if e['item']['item_id'] in fixture_items]
    graves = grave_lane(ctx)
    source = GRAVES if graves else WIKI + 'Death'
    if keep:
        check(ctx, 'keep_inventory_complete_custody', custody(observe(ctx, 'inventory')), expected, source=source)
        check(ctx, 'keep_inventory_no_ground_copy', custody(ground), Counter(), source=source)
        check(ctx, 'keep_inventory_no_grave', block_is(ctx, (4, 180, 4), 'barrel'), False, source=source)
        check(ctx, 'keep_inventory_xp', number(ctx, client(ctx).name, 'XpTotal'), ctx.state['death_xp'], source=source)
        check(ctx, 'keep_inventory_levels', number(ctx, client(ctx).name, 'XpLevel'), ctx.state['death_level'], source=source)
    elif graves:
        check(ctx, 'grave_created_at_death', block_is(ctx, (4, 180, 4), 'barrel'), True, source=GRAVES)
        check(ctx, 'grave_no_ground_copy', custody(ground), Counter(), source=GRAVES)
        command(ctx, f'tp {client(ctx).name} 3.5 180 4.5')
        verb(ctx, 'open_container', (4, 180, 4))
        ctx.wait_until(lambda: client(ctx).snapshot().get('menu'), description='grave barrel menu')
        slots = menu(ctx)['slots']
        grave_contents = [s for s in slots if s and s['slot'] < 27]
        check(ctx, 'grave_exact_inventory_armor_offhand_cursor', custody(grave_contents), expected, source=GRAVES)
        check(ctx, 'grave_player_empty', custody(observe(ctx, 'inventory')), Counter(), source=GRAVES)
        check(ctx, 'grave_retains_xp', number(ctx, client(ctx).name, 'XpTotal'), ctx.state['death_xp'], source=GRAVES)
        check(ctx, 'grave_retains_levels', number(ctx, client(ctx).name, 'XpLevel'), ctx.state['death_level'], source=GRAVES)
        check(ctx, 'grave_no_xp_orbs', len(entities(ctx, 'experience_orb')), 0, source=GRAVES)
    else:
        check(ctx, 'vanilla_exact_ground_custody', custody(ground), expected, source=WIKI + 'Death')
        check(ctx, 'vanilla_player_empty', custody(observe(ctx, 'inventory')), Counter(), source=WIKI + 'Death')
        check(ctx, 'vanilla_xp_reset', number(ctx, client(ctx).name, 'XpTotal'), 0, source=WIKI + 'Experience')
        check(ctx, 'vanilla_level_reset', number(ctx, client(ctx).name, 'XpLevel'), 0, source=WIKI + 'Experience')
        orbs = entities(ctx, 'experience_orb')
        if any('value' not in e or 'count' not in e for e in orbs):
            raise MissingCapability('snapshot:experience_orb:value_count')
        check(ctx, 'vanilla_death_xp_7_times_level', sum(e['value'] * e['count'] for e in orbs), 70, source=WIKI + 'Experience')


def screen(ctx):
    die(ctx)
    check(ctx, 'death_screen_shown', observe(ctx, 'screen'), 'death', source=WIKI + 'Death')
    condition(ctx, 'death_message_present', bool(observe(ctx, 'death_message')), observe(ctx, 'death_message'), source=WIKI + 'Death_messages')
    respawn(ctx)
    position = observe(ctx, 'position')
    condition(ctx, 'respawn_at_world_spawn', abs(position[0] - .5) <= 1 and abs(position[2] - .5) <= 1, position, source=WIKI + 'Spawn')


def keep_inventory(ctx, keep=False):
    grave_lane(ctx)
    command(ctx, 'gamerule keep_inventory ' + str(keep).lower())
    fixture(ctx)
    die(ctx)
    respawn(ctx)
    assert_death_custody(ctx, keep)


def menu_death(ctx):
    grave_lane(ctx)
    fixture(ctx, with_cursor=True)
    die(ctx)
    check(ctx, 'death_closes_menu', client(ctx).snapshot().get('menu'), None, source=WIKI + 'Death')
    respawn(ctx)
    check(ctx, 'death_clears_cursor_once', cursor(ctx)['count'], 0, source=WIKI + 'Death')
    assert_death_custody(ctx)


def bed(ctx):
    if ctx.factors['dimension'] != 'overworld':
        raise MissingCapability('scenario_variant:bed_spawn:wrong_dimension')
    command(ctx, 'setblock 8 180 8 white_bed[part=foot,facing=south]')
    command(ctx, 'setblock 8 180 9 white_bed[part=head,facing=south]')
    command(ctx, f'tp {client(ctx).name} 7.5 180 8.5')
    verb(ctx, 'use_item_on', (8, 180, 8), face='up')
    command(ctx, f'tp {client(ctx).name} 4.5 180 4.5')
    die(ctx)
    respawn(ctx)
    p = observe(ctx, 'position')
    condition(ctx, 'respawn_near_bed', abs(p[0] - 8.5) <= 2 and abs(p[2] - 8.5) <= 2, p, source=WIKI + 'Bed')
    command(ctx, 'setblock 8 180 8 air')
    command(ctx, 'setblock 8 180 9 air')
    die(ctx)
    respawn(ctx)
    p = observe(ctx, 'position')
    condition(ctx, 'missing_bed_falls_back_world_spawn', abs(p[0] - .5) <= 1 and abs(p[2] - .5) <= 1, p, source=WIKI + 'Bed')
    condition(ctx, 'missing_bed_feedback', any('respawn' in str(e).lower() or 'bed' in str(e).lower() for e in observe(ctx, 'events')), observe(ctx, 'events'), source=WIKI + 'Bed')
    command(ctx, 'setblock 8 180 8 white_bed[part=foot,facing=south]')
    command(ctx, 'setblock 8 180 9 white_bed[part=head,facing=south]')
    command(ctx, f'tp {client(ctx).name} 7.5 180 8.5')
    verb(ctx, 'use_item_on', (8, 180, 8), face='up')
    command(ctx, f'tp {client(ctx).name} 4.5 180 4.5')
    command(ctx, 'fill 6 180 6 10 183 11 stone')
    # Preserve the bed itself while obstructing every neighbouring spawn cell.
    command(ctx, 'setblock 8 180 8 white_bed[part=foot,facing=south]')
    command(ctx, 'setblock 8 180 9 white_bed[part=head,facing=south]')
    start = len(observe(ctx, 'events'))
    die(ctx)
    respawn(ctx)
    p = observe(ctx, 'position')
    condition(ctx, 'obstructed_bed_falls_back_world_spawn', abs(p[0] - .5) <= 1 and abs(p[2] - .5) <= 1, p, source=WIKI + 'Bed')
    condition(ctx, 'obstructed_bed_feedback', any('respawn' in str(e).lower() or 'bed' in str(e).lower() for e in observe(ctx, 'events')[start:]), observe(ctx, 'events')[start:], source=WIKI + 'Bed')


def anchor(ctx):
    if ctx.factors['dimension'] != 'the_nether':
        raise MissingCapability('scenario_variant:anchor_spawn:requires_nether')
    command(ctx, 'setblock 8 180 8 respawn_anchor[charges=2]')
    command(ctx, f'tp {client(ctx).name} 7.5 180 8.5')
    verb(ctx, 'use_item_on', (8, 180, 8), face='up')
    command(ctx, f'tp {client(ctx).name} 4.5 180 4.5')
    die(ctx)
    respawn(ctx)
    check(ctx, 'anchor_charge_consumed', block_is(ctx, (8, 180, 8), 'respawn_anchor[charges=1]'), True, source=WIKI + 'Respawn_Anchor')
    check(ctx, 'anchor_respawn_dimension', observe(ctx, 'dimension'), 'minecraft:the_nether', source=WIKI + 'Respawn_Anchor')
    p = observe(ctx, 'position')
    condition(ctx, 'respawn_near_anchor', abs(p[0] - 8.5) <= 2 and abs(p[2] - 8.5) <= 2, p, source=WIKI + 'Respawn_Anchor')


def immediate(ctx):
    command(ctx, 'gamerule immediate_respawn true')
    start = len(observe(ctx, 'events'))
    command(ctx, f'kill {client(ctx).name}')
    ticks(ctx, 5)
    check(ctx, 'immediate_full_health', health(ctx), 20, source=WIKI + 'Game_rule')
    check(ctx, 'immediate_no_death_screen', observe(ctx, 'screen') == 'death', False, source=WIKI + 'Game_rule')
    condition(ctx, 'immediate_actual_respawn_event', any('respawn' in str(e).lower() for e in observe(ctx, 'events')[start:]), observe(ctx, 'events')[start:], source=WIKI + 'Game_rule')


def void(ctx):
    give(ctx, 'diamond', 11)
    command(ctx, f'tp {client(ctx).name} 0 -200 0')
    ctx.wait_until(lambda: health(ctx) <= 0, description='void kills player', timeout_seconds=30)
    check(ctx, 'void_death', health(ctx), 0, source=WIKI + 'Void')
    respawn(ctx)
    check(ctx, 'void_no_retained_diamond', count(ctx, 'diamond'), 0, source=WIKI + 'Void')
    check(ctx, 'void_drops_not_recovered_at_spawn', any(e['item']['item_id'] == 'minecraft:diamond' for e in entities(ctx, 'item')), False, source=WIKI + 'Void')


def other_dimension(ctx):
    if ctx.factors['dimension'] == 'overworld':
        raise MissingCapability('scenario_variant:death_other_dimension')
    die(ctx)
    respawn(ctx)
    check(ctx, 'respawn_changes_to_overworld', observe(ctx, 'dimension'), 'minecraft:overworld', source=WIKI + 'Spawn')


def messages(ctx):
    for enabled in (True, False):
        command(ctx, 'gamerule show_death_messages ' + str(enabled).lower())
        before = [len(c.snapshot().get('chat', [])) for c in ctx.clients]
        die(ctx)
        for i, observer in enumerate(ctx.clients[1:], 1):
            chat = observer.snapshot().get('chat')
            if chat is None:
                raise MissingCapability('snapshot:chat')
            new = chat[before[i]:]
            check(ctx, 'death_message_broadcast_' + str(enabled), any(client(ctx).name in str(m) for m in new), enabled, source=WIKI + 'Death_messages')
        message = observe(ctx, 'death_message')
        if enabled:
            condition(ctx, 'kill_death_translation_key', 'death.attack.genericKill' in str(message), message, source='local:data/vanilla/data/minecraft/damage_type/generic_kill.json')
        respawn(ctx)


def riding(ctx):
    require_verbs(ctx, 'interact_entity')
    boat = summon(ctx, 'oak_boat', pos='4.5 180 4.5')
    verb(ctx, 'interact_entity', entity_id=boat, hand='main')
    check(ctx, 'boat_mount_fixture', observe(ctx, 'vehicle_id'), boat, source=WIKI + 'Boat')
    die(ctx)
    respawn(ctx)
    check(ctx, 'death_dismounts', client(ctx).snapshot().get('vehicle_id'), None, source=WIKI + 'Death')
    condition(ctx, 'vehicle_survives_death', any(e['id'] == boat for e in entities(ctx, 'oak_boat')), entities(ctx, 'oak_boat'), source=WIKI + 'Boat')


def pearls(ctx):
    require_verbs(ctx, 'use_item')
    for vanish in (True, False):
        command(ctx, 'gamerule ender_pearls_vanish_on_death ' + str(vanish).lower())
        give(ctx, 'ender_pearl')
        select(ctx, 'ender_pearl')
        client(ctx).look_at(4.5, 250, 4.5)
        verb(ctx, 'use_item', hand='main')
        condition(ctx, 'pearl_in_flight_fixture', bool(entities(ctx, 'ender_pearl')), entities(ctx, 'ender_pearl'), source=WIKI + 'Ender_Pearl')
        die(ctx)
        check(ctx, 'pearl_death_rule_' + str(vanish), bool(entities(ctx, 'ender_pearl')), not vanish, source=WIKI + 'Game_rule')
        respawn(ctx)
        command(ctx, 'kill @e[type=ender_pearl]')


def invulnerability(ctx):
    survival(ctx)
    if ctx.factors['difficulty'] == 'peaceful':
        raise MissingCapability('scenario_variant:respawn_invulnerability:peaceful')
    die(ctx)
    respawn(ctx)
    summon(ctx, 'zombie', pos='1.5 180 0.5')
    ticks(ctx, 40)
    check(ctx, 'respawn_initial_invulnerability', health(ctx), 20, source=WIKI + 'Spawn')
    ctx.wait_until(lambda: health(ctx) < 20, description='mob hit after spawn protection expires', timeout_seconds=15)
    condition(ctx, 'respawn_protection_expires', health(ctx) < 20, health(ctx), source=WIKI + 'Spawn')


ADAPTERS = {f'death_respawn.{name}': adapter(fn, prepare) for name, fn in {
    'death_screen_respawn': screen, 'keep_inventory_false': keep_inventory,
    'keep_inventory_true': partial(keep_inventory, keep=True), 'bed_spawn': bed,
    'anchor_spawn': anchor, 'immediate_respawn': immediate, 'void_death': void,
    'death_other_dimension': other_dimension, 'death_message': messages,
    'death_while_menu_open': menu_death, 'death_while_riding': riding,
    'ender_pearls_on_death': pearls, 'respawn_invulnerability': invulnerability,
}.items()}
