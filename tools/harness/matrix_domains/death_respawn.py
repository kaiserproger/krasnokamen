"""Vanilla and deployed-graves death lanes share the same death actions.

The canonical profile supplies enabled_plugins from the actual copied package
manifests. A grave lane never silently reuses the vanilla ground-drop oracle.
"""
from collections import Counter
import json
from functools import partial

from ..matrix_api import MissingCapability
from ..protocol_concurrency import submit_click_and_break, committed_destroy_records, verify_destroy_tick_fence
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
    # This collision must be outside basic-economy's protected -8..8 zone.
    x, z = (12, 12) if ctx.row.get('origin') == 'triple:dupe:destroy_during_interaction' else (4, 4)
    ctx.state['death_cell'] = [x, 180, z]
    command(ctx, f'tp {client(ctx).name} {x + .5} 180 {z + .5}')


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
    grave = tuple(ctx.state['death_cell'])
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
        check(ctx, 'keep_inventory_no_grave', block_is(ctx, grave, 'barrel'), False, source=source)
        check(ctx, 'keep_inventory_xp', number(ctx, client(ctx).name, 'XpTotal'), ctx.state['death_xp'], source=source)
        check(ctx, 'keep_inventory_levels', number(ctx, client(ctx).name, 'XpLevel'), ctx.state['death_level'], source=source)
    elif graves:
        check(ctx, 'grave_created_at_death', block_is(ctx, grave, 'barrel'), True, source=GRAVES)
        check(ctx, 'grave_no_ground_copy', custody(ground), Counter(), source=GRAVES)
        command(ctx, f'tp {client(ctx).name} {grave[0] - .5} {grave[1]} {grave[2] + .5}')
        verb(ctx, 'open_container', grave)
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


def destroy_census(stacks, item_ids):
    result = Counter()
    for stack in stacks:
        if stack is None:
            continue
        if (not isinstance(stack, dict) or not isinstance(stack.get('item_id'), str)
                or type(stack.get('count')) is not int or stack['count'] < 0):
            raise MissingCapability('snapshot:invalid_census_stack')
        if stack['count'] and stack['item_id'] in item_ids:
            if stack['item_id'] == 'minecraft:netherite_axe':
                components = stack.get('components')
                if not isinstance(components, dict):
                    raise MissingCapability('snapshot:stack:components')
                # Mining legitimately changes durability, not custody.
                stack = stack | {'components': {key: value for key, value in components.items()
                                                 if key != 'minecraft:damage'}}
            result[identity(stack)] += stack['count']
    return result


