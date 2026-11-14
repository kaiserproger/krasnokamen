"""Focused adapter-contract tests; these are NOT real-server coverage receipts."""
import json
import unittest
from pathlib import Path
from unittest.mock import patch

from ..matrix_api import Check, MatrixContext, MissingCapability, execute_adapter
from . import chat, commands, gamerules, selectors, world_border


class Client:
    name = 'MatrixActor'

    def __init__(self, **snapshot):
        self.data = {'events': [], 'health': 20, 'inventory': [], 'position': [.5, 80, .5], **snapshot}
        self.sent = []
        self.lifecycle = []

    def snapshot(self):
        return self.data

    def command(self, text):
        self.sent.append(text)
        self.data['events'].append('received: ' + text)

    def wait_until(self, predicate, **kwargs):
        value = predicate()
        if not value:
            raise AssertionError('unsatisfied test predicate: ' + kwargs['description'])
        return value

    def disconnect(self):
        self.lifecycle.append('disconnect')

    def reconnect(self):
        self.lifecycle.append('reconnect')


class Server:
    def __init__(self, reply=''):
        self.reply = reply
        self.sent = []
        self.tick = 0
        self.lifecycle = []

    def command(self, text):
        self.sent.append(text)
        return self.reply

    query = command

    def save_all(self):
        self.lifecycle.append('save_all')

    def clean_restart(self):
        self.lifecycle.append('clean_restart')
        return {'same_world': True}

    def ensure_loaded(self, area):
        self.sent.append(('ensure_loaded', area))

    def dashboard_stats(self):
        self.tick += 10
        return {'simulation_tick': self.tick}


def context(permission='op_2', reply='', **snapshot):
    return MatrixContext({'subject': 'commands.give', 'context': {
        'permission': permission, 'dimension': 'overworld', 'game_mode': 'survival',
        'network_condition': 'normal', 'server_lifecycle': 'steady'}},
        Server(reply), [Client(**snapshot)], Path('.'))


