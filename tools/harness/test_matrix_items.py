"""Focused contract tests, not substitutes for canonical real-server receipts."""
import json
import importlib
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from tools.harness.matrix_api import Adapter, MatrixContext, MissingCapability
from tools.harness.matrix_domains import _items
from tools.harness.matrix_domains.death_respawn import custody, grave_lane


class Client:
    name = 'UnitActor'

    def __init__(self, snapshot):
        self.value = snapshot

    def snapshot(self):
        return self.value


class Context:
    def __init__(self, snapshot=None, replies=None):
        self.clients = [Client(snapshot or {})]
        self.state = {}
        self.factors = {'dimension': 'overworld'}
        self.row = {'subject': 'unit.subject'}
        self.boundaries = 0
        self.queries = []
        self.replies = iter(replies or [])
        self.server = SimpleNamespace(query=self.query)

    def query(self, text):
        self.queries.append(text)
        return next(self.replies)

    def boundary(self):
        self.boundaries += 1


class ItemAdaptersTest(unittest.TestCase):
    def test_all_assigned_subjects_export_real_adapters(self):
        expected = {'inventory': 18, 'items': 18, 'crafting': 17, 'effects': 40,
                    'damage': 30, 'combat': 14, 'death_respawn': 13}
        for domain, size in expected.items():
            module = importlib.import_module('tools.harness.matrix_domains.' + domain)
            self.assertEqual(len(module.ADAPTERS), size, domain)
            for key, value in module.ADAPTERS.items():
                self.assertTrue(key.startswith(domain + '.'))
                self.assertIsInstance(value, Adapter)
                self.assertTrue(callable(value.setup))
                self.assertTrue(callable(value.act))
                self.assertTrue(callable(value.oracle))

    def test_sparse_empty_menu_is_empty_not_missing_index(self):
        ctx = Context({'menu': {'slot_count': 63, 'slots': []}})
        self.assertEqual(_items.menu_slot(ctx, 0)['count'], 0)
        self.assertEqual(_items.menu_slot(ctx, 62)['count'], 0)

    def test_sparse_container_mapping_uses_capacity_not_populated_count(self):
        ctx = Context({'menu': {'slot_count': 63, 'slots': [{'slot': 0, 'count': 4}]}})
        self.assertEqual(_items.player_menu_slot(ctx, 9), 27)
        self.assertEqual(_items.player_menu_slot(ctx, 35), 53)
        self.assertEqual(_items.player_menu_slot(ctx, 36), 54)
        self.assertEqual(_items.player_menu_slot(ctx, 44), 62)
        self.assertEqual(_items.menu_slot(ctx, 0)['count'], 4)

    def test_missing_capacity_never_infers_from_sparse_list(self):
        with self.assertRaisesRegex(MissingCapability, 'snapshot:menu:slot_count'):
            _items.player_menu_slot(Context({'menu': {'slots': []}}), 36)

    def test_observation_missing_is_not_empty_or_zero(self):
        for value in ({}, {'health': None}):
            with self.assertRaisesRegex(MissingCapability, 'snapshot:health'):
                _items.observe(Context(value), 'health')
        self.assertEqual(_items.observe(Context({'health': 0}), 'health'), 0)
        self.assertEqual(_items.observe(Context({'effects': []}), 'effects'), [])

    def test_receipt_state_is_json_serializable(self):
        ctx = Context()
        _items.check(ctx, 'custody', 3, 3, source='unit')
        stack = {'item_id': 'minecraft:stone', 'count': 3, 'components': {}}
        ctx.state['death_custody'] = custody([stack])
        self.assertEqual(json.loads(json.dumps(ctx.state))['checks'][0]['passed'], True)

    def test_ack_requires_two_received_unique_markers(self):
        ctx = Context({'events': []})
        counters = iter((0, 20, 20, 40))
        ctx.server.dashboard_stats = lambda: {'simulation_tick': next(counters)}
        sent = []
        def command(text):
            sent.append(text)
            ctx.clients[0].value['events'].append(text)
        ctx.clients[0].command = command
        ctx.wait_until = lambda predicate, **kwargs: self.assertTrue(predicate())
        _items.acknowledge(ctx)
        self.assertEqual(len(sent), 2)
        self.assertTrue(all(text.startswith('tell ') for text in sent))
        self.assertNotEqual(sent[0], sent[1])
        self.assertNotIn('checks', ctx.state)
    def test_block_ack_waits_for_exact_submitted_sequence(self):
        for name in ('use_item_on', 'break_block'):
            ctx = Context({'block_ack_sequence': None})
            setattr(ctx.clients[0], name, lambda *args, **kwargs: 7)
            def wait(predicate, **kwargs):
                self.assertFalse(predicate())
                ctx.clients[0].value['block_ack_sequence'] = 6
                self.assertFalse(predicate())
                ctx.clients[0].value['block_ack_sequence'] = 7
                self.assertTrue(predicate())
            ctx.wait_until = wait
            self.assertEqual(_items.verb(ctx, name, (2, 180, 0)), 7)
            self.assertNotIn('checks', ctx.state)

    def test_block_ack_missing_token_or_field_blocks(self):
        for token, snapshot in ((None, {}), (True, {}), (7, {})):
            ctx = Context(snapshot)
            ctx.clients[0].use_item_on = lambda *args, **kwargs: token
            ctx.wait_until = lambda predicate, **kwargs: predicate()
            with self.assertRaisesRegex(MissingCapability, '^block_ack_sequence$'):
                _items.verb(ctx, 'use_item_on', (2, 180, 0))

    def test_hotbar_actor_uses_selected_stack_without_echoed_slot(self):
        from tools.harness.matrix_domains import inventory
        ctx = Context()
        ctx.factors['game_mode'] = 'survival'
        ctx.clients[0].look_at = lambda *a: None
        with patch.multiple(inventory, give=lambda *a: None, verb=lambda *a, **k: None,
                            command=lambda *a: None, block_is=lambda *a: True,
                            count=lambda *a: 1):
            inventory.hotbar(ctx)
        checks = _items.oracle(ctx)
        self.assertEqual(len(checks), 18)
        self.assertTrue(all(c.passed for c in checks))

    def test_hotbar_multiplayer_never_skips_received_equipment(self):
        from tools.harness.matrix_domains import inventory
        ctx = Context()
        ctx.clients.append(Client({}))
        with patch.multiple(inventory, give=lambda *a: None, verb=lambda *a, **k: None):
            with self.assertRaisesRegex(MissingCapability, 'snapshot:equipment'):
                inventory.hotbar(ctx)

    def test_xp_orb_merge_counts_value_times_multiplicity(self):
        from tools.harness.matrix_domains import inventory
        ctx = Context({'inventory': [{'item_id': 'minecraft:diamond_pickaxe',
                                      'components': {'minecraft:damage': 10}}]})
        orbs = [{'value': 1, 'count': 2}] + [{'value': 1, 'count': 1}] * 39
        with patch.multiple(inventory, give=lambda *a: None, select=lambda *a: None,
                            command=lambda *a: None, ticks=lambda *a: None,
                            entities=lambda *a: orbs), \
             patch.object(inventory, 'number', side_effect=[0, 0, 5]):
            inventory.xp(ctx)
        checks = {c.name: c for c in _items.oracle(ctx)}
        self.assertTrue(checks['merged_orb_xp_conserved'].passed)
        self.assertTrue(checks['equal_value_orbs_merge'].passed)

    def test_xp_orb_merge_cannot_pass_unknown_count(self):
        from tools.harness.matrix_domains import inventory
        ctx = Context({'inventory': [{'item_id': 'minecraft:diamond_pickaxe',
                                      'components': {'minecraft:damage': 10}}]})
        with patch.multiple(inventory, give=lambda *a: None, select=lambda *a: None,
                            command=lambda *a: None, ticks=lambda *a: None,
                            entities=lambda *a: [{'value': 1}]), \
             patch.object(inventory, 'number', side_effect=[0, 0, 5]):
            with self.assertRaisesRegex(MissingCapability, 'snapshot:experience_orb:value_count'):
                inventory.xp(ctx)


    def test_creative_arrow_allowed_adds_item_but_creative_only_does_not(self):
        from tools.harness.matrix_domains import combat
        ctx = Context()
        ctx.factors['game_mode'] = 'creative'
        with patch.multiple(combat, command=lambda *a: None, ticks=lambda *a: None), \
             patch.object(combat, 'count', side_effect=[0, 1, 0]), \
             patch.object(combat, 'entities', side_effect=[[{'id': 1}], [], []]):
            combat.projectile_pickup(ctx)
        self.assertTrue(all(c.passed for c in _items.oracle(ctx)))
        self.assertEqual(len(_items.oracle(ctx)), 6)

    def test_restart_cannot_hide_reverted_hand_swap_behind_equal_totals(self):
        from tools.harness.matrix_domains import inventory
        ctx = Context({'inventory': [
            {'slot': 37, 'item_id': 'minecraft:stone', 'count': 9},
            {'slot': 45, 'item_id': 'minecraft:dirt', 'count': 3}]})
        def restart():
            ctx.clients[0].value['inventory'] = [
                {'slot': 45, 'item_id': 'minecraft:stone', 'count': 9},
                {'slot': 37, 'item_id': 'minecraft:dirt', 'count': 3}]
        ctx.boundary = restart
        with patch.multiple(inventory, give=lambda *a: None, select=lambda *a: None,
                            verb=lambda *a: None, inventory_slot=lambda *a: 37,
                            require_verbs=lambda *a: None):
            inventory.swap_hands(ctx)
        checks = {c.name: c for c in _items.oracle(ctx)}
        self.assertTrue(checks['hands_custody'].passed)
        self.assertFalse(checks['post_boundary_offhand_identity'].passed)
        self.assertFalse(checks['post_boundary_mainhand_identity'].passed)

    def test_recipe_placement_detects_lost_ingredient(self):
        from tools.harness.matrix_domains import inventory
        ctx = Context({'recipe_ghost': {'recipe_id': 'minecraft:diamond_pickaxe'}})
        stage = []
        def action(*args, **kwargs):
            stage[:] = [kwargs['recipe_id'], kwargs['all']]
        def slot(_ctx, index):
            if stage[0] == 'minecraft:diamond_pickaxe':
                return {'count': 0}
            return {'count': 4 if index == 0 else (3 if stage[1] else 1)}
        with patch.multiple(inventory, give=lambda *a: None, open_menu=lambda *a: None,
                            require_verbs=lambda *a: None, verb=action, menu_slot=slot,
                            count=lambda *a: 2):
            inventory.recipe_book(ctx)
        checks = {c.name: c for c in _items.oracle(ctx)}
        self.assertFalse(checks['recipe_placement_conserves_ingredients'].passed)
        self.assertTrue(checks['missing_recipe_ghost'].passed)

    def test_check_keeps_failure_and_calls_boundary_once(self):
        ctx = Context()
        _items.check(ctx, 'custody', 2, 3, source='unit')
        _items.check(ctx, 'remaining', 0, 0, source='unit')
        checks = _items.oracle(ctx)
        self.assertEqual([c.passed for c in checks], [False, True])
        self.assertEqual(ctx.boundaries, 1)
        self.assertEqual(checks[0].detail['expected'], 3)

    def test_empty_oracle_fails_closed(self):
        with self.assertRaisesRegex(RuntimeError, 'No behavioral oracle'):
            _items.oracle(Context())

    def test_scalar_nbt_feedback(self):
        ctx = Context(replies=['UnitActor has the following entity data: 17.5f'])
        self.assertEqual(_items.number(ctx, 'UnitActor', 'Health'), 17.5)
        self.assertIn('data get entity UnitActor Health', ctx.queries[0])

    def test_scalar_does_not_parse_numbers_from_errors(self):
        ctx = Context(replies=['No elements matching Inventory[36]'])
        with self.assertRaisesRegex(MissingCapability, 'scalar_feedback'):
            _items.number(ctx, 'UnitActor', 'Inventory[36]')

    def test_old_binary_query_is_explicit_missing_capability(self):
        ctx = Context(replies=['Supported: data get|merge block <x> <y> <z> [compound] (command blocks only)'])
        with self.assertRaisesRegex(MissingCapability, '^data_get_entity$'):
            _items.number(ctx, 'UnitActor', 'Health')

    def test_custody_includes_components_but_not_slot_identity(self):
        a = {'slot': 36, 'item_id': 'minecraft:stone', 'count': 3, 'components': {}}
        b = dict(a, slot=0, count=2)
        c = dict(a, slot=45, count=1)
        self.assertEqual(custody([a]), custody([b, c]))
        named = dict(a, components={'minecraft:custom_name': 'named'})
        self.assertNotEqual(custody([named]), custody([a]))

    def test_unknown_components_never_collapse_to_empty(self):
        with self.assertRaisesRegex(MissingCapability, 'snapshot:stack:components'):
            custody([{'item_id': 'minecraft:stone', 'count': 1}])

    def test_death_profile_matches_actual_plugin_identity(self):
        for profile, plugins, expected in (
                ('vanilla', [], False), ('live', ['graves'], True)):
            ctx = Context()
            ctx.factors['plugin_profile'] = profile
            ctx.state['enabled_plugins'] = plugins
            self.assertEqual(grave_lane(ctx), expected)
            ctx.state['enabled_plugins'] = [] if expected else ['graves']
            with self.assertRaisesRegex(RuntimeError, 'contradicts'):
                grave_lane(ctx)

    def test_ticks_waits_on_real_simulation_counter(self):
        ctx = Context()
        counters = iter((10, 11, 15))
        ctx.server.dashboard_stats = lambda: {'simulation_tick': next(counters)}
        def wait(predicate, **kwargs):
            self.assertFalse(predicate())
            self.assertTrue(predicate())
            self.assertIn('5 simulation ticks', kwargs['description'])
        ctx.wait_until = wait
        _items.ticks(ctx, 5)

    def test_numeric_components_resolve_actual_registry_names(self):
        from tools.harness.matrix_domains.crafting import named_enchantments, registry_name
        ctx = Context({'registries': {
            'minecraft:enchantment': {'17': 'minecraft:unbreaking'},
            'minecraft:potion': {'9': 'minecraft:strength'}}})
        stack = {'components': {'minecraft:enchantments': [[17, 2]]}}
        self.assertEqual(named_enchantments(ctx, stack), {'minecraft:unbreaking': 2})
        self.assertEqual(registry_name(ctx, 'minecraft:potion', 9), 'minecraft:strength')

    def test_registry_name_missing_never_guesses_wire_order(self):
        from tools.harness.matrix_domains.crafting import registry_name
        with self.assertRaisesRegex(MissingCapability, 'snapshot:registries:minecraft:potion:9'):
            registry_name(Context({'registries': {}}), 'minecraft:potion', 9)

    def test_received_effect_ids_resolve_without_fabricating_entries(self):
        entry = {'amplifier': 1, 'duration_ticks': 600,
                 'flags': {'visible': True, 'show_icon': True}}
        ctx = Context({'effects': {'7': entry},
                       'registries': {'minecraft:mob_effect': {'7': 'minecraft:speed'}}})
        self.assertEqual(_items.observe(ctx, 'effects'), [dict(entry, id='minecraft:speed')])
        self.assertNotIn('id', entry)
        ctx.clients[0].value['registries'] = {}
        with self.assertRaisesRegex(MissingCapability, 'snapshot:registries:minecraft:mob_effect:7'):
            _items.observe(ctx, 'effects')

    def test_received_experience_and_food_do_not_need_entity_nbt_query(self):
        ctx = Context({'xp': {'total': 160, 'level': 10, 'progress': 0.25}, 'food': 0})
        self.assertEqual(_items.number(ctx, 'UnitActor', 'XpTotal'), 160)
        self.assertEqual(_items.number(ctx, 'UnitActor', 'XpLevel'), 10)
        self.assertEqual(_items.number(ctx, 'UnitActor', 'XpP'), .25)
        self.assertEqual(_items.number(ctx, 'UnitActor', 'foodLevel'), 0)
        self.assertEqual(ctx.queries, [])

    def test_effect_lifecycle_asserts_received_observer_flags(self):
        from tools.harness.matrix_domains import effects
        from tools.harness.matrix_execution import offline_uuid
        entry = {'amplifier': 1, 'flags': {'visible': True, 'show_icon': True}}
        ctx = Context()
        ctx.clients[0].command = lambda *a: None
        ctx.clients.append(Client({
            'registries': {'minecraft:mob_effect': {'7': 'minecraft:speed'}},
            'entities': [{'uuid': str(offline_uuid('UnitActor')), 'effects': {'7': entry}}]}))
        ctx.wait_until = lambda predicate, **kwargs: self.assertTrue(predicate())
        with patch.multiple(effects, give=lambda *a: None, consume=lambda *a: None,
                            apply=lambda *a, **k: None, ticks=lambda *a: None,
                            require_verbs=lambda *a: None), \
             patch.object(effects, 'active', side_effect=[entry, None, None, None, None]):
            effects.lifecycle(ctx, 'speed')
        checks = {c.name: c for c in _items.oracle(ctx)}
        self.assertTrue(checks['observer_receives_visible_effect'].passed)
        self.assertTrue(checks['local_effect_icon_flag'].passed)

    def test_merge_custody_excludes_other_items_but_catches_extra_fixture_stone(self):
        from tools.harness.matrix_domains import inventory
        def item(name, count):
            return {'item': {'item_id': 'minecraft:' + name, 'count': count}}
        for remainder, passed in ((6, True), (18, False)):
            ctx = Context()
            first = [item('stone', 64), item('stone', remainder), item('dirt', 12)]
            with patch.multiple(inventory, command=lambda *a: None, ticks=lambda *a: None), \
                 patch.object(inventory, 'entities', side_effect=[first, [item('stone', 30)]]):
                inventory.merge(ctx)
            checks = {c.name: c for c in _items.oracle(ctx)}
            self.assertEqual(checks['merged_total_conserved'].passed, passed)

    def test_death_custody_ignores_ambient_items_but_detects_modified_copies(self):
        from tools.harness.matrix_domains import death_respawn
        diamond = {'item_id': 'minecraft:diamond', 'count': 11, 'components': {}}
        kelp = {'item_id': 'minecraft:kelp', 'count': 41, 'components': {}}
        renamed = dict(diamond, components={'minecraft:custom_name': 'duplicate'})
        for extra, passed in (([], True), ([{'item': renamed}], False)):
            ctx = Context({'inventory': [diamond], 'xp': {'total': 0, 'level': 10}})
            ctx.state.update(death_custody=custody([diamond]), death_xp=0,
                             death_level=10, enabled_plugins=['graves'], death_cell=[4, 180, 4])
            with patch.object(death_respawn, 'entities', return_value=[{'item': kelp}] + extra), \
                 patch.object(death_respawn, 'block_is', return_value=False):
                death_respawn.assert_death_custody(ctx, keep=True)
            checks = {c.name: c for c in _items.oracle(ctx)}
            self.assertEqual(checks['keep_inventory_no_ground_copy'].passed, passed)
            self.assertTrue(checks['keep_inventory_levels'].passed)

    def test_retained_zero_total_does_not_hide_lost_experience_levels(self):
        from tools.harness.matrix_domains import death_respawn
        ctx = Context({'inventory': [], 'xp': {'total': 0, 'level': 0}})
        ctx.state.update(death_custody=custody([]), death_xp=0, death_level=10,
                         enabled_plugins=['graves'], death_cell=[4, 180, 4])
        with patch.object(death_respawn, 'entities', return_value=[]), \
             patch.object(death_respawn, 'block_is', return_value=False):
            death_respawn.assert_death_custody(ctx, keep=True)
        checks = {c.name: c for c in _items.oracle(ctx)}
        self.assertTrue(checks['keep_inventory_xp'].passed)
        self.assertFalse(checks['keep_inventory_levels'].passed)


if __name__ == '__main__':
    unittest.main()