def destroy_grave_during_interaction(ctx):
    """Prove the live grave click/break collision, not just the death custody."""
    if not grave_lane(ctx) or ctx.factors['game_mode'] != 'survival':
        raise MissingCapability('destroy:survival_grave_fixture')
    actor = client(ctx)
    require_verbs(ctx, 'submit_input_batch')
    give(ctx, 'netherite_axe')
    select(ctx, 'netherite_axe')
    command(ctx, f'effect give {actor.name} minecraft:haste 120 100')
    ctx.wait_until(lambda: actor.snapshot()['raw']['player'].get('haste_amplifier') == 100,
                   description='received instant-mining haste')
    snapshot = actor.snapshot()
    player = snapshot['raw']['player']
    if (player.get('game_mode') != 'survival' or player.get('on_ground') is not True
            or player.get('under_water') is not False or 'mining_fatigue_amplifier' in player):
        raise MissingCapability('destroy:instant_mining_player_state')
    selected = snapshot.get('selected_hotbar')
    if type(selected) is not int or not any(
            stack['slot'] == selected + 36 and stack['item_id'] == 'minecraft:netherite_axe'
            for stack in snapshot['inventory']):
        raise MissingCapability('destroy:axe_not_selected')

    data = ctx.server.vanilla_data
    blocks = json.loads((data / 'reports/blocks.json').read_text())
    mining = json.loads((data / 'reports/block_mining.json').read_text())
    tool = json.loads((data.parent / '26.1.2/reports/minecraft/components/item/netherite_axe.json').read_text())
    tag = json.loads((data.parent / '26.1.2/data/minecraft/tags/block/mineable/axe.json').read_text())
    server_tag = json.loads((data / 'data/minecraft/tags/block/mineable/axe.json').read_text())
    if 'minecraft:barrel' not in tag['values'] or 'minecraft:barrel' not in server_tag['values']:
        raise MissingCapability('destroy:axe_not_effective_for_barrel')
    speeds = [rule['speed'] for rule in tool['components']['minecraft:tool']['rules']
              if rule.get('blocks') == '#minecraft:mineable/axe' and 'speed' in rule]
    server_tool = json.loads((data / 'reports/minecraft/components/item/netherite_axe.json').read_text())
    server_speeds = [rule['speed'] for rule in server_tool['components']['minecraft:tool']['rules']
                     if rule.get('blocks') == '#minecraft:mineable/axe' and 'speed' in rule]
    if speeds != server_speeds:
        raise MissingCapability('destroy:client_server_tool_speed_mismatch')
    facts = {tuple(mining['entries'][state['id']]) for state in blocks['minecraft:barrel']['states']}
    if len(facts) != 1:
        raise MissingCapability('destroy:barrel_state_mining_facts_differ')
    hardness, requires_tool = next(iter(facts))
    if len(speeds) != 1 or hardness <= 0 or requires_tool:
        raise MissingCapability('destroy:unsupported_mining_facts')
    rate = speeds[0] * (1 + .2 * (player['haste_amplifier'] + 1)) / hardness / 30
    if rate < 1:
        raise MissingCapability('destroy:not_instant_mining')

    grave = tuple(ctx.state['death_cell'])
    current_menu = snapshot.get('menu')
    if not current_menu or not isinstance(snapshot.get('cursor'), dict) or snapshot['cursor'].get('count') != 0:
        raise MissingCapability('destroy:grave_menu_or_cursor')
    contents = [stack for stack in current_menu['slots'] if stack and stack['slot'] < 27]
    if not contents:
        raise MissingCapability('destroy:empty_grave')
    ids = {stack['item_id'] for stack in snapshot['inventory'] + contents if stack.get('count')}
    if 'minecraft:barrel' in ids:
        raise MissingCapability('destroy:barrel_item_in_baseline')
    # Ambient terrain drops (e.g. kelp popping far below the arena) keep
    # appearing, so like assert_death_custody the ground counts only fixture
    # item types; none may be on the ground before the batch.
    before_entities = entities(ctx, 'item')
    if any(not isinstance(entry.get('item'), dict) for entry in before_entities):
        raise MissingCapability('snapshot:dropped_item_stack')
    if any(entry['item']['item_id'] in ids and entry['item'].get('count', 0) for entry in before_entities):
        raise MissingCapability('destroy:preexisting_fixture_drops')
    baseline = destroy_census(snapshot['inventory'] + contents, ids)
    slot = contents[0]['slot']
    receipt = submit_click_and_break(ctx, actor, slot, grave)
    inputs = receipt['inputs']
    records = committed_destroy_records(ctx, inputs['click'][0], actor.name, grave)
    fence = verify_destroy_tick_fence(inputs, records['block'].get('tick'), records['container'].get('tick'))
    block_change = records['block']['change']['Block']
    barrel_states = {state['id'] for state in blocks['minecraft:barrel']['states']}
    air_states = {state['id'] for state in blocks['minecraft:air']['states']}
    if block_change['before'] not in barrel_states or block_change['after'] not in air_states:
        raise MissingCapability('block_log:not_barrel_removal')
    changes = records['container']['change']['Container']['slots']
    image = records['block']['change']['Block'].get('entity')
    if not image or image.get('slots') is None:
        raise MissingCapability('block_log:missing_grave_preimage')
    matching = [change for change in changes if change[0] == slot]
    if (len(matching) != 1 or image['slots'][slot] != matching[0][2]
            or matching[0][1]['count'] != contents[0]['count'] or matching[0][2]['count'] != 0):
        raise MissingCapability('block_log:grave_preimage_mismatch')
    expected_slots = {stack['slot']: stack['count'] for stack in contents}
    expected_slots[slot] = 0
    if len(image['slots']) != 27 or any(
            stack['count'] != expected_slots.get(index, 0)
            for index, stack in enumerate(image['slots'])):
        raise MissingCapability('block_log:grave_preimage_contents_mismatch')
    ctx.wait_until(lambda: block_is(ctx, grave, 'air'), description='grave destruction committed')
    ctx.wait_until(lambda: not (actor.snapshot()['raw'].get('screen') or {}).get('open'),
                   description='destroyed grave menu closed')
    ticks(ctx, 40)
    after = actor.snapshot()
    dropped = [entry for entry in after['entities'] if entry['type'] == 'minecraft:item']
    if any(not isinstance(entry.get('item'), dict) for entry in dropped):
        raise MissingCapability('snapshot:dropped_item_stack')
    # Ambient ground drops of other types are out of scope; a new item type can
    # only enter custody through the inventory. The holder block's own barrel
    # drop was not a container item.
    inventory_ids = {stack['item_id'] for stack in after['inventory'] if stack and stack.get('count')}
    inventory_ids.discard('minecraft:barrel')
    ground = [entry['item'] for entry in dropped]
    observed = destroy_census(after['inventory'] + ground, ids | inventory_ids)
    condition(ctx, 'destroy_grave_same_tick', True,
              {'tick': fence, 'inputs': inputs, 'block_log': records, 'client_batch': receipt['client_batch'],
               'mining_rate': rate, 'hardness': hardness, 'tool_speed': speeds[0],
               'haste_amplifier': player['haste_amplifier'], 'player_mode': player['game_mode'],
               'on_ground': player['on_ground'], 'under_water': player['under_water']},
              source='local:crates/mc-core/src/tick/mining.rs')
    check(ctx, 'destroy_grave_census', dict(observed), dict(baseline), source=GRAVES)
    ctx.state['composite_origin_verified'] = ctx.row['origin']


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
    if (not keep and ctx.row.get('origin') == 'triple:dupe:destroy_during_interaction'
            and all(check['passed'] for check in ctx.state['checks'])):
        destroy_grave_during_interaction(ctx)


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
