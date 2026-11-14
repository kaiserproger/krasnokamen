"""26.1.2 gamerule registration and behavioral checks.

Defaults/bounds: local GameRules javap, SHA256 of nested vanilla server jar
4723380bd2a0a0206719b50f2e390383afdaf82b0a76a0d573baf788e6aa3e86.
Behavior: https://minecraft.wiki/w/Game_rule . A registration-only roundtrip
NEVER passes a behavioral subject: missing controlled fixtures fail closed.
"""
import re
from dataclasses import asdict
from functools import partial

from ..matrix_api import Adapter, Check, MissingCapability
from .commands import actor, arena, check_block, check_entity, check_score, field, fixture, inventory_count, query, require_event_history, ticks
from .chat import verb


# Explicit 26.1.2 defaults, not inferred from the server under test.
DEFAULTS = dict.fromkeys((
    'advance_time', 'advance_weather', 'allow_entering_nether_using_portals',
    'block_drops', 'block_explosion_drop_decay', 'command_blocks_work',
    'command_block_output', 'drowning_damage', 'elytra_movement_check',
    'ender_pearls_vanish_on_death', 'entity_drops', 'fall_damage', 'fire_damage',
    'forgive_dead_players', 'freeze_damage', 'global_sound_events', 'locator_bar',
    'log_admin_commands', 'mob_drops', 'mob_explosion_drop_decay', 'mob_griefing',
    'natural_health_regeneration', 'player_movement_check', 'projectiles_can_break_blocks',
    'pvp', 'raids', 'send_command_feedback', 'show_advancement_messages',
    'show_death_messages', 'spawner_blocks_work', 'spawn_mobs', 'spawn_monsters',
    'spawn_patrols', 'spawn_phantoms', 'spawn_wandering_traders', 'spawn_wardens',
    'spectators_generate_chunks', 'spread_vines', 'tnt_explodes', 'water_source_conversion'), True)
DEFAULTS.update(dict.fromkeys(('immediate_respawn', 'keep_inventory', 'lava_source_conversion',
                              'limited_crafting', 'reduced_debug_info', 'tnt_explosion_drop_decay',
                              'universal_anger'), False))
DEFAULTS.update(fire_spread_radius_around_player=128, max_block_modifications=32768,
                max_command_forks=65536, max_command_sequence_length=65536,
                max_entity_cramming=24, max_snow_accumulation_height=1,
                players_nether_portal_creative_delay=0, players_nether_portal_default_delay=80,
                players_sleeping_percentage=100, random_tick_speed=3, respawn_radius=10)

# These are missing environment/control capabilities, not successful tests.
# Exact fixtures and machine-checkable expected state are in gamerules.yaml.
CONTROLLED_FIXTURES = {
    'block_explosion_drop_decay': 'controlled_explosion:block_drop_census',
    'mob_explosion_drop_decay': 'controlled_explosion:mob_drop_census',
    'tnt_explosion_drop_decay': 'controlled_explosion:tnt_drop_census',
    'elytra_movement_check': 'raw_move:elytra_overspeed',
    'player_movement_check': 'raw_move:ground_overspeed',
    'ender_pearls_vanish_on_death': 'owned_projectile:inflight_death',
    'forgive_dead_players': 'neutral_mob_anger:target_and_death',
    'universal_anger': 'neutral_mob_anger:second_player',
    'global_sound_events': 'sound_events:remote_boss_spawn',
    'log_admin_commands': 'server_log:admin_command_records',
    'limited_crafting': 'recipe_unlock_and_crafting:locked_recipe',
    'locator_bar': 'snapshot:waypoints',
    'max_snow_accumulation_height': 'controlled_snow_precipitation_tick',
    'mob_griefing': 'controlled_creeper_detonation',
    'raids': 'village_raid_trigger:raid_omen',
    'spawner_blocks_work': 'spawner_nbt_fixture:guaranteed_attempt',
    'spawn_mobs': 'controlled_natural_spawn_attempt',
    'spawn_monsters': 'controlled_monster_spawn_attempt',
    'spawn_patrols': 'controlled_patrol_spawn_attempt',
    'spawn_phantoms': 'player_insomnia_fixture:phantom_attempt',
    'spawn_wandering_traders': 'controlled_trader_spawn_attempt',
    'spawn_wardens': 'sculk_shrieker_warning_fixture',
    'spectators_generate_chunks': 'chunk_generation_census',
    'max_command_sequence_length': 'fixture_datapack:three_command_function',
    'command_block_output': 'admin_chat_observer:command_block_output',
    'respawn_radius': 'respawn_without_personal_spawn',
}


