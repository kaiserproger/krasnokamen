"""Focused adapter regressions; these are not live matrix receipts."""
from pathlib import Path
from types import SimpleNamespace
import io
import json
import struct
import unittest
from unittest.mock import patch

from tools.harness.matrix_api import MatrixContext, MissingCapability
from tools.harness.matrix_domains import load, persistence, session, world_chunks


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
    def test_registry_covers_exact_subject_counts(self):
        for module, count in ((session, 21), (persistence, 7), (world_chunks, 10), (load, 22)):
            self.assertEqual(len(module.ADAPTERS), count)
            self.assertTrue(all(name.startswith(module.__name__.rsplit('.', 1)[1] + '.') for name in module.ADAPTERS))

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

    def test_data_query_is_dimension_scoped(self):
        seen = []
        def query(text):
            seen.append(text)
            return '1, 2, 3 has the following block data: [{id:"minecraft:diamond",count:7}]\n'
        value = session.data(context(query=query), 'block 1 2 3', 'Items')
        self.assertEqual(value, '[{id:"minecraft:diamond",count:7}]')
        self.assertEqual(seen, ['execute in minecraft:the_end run data get block 1 2 3 Items'])

    def test_missing_observation_is_not_empty(self):
        with self.assertRaises(MissingCapability):
            session.required({'inventory': None}, 'inventory')
        self.assertEqual(session.required({'inventory': []}, 'inventory'), [])

    def test_unavailable_load_does_not_return_pass(self):
        with self.assertRaisesRegex(MissingCapability, 'catalog_load_matrix_context_bridge:MIX-08'):
            load.ADAPTERS['load.MIX-08'].setup(context())
        with self.assertRaisesRegex(MissingCapability, 'native_generation_revision_oracle'):
            load.ADAPTERS['load.CHUNK-01'].setup(context())

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

    def test_protocol_mismatch_checks_both_versions_and_roster(self):
        reply = lambda version: {'protocol': version, 'packet': 0,
                                  'reason': {'translate': 'multiplayer.disconnect.incompatible', 'with': ['26.1.2']}}
        state = {'mismatch_replies': [reply(774), reply(776)], 'after_roster': 1,
                 'status_stats': {'players': 1}}
        self.assertTrue(all(c.passed for c in session._mismatch_oracle(context(state))))
        state['after_roster'] = 2
        self.assertFalse(all(c.passed for c in session._mismatch_oracle(context(state))))

    def test_equal_empty_roundtrip_does_not_prove_custody(self):
        empty = {key: '[]' for key in persistence._PLAYER_FIELDS}
        checks = persistence._player_oracle(context({
            'player_before': empty, 'player_after': empty, 'restored_ender_menu': {'slots': []}}))
        self.assertEqual([c.name for c in checks if not c.passed], ['player_roundtrip.diamond_custody'])
        fields = {key: 'same' for key in session._REJOIN_FIELDS}
        checks = session._rejoin_oracle(context({'leave': fields, 'after': fields, 'rejoin_diamonds': 0}))
        self.assertEqual([c.name for c in checks if not c.passed], ['rejoin.diamond_custody'])

    def test_rcon_frame_and_fragmented_response(self):
        packet = session._rcon_frame(46, 0, 'sentinel has 17 [lc_rcon]')
        stream = io.BytesIO(packet)
        connection = SimpleNamespace(recv=lambda n: stream.read(min(n, 2)))
        self.assertEqual(session._rcon_reply(connection),
                         {'id': 46, 'kind': 0, 'text': 'sentinel has 17 [lc_rcon]'})
        stream = io.BytesIO(packet[:-1])
        with self.assertRaises(EOFError):
            session._rcon_reply(SimpleNamespace(recv=stream.read))

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

    def test_query_basic_full_and_session_identity(self):
        session_id = b'\x01\x02\x03\x04'
        challenge = b'\x09' + session_id + b'12345\0'
        basic = (b'\x00' + session_id + b'MOTD\0SMP\0world\0' + b'1\0' + b'20\0'
                 + struct.pack('<H', 26540) + b'127.0.0.1\0')
        full = (b'\x00' + session_id + b'splitnum\0\x80\0hostname\0MOTD\0version\0' +
                b'26.1.2\0\0\x01player_\0\0Matrix0\0\0')
        for extended, packet in ((False, basic), (True, full)):
            with self.subTest(full=extended):
                replies = iter((challenge, packet))
                sent = []
                decoded = session._query_stat(SimpleNamespace(send=sent.append, recv=lambda _: next(replies)),
                                              full=extended, session_id=session_id)
                self.assertEqual(decoded['hostname'], 'MOTD')
                self.assertEqual(sent[1][7:11], struct.pack('>i', 12345))
                if extended:
                    self.assertEqual(decoded['players'], ['Matrix0'])
                    self.assertEqual(decoded['version'], '26.1.2')
                else:
                    self.assertEqual(decoded['hostport'], '26540')
        replies = iter((challenge, b'\x00BAD!' + basic[5:]))
        with self.assertRaisesRegex(ValueError, 'identity mismatch'):
            session._query_stat(SimpleNamespace(send=lambda _: None, recv=lambda _: next(replies)),
                                full=False, session_id=session_id)

    def test_query_oracle_checks_recovery_and_roster(self):
        expected = {'hostname': 'MOTD', 'gametype': 'SMP', 'map': 'world',
                    'numplayers': '1', 'maxplayers': '20', 'hostport': '26540', 'hostip': '127.0.0.1'}
        state = {'admin_config': {'data': {'world_dir': '/fixture/world'},
                                 'network': {'port': 26540, 'bind_address': '127.0.0.1'}},
                 'admin_stats': {'world': {'motd': 'MOTD'}, 'players': {'max': 20}},
                 'query_basic': expected.copy(), 'query_after_malformed': expected.copy(),
                 'query_full': dict(expected, version='26.1.2', game_id='MINECRAFT', players=['Matrix0'])}
        ctx = context(state)
        ctx.clients = [SimpleNamespace(name='Matrix0')]
        self.assertTrue(all(c.passed for c in session._query_oracle(ctx)))
        state['query_after_malformed']['numplayers'] = '0'
        state['query_full']['players'] = []
        self.assertEqual(sum(not c.passed for c in session._query_oracle(ctx)), 2)

    def test_whitelist_requires_real_refusal_and_received_play_progress(self):
        state = {'whitelist_refusal': {'packet': 0, 'reason': {'translate': 'multiplayer.disconnect.not_whitelisted'}},
                 'whitelist_play': [1, 80, 1], 'whitelist_roster': 1, 'whitelist_chat': {'Matrix0': ['echo']}}
        ctx = context(state)
        ctx.clients = [SimpleNamespace(name='Matrix0')]
        self.assertTrue(all(c.passed for c in session._whitelist_oracle(ctx)))
        state['whitelist_refusal']['packet'] = 2
        self.assertFalse(all(c.passed for c in session._whitelist_oracle(ctx)))
        state['whitelist_refusal']['packet'] = 0
        state['whitelist_chat'] = {}
        self.assertFalse(all(c.passed for c in session._whitelist_oracle(ctx)))

    def test_max_players_rejects_extra_without_losing_existing_clients(self):
        state = {'full_refusal': {'packet': 0, 'reason': {'translate': 'multiplayer.disconnect.server_full'}},
                 'full_limit': 1, 'full_roster': 1, 'full_chat': {'Matrix0': ['echo']}}
        ctx = context(state)
        ctx.clients = [SimpleNamespace(name='Matrix0')]
        self.assertTrue(all(c.passed for c in session._full_oracle(ctx)))
        state['full_roster'] = 2
        self.assertEqual([c.name for c in session._full_oracle(ctx) if not c.passed], ['full.existing_roster'])

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

    def test_dashboard_requires_observer_echo_and_real_rss(self):
        state = {'after_stats': {'players': {'count': 2}, 'simulation_tick': 12,
                                 'tps': 20, 'tick': {'p50_us': 100, 'p99_us': 500}},
                 'chat_tick': 11, 'dashboard_chat': {'Matrix0': ['echo']}}
        ctx = context(state)
        ctx.clients = [SimpleNamespace(name='Matrix0'), SimpleNamespace(name='Matrix1')]
        ctx.server.process_stats = lambda: {'rss_bytes': 1024}
        self.assertEqual([c.name for c in session._dashboard_oracle(ctx) if not c.passed], ['dashboard.chat_echo'])
        state['dashboard_chat']['Matrix1'] = ['echo']
        self.assertTrue(all(c.passed for c in session._dashboard_oracle(ctx)))
        del ctx.server.process_stats
        with self.assertRaisesRegex(MissingCapability, '^observe:server_rss$'):
            session._dashboard_oracle(ctx)

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

    def test_stubs_name_actual_remaining_capabilities(self):
        cases = [
            (session, 'session.join_fresh_spawn', 'snapshot:chunk_collision_shapes'),
            (session, 'session.mute_warn', 'observe:command_roots'),
            (session, 'session.connection_limit', 'same_tick_admission_fence'),
            (session, 'session.locale_presentation', 'observe:localized_dialog'),
            (persistence, 'persistence.block_entities_roundtrip', 'data_get_block'),
            (persistence, 'persistence.entities_roundtrip', 'data_get_entity'),
            (world_chunks, 'world_chunks.anti_xray', 'snapshot:chunk_block_state'),
        ]
        for module, subject, capability in cases:
            with self.subTest(subject=subject), self.assertRaisesRegex(MissingCapability, '^' + capability + '$'):
                module.ADAPTERS[subject].setup(context())

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
