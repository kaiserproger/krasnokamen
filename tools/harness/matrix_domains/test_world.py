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

    def test_command_echo_is_never_a_success_oracle(self):
        ctx = self.context('MatrixConsoleFence00000001')
        with self.assertRaisesRegex(MissingCapability, 'console_scoreboard_numeric_feedback'):
            _world.condition(ctx, 'block 8 100 8 water')

    def test_condition_requires_exact_holder_score(self):
        ctx = self.context('dw1 has 0 [dw_probe]')
        self.assertFalse(_world.condition(ctx, 'block 8 100 8 water'))
        ctx = self.context('dw1 has 1 [dw_probe]')
        self.assertTrue(_world.condition(ctx, 'block 8 100 8 water'))
        ctx = self.context('dw11 has 1 [dw_probe]')
        with self.assertRaises(MissingCapability):
            _world.condition(ctx, 'block 8 100 8 water')

    def test_unloaded_fixture_is_blocked_not_gameplay_failure(self):
        ctx = self.context('Position is not loaded')
        with self.assertRaisesRegex(MissingCapability, 'fixture_chunks_loaded'):
            _world.command(ctx, 'fill 0 99 0 47 99 31 stone')

    def test_unknown_observations_fail_closed(self):
        ctx = self.context()
        with self.assertRaisesRegex(MissingCapability, 'snapshot:inventory'):
            _world.inventory(ctx, 'stone')
        with self.assertRaisesRegex(MissingCapability, 'client:move'):
            _world.verb(ctx, 'move', forward=1)

    def test_data_query_rejects_old_binary_surface(self):
        ctx = self.context('Supported: data get|merge block <x> <y> <z> [compound] (command blocks only)')
        with self.assertRaisesRegex(MissingCapability, 'data_get_entity'):
            _world.data(ctx, 'Proto0', 'Pos')

    def test_data_scalar_and_numeric_list_decode(self):
        for text, expected in [('20.0f', 20.0), ('1b', 1),
                               ('[1.0d, 100.0d, -2.0d]', [1, 100, -2]),
                               ('[I; 1, 2, 3, 4]', [1, 2, 3, 4]),
                               ('"minecraft:stone"', 'minecraft:stone')]:
            ctx = self.context('Proto0 has the following entity data: ' + text)
            self.assertEqual(expected, _world.data(ctx, 'Proto0', 'value'))

    def test_unsupported_compound_decode_never_defaults_empty(self):
        ctx = self.context('Proto0 has the following entity data: {X:1,Y:2,Z:3}')
        with self.assertRaisesRegex(MissingCapability, 'data_snbt_decode'):
            _world.data(ctx, 'Proto0', 'leash')

    def test_boundary_is_once_and_not_during_setup(self):
        ctx = self.context()
        _world.boundary(ctx)
        ctx.boundary.assert_not_called()
        ctx.state['world_act'] = True
        _world.boundary(ctx)
        _world.boundary(ctx)
        ctx.boundary.assert_called_once()

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

    def test_missing_nbt_path_cannot_pass_as_empty_list(self):
        ctx = self.context('Nothing found at path Passengers')
        ctx.server.query.return_value = 'dw_size has 0 [dw_probe]'
        with self.assertRaisesRegex(MissingCapability, 'data_path:Passengers'):
            _world.data_size(ctx, '@e[limit=1]', 'Passengers')
        ctx.server.query.assert_not_called()

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

    def test_nested_uuid_arrays_decode_without_inventing_compound_values(self):
        ctx = self.context('Fox has the following entity data: [[I;1,2,-3,4]]')
        self.assertEqual([[1, 2, -3, 4]], _world.data(ctx, 'fox', 'Trusted'))

    def test_comparator_covers_fullness_and_all_frame_rotations(self):
        from . import redstone
        ctx = self.context()
        ctx.clients[0].teleport_to = Mock()
        ctx.server.query.side_effect = lambda _: f"dw{ctx.state['query_index']} has 1 [dw_probe]"
        with patch.object(redstone, 'ticks'), patch.object(redstone, 'verb'), \
                patch.object(redstone, '_load_container') as load:
            redstone.comparator_modes(ctx)
        self.assertEqual([64, 864, 1728], [call.args[-1] for call in load.call_args_list])
        names = {entry['name'] for entry in ctx.state['world_checks']}
        for strength in (*range(9), 15):
            self.assertIn(f'block:(9, 100, 16):redstone_wire[power={strength}]', names)

    def test_dispenser_runs_all_item_families_and_checks_residue(self):
        from . import redstone
        ctx = self.context()
        ctx.clients[0].teleport_to = Mock()
        ctx.clients[0].open_container = Mock()
        ctx.clients[0].snapshot = lambda: {
            'entities': [{'type': 'minecraft:' + kind}
                         for kind in ('arrow', 'small_fireball', 'tnt', 'minecart', 'oak_boat')] + [
                {'type': 'minecraft:item', 'item': {'item_id': 'minecraft:honeycomb', 'count': 3}}],
            'menu': {'slots': [{'slot': 0, 'item_id': 'minecraft:shears', 'count': 1,
                               'components': {'minecraft:damage': 1}}]}}
        ctx.server.query.side_effect = lambda _: f"dw{ctx.state['query_index']} has 1 [dw_probe]"
        residue = lambda ctx, pos, slots, item: int(item in ('bucket', 'water_bucket', 'shears'))
        with patch.object(redstone, 'ticks'), patch.object(redstone, 'verb'), \
                patch.object(redstone, '_load_container') as load, \
                patch.object(redstone, '_container_count', side_effect=residue), \
                patch.object(redstone, 'entity_field', return_value='minecraft:iron_chestplate'):
            redstone.dispenser_behaviours(ctx)
        self.assertEqual(13, load.call_count)
        self.assertTrue(all(call.args[1] == (3, 100, 16) for call in load.call_args_list))
        checks = {entry['name']: entry['passed'] for entry in ctx.state['world_checks']}
        for item in ('water_bucket', 'lava_bucket', 'powder_snow_bucket', 'bucket',
                     'bone_meal', 'shulker_box', 'shears', 'iron_chestplate'):
            self.assertTrue(checks['dispenser-custody:' + item])
        self.assertTrue(checks['dispenser-equips-armor'])
        self.assertTrue(checks['dispenser-three-honeycombs'])
        self.assertTrue(checks['dispenser-shears-wear'])
        for item in ('arrow', 'fire_charge', 'tnt', 'minecart', 'oak_boat'):
            self.assertTrue(checks['dispenser-spawn-count:' + item])

    def test_hopper_lock_checks_all_three_custody_locations(self):
        from . import redstone
        ctx = self.context()
        ctx.clients[0].teleport_to = Mock()
        ctx.clients[0].open_container = Mock()
        ctx.clients[0].snapshot = lambda: {'menu': {'slots': [
            {'slot': 1, 'item_id': 'minecraft:coal', 'count': 2}]}}
        ctx.server.query.side_effect = lambda _: f"dw{ctx.state['query_index']} has 1 [dw_probe]"
        with patch.object(redstone, 'ticks'), patch.object(redstone, 'verb'), \
                patch.object(redstone, '_load_container'), \
                patch.object(redstone, '_container_count', side_effect=[2, 0, 0, 0, 0, 0, 0, 2]):
            with self.assertRaisesRegex(MissingCapability, 'server_tick_container_transfers'):
                redstone.hopper_chains(ctx)
        self.assertEqual(3, sum(entry['name'].startswith('locked-hopper-custody:')
                                for entry in ctx.state['world_checks']))
        self.assertTrue(all(entry['passed'] for entry in ctx.state['world_checks']))

    def test_sculk_latches_distance_frequency_and_rejection_after_pulse(self):
        from . import redstone
        ctx = self.context()
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
        self.assertEqual([45, 42, 45, 46, 45, 42, 45, 42, 45, 52],
                         [call.args[1] for call in wait.call_args_list])

    def test_repeater_cannot_claim_delay_from_settled_output(self):
        from . import redstone
        ctx = self.context()
        ctx.server.query.side_effect = lambda _: f"dw{ctx.state['query_index']} has 1 [dw_probe]"
        with patch.object(redstone, 'ticks'):
            with self.assertRaisesRegex(MissingCapability, 'server_tick_block_updates'):
                redstone.repeater_delay_lock(ctx)
        self.assertEqual(12, len(ctx.state['world_checks']))

    def test_taming_rejects_nonempty_wrong_owner_and_wrong_fox_trust(self):
        from . import farming
        ctx = self.context()
        ctx.clients[0].name = 'Fixture'
        uuid = [1, 2, 3, 4]
        other = [5, 6, 7, 8]
        foxes = [{'type': 'minecraft:fox', 'id': 1, 'baby': False},
                 {'type': 'minecraft:fox', 'id': 2, 'baby': False}]
        baby = {'type': 'minecraft:fox', 'id': 3, 'baby': True, 'uuid': 'actual-fox-uuid'}
        fields = lambda ctx, kind, field: {
            'Owner': other, 'Sitting': 1, 'Pos': [18.5, 100, 3.5], 'Tame': True}[field]
        with patch.object(farming, 'data', side_effect=[uuid, [other]]), \
                patch.object(farming, 'entity_field', side_effect=fields), \
                patch.object(farming, 'observe', side_effect=[foxes, foxes + [baby]]), \
                patch.object(farming, 'interact'), patch.object(farming, 'give'), \
                patch.object(farming, 'verb'), patch.object(farming, 'ticks'):
            farming.taming(ctx)
        checks = {entry['name']: entry['passed'] for entry in ctx.state['world_checks']}
        for kind in ('wolf', 'cat', 'parrot', 'horse', 'llama'):
            self.assertFalse(checks[kind + ':owner'])
        self.assertFalse(checks['fox-offspring-trusts-breeder'])


    def test_entity_position_and_health_use_received_packets_not_nbt(self):
        ctx = self.context()
        ctx.clients[0].snapshot = lambda: {'entities': [
            {'id': 7, 'type': 'minecraft:cow', 'position': [8, 100, 8], 'health': 17}]}
        self.assertEqual([8, 100, 8], _world.entity_field(ctx, 'cow', 'Pos'))
        self.assertEqual(17, _world.entity_field(ctx, 'cow', 'Health'))
        ctx.server.command.assert_not_called()

    def test_unreceived_entity_health_is_not_invented(self):
        ctx = self.context()
        ctx.clients[0].snapshot = lambda: {'entities': [{'id': 7, 'type': 'minecraft:cow'}]}
        with self.assertRaisesRegex(MissingCapability, 'snapshot:entities.health'):
            _world.entity_field(ctx, 'cow', 'Health')
        ctx.server.command.assert_not_called()

    def test_egg_oracle_reads_received_entity_stack(self):
        from . import farming
        ctx = self.context()
        ctx.clients[0].snapshot = lambda: {'entities': [
            {'id': 8, 'type': 'minecraft:item',
             'item': {'item_id': 'minecraft:egg', 'count': 1}}]}
        with patch.object(farming, 'ticks'), patch.object(farming, 'exists'):
            farming.chicken_eggs(ctx)
        self.assertTrue(ctx.state['world_checks'][0]['passed'])

    def test_experience_and_mending_use_received_xp_and_stack_components(self):
        from . import entities_misc
        ctx = self.context()
        ctx.clients[0].name = 'Fixture'
        ctx.clients[0].snapshot = lambda: {'xp': {'total': 1}}
        ctx.wait_until = lambda predicate, **kwargs: self.assertTrue(predicate())
        tool = {'item_id': 'minecraft:diamond_pickaxe', 'count': 1,
                'components': {'minecraft:damage': 14}}
        with patch.object(entities_misc, 'ticks'), patch.object(entities_misc, 'give'), \
                patch.object(entities_misc, 'observe', side_effect=[1, 7, [tool]]):
            entities_misc.experience_orbs(ctx)
        self.assertEqual(2, len(ctx.state['world_checks']))
        self.assertTrue(all(check['passed'] for check in ctx.state['world_checks']))
        self.assertFalse(any('data get' in call.args[0] for call in ctx.server.command.call_args_list))


    def test_bee_crop_oracle_excludes_random_growth(self):
        from . import farming
        ctx = self.context()
        with patch.object(farming, 'ticks'), \
                patch.object(farming, 'condition', side_effect=lambda ctx, pred: 'age=0' not in pred):
            farming.bees_pollinate(ctx)
        commands = [call.args[0] for call in ctx.server.command.call_args_list]
        disabled = next(i for i, cmd in enumerate(commands) if 'random_tick_speed 0' in cmd)
        planted = next(i for i, cmd in enumerate(commands) if 'wheat[age=0]' in cmd)
        self.assertLess(disabled, planted)
        self.assertTrue(any('10 102 8 beehive' in cmd for cmd in commands))
        self.assertEqual(2, len(ctx.state['world_checks']))
        self.assertTrue(all(check['passed'] for check in ctx.state['world_checks']))

    def test_seagrass_fixture_is_underwater_outside_shop(self):
        from . import farming
        ctx = self.context()
        ctx.factors['game_mode'] = 'survival'
        ctx.clients[0].teleport_to = Mock()
        with patch.object(farming, 'ticks'), patch.object(farming, 'growth'), \
                patch.object(farming, 'block'), patch.object(farming, 'use') as use:
            farming.kelp_seagrass(ctx)
        commands = [call.args[0] for call in ctx.server.command.call_args_list]
        water = next(i for i, cmd in enumerate(commands) if 'fill 3 100 16 3 101 16 water' in cmd)
        seagrass = next(i for i, cmd in enumerate(commands) if 'minecraft:seagrass' in cmd)
        self.assertLess(water, seagrass)
        ctx.clients[0].teleport_to.assert_called_once_with(2.5, 100, 13.5)
        use.assert_called_once_with(ctx, (3, 100, 16), 'bone_meal')

    def test_daylight_and_bulb_cover_night_and_second_pulse(self):
        from . import redstone
        ctx = self.context()
        with patch.object(redstone, 'ticks'), patch.object(redstone, 'block') as block:
            redstone.daylight_target_bulb_lamp(ctx)
        states = [call.args[2] for call in block.call_args_list]
        self.assertEqual(['daylight_detector[power=15]', 'daylight_detector[power=0]'], states[:2])
        self.assertEqual(['copper_bulb[lit=true]', 'copper_bulb[lit=true]',
                          'copper_bulb[lit=false]', 'copper_bulb[lit=false]'], states[2:6])
        self.assertEqual(['redstone_lamp[lit=true]', 'redstone_lamp[lit=false]',
                          'redstone_lamp[lit=true]', 'redstone_lamp[lit=false]'], states[6:])

    def test_missing_item_metadata_cannot_be_an_empty_egg_result(self):
        from . import farming
        ctx = self.context()
        ctx.clients[0].snapshot = lambda: {'entities': [{'id': 8, 'type': 'minecraft:item'}]}
        with patch.object(farming, 'ticks'), patch.object(farming, 'exists'):
            with self.assertRaisesRegex(MissingCapability, 'snapshot:entities.item'):
                farming.chicken_eggs(ctx)


    def test_weighted_plate_thresholds_and_arrow_button_rules(self):
        from . import redstone
        ctx = self.context()
        ctx.factors['game_mode'] = 'survival'
        ctx.clients[0].teleport_to = Mock()
        with patch.object(redstone, 'ticks'), patch.object(redstone, 'use'), \
                patch.object(redstone, 'block') as block:
            redstone.pressure_plates_buttons(ctx)
        checks = {call.kwargs.get('name'): call.args[2] for call in block.call_args_list}
        self.assertEqual('oak_button[powered=true]', checks['oak_button:arrow'])
        self.assertEqual('stone_button[powered=false]', checks['stone_button:arrow'])
        for count, power in ((1, 1), (10, 1), (11, 2), (150, 15)):
            self.assertEqual(f'heavy_weighted_pressure_plate[power={power}]',
                             checks[f'heavy_weighted_pressure_plate:{count}-entities'])
        for count, power in ((1, 1), (14, 14), (15, 15), (16, 15)):
            self.assertEqual(f'light_weighted_pressure_plate[power={power}]',
                             checks[f'light_weighted_pressure_plate:{count}-entities'])
        self.assertEqual('stone_pressure_plate[powered=true]',
                         checks['stone_pressure_plate:living-entity'])

    def test_portal_checks_every_minimum_and_maximum_interior_cell(self):
        from . import portals_dimensions as portals
        ctx = self.context()
        ctx.factors['game_mode'] = 'survival'
        ctx.clients[0].teleport_to = Mock()
        with patch.object(portals, 'ticks'), patch.object(portals, 'use'), \
                patch.object(portals, 'block') as block:
            portals.nether_portal_build_light(ctx)
        interiors = [call for call in block.call_args_list if call.kwargs.get('name', '').startswith('frame:')]
        self.assertEqual(6 + 441, len(interiors))
        maximum = {call.args[1] for call in interiors if call.kwargs['name'].startswith('frame:23x23')}
        self.assertEqual({(x, y, 16) for x in range(11, 32) for y in range(101, 122)}, maximum)
        self.assertTrue(all(call.args[2] == 'nether_portal[axis=x]' for call in interiors))
        self.assertTrue(any(call.kwargs.get('name') == 'oversize-frame-does-not-light'
                            and call.args[2] == 'air' for call in block.call_args_list))

    def test_command_block_chain_origin_and_repeat_do_not_claim_output(self):
        from . import redstone
        ctx = self.context()
        with patch.object(redstone, 'ticks'), patch.object(redstone, 'block') as block:
            with self.assertRaisesRegex(MissingCapability, 'command_block_output'):
                redstone.command_block(ctx)
        checks = {call.kwargs['name']: (call.args[1], call.args[2]) for call in block.call_args_list}
        self.assertEqual(((8, 100, 10), 'air'), checks['command_block:held-power'])
        self.assertEqual(((8, 100, 10), 'gold_block'), checks['repeating_command_block:held-power'])
        self.assertEqual(((9, 100, 10), 'diamond_block'), checks['command_block:chain'])
        commands = [call.args[0] for call in ctx.server.command.call_args_list]
        self.assertTrue(any('setblock ~ ~ ~2 minecraft:gold_block' in cmd for cmd in commands))
        self.assertTrue(any('auto:1b' in cmd for cmd in commands))

    def test_berry_fixture_observes_growth_before_harvesting(self):
        from . import farming
        ctx = self.context()
        ctx.factors['game_mode'] = 'survival'
        ctx.clients[0].teleport_to = Mock()
        with patch.object(farming, 'growth', return_value=True) as growth, \
                patch.object(farming, 'give'), patch.object(farming, 'use'), \
                patch.object(farming, 'block'):
            farming.sweet_berries_cocoa_glow_berries(ctx)
        self.assertEqual('block 3 100 16 sweet_berry_bush[age=3]', growth.call_args_list[0].args[1])
        self.assertEqual('sweet-berry-matures', ctx.state['world_checks'][0]['name'])
        self.assertTrue(ctx.state['world_checks'][0]['passed'])

    def test_shearing_counts_dropped_and_picked_up_wool(self):
        from . import farming
        for dropped, expected in ((2, True), (3, False)):
            with self.subTest(dropped=dropped):
                ctx = self.context()
                ctx.factors['game_mode'] = 'survival'
                ctx.wait_until = lambda predicate, **kwargs: self.assertTrue(predicate())
                ctx.clients[0].snapshot = lambda: {'entities': [
                    {'type': 'minecraft:item', 'item': {'item_id': 'minecraft:white_wool', 'count': dropped}}]}
                with patch.object(farming, 'ticks'), patch.object(farming, 'interact'), \
                        patch.object(farming, 'inventory', return_value=1), \
                        patch.object(farming, 'entity_field', side_effect=[True, False, False]):
                    farming.shear_sheep_wool_regrow(ctx)
                self.assertEqual(expected, ctx.state['world_checks'][0]['passed'])


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