def value_for(rule):
    default = DEFAULTS[rule]
    if type(default) is bool:
        return not default
    return {'max_block_modifications': 1, 'max_command_forks': 1,
            'max_command_sequence_length': 1, 'max_snow_accumulation_height': 0,
            'players_nether_portal_creative_delay': 20,
            'players_nether_portal_default_delay': 20,
            'players_sleeping_percentage': 101}.get(rule, 0)


def literal(value):
    return str(value).lower()


def rule_query(ctx, rule, expected):
    reply = query(ctx, 'gamerule minecraft:' + rule)
    matched = re.search(r'(?<![\w.-])' + re.escape(literal(expected)) + r'(?![\w.-])', reply)
    return Check('gamerule_value:' + rule, matched is not None, {'expected': expected, 'reply': reply})


SPLIT_RULES = {'pvp', 'players_sleeping_percentage', 'universal_anger'}


def actor_only_row(ctx, rule):
    # Main explicitly approved these baseline registration/persistence rows;
    # their separate two-player row remains required for subject coverage.
    return (rule in SPLIT_RULES and str(ctx.factors.get('player_count')) == '1'
            and any(clause.startswith('player_count 1:') for clause in ctx.row.get('oracle', [])))


def persisted_rule(ctx, rule):
    restart = getattr(ctx.server, 'clean_restart', None)
    if not callable(restart):
        raise MissingCapability('server_clean_restart')
    ctx.server.save_all()
    for client in ctx.clients:
        client.disconnect()
    ctx.state['rule_persistence_restart'] = restart()
    for client in ctx.clients:
        client.reconnect()
    return rule_query(ctx, rule, value_for(rule))


def nearby_entities(ctx, kind, position, radius=.75):
    return [entity for entity in field(actor(ctx), 'entities')
            if entity['type'] == 'minecraft:' + kind and
            sum((a - b) ** 2 for a, b in zip(entity['position'], position)) <= radius ** 2]


def entity_health(entity):
    if 'health' not in entity:
        raise MissingCapability('snapshot:entities.health')
    return entity['health']


def prepare_bow(ctx):
    if ctx.factors['game_mode'] not in {'survival', 'adventure'}:
        raise MissingCapability('consuming_bow_actor')
    p = actor(ctx)
    fixture(ctx, f'clear {p.name}')
    fixture(ctx, f'give {p.name} bow 1')
    fixture(ctx, f'give {p.name} arrow 4')
    ctx.wait_until(lambda: inventory_count(p, 'bow') == 1 and inventory_count(p, 'arrow') == 4,
                   description='bow and four arrows received')
    slots = [item['slot'] for item in field(p, 'inventory')
             if item['item_id'] == 'minecraft:bow' and 36 <= item['slot'] <= 44]
    if len(slots) != 1:
        raise MissingCapability('fixture_hotbar_item:bow')
    p.select_hotbar(slots[0] - 36)


def shoot(ctx, target):
    p = actor(ctx)
    arrows = inventory_count(p, 'arrow')
    p.look_at(*target)
    verb(p, 'use_item', ticks=20)
    verb(p, 'release_use_item')
    ctx.wait_until(lambda: inventory_count(p, 'arrow') == arrows - 1,
                   description='charged bow release consumes actual arrow')
    ticks(ctx, 4)

