"""Focused adapter regressions; these are not live matrix receipts."""
from pathlib import Path
from types import SimpleNamespace
import json
import unittest
from unittest.mock import patch

from tools.harness.matrix_api import MatrixContext, MissingCapability
from tools.harness.matrix_domains import persistence, session, world_chunks


def context(state=None, query=None):
    server = SimpleNamespace(query=query or (lambda _: 'Unknown or incomplete command'))
    row = {'id': 'unit', 'subject': 'persistence.save_all_flush', 'context': {
        'dimension': 'the_end', 'permission': 'op_2', 'game_mode': 'survival',
        'server_lifecycle': 'steady', 'network_condition': 'normal'}}
    return MatrixContext(row, server, [], Path('/unused-unit-artifacts'), state or {})


def save_context(state):
    baseline = {'block_token': 'token', 'block_output': '[Server] token',
                'score_output': 'sentinel has 17 [lc_save]', 'before_tick': 10,
                'after_stats': {'simulation_tick': 11}, 'chat_token': 'chat-token',
                'chat_events': {'Matrix0': ['chat-token']}}
    ctx = context(dict(baseline, **state))
    ctx.clients = [SimpleNamespace(name='Matrix0')]
    return ctx


class LifecycleOracles(unittest.TestCase):
    def test_absent_data_command_blocks_exact_capability(self):
        with self.assertRaisesRegex(MissingCapability, '^data_get_entity$'):
            session.data(context(), 'entity actor', 'Inventory')

    def test_command_block_only_data_refusal_has_exact_capability(self):
        output = 'Supported: data get|merge block <x> <y> <z> [compound] (command blocks only)'
        for target, capability in (('entity actor', 'data_get_entity'), ('block 1 2 3', 'data_get_block')):
            with self.subTest(target=target), self.assertRaisesRegex(MissingCapability, '^' + capability + '$'):
                session.data(context(query=lambda _: output), target)

    def test_console_fence_is_not_data(self):
        ctx = context(query=lambda _: 'chat.type.announcement: Server, MatrixConsoleFence1')
        with self.assertRaisesRegex(MissingCapability, 'data_get_block:decoded_feedback'):
            session.data(ctx, 'block 1 2 3', 'Items')

    def test_op2_save_requires_refusal_not_saved_output(self):
        state = {'negative': True, 'command_roots': ['say'],
                 'refusal_events': ['Unknown or incomplete command'], 'save_output': ''}
        self.assertTrue(all(c.passed for c in persistence._save_oracle(save_context(state))))
        state['save_output'] = 'Saved the game'
        self.assertFalse(all(c.passed for c in persistence._save_oracle(save_context(state))))

    def test_save_root_must_not_be_advertised_to_op2(self):
        state = {'negative': True, 'command_roots': ['save-all'],
                 'refusal_events': ['Unknown or incomplete command'], 'save_output': ''}
        self.assertFalse(all(c.passed for c in persistence._save_oracle(save_context(state))))

    def test_command_echo_is_not_block_condition_success(self):
        state = {'negative': False, 'save_output': 'Saved the game',
                 'block_token': 'token', 'block_output': 'execute if block 1 2 3 gold_block run say token\nMatrixConsoleFence1',
                 'score_output': 'sentinel has 17 [lc_save]', 'before_tick': 10,
                 'after_stats': {'simulation_tick': 11}, 'chat_token': 'chat-token',
                 'chat_events': {'Matrix0': ['chat-token']}}
        self.assertFalse(all(c.passed for c in persistence._save_oracle(save_context(state))))
        state['block_output'] = 'chat.type.announcement: Server, token\n'
        self.assertTrue(all(c.passed for c in persistence._save_oracle(save_context(state))))

    def test_player_roundtrip_loss_fails(self):
        before = {key: 'same' for key in persistence._PLAYER_FIELDS}
        after = dict(before, EnderItems='[]')
        checks = persistence._player_oracle(context({
            'player_before': before, 'player_after': after,
            'restored_ender_menu': {'slots': [{'slot': 0, 'item_id': 'minecraft:diamond', 'count': 7}]}}))
        self.assertEqual([c.name for c in checks if not c.passed], ['player_roundtrip.EnderItems'])

    def test_forced_furnace_requires_exact_one_output(self):
        state = {'items': '[{Slot:2b,id:"minecraft:iron_ingot",count:2}]',
                 'force_query': 'Chunk is marked for force loading',
                 'old_rule_output': 'Unknown or incomplete command'}
        self.assertFalse(all(c.passed for c in world_chunks._force_oracle(context(state))))
        state['items'] = '[{Slot:2b,id:"minecraft:iron_ingot",count:1}]'
        self.assertTrue(all(c.passed for c in world_chunks._force_oracle(context(state))))
        state['old_rule_output'] = 'spawnChunkRadius = 2'
        self.assertFalse(all(c.passed for c in world_chunks._force_oracle(context(state))))

    def test_equal_empty_roundtrip_does_not_prove_custody(self):
        empty = {key: '[]' for key in persistence._PLAYER_FIELDS}
        checks = persistence._player_oracle(context({
            'player_before': empty, 'player_after': empty, 'restored_ender_menu': {'slots': []}}))
        self.assertEqual([c.name for c in checks if not c.passed], ['player_roundtrip.diamond_custody'])
        fields = {key: 'same' for key in session._REJOIN_FIELDS}
        checks = session._rejoin_oracle(context({'leave': fields, 'after': fields, 'rejoin_diamonds': 0}))
        self.assertEqual([c.name for c in checks if not c.passed], ['rejoin.diamond_custody'])

    def test_rcon_oracle_requires_auth_and_no_unauth_mutation(self):
        state = {'wrong_auth': {'id': -1, 'kind': 2, 'text': ''},
                 'unauth_command': {'id': -1, 'kind': 2, 'text': ''},
                 'unauth_score': 'sentinel has 3 [lc_rcon]',
                 'valid_auth': {'id': 44, 'kind': 2, 'text': ''},
                 'rcon_get': {'id': 46, 'kind': 0, 'text': 'sentinel has 17 [lc_rcon]'},
                 'rcon_score': 'sentinel has 17 [lc_rcon]'}
        self.assertTrue(all(c.passed for c in session._rcon_oracle(context(state))))
        state['unauth_score'] = 'sentinel has 999 [lc_rcon]'
        self.assertEqual([c.name for c in session._rcon_oracle(context(state)) if not c.passed],
                         ['rcon.unauth_no_mutation'])

    def test_save_setup_keeps_json_state(self):
        ctx = context()
        ctx.row['context']['permission'] = 'console'
        ctx.server.log_path = Path('/unused-unit-server.log')
        ctx.server.command = lambda _: ''
        ctx.server.dashboard_stats = lambda: {'simulation_tick': 1}
        ctx.clients = [SimpleNamespace(name='Matrix0', snapshot=lambda: {'position': [1, 80, 1], 'events': []},
                                       chat=lambda _: None)]
        persistence._save_setup(ctx)
        json.dumps(ctx.state, allow_nan=False)
        self.assertEqual(ctx.state['log_path'], '/unused-unit-server.log')

    def test_ban_requires_both_rejections_and_conserved_inventory(self):
        state = {'name_ban': {'packet': 0, 'reason': {'text': 'matrix-name-ban'}},
                 'ip_ban': {'packet': 0, 'reason': {'text': 'matrix-ip-ban'}},
                 'name_pardon_diamonds': 7, 'ip_pardon_diamonds': 7,
                 'ban_chat': {'Matrix0': ['echo']}}
        ctx = context(state)
        ctx.clients = [SimpleNamespace(name='Matrix0')]
        self.assertTrue(all(c.passed for c in session._ban_oracle(ctx)))
        state['ip_pardon_diamonds'] = 14
        state['name_ban']['packet'] = 2
        self.assertEqual(sum(not c.passed for c in session._ban_oracle(ctx)), 2)

    def test_denied_save_still_requires_unchanged_world_and_observer_progress(self):
        ctx = save_context({'negative': True, 'command_roots': ['say'],
                            'refusal_events': ['Unknown or incomplete command'], 'save_output': '',
                            'score_output': 'sentinel has 999 [lc_save]'})
        ctx.clients.append(SimpleNamespace(name='Matrix1'))
        self.assertEqual([c.name for c in persistence._save_oracle(ctx) if not c.passed],
                         ['save.score_preserved', 'save.client_progress'])

    def test_unfenced_crash_accepts_only_atomic_ordered_custody_prefixes(self):
        baseline = {field: 'durable-baseline' for field in persistence._PLAYER_FIELDS}
        def crash_context(inventory, ender, emeralds):
            slots = [{'slot': slot, 'item_id': item, 'count': count}
                     for slot, item, count in ((27, 'minecraft:diamond', inventory),
                                               (0, 'minecraft:diamond', ender),
                                               (28, 'minecraft:emerald', emeralds)) if count]
            ctx = context({'player_baseline': baseline, 'player_after': dict(baseline),
                           'restored_ender_menu': {'slots': slots}, 'restored_cursor': None})
            ctx.row['context']['server_lifecycle'] = 'crash_kill9_wal_replay'
            return ctx
        for prefix in ((7, 0, 0), (0, 7, 0), (0, 7, 3)):
            with self.subTest(recoverable_prefix=prefix):
                self.assertTrue(all(c.passed for c in persistence._player_oracle(crash_context(*prefix))))
        for prefix in ((0, 0, 0), (7, 7, 0), (3, 4, 0), (7, 0, 3), (0, 7, 1)):
            with self.subTest(torn_or_invalid_prefix=prefix):
                self.assertFalse(all(c.passed for c in persistence._player_oracle(crash_context(*prefix))))
        ctx = crash_context(0, 7, 0)
        ctx.state['player_after']['XpLevel'] = 'lost durable baseline'
        self.assertFalse(all(c.passed for c in persistence._player_oracle(ctx)))
        ctx = crash_context(0, 7, 0)
        ctx.state['restored_cursor'] = {'item_id': 'minecraft:diamond', 'count': 7}
        self.assertFalse(all(c.passed for c in persistence._player_oracle(ctx)))

    def test_crash_action_reads_recovery_without_post_kill_save_or_restart(self):
        calls = []
        menu = {'slots': [{'slot': 27, 'item_id': 'minecraft:diamond', 'count': 7}]}
        def click(*args, **kwargs):
            calls.append('transfer')
            menu['slots'] = [{'slot': 0, 'item_id': 'minecraft:diamond', 'count': 7}]
        def killed(ctx):
            calls.append('kill9')
            # A valid lost-tail recovery: all seven remain in player inventory.
            menu['slots'] = [{'slot': 27, 'item_id': 'minecraft:diamond', 'count': 7}]
        actor = SimpleNamespace(name='Matrix0', snapshot=lambda: {'menu': menu, 'cursor': None},
                                open_container=lambda _: calls.append('open'),
                                close_container=lambda: calls.append('close'), click_slot=click,
                                wait_until=lambda predicate, **kwargs: predicate())
        ctx = context({'ender': [1, 80, 1]})
        ctx.row['context']['server_lifecycle'] = 'crash_kill9_wal_replay'
        ctx.clients = [actor]
        ctx.boundary_hook = killed
        ctx.server.command = lambda _: calls.append('grant')
        def forbidden():
            self.fail('mid-action kill9 must not be followed by a masking save/restart')
        ctx.server.save_all = forbidden
        ctx.server.clean_restart = forbidden
        def read_player(*args):
            calls.append('read-recovered')
            return {field: 'baseline' for field in persistence._PLAYER_FIELDS}
        with patch.object(persistence, 'player_data', read_player):
            persistence._player_act(ctx)
        self.assertEqual(calls, ['open', 'transfer', 'close', 'grant', 'kill9', 'read-recovered', 'open'])
        self.assertNotIn('player_before', ctx.state)
        self.assertEqual(ctx.state['restored_ender_menu']['slots'][0]['slot'], 27)


if __name__ == '__main__':
    unittest.main()