class DomainContractTests(unittest.TestCase):
    def test_every_assigned_subject_has_adapter(self):
        expected = [(commands, 66), (selectors, 18), (gamerules, 58), (chat, 10), (world_border, 6)]
        for module, count in expected:
            self.assertEqual(len(module.ADAPTERS), count, module.__name__)
            self.assertTrue(all(callable(a.setup) and callable(a.act) and callable(a.oracle)
                                for a in module.ADAPTERS.values()))

    def test_missing_inventory_does_not_mean_empty(self):
        c = Client()
        del c.data['inventory']
        with self.assertRaisesRegex(MissingCapability, 'snapshot:inventory'):
            commands.inventory_count(c, 'stone')

    def test_inventory_uses_namespaced_ids_and_sparse_slots(self):
        c = Client(inventory=[{'slot': 36, 'item_id': 'minecraft:stone', 'count': 4},
                              {'slot': 9, 'item_id': 'minecraft:stone', 'count': 3},
                              {'slot': 37, 'item_id': 'minecraft:dirt', 'count': 5}])
        self.assertEqual(commands.inventory_count(c, 'stone'), 7)

    def test_permission_values_match_matrix(self):
        for text, value in [('non_op_0', 0), ('op_1', 1), ('op_2', 2), ('op_4', 4), ('console', 4)]:
            self.assertEqual(commands.permission_level(context(text)), value)

    def test_action_preserves_client_authority(self):
        ctx = context('op_1')
        commands.issue(ctx, 'give MatrixActor stone 1')
        commands.issue(ctx, 'list')
        self.assertEqual(ctx.server.sent, [])
        self.assertEqual(len(ctx.clients[0].sent), 2)
        self.assertTrue(ctx._boundary_called)

    def test_console_authority_and_single_boundary(self):
        ctx = context('console')
        commands.issue(ctx, 'worldborder set 100')
        commands.issue(ctx, 'worldborder get')
        self.assertEqual(ctx.server.sent, ['worldborder set 100', 'worldborder get'])
        self.assertEqual(ctx.clients[0].sent, [])
        self.assertTrue(ctx._boundary_called)

    def test_fence_is_not_a_block_oracle(self):
        ctx = context(reply='[Server] MatrixConsoleFence00001')
        self.assertFalse(commands.check_block(ctx, (1, 80, 1), 'stone').passed)

    def test_authoritative_positive_marker(self):
        ctx = context(reply='[Server] MatrixBlock1801')
        self.assertTrue(commands.check_block(ctx, (1, 80, 1), 'stone').passed)

    def test_simulation_tick_counter_not_wall_clock(self):
        commands.ticks(context(), 10)
        ctx = context()
        ctx.server.dashboard_stats = lambda: {'uptime_secs': 100}
        with self.assertRaisesRegex(MissingCapability, 'simulation_tick'):
            commands.ticks(ctx, 10)

    def test_optional_verbs_fail_closed(self):
        with self.assertRaisesRegex(MissingCapability, 'client:send_signed_chat'):
            chat.verb(Client(), 'send_signed_chat', 'test')

    def test_gamerule_value_not_substring(self):
        ctx = context(reply='max_block_modifications = 32768')
        self.assertFalse(gamerules.rule_query(ctx, 'max_block_modifications', 1).passed)
        ctx.server.reply = 'keep_inventory = false'
        self.assertFalse(gamerules.rule_query(ctx, 'keep_inventory', True).passed)

    def test_registration_is_not_behavioral_pass(self):
        ctx = context('console', reply='spawn_patrols = false')
        with self.assertRaisesRegex(MissingCapability, 'controlled_patrol_spawn_attempt'):
            gamerules.act(ctx, 'spawn_patrols')
        self.assertIn('changed_check', ctx.state)

    def test_selector_uses_actual_teleport_not_fake_entity_score_names(self):
        ctx = context('console')
        selectors.act(ctx, 'sort_random')
        self.assertIn('run tp @e[type=pig,sort=random,limit=1] 0 80 6', ctx.server.sent[0])
        self.assertNotIn('scoreboard players set @s', ctx.server.sent[0])

    def test_data_unavailable_is_not_query_pass(self):
        ctx = context('console', reply='Supported: data get|merge block ...')
        ctx.state['command_reply'] = ctx.server.reply
        with self.assertRaisesRegex(MissingCapability, 'data_get_entity'):
            commands._oracle(ctx, 'data')

    def test_worldborder_fence_does_not_parse_as_size(self):
        with self.assertRaisesRegex(MissingCapability, 'worldborder_get'):
            world_border.size(context(reply='MatrixConsoleFence00001'))
        self.assertEqual(world_border.size(context(reply='The world border is 50 blocks wide')), 50)

    def test_exact_list_count_and_names(self):
        ctx = context()
        ctx.state['command_reply'] = 'There are 1 of a max of 20 players online: MatrixActor\x00'
        self.assertTrue(all(c.passed for c in commands._oracle(ctx, 'list')))
        ctx.state['command_reply'] = 'Error mentioning MatrixActor'
        self.assertFalse(any(c.passed for c in commands._oracle(ctx, 'list')))
        ctx.state['command_reply'] = 'There are 2 of a max of 20 players online: MatrixActor, Extra\x00'
        self.assertFalse(any(c.passed for c in commands._oracle(ctx, 'list')))

    def test_each_invalid_selector_error_survives_boundary(self):
        ctx = context(reply='MatrixScoreOracle')
        ctx.state['names'] = []
        ctx.state['rejections'] = ['Unknown selector type', "Unknown option 'foo'",
                                   "Option 'limit' isn't applicable here",
                                   'Integer must not be less than 1, found 0',
                                   'Distance cannot be negative']
        self.assertTrue(all(c.passed for c in selectors.oracle(ctx, 'invalid_selector')))
        ctx.state['rejections'][-1] = 'Successfully executed command'
        self.assertFalse(all(c.passed for c in selectors.oracle(ctx, 'invalid_selector')))

    def test_solaris_debug_fixture_is_not_vanilla_profiler_pass(self):
        ctx = context()
        ctx.state['command_reply'] = 'Unknown or incomplete command, see below for error'
        self.assertFalse(commands._oracle(ctx, 'debug')[0].passed)

    def test_gamerule_state_satisfies_shared_json_contract(self):
        ctx = context('console', reply='spawn_patrols = false')
        with self.assertRaisesRegex(MissingCapability, 'controlled_patrol_spawn_attempt'):
            execute_adapter(gamerules.ADAPTERS['gamerules.spawn_patrols'], ctx)
        restored = json.loads(json.dumps(ctx.state, allow_nan=False))
        self.assertIsInstance(restored['baseline_check'], dict)
        self.assertIs(restored['changed_check']['passed'], True)

    def test_nonop_msg_rejection_tracks_exact_plugin_profile(self):
        ctx = context('op_1')
        ctx.state['names'] = []
        ctx.state['recipient_starts'] = [0]
        ctx.factors['plugin_profile'] = 'live'
        ctx.state['command_reply'] = 'Online player not found or ambiguous.'
        self.assertTrue(selectors.oracle(ctx, 'nonop_selector_msg')[0].passed)
        ctx.factors['plugin_profile'] = 'vanilla'
        self.assertFalse(selectors.oracle(ctx, 'nonop_selector_msg')[0].passed)
        ctx.state['command_reply'] = 'Selector not allowed'
        self.assertTrue(selectors.oracle(ctx, 'nonop_selector_msg')[0].passed)

    def test_live_private_message_requires_essentials_format(self):
        ctx = context()
        target = Client()
        target.name = 'MatrixTarget'
        ctx.clients.append(target)
        ctx.state.update(events_before=[0, 0], starts=[0, 0])
        ctx.clients[0].data['events'] = ['[to MatrixTarget] MatrixPrivate']
        target.data['events'] = ['[from MatrixActor] MatrixPrivate']
        ctx.factors['plugin_profile'] = 'live'
        self.assertTrue(all(c.passed for c in commands._oracle(ctx, 'msg')))
        with self.assertRaisesRegex(MissingCapability, 'nonop_client:selector_permission_probe'):
            chat.oracle(ctx, 'private_message')
        target.data['events'] = ['MatrixPrivate']
        self.assertFalse(all(c.passed for c in commands._oracle(ctx, 'msg')))
        self.assertFalse(all(c.passed for c in chat.oracle(ctx, 'private_message')))
        ctx.factors['plugin_profile'] = 'vanilla'
        self.assertTrue(all(c.passed for c in commands._oracle(ctx, 'msg')))

    def test_low_permission_skips_privileged_fixture_and_checks_root(self):
        ctx = context('op_1', command_roots=['list', 'help'])
        commands._setup(ctx, 'give')
        self.assertEqual(ctx.server.sent, [])
        ctx.state['command_reply'] = 'Unknown or incomplete command'
        self.assertTrue(all(c.passed for c in commands.complete_oracle(ctx, 'give')))
        ctx.clients[0].data['command_roots'].append('give')
        self.assertFalse(all(c.passed for c in commands.complete_oracle(ctx, 'give')))

    def test_permission_probe_cannot_turn_success_into_complete_coverage(self):
        ctx = context('console', reply='keep_inventory = true')
        with self.assertRaisesRegex(MissingCapability, 'client:permission_probe:op_1'):
            commands.complete_oracle(ctx, 'gamerule')
        self.assertTrue(ctx.state['domain_checks'][0]['passed'])

    def test_numeric_reports_enforce_range(self):
        ctx = context()
        for subject, reply in [('tps', 'TPS: 21.0'), ('mspt', 'MSPT: invalid')]:
            ctx.state['command_reply'] = reply
            self.assertFalse(commands._oracle(ctx, subject)[0].passed)
        ctx.state['command_reply'] = 'TPS: 19.8'
        self.assertTrue(commands._oracle(ctx, 'tps')[0].passed)

    def test_selector_rejection_checks_all_recipients(self):
        ctx = context('op_1')
        ctx.state.update(names=[], recipient_starts=[0], command_reply='Selector not allowed')
        ctx.clients[0].data['events'] = ['MatrixForbiddenSelector']
        self.assertFalse(all(c.passed for c in selectors.oracle(ctx, 'nonop_selector_msg')))

    def test_negated_type_selection_is_actually_exercised(self):
        ctx = context('console', reply='MatrixScoreOracle')
        ctx.state['names'] = ['MatrixActor', 'MatrixNear', 'MatrixMid', 'MatrixFar', 'MatrixCow']
        selectors.oracle(ctx, 'opt_type')
        self.assertTrue(any('tp @e[type=!pig]' in command for command in ctx.server.sent))

    def test_volume_probe_includes_intersecting_entity_origin_outside_box(self):
        ctx = context('console', reply='MatrixScoreOracle')
        ctx.state['names'] = ['MatrixNear', 'MatrixMid', 'MatrixFar', 'MatrixCow']
        selectors.oracle(ctx, 'opt_dxdydz')
        self.assertIn('tp @e[name=MatrixNear] -0.2 80 0', ctx.server.sent)
        self.assertTrue(any('dx=-4' in command for command in ctx.server.sent))

    def test_fork_limit_is_real_action_not_registered_stub(self):
        ctx = context('non_op_0', reply='max_command_forks = 1')
        with patch.object(gamerules, 'ticks'):
            gamerules.act(ctx, 'max_command_forks')
        self.assertNotIn('max_command_forks', gamerules.CONTROLLED_FIXTURES)
        self.assertTrue(any('execute as @e[type=pig,name=MatrixFork]' in command for command in ctx.server.sent))
        self.assertEqual(ctx.clients[0].sent, [])

    def test_pearl_staying_at_launch_position_does_not_pass(self):
        ctx = context(position=[8, 80, 0])
        ctx.state['tp_position'] = [15, 80, 0]
        ctx.state.update(health_before=20, tp_health_after=19)
        self.assertFalse(all(c.passed for c in world_border.oracle(ctx, 'teleport_outside_clamped')))

    def test_gamerule_suppression_checks_observer_not_only_actor(self):
        ctx = context(reply='show_death_messages = false')
        ctx.clients.append(Client(events=['MatrixActor fell out of the world']))
        check = {'name': 'registration', 'passed': True, 'detail': None}
        ctx.state.update(baseline_check=check, changed_check=check, observer_starts=[0, 0])
        self.assertFalse(all(c.passed for c in gamerules.oracle(ctx, 'show_death_messages')))

    def test_fixture_and_oracle_commands_stay_in_assigned_dimension(self):
        ctx = context('console')
        ctx.factors['dimension'] = 'the_end'
        commands.fixture(ctx, 'summon pig 3 80 0')
        commands.query(ctx, 'execute if block 3 80 0 stone run say marker')
        self.assertEqual(ctx.server.sent, [
            'execute in minecraft:the_end run summon pig 3 80 0',
            'execute in minecraft:the_end run execute if block 3 80 0 stone run say marker'])
        commands.fixture(ctx, 'gamerule minecraft:block_drops false')
        self.assertEqual(ctx.server.sent[-1], 'gamerule minecraft:block_drops false')

    def test_enchantment_identity_uses_received_registry_not_hardcoded_id(self):
        ctx = context(inventory=[{'slot': 36, 'item_id': 'minecraft:diamond_sword', 'count': 1,
                                  'components': {'minecraft:enchantments': [[77, 2]]}}],
                      registries={'minecraft:enchantment': {'77': 'minecraft:sharpness'}})
        ctx.state['enchanted_slot'] = 36
        self.assertTrue(commands._oracle(ctx, 'enchant')[0].passed)
        ctx.clients[0].data['registries']['minecraft:enchantment']['77'] = 'minecraft:smite'
        self.assertFalse(commands._oracle(ctx, 'enchant')[0].passed)
        ctx.clients[0].data['registries']['minecraft:enchantment'].clear()
        with self.assertRaisesRegex(MissingCapability, 'snapshot:registries.minecraft:enchantment:77'):
            commands._oracle(ctx, 'enchant')

    def test_negative_mspt_is_not_accepted_as_positive_number(self):
        ctx = context()
        ctx.state['command_reply'] = 'MSPT: -3.0'
        self.assertFalse(commands._oracle(ctx, 'mspt')[0].passed)

    def test_regeneration_requires_positive_enabled_control(self):
        ctx = context(reply='natural_health_regeneration = false', food=20, health=14)
        check = {'name': 'registration', 'passed': True, 'detail': None}
        ctx.state.update(baseline_check=check, changed_check=check, damaged_health=14)
        with patch.object(gamerules, 'ticks'):
            checks = gamerules.oracle(ctx, 'natural_health_regeneration')
        self.assertTrue(next(c for c in checks if c.name == 'full_food_regeneration_disabled').passed)
        self.assertFalse(next(c for c in checks if c.name == 'full_food_regeneration_enabled').passed)
        self.assertNotIn('natural_health_regeneration', gamerules.CONTROLLED_FIXTURES)

    def test_cramming_requires_actual_entity_health(self):
        ctx = context(entities=[{'id': 5, 'uuid': 'cow', 'type': 'minecraft:cow', 'position': [3.5, 80, .5]}])
        self.assertEqual(len(gamerules.nearby_entities(ctx, 'cow', (3.5, 80, .5))), 1)
        with self.assertRaisesRegex(MissingCapability, 'snapshot:entities.health'):
            gamerules.entity_health(ctx.clients[0].data['entities'][0])
        self.assertNotIn('max_entity_cramming', gamerules.CONTROLLED_FIXTURES)

    def test_single_player_pvp_is_explicit_missing_victim(self):
        ctx = context('console')
        with self.assertRaisesRegex(MissingCapability, 'multi_client_2:pvp_victim'):
            gamerules.setup(ctx, 'pvp')

    def test_mob_drop_probe_uses_received_entity_attack_not_console_kill(self):
        ctx = context('console', reply='mob_drops = false',
                      entities=[{'id': 77, 'uuid': 'victim', 'type': 'minecraft:cow', 'position': [3, 80, 0]}])
        attacks = []
        def attack(entity_id):
            attacks.append(entity_id)
            ctx.clients[0].data['entities'] = []
        ctx.clients[0].attack = attack
        with patch.object(gamerules, 'ticks'):
            gamerules.act(ctx, 'mob_drops')
        self.assertEqual(attacks, [77])
        self.assertFalse(any(command.startswith('kill ') for command in ctx.server.sent))

    def test_bounded_native_chat_cannot_supply_indexed_receipt(self):
        ctx = context()
        ctx.factors['client_kind'] = 'native_linux'
        with self.assertRaisesRegex(MissingCapability, 'native_cumulative_received_chat'):
            commands.issue(ctx, 'list')
        self.assertEqual(ctx.clients[0].sent, [])

    def test_absent_java_chat_history_has_exact_missing_capability(self):
        ctx = context(events=None)
        with self.assertRaisesRegex(MissingCapability, 'cumulative_chat_history'):
            commands.issue(ctx, 'list')

    def test_approved_actor_only_split_preserves_rule_without_fake_victim(self):
        ctx = context('op_2', reply='pvp = true')
        ctx.factors['player_count'] = '1'
        ctx.row['oracle'] = ['player_count 1: verify the actor-side part only']
        gamerules.setup(ctx, 'pvp')
        ctx.server.reply = 'pvp = false'
        gamerules.act(ctx, 'pvp')
        calls = []
        ctx.server.save_all = lambda: calls.append('save')
        ctx.server.clean_restart = lambda: {'same_world': True}
        ctx.clients[0].disconnect = lambda: calls.append('disconnect')
        ctx.clients[0].reconnect = lambda: calls.append('reconnect')
        checks = gamerules.oracle(ctx, 'pvp')
        self.assertTrue(all(check.passed for check in checks))
        self.assertEqual(calls, ['save', 'disconnect', 'reconnect'])
        self.assertTrue(any(check.name == 'actor_side_only_not_subject_coverage' for check in checks))

    def test_multiplayer_row_cannot_take_actor_only_registration_shortcut(self):
        ctx = context()
        ctx.factors['player_count'] = '2'
        ctx.row['oracle'] = ['player_count 1: verify the actor-side part only']
        self.assertFalse(gamerules.actor_only_row(ctx, 'pvp'))
        ctx.factors['player_count'] = '1'
        self.assertFalse(gamerules.actor_only_row(ctx, 'spawn_patrols'))

    def test_bow_probe_requires_consumed_arrow_not_only_input_submission(self):
        ctx = context(inventory=[{'slot': 37, 'item_id': 'minecraft:arrow', 'count': 4}])
        submitted = []
        ctx.clients[0].look_at = lambda *pos: submitted.append(('look', pos))
        ctx.clients[0].use_item = lambda **kw: submitted.append(('hold', kw))
        ctx.clients[0].release_use_item = lambda: None
        with self.assertRaisesRegex(AssertionError, 'consumes actual arrow'):
            gamerules.shoot(ctx, (3.5, 80.5, .5))
        def consume():
            ctx.clients[0].data['inventory'][0]['count'] -= 1
        ctx.clients[0].release_use_item = consume
        with patch.object(gamerules, 'ticks'):
            gamerules.shoot(ctx, (3.5, 80.5, .5))
        self.assertIn(('hold', {'ticks': 20}), submitted)

    def test_projectile_rule_requires_enabled_pot_break_control(self):
        ctx = context(reply='projectiles_can_break_blocks = false')
        check = {'name': 'registration', 'passed': True, 'detail': None}
        ctx.state.update(baseline_check=check, changed_check=check)
        with patch.object(gamerules, 'shoot') as shot, patch.object(gamerules, 'check_block',
                side_effect=[Check('disabled_intact', True), Check('enabled_broken', False)]):
            checks = gamerules.oracle(ctx, 'projectiles_can_break_blocks')
        self.assertEqual(shot.call_count, 1)
        self.assertFalse(all(c.passed for c in checks))
        self.assertNotIn('projectiles_can_break_blocks', gamerules.CONTROLLED_FIXTURES)

    def test_pvp_covers_melee_and_projectiles_with_received_victim_identity(self):
        ctx = context(reply='pvp = false',
                      entities=[{'id': 77, 'uuid': 'victim', 'type': 'minecraft:player', 'position': [2.5, 80, .5]}])
        victim = Client()
        ctx.clients.append(victim)
        check = {'name': 'registration', 'passed': True, 'detail': None}
        ctx.state.update(baseline_check=check, changed_check=check, victim_health=20, victim_uuid='victim')
        ctx.clients[0].attack = lambda entity_id: victim.data.update(health=19)
        shots = []
        def shoot(*args):
            shots.append(1)
            if len(shots) == 2:
                victim.data['health'] = 10
        with patch.object(gamerules, 'prepare_bow'), patch.object(gamerules, 'shoot', side_effect=shoot), patch.object(gamerules, 'ticks'):
            checks = gamerules.oracle(ctx, 'pvp')
        self.assertTrue(all(c.passed for c in checks))
        self.assertEqual(len(shots), 2)

    def test_sleep_quorum_requires_positive_skip_and_both_wake_publications(self):
        ctx = context(reply='players_sleeping_percentage = 101')
        ctx.clients.append(Client())
        check = {'name': 'registration', 'passed': True, 'detail': None}
        ctx.state.update(baseline_check=check, changed_check=check)
        for client in ctx.clients:
            client.read_block_state = lambda pos: {'block_id': 'minecraft:red_bed',
                                                    'properties': {'occupied': 'true'}}
        with patch.object(gamerules, 'ticks'), patch.object(gamerules, 'daytime',
                side_effect=[13160, 13320, 13480]):
            checks = gamerules.oracle(ctx, 'players_sleeping_percentage')
        self.assertTrue(next(c for c in checks if c.name == 'one_of_two_at_100_no_skip').passed)
        self.assertFalse(next(c for c in checks if c.name == 'one_of_two_at_50_skips_night').passed)
        self.assertEqual(len([c for c in checks if c.name.startswith('wake_publication:') and not c.passed]), 2)
        self.assertNotIn('players_sleeping_percentage', gamerules.CONTROLLED_FIXTURES)

    def test_missing_bed_is_not_a_successful_wake_publication(self):
        client = Client()
        client.read_block_state = lambda pos: {'block_id': 'minecraft:air', 'properties': {}}
        self.assertFalse(gamerules.bed_occupied(client, False))
        client.read_block_state = lambda pos: {'block_id': 'minecraft:red_bed', 'properties': {'occupied': 'false'}}
        self.assertTrue(gamerules.bed_occupied(client, False))

    def test_daytime_uses_command_value_not_log_timestamp(self):
        ctx = context(reply='2026-10-24 commands.time.query: 13000')
        self.assertEqual(gamerules.daytime(ctx), 13000)
        ctx.server.reply = 'Unknown command time; MatrixConsoleFence12345'
        with self.assertRaisesRegex(MissingCapability, 'query:time.daytime.numeric_feedback'):
            gamerules.daytime(ctx)

    def test_behavioral_rule_requires_restart_persistence(self):
        ctx = context(reply='commands.time.query: 6000; advance_time = false')
        check = {'name': 'registration', 'passed': True, 'detail': None}
        ctx.state.update(baseline_check=check, changed_check=check)
        checks = gamerules.oracle(ctx, 'advance_time')
        self.assertTrue(all(check.passed for check in checks))
        self.assertEqual(ctx.server.lifecycle, ['save_all', 'clean_restart'])
        self.assertEqual(ctx.clients[0].lifecycle, ['disconnect', 'reconnect'])
        ctx.server.clean_restart = lambda: setattr(ctx.server, 'reply', 'advance_time = true')
        checks = gamerules.oracle(ctx, 'advance_time')
        self.assertFalse(checks[-1].passed)

    def test_feedback_requires_other_operator_delivery_and_received_style(self):
        for delivered, styled in ((True, True), (False, False), (True, False)):
            with self.subTest(delivered=delivered, styled=styled):
                ctx = context(reply='send_command_feedback = false')
                component = {'translate': 'chat.type.admin'}
                if styled:
                    component.update(color='gray', italic=True)
                observer = Client(chat_components=[component])
                observer.name = 'Observer'
                ctx.clients.append(observer)
                check = {'name': 'registration', 'passed': True, 'detail': None}
                ctx.state.update(baseline_check=check, changed_check=check, observer_starts=[0, 0])
                def command(text):
                    ctx.clients[0].data['events'].append('commands.time.set: 2000')
                    if delivered:
                        observer.data['events'].append('chat.type.admin: MatrixActor, commands.time.set: 2000')
                def fixture(context, text):
                    if text.startswith('say '):
                        for client in context.clients:
                            client.data['events'].append(text[4:])
                ctx.clients[0].command = command
                with patch.object(gamerules, 'fixture', side_effect=fixture), patch.object(
                        gamerules, 'daytime', side_effect=[1000, 2000]):
                    if delivered and not styled:
                        with self.assertRaisesRegex(MissingCapability, 'admin_color_and_italic'):
                            gamerules.oracle(ctx, 'send_command_feedback')
                        self.assertTrue(all(c['passed'] for c in ctx.state['domain_checks']))
                    else:
                        checks = gamerules.oracle(ctx, 'send_command_feedback')
                        self.assertEqual(all(c.passed for c in checks), delivered)

    def test_log_admin_commands_never_uses_obsolete_observer_split(self):
        ctx = context()
        ctx.factors['player_count'] = '1'
        ctx.row['oracle'] = ['player_count 1: obsolete observer requirement']
        self.assertFalse(gamerules.actor_only_row(ctx, 'log_admin_commands'))
        self.assertEqual(gamerules.CONTROLLED_FIXTURES['log_admin_commands'], 'server_log:admin_command_records')

    def test_refused_border_placement_cannot_consume_inventory(self):
        ctx = context(inventory=[{'slot': 36, 'item_id': 'minecraft:dirt', 'count': 7}])
        with patch.object(world_border, 'check_block', return_value=Check('unchanged', True)):
            checks = world_border.oracle(ctx, 'block_actions_outside')
        self.assertFalse(checks[-1].passed)

    def test_border_warning_requires_observed_red_warning_not_only_configuration(self):
        ctx = context(health=19, world_border={'warning_distance': 7, 'warning_time': 3})
        ctx.state.update(health_before=20, buffer_health_before=20, buffer_health_after=20)
        with self.assertRaisesRegex(MissingCapability, 'snapshot:border_warning_visible'):
            world_border.oracle(ctx, 'damage_buffer_warning')
        ctx.clients[0].data['border_warning_visible'] = False
        self.assertFalse(world_border.oracle(ctx, 'damage_buffer_warning')[-1].passed)

    def test_border_persistence_reconnects_every_observer(self):
        ctx = context(reply='The world border is 50 blocks wide',
                      world_border={'damage_buffer': 2, 'target_size': 100, 'remaining_ticks': 2000})
        ctx.clients.append(Client())
        with patch.object(world_border, 'ticks'):
            world_border.act(ctx, 'border_persistence')
        for client in ctx.clients:
            self.assertEqual(client.lifecycle, ['disconnect', 'reconnect', 'disconnect', 'reconnect'])

    def test_join_leave_requires_received_yellow_style(self):
        components = [{'translate': key, 'with': [{'text': 'Observer'}]}
                      for key in ('multiplayer.player.left', 'multiplayer.player.joined')]
        ctx = context(events=['Observer left', 'Observer joined'], chat_components=components)
        observer = Client()
        observer.name = 'Observer'
        ctx.clients.append(observer)
        ctx.state['starts'] = [0, 0]
        with self.assertRaisesRegex(MissingCapability, 'join_leave_color'):
            chat.oracle(ctx, 'join_leave_messages')
        for component in components:
            component['color'] = 'yellow'
        self.assertTrue(all(c.passed for c in chat.oracle(ctx, 'join_leave_messages')))
        components[0]['color'] = 'red'
        self.assertFalse(all(c.passed for c in chat.oracle(ctx, 'join_leave_messages')))

    def test_unrelated_departure_cannot_satisfy_named_leave(self):
        ctx = context(events=['SomeoneElse left', 'Observer joined'])
        observer = Client()
        observer.name = 'Observer'
        ctx.clients.append(observer)
        ctx.state['starts'] = [0, 0]
        self.assertFalse(chat.oracle(ctx, 'join_leave_messages')[0].passed)

    def test_visibility_accepts_actual_translated_list_component(self):
        ctx = context()
        ctx.state.update(starts=[0], visibility_results=[
            ('full', ['MatrixVisibilityfull', 'commands.list.players: 2, 20']),
            ('commands_only', ['commands.list.players: 2, 20']),
            ('hidden', [])])
        self.assertTrue(all(c.passed for c in chat.oracle(ctx, 'chat_visibility_setting')))

    def test_border_lerp_duration_is_explicit_vanilla_ticks(self):
        ctx = context()
        with patch.object(world_border, 'ticks') as wait_ticks, patch.object(world_border, 'size', return_value=75):
            world_border.act(ctx, 'set_and_lerp')
        self.assertIn('worldborder set 50 200t', ctx.server.sent)
        self.assertEqual([call.args[1] for call in wait_ticks.call_args_list], [100, 110])
        self.assertEqual(ctx.state['middle_size'], 75)


if __name__ == '__main__':
    unittest.main()