def daytime(ctx):
    reply = query(ctx, 'time query daytime')
    match = re.search(r'(?:commands\.time\.query:\s*|The time is\s+)(\d+)', reply)
    if not match:
        raise MissingCapability('query:time.daytime.numeric_feedback')
    return int(match.group(1))


def bed_occupied(client, expected=True):
    state = verb(client, 'read_block_state', (2, 80, 1))
    return state['block_id'] == 'minecraft:red_bed' and state['properties'].get('occupied') == literal(expected)



def setup(ctx, rule):
    ctx.state['actor_only_row'] = actor_only_row(ctx, rule)
    if ctx.state['actor_only_row']:
        fixture(ctx, 'gamerule minecraft:' + rule + ' ' + literal(DEFAULTS[rule]))
        ctx.state['baseline_check'] = asdict(rule_query(ctx, rule, DEFAULTS[rule]))
        return
    if rule in {'send_command_feedback', 'show_advancement_messages', 'show_death_messages'}:
        require_event_history(ctx)
    arena(ctx)
    p = actor(ctx)
    # Explicit baseline, because a matrix row may inherit a lifecycle/context
    # fixture that legitimately changes defaults before this adapter starts.
    fixture(ctx, 'gamerule minecraft:' + rule + ' ' + literal(DEFAULTS[rule]))
    ctx.state['baseline_check'] = asdict(rule_query(ctx, rule, DEFAULTS[rule]))
    ctx.state['events_start'] = len(field(p, 'events'))
    ctx.state['health_start'] = field(p, 'health')
    ctx.state['observer_starts'] = [len(field(client, 'events')) for client in ctx.clients]
    if rule == 'max_command_forks':
        fixture(ctx, 'scoreboard objectives add matrix_rule dummy')
        fixture(ctx, 'scoreboard players set result matrix_rule 0')
        for x in (3, 4):
            fixture(ctx, f'summon pig {x} 80 0 {{NoAI:1b,CustomName:"MatrixFork"}}')
    if rule == 'max_entity_cramming':
        fixture(ctx, 'gamerule minecraft:max_entity_cramming 0')
        fixture(ctx, 'fill 2 79 -1 4 83 1 stone')
        fixture(ctx, 'fill 3 80 0 3 81 0 air')
        for _ in range(26):
            fixture(ctx, 'summon cow 3.5 80 .5 {NoAI:1b,Health:10f}')
        ctx.wait_until(lambda: len(nearby_entities(ctx, 'cow', (3.5, 80, .5))) == 26,
                       description='26 received cows in one-block cramming cell')
        ctx.state['cramming_health'] = {e['uuid']: entity_health(e)
                                       for e in nearby_entities(ctx, 'cow', (3.5, 80, .5))}
    if rule == 'pvp':
        if len(ctx.clients) < 2:
            raise MissingCapability('multi_client_2:pvp_victim')
        if ctx.factors['game_mode'] not in {'survival', 'adventure'}:
            raise MissingCapability('vulnerable_actor:pvp_victim')
        fixture(ctx, f'tp {ctx.clients[1].name} 2.5 80 .5')
        ctx.wait_until(lambda: len(nearby_entities(ctx, 'player', (2.5, 80, .5), .1)) == 1,
                       description='received unique nearby victim entity')
        ctx.state['victim_health'] = field(ctx.clients[1], 'health')
        ctx.state['victim_uuid'] = field(ctx.clients[1], 'uuid')
        fixture(ctx, 'gamerule minecraft:natural_health_regeneration false')
    if rule == 'projectiles_can_break_blocks':
        fixture(ctx, 'setblock 3 80 0 decorated_pot')
        prepare_bow(ctx)
    if rule == 'send_command_feedback':
        fixture(ctx, 'gamerule minecraft:advance_time false')
    if rule == 'players_sleeping_percentage':
        if len(ctx.clients) != 2 or ctx.factors['dimension'] != 'overworld':
            raise MissingCapability('two_overworld_clients:sleep_quorum')
        if ctx.factors['game_mode'] == 'spectator':
            raise MissingCapability('bed_capable_actor:sleep_quorum')
        fixture(ctx, 'gamerule minecraft:advance_time true')
        fixture(ctx, 'time set 13000')
        fixture(ctx, 'weather clear')
        fixture(ctx, f'clear {p.name}')
        fixture(ctx, f'tp {ctx.clients[1].name} 6.5 80 6.5')
        fixture(ctx, 'setblock 2 80 0 red_bed[part=foot,facing=south]')
        fixture(ctx, 'setblock 2 80 1 red_bed[part=head,facing=south]')
    if rule in {'block_drops', 'tnt_explodes'}:
        fixture(ctx, 'setblock 3 80 0 stone')
    if rule in {'mob_drops', 'entity_drops'}:
        kind = 'cow' if rule == 'mob_drops' else 'oak_boat'
        nbt = '{CustomName:"MatrixDrop",NoAI:1b,Health:1f}' if rule == 'mob_drops' else '{CustomName:"MatrixDrop"}'
        fixture(ctx, f'summon {kind} 3 80 0 {nbt}')
    if rule == 'keep_inventory':
        fixture(ctx, f'clear {p.name}')
        fixture(ctx, f'give {p.name} diamond 3')
        fixture(ctx, f'experience set {p.name} 7 levels')
    if rule == 'show_advancement_messages':
        fixture(ctx, f'advancement revoke {p.name} only minecraft:story/root')
    if rule == 'command_blocks_work':
        fixture(ctx, 'scoreboard objectives add matrix_rule dummy')
        fixture(ctx, 'scoreboard players set result matrix_rule 0')
        fixture(ctx, 'setblock 3 80 0 command_block')
    if rule in {'allow_entering_nether_using_portals', 'players_nether_portal_creative_delay', 'players_nether_portal_default_delay'}:
        fixture(ctx, 'fill 2 79 0 5 83 0 obsidian')
        fixture(ctx, 'fill 3 80 0 4 82 0 nether_portal[axis=x]')


