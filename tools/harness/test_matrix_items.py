"""Focused contract tests, not substitutes for canonical real-server receipts."""
import json
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from tools.harness.matrix_api import MatrixContext, MissingCapability
from tools.harness.matrix_domains import _items
from tools.harness.matrix_domains.death_respawn import custody


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

    def test_receipt_state_is_json_serializable(self):
        ctx = Context()
        _items.check(ctx, 'custody', 3, 3, source='unit')
        stack = {'item_id': 'minecraft:stone', 'count': 3, 'components': {}}
        ctx.state['death_custody'] = custody([stack])
        self.assertEqual(json.loads(json.dumps(ctx.state))['checks'][0]['passed'], True)

    def test_ack_missing_chat_is_capability_failure_not_type_error(self):
        ctx = Context({'events': None})
        counters = iter((0, 20))
        ctx.server.dashboard_stats = lambda: {'simulation_tick': next(counters)}
        ctx.clients[0].command = lambda text: None
        ctx.wait_until = lambda predicate, **kwargs: predicate()
        with self.assertRaisesRegex(MissingCapability, 'snapshot:events'):
            _items.acknowledge(ctx)

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

    def test_xp_orb_merge_queries_server_multiplicity_per_received_uuid(self):
        from tools.harness.matrix_domains import inventory
        ctx = Context({'inventory': [{'item_id': 'minecraft:diamond_pickaxe',
                                      'components': {'minecraft:damage': 10}}]})
        orbs = [{'uuid': f'00000000-0000-0000-0000-{i:012d}', 'value': 1,
                 'count': 999} for i in range(1, 41)]
        authoritative = {orb['uuid']: 2 if i == 0 else 1 for i, orb in enumerate(orbs)}
        player_xp = iter((0, 0, 5))
        queries = []
        def number(_ctx, target, path):
            if target == 'UnitActor':
                return next(player_xp)
            queries.append((target, path))
            return 1 if path == 'Value' else authoritative[target]
        with patch.multiple(inventory, give=lambda *a: None, select=lambda *a: None,
                            command=lambda *a: None, ticks=lambda *a: None,
                            entities=lambda *a: orbs, number=number):
            inventory.xp(ctx)
        checks = {c.name: c for c in _items.oracle(ctx)}
        self.assertTrue(checks['merged_orb_xp_conserved'].passed)
        self.assertTrue(checks['equal_value_orbs_merge'].passed)
        self.assertTrue(checks['received_orb_values_match_server'].passed)
        self.assertEqual(queries, [(orb['uuid'], field) for orb in orbs
                                   for field in ('Value', 'Count')])
        fields = checks['merged_orb_xp_conserved'].detail['source']['fields']
        self.assertEqual(fields['value'], 'server.console.data_get_entity.Value')
        self.assertEqual(fields['count'], 'server.console.data_get_entity.Count')
        self.assertEqual(fields['received_value'], 'client.synced_entity_data.DATA_VALUE')

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

    def test_scalar_nbt_feedback(self):
        ctx = Context(replies=['UnitActor has the following entity data: 17.5f'])
        self.assertEqual(_items.number(ctx, 'UnitActor', 'Health'), 17.5)
        self.assertIn('data get entity UnitActor Health', ctx.queries[0])

    def test_scalar_nbt_feedback_ignores_console_fence(self):
        ctx = Context(replies=['Orb has the following entity data: 2\n'
                               '[Server] MatrixConsoleFence00000042'])
        self.assertEqual(_items.number(ctx, '00000000-0000-0000-0000-000000000001', 'Count'), 2)

    def test_scalar_does_not_parse_numbers_from_errors(self):
        ctx = Context(replies=['No elements matching Inventory[36]'])
        with self.assertRaisesRegex(MissingCapability, 'scalar_feedback'):
            _items.number(ctx, 'UnitActor', 'Inventory[36]')

    def test_old_binary_query_is_explicit_missing_capability(self):
        ctx = Context(replies=['Supported: data get|merge block <x> <y> <z> [compound] (command blocks only)'])
        with self.assertRaisesRegex(MissingCapability, '^data_get_entity$'):
            _items.number(ctx, 'UnitActor', 'Health')

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

if __name__ == '__main__':
    unittest.main()
