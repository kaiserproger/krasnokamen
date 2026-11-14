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
from .matrix_api import MissingCapability

from .matrix_domains.random import invariant_checks, mutate_observation, stock, _published, setup, recovery_matches, _mine, _exact_slots, _action, _fixture_item_stock
from .quirk_parity.seeded import (player_scenario, PLAYER_ACTIONS, DETERMINISTIC_ACTIONS,
                                shrink_player_scenario, failure_identity)


class RandomMatrixTests(unittest.TestCase):
    def test_seed19_adapter_consumes_received_packet_transitions(self):
        import socket
        from . import wire
        from .protocol_matrix_client import ProtocolMatrixClient
        a,b=socket.socketpair()
        observer=None
        try:
            with patch.object(wire,'join_offline_socket',return_value=(a,{})):
                observer=wire.PlayObserver(('unused',0),'Probe',account_uuid=None,timeout=1)
            client=object.__new__(ProtocolMatrixClient)
            client.observer=observer; client.items={163:'minecraft:oak_log',63:'minecraft:oak_planks'}
            client.teleport_to=lambda *args:None
            client.open_inventory=lambda:None
            send=lambda packet,body:wire.send_compressed_frame(b,packet,bytes.fromhex(body))
            slots=[b'\x00']*46; slots[36]=bytes.fromhex('01a3010000')
            wire.send_compressed_frame(b,0x12,bytes([0,1,46])+b''.join(slots)+b'\x00')
            observer.wait(lambda p:0 in p.menus,description='initial player menu')
            def click(slot,**kwargs):
                if slot==36:
                    send(0x14,'0002002400'); send(0x60,'01a3010000')
                elif slot==1:
                    send(0x14,'00040000043f0000')
                elif slot==0:
                    send(0x14,'0006000000')
                else:
                    raise AssertionError(slot)
            client.click_slot=click
            deliver=[True]
            def wait(predicate,*,description,timeout_seconds=30):
                if description=='received crafting input and cursor':
                    self.assertFalse(predicate())
                    if deliver[0]:
                        send(0x14,'0005000101a3010000'); send(0x60,'00')
                return observer.wait(lambda _:predicate(),description=description,timeout=0.1)
            client.wait_until=wait
            ctx=SimpleNamespace(clients=[client])
            with patch('tools.harness.matrix_domains.random._supply'), \
                 patch('tools.harness.matrix_domains.random._close'):
                _action(ctx,{'actor':0,'verb':'craft','count':2,'target':0})
            self.assertEqual(observer.menus[0]['state_id'],6)
            self.assertIsNone(observer.menus[0]['slots'][0])
            deliver[0]=False
            wire.send_compressed_frame(b,0x12,bytes([0,1,46])+b''.join(slots)+b'\x00')
            observer.wait(lambda p:p.menus[0]['state_id']==1,description='reset menu')
            with patch('tools.harness.matrix_domains.random._supply'), \
                 patch('tools.harness.matrix_domains.random._close'):
                with self.assertRaisesRegex(TimeoutError,'received crafting input and cursor'):
                    _action(ctx,{'actor':0,'verb':'craft','count':2,'target':0})
        finally:
            if observer is not None: observer.close()
            else: a.close()
            b.close()

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
            if description == 'received crafting input and cursor':
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
    def test_replacing_dirt_moves_actor_out_of_target_before_placing(self):
        positions = []
        states = iter(['minecraft:dirt', 'minecraft:air', 'minecraft:dirt'])
        actor = SimpleNamespace(
            teleport_to=lambda *pos: positions.append(pos),
            read_block_state=lambda pos: {'block_id': next(states)},
            wait_until=lambda predicate, **kwargs: self.assertTrue(predicate()),
            wait_ticks=lambda count: None, open_inventory=lambda: None,
            select_hotbar=lambda slot: None,
            use_item_on=lambda pos: self.assertEqual(positions[-1], (34.5, 80, 32.5)))
        ctx = SimpleNamespace(clients=[actor], state={'touched': []})
        with patch('tools.harness.matrix_domains.random._mine'), \
             patch('tools.harness.matrix_domains.random._supply'), \
             patch('tools.harness.matrix_domains.random._slot', return_value=36), \
             patch('tools.harness.matrix_domains.random._close'):
            _action(ctx, {'actor': 0, 'verb': 'place', 'count': 1, 'target': 2})

    def test_ground_stock_only_counts_physical_fixture_area(self):
        entities = [
            {'position': [33, 80, 34], 'item': {'item_id': 'minecraft:diamond', 'count': 2}},
            {'position': [33, 63, 34], 'item': {'item_id': 'minecraft:kelp', 'count': 14}},
        ]
        self.assertEqual(_fixture_item_stock(entities), {'minecraft:diamond': 2})
        with self.assertRaisesRegex(MissingCapability, 'random:received_item_position'):
            _fixture_item_stock([{'item': {'item_id': 'minecraft:dirt', 'count': 1}}])
        with self.assertRaisesRegex(MissingCapability, 'random:received_item_stack'):
            _fixture_item_stock([{'type': 'minecraft:item', 'position': [33, 80, 34]}])


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
    def test_nightly_shrink_runs_each_replay_in_separate_scope(self):
        row = player_scenario(1, 1, 2)
        report = {'status': 'fail', 'note': 'random:conservation:step=0: planted', 'row': row}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            script = root / 'input.json'
            script.write_text(json.dumps(row))
            ctx = SimpleNamespace(artifact_dir=root / 'outer', extra_args=['--script', str(script), '--shrink'],
                                  server_bin=Path('/pin/server'), plugins_dir=None, timeout_seconds=600,
                                  commands=[], cwds=[], logs=[], exits=[], client_evidence={})
            ctx.artifact_dir.mkdir()
            launches = []

            def launch(argv, *, cwd, env, stdout, stderr, timeout):
                launches.append(argv)
                self.assertEqual(argv[:11], ['/mem-run', '-m', '6G', 'timeout', '600',
                                              'nice', '-n', '19', 'ionice', '-c', '3'])
                self.assertIsNone(timeout)
                self.assertNotIn('KRASNOKAMEN_RANDOM_REPLAY_MEM_RUN', env)
                receipt = Path(env['KRASNOKAMEN_VALIDATION_ROOT']) / 'run' / 'result.json'
                receipt.parent.mkdir(parents=True)
                receipt.write_text(json.dumps({'details': {'protocol_matrix': report}}))
                return SimpleNamespace(returncode=1)

            with patch.dict('os.environ', {'KRASNOKAMEN_RANDOM_REPLAY_MEM_RUN': '/mem-run'}), \
                    patch.object(matrix_random.shutil, 'disk_usage',
                                 return_value=SimpleNamespace(free=100 * 1024**3)), \
                    patch.object(matrix_random.subprocess, 'run', side_effect=launch), \
                    patch.object(matrix_random, 'run_protocol_matrix') as unscoped:
                with self.assertRaisesRegex(RuntimeError, 'planted'):
                    matrix_random.run(ctx)
            unscoped.assert_not_called()
            self.assertGreaterEqual(len(launches), 5)
            self.assertEqual(ctx.commands, launches)
            result = json.loads((ctx.artifact_dir / 'shrink' / 'result.json').read_text())
            self.assertEqual(result['status'], 'reproduced')
            self.assertEqual(len(result['confirmations']), 3)
            self.assertTrue(all(Path(item['receipt']).is_file()
                                for item in result['trials'] + result['confirmations']))

    def test_unwrapped_replay_keeps_subprocess_timeout(self):
        with tempfile.TemporaryDirectory() as directory:
            ctx = SimpleNamespace(extra_args=[], server_bin=Path('/pin/server'), plugins_dir=None,
                                  timeout_seconds=37, commands=[], cwds=[], logs=[], exits=[])

            def launch(argv, *, cwd, env, stdout, stderr, timeout):
                self.assertEqual(timeout, 37)
                receipt = Path(env['KRASNOKAMEN_VALIDATION_ROOT']) / 'run' / 'result.json'
                receipt.parent.mkdir(parents=True)
                receipt.write_text(json.dumps({'details': {'protocol_matrix': {'status': 'pass'}}}))
                return SimpleNamespace(returncode=0)

            with patch.dict('os.environ', {}, clear=True), \
                    patch.object(matrix_random.shutil, 'disk_usage',
                                 return_value=SimpleNamespace(free=100 * 1024**3)), \
                    patch.object(matrix_random.subprocess, 'run', side_effect=launch):
                report, _, _ = matrix_random.replay(ctx, {}, Path(directory) / 'trial')
            self.assertEqual(report['status'], 'pass')

    def test_reduction_refuses_world_launch_below_disk_floor(self):
        with tempfile.TemporaryDirectory() as directory:
            ctx = SimpleNamespace(extra_args=[], server_bin=Path('/pin/server'), plugins_dir=None,
                                  timeout_seconds=600, commands=[], cwds=[], logs=[], exits=[])
            owner = Path('/run/media/kaiserroman/games/solaris-work/wt/scenario')
            with patch.object(matrix_random.runtime, 'REPO_ROOT', owner), \
                    patch.object(matrix_random.shutil, 'disk_usage',
                                 return_value=SimpleNamespace(free=100 * 1024**3 - 1)), \
                    patch.object(matrix_random.subprocess, 'run') as launch:
                with self.assertRaisesRegex(RuntimeError, 'less than 100 GiB'):
                    matrix_random.replay(ctx, {}, Path(directory) / 'trial')
            launch.assert_not_called()


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
        snapshots = iter([{'entities': [{'type': 'minecraft:item'}]},
                          {'entities': []}])
        def wait(predicate, **kwargs):
            self.assertFalse(predicate())
            self.assertTrue(predicate())
            events.append('empty_ground')
        actor = SimpleNamespace(teleport_to=lambda *args: events.append('teleport'),
                                ensure_loaded=lambda *args: None,
                                snapshot=lambda: next(snapshots), wait_until=wait)
        server = SimpleNamespace(ensure_loaded=lambda *args: events.append('loaded'),
                                 command=lambda text: events.append(text))
        setup(SimpleNamespace(clients=[actor, actor], server=server, state={}))
        self.assertLess(events.index('loaded'), events.index('fill 28 79 28 40 79 40 stone'))
        self.assertIn('gamerule spawn_mobs false', events)
        self.assertLess(events.index('setblock 34 80 32 chest'), events.index('kill @e[type=item]'))
        self.assertLess(events.index('kill @e[type=item]'), events.index('empty_ground'))

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
    def test_published_fence_skips_batch_retired_during_read(self):
        with tempfile.TemporaryDirectory() as directory:
            world = Path(directory)
            journal = world / 'journal'
            journal.mkdir()
            vanished = journal / 'batch-1-1.bin'
            current = journal / 'batch-2-12.bin'
            payload = b'{}\n'
            current.write_bytes(struct.pack('<IIQQII', 0x534F4C4A, 1, 2, 12, len(payload),
                                            zlib.crc32(payload)) + payload)
            original_read = Path.read_bytes
            def read(path):
                if path == vanished:
                    raise FileNotFoundError(path)
                return original_read(path)
            ctx = SimpleNamespace(server=SimpleNamespace(world_dir=world))
            with patch.object(Path, 'glob', return_value=iter([vanished, current])), \
                    patch.object(Path, 'read_bytes', autospec=True, side_effect=read), \
                    patch.object(matrix_random.runtime, 'wait_for_path_condition',
                                 side_effect=lambda path, timeout, predicate: predicate()[1]):
                self.assertEqual(_published(ctx, 12)['high'], 12)



if __name__ == '__main__':
    unittest.main()