def act(ctx, rule):
    p = actor(ctx)
    fixture(ctx, 'gamerule minecraft:' + rule + ' ' + literal(value_for(rule)))
    ctx.boundary()
    ctx.state['changed_check'] = asdict(rule_query(ctx, rule, value_for(rule)))
    if ctx.state.get('actor_only_row'):
        return
    if rule in CONTROLLED_FIXTURES:
        # Do not turn mere set/query success into behavioral coverage.
        raise MissingCapability(CONTROLLED_FIXTURES[rule])
    if rule == 'max_command_forks':
        ctx.state['fork_reply'] = fixture(ctx, 'execute as @e[type=pig,name=MatrixFork] run scoreboard players add result matrix_rule 1')
    if rule == 'natural_health_regeneration':
        if ctx.factors['game_mode'] not in {'survival', 'adventure'} or ctx.factors['difficulty'] == 'peaceful':
            raise MissingCapability('vulnerable_nonpeaceful_actor:natural_regeneration')
        fixture(ctx, f'effect give {p.name} saturation 1 10 true')
        ctx.wait_until(lambda: field(p, 'food') == 20, description='saturation fixture fills food')
        fixture(ctx, f'effect give {p.name} instant_damage 1 0 true')
        ctx.wait_until(lambda: 0 < field(p, 'health') < ctx.state['health_start'],
                       description='one-tick instant damage fixture received')
        ctx.state['damaged_health'] = field(p, 'health')
    if rule == 'advance_time':
        fixture(ctx, 'time set 6000')
    if rule == 'advance_weather':
        fixture(ctx, 'weather rain 1t')
    if rule in {'lava_source_conversion', 'water_source_conversion'}:
        fluid = rule.split('_')[0]
        fixture(ctx, 'fill 2 79 2 4 80 4 stone')
        fixture(ctx, 'fill 2 80 2 4 80 2 air')
        fixture(ctx, f'setblock 2 80 2 {fluid}[level=0]')
        fixture(ctx, f'setblock 4 80 2 {fluid}[level=0]')
    if rule == 'random_tick_speed':
        fixture(ctx, 'setblock 3 80 0 farmland[moisture=7]')
        fixture(ctx, 'setblock 3 81 0 wheat[age=0]')
    if rule == 'spread_vines':
        fixture(ctx, 'fill 2 80 2 4 83 2 stone')
        fixture(ctx, 'setblock 3 82 3 vine[north=true]')
    if rule == 'fire_spread_radius_around_player':
        fixture(ctx, 'fill 2 79 2 4 79 4 oak_planks')
        fixture(ctx, 'setblock 3 80 3 fire')
    if rule == 'block_drops':
        fixture(ctx, 'setblock 3 80 0 air destroy')
    elif rule == 'mob_drops':
        if ctx.factors['game_mode'] == 'spectator':
            raise MissingCapability('attack_capable_actor:mob_drops')
        ctx.wait_until(lambda: len(nearby_entities(ctx, 'cow', (3, 80, 0))) == 1,
                       description='received weak cow fixture')
        victim = nearby_entities(ctx, 'cow', (3, 80, 0))[0]
        ticks(ctx, 20)
        verb(p, 'attack', victim['id'])
        ctx.wait_until(lambda: all(e['uuid'] != victim['uuid'] for e in field(p, 'entities')),
                       description='player attack removes weak cow')
    elif rule == 'entity_drops':
        fixture(ctx, 'kill @e[name=MatrixDrop]')
    elif rule in {'keep_inventory', 'immediate_respawn', 'show_death_messages'}:
        fixture(ctx, f'kill {p.name}')
        if rule == 'keep_inventory':
            verb(p, 'respawn')
    elif rule == 'fall_damage':
        fixture(ctx, f'tp {p.name} 0.5 94 0.5')
        verb(p, 'move', forward=0, strafe=0, ticks=40)
    elif rule == 'fire_damage':
        fixture(ctx, 'setblock 0 80 0 fire')
    elif rule == 'drowning_damage':
        fixture(ctx, 'fill -1 80 -1 1 82 1 water')
    elif rule == 'freeze_damage':
        fixture(ctx, 'fill -1 80 -1 1 82 1 powder_snow')
    elif rule == 'max_block_modifications':
        fixture(ctx, 'fill 2 80 2 3 80 2 gold_block')
    elif rule == 'send_command_feedback':
        ctx.state['observer_starts'] = [len(field(client, 'events')) for client in ctx.clients]
        p.command('time set 1000')
        fixture(ctx, 'say MatrixFeedbackDisabledFence')
        ctx.wait_until(lambda: all(any('MatrixFeedbackDisabledFence' in str(e)
                       for e in field(client, 'events')[start:])
                       for client, start in zip(ctx.clients, ctx.state['observer_starts'])),
                       description='all clients receive the suppressed command fence')
    elif rule == 'show_advancement_messages':
        fixture(ctx, f'advancement grant {p.name} only minecraft:story/root')
    elif rule == 'command_blocks_work':
        fixture(ctx, 'data merge block 3 80 0 {Command:"scoreboard players set result matrix_rule 1",auto:1b}')
    elif rule == 'tnt_explodes':
        fixture(ctx, 'summon tnt 3 80 0 {fuse:1s}')
    elif rule in {'allow_entering_nether_using_portals', 'players_nether_portal_creative_delay', 'players_nether_portal_default_delay'}:
        fixture(ctx, f'tp {p.name} 3.5 80 .5')
    elif rule == 'pvp':
        victim = nearby_entities(ctx, 'player', (2.5, 80, .5), .1)
        if len(victim) != 1 or victim[0]['uuid'] != ctx.state['victim_uuid']:
            raise MissingCapability('snapshot:entities.unique_pvp_victim')
        verb(p, 'attack', victim[0]['id'])
    elif rule == 'projectiles_can_break_blocks':
        shoot(ctx, (3.5, 80.5, .5))
    elif rule == 'players_sleeping_percentage':
        p.look_at(2.5, 80.5, 1.5)
        token = p.use_item_on((2, 80, 1), face='up')
        if not isinstance(token, int):
            raise MissingCapability('block_ack_sequence')
        ctx.wait_until(lambda: field(p, 'block_ack_sequence') >= token,
                       description='sleep interaction acknowledged')
        ctx.wait_until(lambda: bed_occupied(p) and bed_occupied(ctx.clients[1]),
                       description='both clients receive the occupied bed')
    if rule not in {'immediate_respawn', 'keep_inventory', 'show_death_messages'}:
        ticks(ctx, 400 if rule == 'drowning_damage' else 160)


