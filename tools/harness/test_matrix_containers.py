"""Pure adapter contract regressions; these do not claim live scenario coverage."""
from pathlib import Path
from types import SimpleNamespace
import json
import unittest
from unittest.mock import patch

from .matrix_api import MatrixContext, MissingCapability
from .matrix_domains import containers as c


class ContainerContractTests(unittest.TestCase):
    def context(self, snapshot=None, reply=''):
        snapshot = {} if snapshot is None else snapshot
        client = SimpleNamespace(snapshot=lambda: snapshot)
        server = SimpleNamespace(query=lambda text: reply)
        ctx = MatrixContext({'subject': 'containers.chest', 'context': {
            'network_condition': 'normal', 'server_lifecycle': 'steady', 'game_mode': 'survival'}},
            server, [client], Path('/run/media/kaiserroman/games/solaris-work/scenarios/runs/DomContainers'))
        ctx.state.update(subject='chest', checks=[])
        return ctx

    def test_every_catalog_subject_has_an_adapter_and_source(self):
        expected = set('player_inventory_2x2 crafting_table chest trapped_chest double_chest barrel '
                       'copper_chest ender_chest shulker_box hopper dispenser dropper crafter furnace '
                       'smoker blast_furnace brewing_stand enchanting_table anvil grindstone smithing_table '
                       'loom stonecutter cartography_table beacon merchant_villager merchant_wandering_trader '
                       'chest_boat chest_minecart hopper_minecart plugin_chest_menu mount_inventory lectern'.split())
        self.assertEqual(set(c.SOURCES), expected)
        self.assertEqual(set(c.ADAPTERS), {'containers.' + s for s in expected})
        self.assertEqual(len(expected), 33)

    def test_missing_menu_is_not_empty_storage(self):
        with self.assertRaisesRegex(MissingCapability, 'observation.menu'):
            c._slot(self.context(), 0)

    def test_sparse_empty_slot_is_zero_only_with_observed_menu(self):
        ctx = self.context({'menu': {'slots': []}})
        self.assertEqual(c._slot(ctx, 0)['count'], 0)

    def test_missing_slot_index_fails_closed(self):
        ctx = self.context({'menu': {'slots': [{'item_id': 'minecraft:stone', 'count': 1}]}})
        with self.assertRaisesRegex(MissingCapability, 'slots.slot'):
            c._slot(ctx, 0)

    def test_menu_capacity_is_not_sparse_list_length(self):
        ctx = self.context({'menu': {'slots': [], 'slot_count': 63}})
        c._size(ctx, 63)
        self.assertTrue(c.oracle(ctx)[0].passed)
        with self.assertRaisesRegex(MissingCapability, 'slot_count'):
            c._size(self.context({'menu': {'slots': []}}), 63)

    def test_components_absent_are_not_unenchanted(self):
        with self.assertRaisesRegex(MissingCapability, 'components'):
            c._components({'item_id': 'minecraft:diamond_sword', 'count': 1})
        self.assertEqual(c._components({'components': {}}), {})

    def test_zero_false_are_valid_observations(self):
        self.assertEqual(c._field({'v': 0}, 'v', 'v'), 0)
        self.assertIs(c._field({'v': False}, 'v', 'v'), False)
        with self.assertRaises(MissingCapability):
            c._field({'v': None}, 'v', 'v')

    def test_missing_verb_names_the_capability(self):
        with self.assertRaisesRegex(MissingCapability, '^client:close_container$'):
            c._call(SimpleNamespace(), 'close_container')

    def test_data_number_requires_actual_numeric_feedback(self):
        ctx = self.context(reply='Items[0].count on block 32, 80, 32 after scale factor of 1.00 is 16')
        self.assertEqual(c._data_number(ctx, 'block 32 80 32', 'Items[0].count', 'data_get_block'), 16)
        for reply in ('Unknown or incomplete command, see below for error',
                      'Data editing currently supports command blocks only\nMatrixConsoleFence'):
            ctx = self.context(reply=reply)
            with self.subTest(reply=reply), self.assertRaisesRegex(MissingCapability, '^data_get_block$'):
                c._data_number(ctx, 'block 32 80 32', 'Items[0].count', 'data_get_block')
        for reply in ('MatrixFence', 'Found no elements matching Items[0].count', ''):
            with self.subTest(reply=reply), self.assertRaises(AssertionError):
                c._data_number(self.context(reply=reply), 'block 32 80 32', 'Items[0].count', 'data_get_block')

    def test_custody_counts_menu_suffix_once(self):
        snapshot = {'inventory': [{'slot': 36, 'item_id': 'minecraft:stone', 'count': 16}],
                    'menu': {'slots': [{'slot': 0, 'item_id': 'minecraft:stone', 'count': 8},
                                       {'slot': 54, 'item_id': 'minecraft:stone', 'count': 8}]}}
        ctx = self.context(snapshot)
        self.assertEqual(c._count(c._slots(ctx), 'stone'), 16)

    def test_no_oracle_is_not_a_pass(self):
        with self.assertRaisesRegex(AssertionError, 'no observations'):
            c.oracle(self.context())

    def test_failures_remain_checks_with_source(self):
        ctx = self.context()
        c._record(ctx, 'custody', 32, 16)
        check = c.oracle(ctx)[0]
        self.assertFalse(check.passed)
        self.assertEqual(check.detail['expected'], 16)
        self.assertEqual(check.detail['source'], c.SOURCES['chest'])

    def test_steady_boundary_uses_shared_once_contract(self):
        ctx = self.context()
        c._boundary(ctx)
        self.assertTrue(ctx._boundary_called)
        with self.assertRaisesRegex(RuntimeError, 'more than once'):
            c._boundary(ctx)

    def test_nonsteady_requires_real_boundary_hook(self):
        ctx = self.context()
        ctx.row['context']['network_condition'] = 'disconnect_during_menu'
        with self.assertRaisesRegex(MissingCapability, 'action_boundary:disconnect_during_menu'):
            c._boundary(ctx)

    def test_fixture_never_fills_before_received_chunk_barrier(self):
        ctx = self.context()
        calls = []
        ctx.server.command = calls.append
        ctx.clients[0].teleport_to = lambda *position: calls.append(('teleport', position))
        with self.assertRaisesRegex(MissingCapability, '^client:wait_for_chunk$'):
            c.setup(ctx, 'chest')
        self.assertEqual(calls[0], 'forceload add 26 26 40 40')
        self.assertEqual(calls[1][0], 'teleport')
        self.assertFalse(any(isinstance(call, str) and call.startswith('fill') for call in calls))

    def test_extra_origins_do_not_inherit_ordinary_transfer_pass(self):
        cases = [('triple:dupe:two_players_same_slot_same_tick', 'same_tick_admission_fence'),
                 ('triple:dupe:break_container_while_open', 'container_break_while_open_custody')]
        for origin, capability in cases:
            ctx = self.context()
            ctx.row['origin'] = origin
            with self.subTest(origin=origin), self.assertRaisesRegex(MissingCapability, capability):
                c.setup(ctx, 'chest')

    def test_recorded_checks_remain_json_serializable(self):
        ctx = self.context()
        c._record(ctx, 'count', 16, 16)
        c._predicate(ctx, 'positive_count', True, {'count': 16})
        restored = json.loads(json.dumps(ctx.state, allow_nan=False))
        self.assertEqual(restored, ctx.state)
        self.assertTrue(all(check.passed for check in c.oracle(ctx)))

    def test_ticks_read_authoritative_facade_counter(self):
        ctx = self.context()
        counters = iter([100, 101, 102])
        ctx.server.dashboard_stats = lambda: {'simulation_tick': next(counters)}
        def wait(predicate, **kwargs):
            self.assertFalse(predicate())
            self.assertTrue(predicate())
        ctx.clients[0].wait_until = wait
        c._ticks(ctx, 2)

    def test_partial_real_adapters_cannot_award_full_subject_pass(self):
        for subject, (capability, clause) in c.CLAUSE_GAPS.items():
            ctx = self.context()
            ctx.state['subject'] = subject
            c._record(ctx, 'implemented_clause', 1, 1)
            with self.subTest(subject=subject), self.assertRaisesRegex(MissingCapability, capability):
                c.oracle(ctx)
            self.assertEqual(ctx.state['unproved_oracle_clause'], clause)

    def test_partial_oracle_keeps_observed_product_failure(self):
        ctx = self.context()
        ctx.state['subject'] = 'hopper'
        c._record(ctx, 'powered_hopper_locked', 0, 3)
        self.assertFalse(c.oracle(ctx)[0].passed)

    def test_exact_ack_requires_real_token_and_observation(self):
        for token, snapshot in [(None, {'revision': 99}), (True, {'block_ack_sequence': 99}),
                                (3, {'revision': 99})]:
            ctx = self.context(snapshot)
            ctx.clients[0].use_item_on = lambda pos, token=token: token
            ctx.clients[0].wait_until = lambda predicate, **kwargs: predicate()
            with self.subTest(token=token), self.assertRaisesRegex(MissingCapability, '^block_ack_sequence$'):
                c._use_on_ack(ctx)

    def test_exact_ack_waits_for_matching_sequence_not_revision(self):
        snapshot = {'block_ack_sequence': None, 'revision': 100}
        ctx = self.context(snapshot)
        ctx.clients[0].use_item_on = lambda pos: 4
        def wait(predicate, **kwargs):
            self.assertFalse(predicate())
            snapshot['block_ack_sequence'] = 3
            self.assertFalse(predicate())
            snapshot['block_ack_sequence'] = 4
            self.assertTrue(predicate())
        ctx.clients[0].wait_until = wait
        c._use_on_ack(ctx)

    def test_furnace_progress_property_indices(self):
        ctx = self.context({'menu': {'properties': {'0': 1590, '1': 1600, '2': 10, '3': 200}}})
        self.assertEqual([c._property(ctx, key) for key in
                          ('burn_remaining', 'burn_total', 'cook_progress', 'cook_total')],
                         [1590, 1600, 10, 200])

    def test_spectator_closed_workstation_is_not_player_menu(self):
        ctx = self.context({'inventory': [], 'menu': {'id': 0, 'slots': [], 'slot_count': 46}})
        ctx.row['context']['game_mode'] = 'spectator'
        ctx.state['subject'] = 'crafting_table'
        with patch.object(c, '_use_on_ack'), patch.object(c, '_ticks'):
            self.assertTrue(c._spectator(ctx))
        checks = {check.name: check for check in c.oracle(ctx)}
        self.assertTrue(checks['spectator_cannot_open_workstation'].passed)

    def test_merchant_uses_received_offer_and_menu_fields(self):
        offer = {'input1': {'item_id': 'minecraft:emerald', 'count': 1}, 'input2': None,
                 'output': {'item_id': 'minecraft:stone', 'count': 1},
                 'uses': 0, 'max_uses': 1, 'special_price': 0, 'demand': 0}
        menu = {'offers': [offer], 'xp': 0, 'show_progress': False}
        ctx = self.context({'menu': menu})
        ctx.state['subject'] = 'merchant_wandering_trader'
        ctx.clients[0].wait_until = lambda predicate, **kwargs: self.assertTrue(predicate())
        paid = []
        funded = False
        def take(context, slot):
            nonlocal funded
            funded = False
            menu['offers'] = [dict(offer, uses=1, out_of_stock=True)]
        def put(context, item, target, start):
            nonlocal funded
            funded = True
            paid.append((item, target))
        def slot(context, index):
            result = index == 2
            count = int(funded and (not result or menu['offers'][0]['uses'] == 0))
            return {'item_id': 'minecraft:stone' if result else 'minecraft:emerald', 'count': count}
        with patch.object(c, '_entity', return_value={'id': 1}), patch.object(c, '_call'), \
             patch.object(c, '_close'), patch.object(c, '_give'), patch.object(c, '_boundary'), \
             patch.object(c, '_put', side_effect=put), \
             patch.object(c, '_slot', side_effect=slot), patch.object(c, '_take', side_effect=take):
            c.merchant(ctx)
        self.assertEqual(paid, [('emerald', 0), ('emerald', 0)])
        self.assertTrue(all(check['passed'] for check in ctx.state['checks']))

    def test_merchant_costs_preserve_optional_second_payment(self):
        offer = {'input1': {'item_id': 'minecraft:emerald', 'count': 1},
                 'input2': {'item_id': 'minecraft:book', 'count': 1},
                 'special_price': 0, 'demand': 0}
        self.assertEqual(c._merchant_costs(offer), [offer['input1'], offer['input2']])
        offer['special_price'] = -1
        with self.assertRaisesRegex(MissingCapability, 'effective_costs'):
            c._merchant_costs(offer)

    def test_components_resolve_authoritative_registry_names(self):
        ctx = self.context({'registries': {
            'minecraft:enchantment': {'87': 'minecraft:vanishing_curse'},
            'minecraft:potion': {'91': 'minecraft:awkward'}}})
        stack = {'components': {'minecraft:enchantments': [[87, 1]],
                                 'minecraft:potion_contents': {'potion': 91}}}
        self.assertEqual(c._enchantments(ctx, stack), {'minecraft:vanishing_curse': 1})
        self.assertEqual(c._potion(ctx, stack), 'minecraft:awkward')

    def test_missing_registry_names_are_not_hardcoded_ids(self):
        stack = {'components': {'minecraft:enchantments': [[87, 1]]}}
        for snapshot in ({}, {'registries': {'minecraft:enchantment': {}}}):
            with self.subTest(snapshot=snapshot), self.assertRaisesRegex(MissingCapability, 'observation.registries'):
                c._enchantments(self.context(snapshot), stack)

    def test_native_named_components_need_no_fabricated_registry(self):
        ctx = self.context()
        stack = {'components': {'minecraft:enchantments': {'minecraft:vanishing_curse': 1},
                                 'minecraft:potion_contents': {'potion': 'minecraft:awkward'}}}
        self.assertEqual(c._enchantments(ctx, stack), {'minecraft:vanishing_curse': 1})
        self.assertEqual(c._potion(ctx, stack), 'minecraft:awkward')

    def test_crafter_waits_for_all_received_property_packets(self):
        properties = {index: 0 for index in range(8)}
        ctx = self.context({'menu': {'properties': properties}})
        def wait(predicate, **kwargs):
            self.assertFalse(predicate())
            properties[8] = 1
            self.assertTrue(predicate())
        ctx.clients[0].wait_until = wait
        self.assertEqual(c._property(ctx, 'disabled_slots'), [8])

    def test_ordinary_second_client_needs_received_subject_evidence(self):
        ctx = self.context()
        ctx.clients.append(SimpleNamespace())
        c._record(ctx, 'deposit_16', 16, 16)
        with self.assertRaisesRegex(MissingCapability, '^observer_subject_receipt$'):
            c.oracle(ctx)
        c._record(ctx, 'viewer_1_storage', 16, 16)
        self.assertTrue(all(check.passed for check in c.oracle(ctx)))
        ctx.state['subject'] = 'crafting_table'
        with self.assertRaisesRegex(MissingCapability, '^observer_subject_receipt$'):
            c.oracle(ctx)

    def test_packet_xp_and_native_xp_are_actual_observations(self):
        self.assertEqual(c._xp(self.context({'xp': {'level': 27, 'total': 900}}), 'level'), 27)
        self.assertEqual(c._xp(self.context({'experience_total': 12}), 'total'), 12)
        with self.assertRaises(MissingCapability):
            c._xp(self.context(), 'total')

    def test_packet_effects_resolve_received_registry(self):
        ctx = self.context({'effects': {'71': {'amplifier': 0, 'duration_ticks': 100}},
                            'registries': {'minecraft:mob_effect': {'71': 'minecraft:speed'}}})
        self.assertEqual(c._effects(ctx), [
            {'id': 'minecraft:speed', 'amplifier': 0, 'duration_ticks': 100}])
        self.assertEqual(c._effects(self.context({'effects': {}})), [])

    def test_drop_custody_counts_inventory_and_entities_once(self):
        stack = {'item_id': 'minecraft:stone', 'count': 9}
        ctx = self.context({'inventory': [stack],
                            'entities': [{'type': 'minecraft:item', 'item': dict(stack, count=7)},
                                         {'type': 'minecraft:oak_chest_boat'}]})
        self.assertEqual(c._world_stock(ctx, 'stone'), 16)
        ctx.clients[0].snapshot()['entities'][0].pop('item')
        with self.assertRaisesRegex(MissingCapability, 'observation.entities.item'):
            c._world_stock(ctx, 'stone')

    def test_cooking_places_fuel_before_starting_recipe(self):
        ctx = self.context()
        ctx.state['subject'] = 'furnace'
        ctx.clients[0].wait_until = lambda predicate, **kwargs: predicate()
        placed = []
        with patch.object(c, '_give'), patch.object(c, '_open'), patch.object(c, '_size'), \
                patch.object(c, '_put', side_effect=lambda ctx, item, *a: placed.append(item)), \
                patch.object(c, '_property', side_effect=RuntimeError('stop after inputs')):
            with self.assertRaisesRegex(RuntimeError, 'stop after inputs'):
                c.cooking(ctx)
        self.assertEqual(placed, ['coal', 'iron_ore'])

    def test_entity_menu_waits_past_player_inventory(self):
        snapshot = {'menu': {'id': 0, 'slots': []}}
        ctx = self.context(snapshot)
        sent = []
        ctx.clients[0].interact_entity = lambda entity, **kw: sent.append((entity, kw))
        def wait(predicate, **kwargs):
            self.assertFalse(predicate())
            snapshot['menu'] = {'id': 5, 'slots': []}
            self.assertTrue(predicate())
        ctx.clients[0].wait_until = wait
        c._entity_menu(ctx, 42)
        self.assertEqual(sent, [(42, {'secondary': True})])

    def test_mount_transfers_finite_cargo_across_boundary(self):
        inventory = []
        cargo = {'count': 0}
        ctx = self.context({'inventory': inventory})
        ctx.state['subject'] = 'mount_inventory'
        ctx.clients[0].select_hotbar = lambda slot: None
        ctx.clients[0].interact_entity = lambda entity: inventory.clear()
        ctx.clients[0].wait_until = lambda predicate, **kwargs: self.assertTrue(predicate())
        def give(ctx, item, count=1):
            inventory.append({'slot': 36, 'item_id': 'minecraft:' + item, 'count': count})
        def put(ctx, item, destination, player_start):
            self.assertEqual((item, destination, player_start), ('stone', 2, 17))
            cargo['count'] = inventory.pop()['count']
        def take(ctx, slot):
            self.assertEqual(slot, 2)
            give(ctx, 'stone', cargo['count'])
            cargo['count'] = 0
        with patch.object(c, '_give', side_effect=give), \
                patch.object(c, '_entity', return_value={'id': 42}), \
                patch.object(c, '_entity_menu'), patch.object(c, '_size'), patch.object(c, '_close'), \
                patch.object(c, '_put', side_effect=put), patch.object(c, '_take', side_effect=take), \
                patch.object(c, '_slot', side_effect=lambda ctx, slot: dict(cargo)), \
                patch.object(c, '_boundary', side_effect=lambda ctx: self.assertEqual(cargo['count'], 16)):
            c.mount(ctx)
        self.assertEqual([check['name'] for check in ctx.state['checks']], [
            'mount_chest_consumed', 'mount_cargo_deposit_16', 'mount_cargo_retained_16',
            'mount_cargo_withdrawn', 'mount_finite_stock_conserved'])
        self.assertTrue(all(check['passed'] for check in ctx.state['checks']))

    def test_lectern_waits_for_received_page_and_returned_book(self):
        inventory = [{'slot': 36, 'item_id': 'minecraft:writable_book', 'count': 1}]
        properties = {'0': 0}
        ctx = self.context({'inventory': inventory, 'menu': {'properties': properties}})
        ctx.state['subject'] = 'lectern'
        pending = []
        ctx.clients[0].select_hotbar = lambda slot: None
        ctx.clients[0].container_button = pending.append
        def wait(predicate, **kwargs):
            self.assertFalse(predicate())
            button = pending.pop(0)
            if button == 2:
                properties['0'] = 1
            elif button == 3:
                inventory.append({'slot': 36, 'item_id': 'minecraft:writable_book', 'count': 1})
            else:
                self.fail(f'unexpected button {button}')
            self.assertTrue(predicate())
        ctx.clients[0].wait_until = wait
        with patch.object(c, '_give'), patch.object(c, '_open'), patch.object(c, '_boundary'), \
                patch.object(c, '_use_on_ack', side_effect=lambda ctx: inventory.clear()):
            c.lectern(ctx)
        self.assertFalse(pending)
        self.assertTrue(all(check['passed'] for check in ctx.state['checks']))

    def test_entity_clauses_no_longer_marked_unimplemented(self):
        self.assertEqual(set(c.CLAUSE_GAPS), {'hopper', 'brewing_stand', 'plugin_chest_menu'})


if __name__ == '__main__':
    unittest.main()
