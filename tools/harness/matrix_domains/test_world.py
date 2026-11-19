"""Contract tests, not live gameplay evidence; run with unittest."""
import importlib
import json
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from ..matrix_api import Check, MissingCapability
from . import _world

DOMAINS = {'farming': 18, 'redstone': 17, 'fluids': 12, 'entities_misc': 11,
           'weather_time': 8, 'portals_dimensions': 11, 'movement': 15}


class WorldAdapterTests(unittest.TestCase):
    def context(self, response=''):
        server = SimpleNamespace(command=Mock(return_value=response),
                                 query=Mock(return_value=response))
        return SimpleNamespace(server=server, factors={'dimension': 'overworld'},
                               state={'query_index': 0, 'world_checks': []},
                               clients=[SimpleNamespace(snapshot=lambda: {})],
                               boundary=Mock())

    def test_all_assigned_subjects_have_real_adapters(self):
        found = set()
        for domain, count in DOMAINS.items():
            module = importlib.import_module(__package__ + '.' + domain)
            self.assertEqual(count, len(module.ADAPTERS), domain)
            for name, adapter in module.ADAPTERS.items():
                self.assertTrue(name.startswith(domain + '.'))
                self.assertTrue(callable(adapter.setup))
                self.assertTrue(callable(adapter.act))
                self.assertTrue(callable(adapter.oracle))
            found.update(module.ADAPTERS)
        self.assertEqual(92, len(found))
        matrix = Path('/run/media/kaiserroman/games/solaris-work/scenarios/matrix.yaml')
        if matrix.is_file():
            import yaml
            expected = {row['subject'] for row in yaml.safe_load(matrix.read_text())['scenarios']
                        if row['domain'] in DOMAINS}
            self.assertEqual(expected, found)

    def test_checks_are_json_serializable_in_receipt_state(self):
        ctx = self.context()
        _world.check(ctx, 'water', False, False, True, 'Water')
        json.dumps(ctx.state)
        checks = _world.oracle(ctx)
        self.assertEqual([False], [check.passed for check in checks])
        self.assertIsInstance(checks[0], Check)

    def test_unloaded_fixture_is_blocked_not_gameplay_failure(self):
        ctx = self.context('Position is not loaded')
        with self.assertRaisesRegex(MissingCapability, 'fixture_chunks_loaded'):
            _world.command(ctx, 'fill 0 99 0 47 99 31 stone')

    def test_water_flow_checks_nearest_drop_as_well_as_spread_and_fall(self):
        from . import fluids
        ctx = self.context()
        ctx.server.query.side_effect = lambda _: f"dw{ctx.state['query_index']} has 1 [dw_probe]"
        with patch.object(fluids, 'ticks'):
            fluids.water_flow(ctx)
        names = {entry['name'] for entry in ctx.state['world_checks']}
        self.assertEqual(12, len(names))
        for name in ('block:(23, 100, 16):water[level=7]',
                     'block:(24, 100, 16):air',
                     'block:(18, 98, 16):water',
                     'block:(36, 100, 16):water[level=1]',
                     'block:(37, 99, 16):water',
                     'block:(34, 100, 16):air'):
            self.assertIn(name, names)

    def test_setup_fences_all_fixture_chunks_before_fill(self):
        ctx = self.context()
        ctx.clients[0].name = 'Fixture'
        ctx.clients[0].teleport_to = Mock()
        ctx.server.ensure_loaded = Mock()
        ctx.server.command.side_effect = lambda _: ctx.server.ensure_loaded.assert_called_once_with(
            ((0, 96, 0), (47, 115, 31))) or ''
        _world.setup(ctx)

    def test_block_input_boundary_requires_received_sequence(self):
        ctx = self.context()
        ctx.state['world_act'] = True
        ctx.clients[0].look_at = Mock()
        ctx.clients[0].use_item_on = Mock(return_value=7)
        ctx.wait_until = lambda predicate, **kwargs: predicate()
        with self.assertRaisesRegex(MissingCapability, 'block_ack_sequence'):
            _world.use(ctx, (3, 99, 3))
        ctx.boundary.assert_not_called()
        ctx.clients[0].use_item_on.assert_called_once_with((3, 99, 3), 'up')

    def test_block_ack_waits_past_old_or_absent_packet(self):
        ctx = self.context()
        ctx.state['world_act'] = True
        ctx.clients[0].snapshot = Mock(side_effect=[
            {'block_ack_sequence': None}, {'block_ack_sequence': 6},
            {'block_ack_sequence': 7}])
        def wait(predicate, **kwargs):
            self.assertFalse(predicate())
            self.assertFalse(predicate())
            ctx.boundary.assert_not_called()
            self.assertTrue(predicate())
        ctx.wait_until = wait
        _world.block_ack(ctx, 7)
        ctx.boundary.assert_called_once()
        self.assertEqual([], ctx.state['world_checks'])

    def test_verb_submits_bound_arguments_through_shared_receipt_hook(self):
        ctx = self.context()
        ctx.state['world_act'] = True
        def attack(entity_id):
            self.fail('helper must not bypass shared staging')
        ctx.clients[0].attack = attack
        ctx.concurrently = Mock(return_value=[{'tick': 42, 'outcome': 'rejected'}])
        receipts = _world.verb(ctx, 'attack', 7)
        ctx.concurrently.assert_called_once_with([(ctx.clients[0], 'attack', {'entity_id': 7})])
        ctx.boundary.assert_called_once()
        self.assertEqual('rejected', receipts[0]['outcome'])
        self.assertEqual([], ctx.state['world_checks'])

    def test_sponge_fixture_keeps_actor_outside_pool_and_spawn_shop(self):
        from . import fluids
        ctx = self.context()
        ctx.factors['game_mode'] = 'survival'
        ctx.clients[0].teleport_to = Mock()
        ctx.server.query.side_effect = lambda _: f"dw{ctx.state['query_index']} has 1 [dw_probe]"
        with patch.object(fluids, 'give'), patch.object(fluids, 'verb'), patch.object(fluids, 'use') as use:
            fluids.sponge(ctx)
        ctx.clients[0].teleport_to.assert_called_once_with(2.5, 100, 13.5)
        use.assert_called_once_with(ctx, (3, 99, 16))
        self.assertEqual({'block:(3, 100, 16):wet_sponge', 'block:(3, 101, 16):air'},
                         {entry['name'] for entry in ctx.state['world_checks']})

    def test_sculk_latches_distance_frequency_and_rejection_after_pulse(self):
        from . import redstone
        ctx = self.context()
        ctx.clients[0].call = Mock(side_effect=lambda _tool, args: {
            'block': {'block_id': args['block_id'], 'properties': dict(args['properties'])}})
        with patch.object(redstone, 'ticks') as wait, patch.object(redstone, 'block') as block:
            redstone.sculk_sensor_signal(ctx)
        cases = [(False, 2, 0, 12), (False, 6, 0, 4),
                 (True, 2, 10, 14), (True, 2, 8, 0), (True, 12, 10, 4)]
        calls = {call.kwargs.get('name'): call.args[1:] for call in block.call_args_list}
        for calibrated, distance, selected, strength in cases:
            label = f'calibrated={calibrated}:distance={distance}:selected={selected}'
            for index in range(1, 16):
                self.assertEqual(((24+index, 99, 16),
                                  f'copper_bulb[lit={str(index <= strength).lower()}]'),
                                 calls[f'{label}:strength-latch:{index}'])
                self.assertEqual(((23-index, 99, 16),
                                  f'copper_bulb[lit={str(strength > 0 and index <= 10).lower()}]'),
                                 calls[f'{label}:frequency-latch:{index}'])
            self.assertEqual(((27, 100, 17), 'copper_bulb[lit=false]'),
                             calls[f'{label}:perpendicular-control'])
        calls_received = [entry.args[1] for entry in ctx.clients[0].call.call_args_list]
        self.assertEqual(155, sum(args['block_id'] == 'minecraft:copper_bulb' for args in calls_received))
        phases = [args['properties']['sculk_sensor_phase'] for args in calls_received
                  if 'sculk_sensor_phase' in args['properties']]
        self.assertEqual(['active', 'cooldown', 'inactive'] * 4, phases)
        self.assertEqual([45, 42, 45, 46, 45, 42, 45, 42, 45, 52],
                         [call.args[1] for call in wait.call_args_list])

    def test_non_inverted_detector_is_zero_in_nether_and_end(self):
        from . import redstone
        for dimension in ('the_nether', 'the_end'):
            with self.subTest(dimension=dimension):
                ctx = self.context()
                ctx.factors['dimension'] = dimension
                with patch.object(redstone, 'ticks'), patch.object(redstone, 'block') as block:
                    redstone.daylight_target_bulb_lamp(ctx)
                self.assertEqual('daylight_detector[power=0]', block.call_args_list[0].args[2])


if __name__ == '__main__':
    unittest.main()