def oracle(ctx, rule):
    p = actor(ctx)
    checks = [Check(**ctx.state['baseline_check']), Check(**ctx.state['changed_check'])]
    if ctx.state.get('actor_only_row'):
        checks.append(persisted_rule(ctx, rule))
        checks.append(Check('actor_side_only_not_subject_coverage', True,
                            'Separate multiplayer behavioral row must pass independently'))
        return checks
    if rule == 'advance_time':
        reply = query(ctx, 'time query daytime')
        checks.append(Check('clock_stopped', re.search(r'\b6000\b', reply) is not None, reply))
    elif rule == 'advance_weather':
        checks.append(Check('weather_cycle_stopped', field(p, 'weather') == 'rain', field(p, 'weather')))
    elif rule == 'players_sleeping_percentage':
        checks.append(Check('quorum_101_no_skip', 13000 <= daytime(ctx) < 23000))
        fixture(ctx, 'gamerule minecraft:players_sleeping_percentage 100')
        ticks(ctx, 160)
        checks.append(Check('one_of_two_at_100_no_skip', 13000 <= daytime(ctx) < 23000))
        checks.append(Check('observer_receives_sleeping_bed', bed_occupied(ctx.clients[1])))
        fixture(ctx, 'gamerule minecraft:players_sleeping_percentage 50')
        ticks(ctx, 160)
        checks.append(Check('one_of_two_at_50_skips_night', daytime(ctx) < 1000))
        for client in ctx.clients:
            checks.append(Check('wake_publication:' + client.name, bed_occupied(client, False)))
        fixture(ctx, 'gamerule minecraft:players_sleeping_percentage 101')
    elif rule in {'block_drops', 'mob_drops', 'entity_drops'}:
        checks.append(check_entity(ctx, '@e[type=item,x=3,y=80,z=0,distance=..3]', False))
        if rule == 'mob_drops':
            checks.append(check_entity(ctx, '@e[type=experience_orb,x=3,y=80,z=0,distance=..3]', False))
    elif rule in {'fall_damage', 'fire_damage', 'drowning_damage', 'freeze_damage'}:
        checks.append(Check('damage_disabled', field(p, 'health') == ctx.state['health_start'], field(p, 'health')))
    elif rule in {'water_source_conversion', 'lava_source_conversion'}:
        fluid = rule.split('_')[0]
        checks.append(check_block(ctx, (3, 80, 2), fluid + ('[level=0]' if rule == 'lava_source_conversion' else '[level=1]')))
    elif rule == 'random_tick_speed':
        checks.append(check_block(ctx, (3, 81, 0), 'wheat[age=0]'))
    elif rule == 'spread_vines':
        checks.append(check_block(ctx, (3, 81, 3), 'air'))
    elif rule == 'fire_spread_radius_around_player':
        checks.append(check_block(ctx, (2, 79, 2), 'oak_planks'))
    elif rule == 'max_block_modifications':
        checks.extend([check_block(ctx, (x, 80, 2), 'air') for x in (2, 3)])
    elif rule == 'tnt_explodes':
        checks.append(check_block(ctx, (3, 80, 0), 'stone'))
    elif rule == 'command_blocks_work':
        checks.append(check_score(ctx, 'result', 'matrix_rule', 0))
    elif rule in {'send_command_feedback', 'show_advancement_messages', 'show_death_messages'}:
        needle = {'send_command_feedback': ('commands.time.set', 'Set the time'),
                  'show_advancement_messages': ('chat.type.advancement', 'has made the advancement'),
                  'show_death_messages': ('death.', 'was killed', 'fell out of the world')}[rule]
        observers = zip(ctx.clients, ctx.state['observer_starts'])
        for client, start in observers:
            events = field(client, 'events')[start:]
            checks.append(Check('message_suppressed:' + client.name,
                                not any(token in str(e) for e in events for token in needle), events))
        if rule == 'send_command_feedback':
            checks.append(Check('suppressed_command_still_executes', daytime(ctx) == 1000))
            fixture(ctx, 'gamerule minecraft:send_command_feedback true')
            starts = [len(field(client, 'events')) for client in ctx.clients]
            p.command('time set 2000')
            fixture(ctx, 'say MatrixFeedbackEnabledFence')
            ctx.wait_until(lambda: all(any('MatrixFeedbackEnabledFence' in str(e)
                           for e in field(client, 'events')[start:])
                           for client, start in zip(ctx.clients, starts)),
                           description='all clients receive the feedback command fence')
            for client, start in zip(ctx.clients, starts):
                events = field(client, 'events')[start:]
                matches = [event for event in events if any(token in str(event) for token in needle)]
                checks.append(Check('enabled_command_feedback:' + client.name,
                                    len(matches) == 1, matches))
            checks.append(Check('enabled_command_executes', daytime(ctx) == 2000))
            ctx.state['domain_checks'] = [asdict(check) for check in checks]
            if all(check.passed for check in checks):
                for client in ctx.clients[1:]:
                    admin = [c for c in field(client, 'chat_components')
                             if c.get('translate') == 'chat.type.admin']
                    if not admin or not {'color', 'italic'} <= admin[-1].keys():
                        raise MissingCapability('snapshot:chat_components.admin_color_and_italic')
                    checks.append(Check('admin_broadcast_style:' + client.name,
                                        admin[-1]['color'] == 'gray' and admin[-1]['italic'] is True,
                                        admin[-1]))
            fixture(ctx, 'gamerule minecraft:send_command_feedback false')
    elif rule == 'keep_inventory':
        from .commands import inventory_count
        checks.append(Check('inventory_kept_after_death', inventory_count(p, 'diamond') == 3, field(p, 'inventory')))
        reply = query(ctx, f'experience query {p.name} levels')
        checks.append(Check('experience_kept_after_death', re.search(r'\b7\b', reply) is not None, reply))
    elif rule == 'immediate_respawn':
        checks.append(Check('no_death_screen', field(p, 'screen') != 'death', field(p, 'screen')))
        checks.append(Check('alive_after_death', field(p, 'health') > 0, field(p, 'health')))
    elif rule == 'reduced_debug_info':
        checks.append(Check('reduced_debug_packet', field(p, 'reduced_debug_info') is True))
    elif rule == 'allow_entering_nether_using_portals':
        checks.append(Check('portal_entry_disabled', field(p, 'dimension') == ctx.factors['dimension']))
    elif rule in {'players_nether_portal_default_delay', 'players_nether_portal_creative_delay'}:
        # Timing needs the transition tick, not merely eventual dimension change.
        entry, transition = field(p, 'portal_enter_tick'), field(p, 'dimension_change_tick')
        applies = (ctx.factors['game_mode'] == 'creative') == (rule == 'players_nether_portal_creative_delay')
        expected = 20 if applies else (0 if ctx.factors['game_mode'] == 'creative' else 80)
        checks.append(Check('portal_delay_ticks', transition - entry == expected, {'entry': entry, 'transition': transition, 'expected': expected}))
    elif rule == 'max_command_forks':
        checks.append(check_score(ctx, 'result', 'matrix_rule', '0..1'))
        reply = ctx.state['fork_reply']
        checks.append(Check('fork_limit_reported',
                            'limit' in reply.lower() or 'fork' in reply.lower(), reply))
    elif rule == 'projectiles_can_break_blocks':
        checks.append(check_block(ctx, (3, 80, 0), 'decorated_pot'))
        fixture(ctx, 'gamerule minecraft:projectiles_can_break_blocks true')
        shoot(ctx, (3.5, 80.5, .5))
        checks.append(check_block(ctx, (3, 80, 0), 'air'))
        fixture(ctx, 'gamerule minecraft:projectiles_can_break_blocks false')
    elif rule == 'natural_health_regeneration':
        before = ctx.state['damaged_health']
        checks.append(Check('full_food_regeneration_disabled', field(p, 'health') == before, field(p, 'health')))
        fixture(ctx, 'gamerule minecraft:natural_health_regeneration true')
        ticks(ctx, 160)
        checks.append(Check('full_food_regeneration_enabled', field(p, 'health') > before, field(p, 'health')))
        fixture(ctx, 'gamerule minecraft:natural_health_regeneration false')
    elif rule == 'max_entity_cramming':
        before = ctx.state['cramming_health']
        after = {e['uuid']: entity_health(e) for e in nearby_entities(ctx, 'cow', (3.5, 80, .5))}
        checks.append(Check('cramming_disabled_health_and_census', after == before, after))
        fixture(ctx, 'gamerule minecraft:max_entity_cramming 1')
        ticks(ctx, 80)
        enabled = {e['uuid']: entity_health(e) for e in nearby_entities(ctx, 'cow', (3.5, 80, .5))}
        checks.append(Check('cramming_enabled_damages_crowd',
                            len(enabled) < len(before) or any(uuid in before and health < before[uuid] for uuid, health in enabled.items()), enabled))
        fixture(ctx, 'gamerule minecraft:max_entity_cramming 0')
    elif rule == 'pvp':
        victim = ctx.clients[1]
        checks.append(Check('pvp_disabled_preserves_health', field(victim, 'health') == ctx.state['victim_health'], field(victim, 'health')))
        prepare_bow(ctx)
        shoot(ctx, (2.5, 81, .5))
        checks.append(Check('pvp_disabled_projectile_preserves_health',
                            field(victim, 'health') == ctx.state['victim_health'], field(victim, 'health')))
        fixture(ctx, 'gamerule minecraft:pvp true')
        targets = nearby_entities(ctx, 'player', (2.5, 80, .5), .1)
        if len(targets) != 1 or targets[0]['uuid'] != ctx.state['victim_uuid']:
            raise MissingCapability('snapshot:entities.unique_pvp_victim')
        verb(p, 'attack', targets[0]['id'])
        ticks(ctx, 2)
        checks.append(Check('pvp_enabled_damages_victim', field(victim, 'health') < ctx.state['victim_health'], field(victim, 'health')))
        before_arrow = field(victim, 'health')
        shoot(ctx, (2.5, 81, .5))
        checks.append(Check('pvp_enabled_projectile_damages_victim', field(victim, 'health') < before_arrow, field(victim, 'health')))
        fixture(ctx, 'gamerule minecraft:pvp false')
    else:
        raise AssertionError('No behavioral oracle for registered rule: ' + rule)
    checks.append(rule_query(ctx, rule, value_for(rule)))
    ctx.state['domain_checks'] = [asdict(check) for check in checks]
    # A failed behavioral assertion stays a failure even if restart is absent.
    if all(check.passed for check in checks):
        checks.append(persisted_rule(ctx, rule))
    return checks


ADAPTERS = {'gamerules.' + rule: Adapter(partial(setup, rule=rule), partial(act, rule=rule), partial(oracle, rule=rule)) for rule in DEFAULTS}
