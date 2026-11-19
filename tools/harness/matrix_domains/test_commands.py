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

    def test_console_acknowledgment_is_never_received_player_chat(self):
        # Regression TEST-MATRIX-CONSOLE-FENCE-CHAT: the harness's own console
        # fence (scoreboard players list MatrixConsoleFence<hash>) reached the
        # observer's chat and was read back as player-visible command feedback
        # (winqa-8796727.yaml:63-68, shots/218-face-platform.png). A fence line
        # must neither satisfy the receipt wait nor enter the parsed reply.
        ctx = context('op_1')
        client = ctx.clients[0]

        def fence_only(text):
            client.sent.append(text)
            client.data['events'].append('[Server] MatrixConsoleFence0000023')

        client.command = fence_only
        with self.assertRaisesRegex(AssertionError, 'received command feedback fence'):
            commands.issue(ctx, 'time set day')
        self.assertNotIn('command_reply', ctx.state)

        def feedback_after_fence(text):
            client.sent.append(text)
            client.data['events'].extend(['[Server] MatrixConsoleFence0000024',
                                          'Set the time to 1000'])

        client.command = feedback_after_fence
        commands.issue(ctx, 'time set day')
        self.assertEqual(ctx.state['command_reply'], 'Set the time to 1000')

    def test_registration_is_not_behavioral_pass(self):
        ctx = context('console', reply='spawn_patrols = false')
        with self.assertRaisesRegex(MissingCapability, 'controlled_patrol_spawn_attempt'):
            gamerules.act(ctx, 'spawn_patrols')
        self.assertIn('changed_check', ctx.state)

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

    def test_krasnokamen_debug_fixture_is_not_vanilla_profiler_pass(self):
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

    def test_volume_probe_includes_intersecting_entity_origin_outside_box(self):
        ctx = context('console', reply='MatrixScoreOracle')
        ctx.state['names'] = ['MatrixNear', 'MatrixMid', 'MatrixFar', 'MatrixCow']
        selectors.oracle(ctx, 'opt_dxdydz')
        self.assertIn('tp @e[name=MatrixNear] -0.2 80 0', ctx.server.sent)
        self.assertTrue(any('dx=-4' in command for command in ctx.server.sent))

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

    def test_border_lerp_duration_is_explicit_vanilla_ticks(self):
        ctx = context()
        with patch.object(world_border, 'ticks') as wait_ticks, patch.object(world_border, 'size', return_value=75):
            world_border.act(ctx, 'set_and_lerp')
        self.assertIn('worldborder set 50 200t', ctx.server.sent)
        self.assertEqual([call.args[1] for call in wait_ticks.call_args_list], [100, 110])
        self.assertEqual(ctx.state['middle_size'], 75)


if __name__ == '__main__':
    unittest.main()
