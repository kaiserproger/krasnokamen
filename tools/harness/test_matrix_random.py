"""Seed reproducibility and independent fault controls for every invariant."""
import copy
import json
from pathlib import Path
import struct
import tempfile
from types import SimpleNamespace
import unittest
import zlib
from unittest.mock import patch

from .backends import matrix_random
from .quirk_parity import player

from .matrix_domains.random import invariant_checks, mutate_observation, stock, _published, setup, recovery_matches, _mine, _exact_slots, _action
from .quirk_parity.fuzz import (player_scenario, PLAYER_ACTIONS, DETERMINISTIC_ACTIONS,
                                shrink_player_scenario, failure_identity)


class RandomMatrixTests(unittest.TestCase):
    def test_differential_rejects_planted_inventory_divergence(self):
        state = {'inventories': [{}], 'container': {}, 'blocks': {}}
        self.assertTrue(player.compare_player_states(state, state)['passed'])
        self.assertFalse(player.compare_player_states(state, state, fault=True)['passed'])

    def test_vanilla_game_time_fence_parses_actual_feedback(self):
        oracle = object.__new__(player.VanillaMatrixServer)
        oracle.query = lambda _: 'System chat: The game time is 642 tick(s)'
        self.assertEqual(oracle.dashboard_stats(), {'simulation_tick': 642})

    def test_vanilla_network_fixture_disables_default_whitelist(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            jar = root / 'server.jar'
            jar.write_bytes(b'test')
            ctx = SimpleNamespace(artifact_dir=root, client_evidence={}, logs=[])
            with patch.object(player, 'VanillaMatrixServer') as server:
                server.return_value.start.side_effect = RuntimeError('stop after configuration')
                with self.assertRaisesRegex(RuntimeError, 'stop after configuration'):
                    player.run_player_script(ctx, player_scenario(7, 1, 2), jar, root)
            self.assertEqual(server.call_args.kwargs['properties']['white-list'], 'false')
            settings = json.loads(server.call_args.kwargs['properties']['generator-settings'])
            self.assertEqual(settings['layers'], [{'height': 1, 'block': 'minecraft:stone'}])

    def test_mining_waits_for_exact_ack_including_sequence_zero(self):
        snapshots = iter([{'block_ack_sequence': None}, {'block_ack_sequence': 0}])
        actor = SimpleNamespace(look_at=lambda *args: None, break_block=lambda *args, **kwargs: 0,
                                snapshot=lambda: next(snapshots), wait_ticks=lambda count: None)
        def wait(predicate, **kwargs):
            self.assertFalse(predicate())
            self.assertTrue(predicate())
        actor.wait_until = wait
        _mine(actor, (1, 2, 3))

    def test_craft_waits_for_input_after_early_result_packet(self):
        pending = {'cursor': {'item_id': 'minecraft:oak_log', 'count': 1},
                   'menu': {'slots': [{'slot': 0, 'item_id': 'minecraft:oak_planks'}]}}
        applied = {'cursor': None, 'menu': {'slots': [
            {'slot': 0, 'item_id': 'minecraft:oak_planks'},
            {'slot': 1, 'item_id': 'minecraft:oak_log'}]}}
        state = [pending]
        clicks = []
        def wait(predicate, *, description):
            if description == 'received crafting input':
                self.assertFalse(predicate())
                state[0] = applied
            self.assertTrue(predicate())
        actor = SimpleNamespace(teleport_to=lambda *args: None, open_inventory=lambda: None,
                                snapshot=lambda: state[0], wait_until=wait)
        ctx = SimpleNamespace(clients=[actor])
        with patch('tools.harness.matrix_domains.random._supply'), \
             patch('tools.harness.matrix_domains.random._slot', return_value=36), \
             patch('tools.harness.matrix_domains.random._click',
                   side_effect=lambda actor, slot, **kw: clicks.append(slot)), \
             patch('tools.harness.matrix_domains.random._close'):
            _action(ctx, {'actor': 0, 'verb': 'craft', 'count': 1, 'target': 0})
        self.assertEqual(clicks, [36, 1, 0])

    def test_ddmin_keeps_exact_failure_and_reduces_parameters(self):
        row = player_scenario(1, 40, 4)
        row['actions'][27]['verb'] = 'craft'
        original = copy.deepcopy(row)
        minimal = shrink_player_scenario(row, lambda value:
                                        any(step['verb'] == 'craft' for step in value['actions']))
        self.assertEqual(minimal['actions'], [{'verb': 'craft', 'actor': 0, 'count': 1, 'target': 0}])
        self.assertEqual(row, original)
        self.assertIsNone(failure_identity({'status': 'blocked', 'note': 'random:conservation:step=0:'}))
        self.assertEqual(failure_identity({'status': 'fail', 'note': 'random:conservation:step=22: bad'}),
                         'random:conservation')

    def test_reduced_repro_retains_trial_selectors(self):
        row = player_scenario(1, 1, 2)
        report = {'status': 'fail', 'note': 'random:conservation:step=0: planted', 'row': row}
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / 'shrink').mkdir()
            ctx = SimpleNamespace(artifact_dir=Path(directory))
            argv = ['python3', '-m', 'tools.harness', 'run', 'matrix-random',
                    '--server-bin', '/pin/server', '--script', '/trial/script.json',
                    '--vanilla-data', '/data', '--differential', '--vanilla-jar', '/vanilla.jar',
                    '--plugins-dir', '/plugins']
            with patch.object(matrix_random, 'replay', return_value=(report, '/trial/result.json', argv)):
                result = matrix_random.shrink(ctx, report)
            self.assertEqual(result['status'], 'reproduced')
            self.assertIn('--differential', result['repro_command'])
            self.assertIn('--vanilla-jar /vanilla.jar', result['repro_command'])
            self.assertIn('--plugins-dir /plugins', result['repro_command'])
            self.assertIn(result['minimal_script'], result['repro_command'])

    def test_recovery_rejects_lost_published_transfer_and_craft_equivalent_substitution(self):
        committed = {'inventories': [{}, {'minecraft:oak_log': 1}],
                     'container': {}, 'blocks': {'32,80,34': 'minecraft:dirt'}}
        cursor = {'item_id': 'minecraft:dirt', 'count': 8}
        recovered = copy.deepcopy(committed)
        recovered['inventories'][0] = {'minecraft:dirt': 8}
        self.assertTrue(recovery_matches(committed, recovered, 0, cursor))
        lost_transfer = copy.deepcopy(committed)
        lost_transfer['container'] = {'minecraft:dirt': 8}
        self.assertFalse(recovery_matches(committed, lost_transfer, 0, cursor))
        wrong_item = copy.deepcopy(recovered)
        wrong_item['inventories'][1] = {'minecraft:oak_planks': 4}
        self.assertFalse(recovery_matches(committed, wrong_item, 0, cursor))

    def test_exact_post_image_distinguishes_slots_and_components(self):
        image = _exact_slots('[{Slot: 2b, id: "minecraft:dirt", count: 1, '
                             'components: {"example:key": {count: 5}}}]')
        self.assertNotEqual(image, _exact_slots('[{Slot: 3b, id: "minecraft:dirt", count: 1, '
                                                'components: {"example:key": {count: 5}}}]'))
        self.assertNotEqual(image, _exact_slots('[{Slot: 2b, id: "minecraft:dirt", count: 1, '
                                                'components: {"example:key": {count: 6}}}]'))
        with self.assertRaises(ValueError):
            _exact_slots('[{Slot: 2b, id: "minecraft:dirt"}, {Slot: 2b, id: "minecraft:stone"}]')


    def test_fixture_commands_follow_received_chunk_loading(self):
        events = []
        actor = SimpleNamespace(teleport_to=lambda *args: events.append('teleport'),
                                ensure_loaded=lambda *args: None)
        server = SimpleNamespace(ensure_loaded=lambda *args: events.append('loaded'),
                                 command=lambda text: events.append(text))
        setup(SimpleNamespace(clients=[actor, actor], server=server, state={}))
        self.assertLess(events.index('loaded'), events.index('fill 28 79 28 40 79 40 stone'))
        self.assertIn('gamerule spawn_mobs false', events)

    def observation(self):
        authority = {'inventories': [{'minecraft:dirt': 4}], 'container': {}, 'blocks': {}}
        return {'running': True, 'errors': [], 'resources': {'earth': 4},
                'sources': {'earth': 4}, 'refusals': [], 'progress': True,
                'authority': authority, 'observer': copy.deepcopy(authority), 'recovery_ok': True,
                'recovery': {'exact_post_image': True, 'actor_index': 0, 'cursor': None,
                             'cursor_recovery_ok': True,
                             'expected_exact_image': {'inventories': [{}]},
                             'recovered_exact_image': {'inventories': [{}]},
                             'expected_committed_image': copy.deepcopy(authority),
                             'recovered_image': copy.deepcopy(authority)}}

    def test_recovery_is_not_reported_without_a_restart(self):
        observation = self.observation()
        observation['recovery'] = None
        self.assertNotIn('recovery_atomic_durable',
                         [check.name for check in invariant_checks(observation)])

    def test_every_oracle_passes_valid_state_and_rejects_its_own_mutation(self):
        good = self.observation()
        self.assertTrue(all(check.passed for check in invariant_checks(good)))
        for check in invariant_checks(good):
            with self.subTest(oracle=check.name):
                mutated = mutate_observation(good, check.name)
                failed = [result.name for result in invariant_checks(mutated) if not result.passed]
                self.assertEqual(failed, [check.name])
                self.assertTrue(all(result.passed for result in invariant_checks(good)))

    def test_seed_and_serialized_actions_replay_exactly(self):
        first = player_scenario(101, 100, 4)
        self.assertEqual(first, player_scenario(101, 100, 4))
        self.assertNotEqual(first['actions'], player_scenario(102, 100, 4)['actions'])
        self.assertEqual(set(PLAYER_ACTIONS) | {'concurrent'}, {a['verb'] for a in first['actions']})
        self.assertTrue(all(a['actor'] < 3 for a in first['actions']))
        self.assertTrue(set(a['verb'] for a in player_scenario(101, 100, 4, differential=True)['actions'])
                        <= set(DETERMINISTIC_ACTIONS))

    def test_explicit_resource_units_preserve_crafting_inputs_and_outputs(self):
        self.assertEqual(stock([{'item_id': 'minecraft:oak_log', 'count': 2}]),
                         stock([{'item_id': 'minecraft:oak_planks', 'count': 8}]))
        self.assertNotEqual(stock([{'item_id': 'minecraft:oak_log', 'count': 2}]),
                            stock([{'item_id': 'minecraft:oak_planks', 'count': 9}]))
        self.assertEqual(stock([{'item_id': 'minecraft:diamond', 'count': 1}]),
                         {'minecraft:diamond': 1})

    def test_published_wal_fence_checks_framing_checksum_and_tick(self):
        with tempfile.TemporaryDirectory() as directory:
            world = Path(directory)
            journal = world / 'journal'
            journal.mkdir()
            payload = b'{}\n'
            path = journal / 'batch-1-12.bin'
            header = struct.pack('<IIQQII', 0x534F4C4A, 1, 1, 12, len(payload), zlib.crc32(payload))
            path.write_bytes(header + payload)
            ctx = SimpleNamespace(server=SimpleNamespace(world_dir=world))
            self.assertEqual(_published(ctx, 12)['high'], 12)
            path.write_bytes(header + b'bad')
            with self.assertRaisesRegex(AssertionError, 'invalid published'):
                _published(ctx, 12)


if __name__ == '__main__':
    unittest.main()
