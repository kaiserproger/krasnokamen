"""Focused platform adapter contract tests; not live gameplay receipts."""
from __future__ import annotations
from pathlib import Path
import unittest
from unittest.mock import patch
import tempfile
import tomllib
from types import SimpleNamespace
from tools.harness.matrix_api import MatrixContext, MissingCapability, Adapter
from tools.harness.matrix_domains import _platform as p

class Client:
    name = 'Actor'
    def __init__(self, data):
        self.data = data
        self.calls = []
    def snapshot(self):
        return self.data
    def command(self, text):
        self.calls.append(text)
        self.data['events'] = ['retained', 'matching response']
    def wait_until(self, predicate, **kwargs):
        value = predicate()
        if not value:
            raise TimeoutError(kwargs['description'])
        return value
    def click_slot(self, slot, kind='pickup', button=0):
        self.calls.append(('click', slot, kind, button))
    def select_hotbar(self, slot):
        self.calls.append(('hotbar', slot))

class PlatformContractTests(unittest.TestCase):
    def context(self, data=None, server=None):
        client = Client(data or {'events': [], 'position': [0,80,0]})
        ctx = MatrixContext({'subject':'unit.subject', 'context': {
            'network_condition':'normal', 'server_lifecycle':'steady'}},
            server or SimpleNamespace(), [client], Path('/unused'))
        p.setup(ctx)
        return ctx

    def test_missing_snapshot_is_not_empty(self):
        ctx = self.context()
        with self.assertRaisesRegex(MissingCapability, '^snapshot:inventory$'):
            p.observe(ctx, 'inventory')

    def test_empty_oracle_never_passes(self):
        with self.assertRaisesRegex(MissingCapability, '^oracle_observation:unit.subject$'):
            p.oracle(self.context())

    def test_checks_retain_failure_and_boundary_once(self):
        ctx=self.context()
        hits=[]
        ctx.boundary_hook=lambda _: hits.append('boundary')
        p.check(ctx,'equal',1,1)
        p.check(ctx,'mismatch',1,2)
        p.test(ctx,'negative',False,'observed')
        self.assertEqual([c.passed for c in p.oracle(ctx)],[True,False,False])
        self.assertEqual(hits,['boundary'])

    def test_bounded_event_stream_fresh_response(self):
        ctx=self.context({'events':['discarded','retained']})
        self.assertEqual(p.response(ctx,'test','matching response'),'matching response')
        self.assertTrue(p.oracle(ctx)[0].passed)

    def test_old_matching_line_not_a_response(self):
        ctx=self.context({'events':['retained','matching response']})
        with self.assertRaises(TimeoutError):
            p.response(ctx,'test','matching response')
        self.assertEqual(ctx.state['checks'],[])

    def test_inventory_count_sparse_slots(self):
        ctx=self.context({'events':[], 'inventory':[{'slot':44,'item_id':'minecraft:stone','count':3},
            {'slot':9,'item_id':'minecraft:stone','count':2}, {'slot':10,'item_id':'minecraft:dirt','count':8}]})
        self.assertEqual(p.inventory(ctx,'minecraft:stone'),5)

    def test_select_hotbar_uses_protocol_slot_offset(self):
        ctx=self.context({'events':[], 'inventory':[{'slot':44,'item_id':'minecraft:stone','count':3}]})
        p.rich_action(ctx,'select_inventory_item','minecraft:stone')
        self.assertEqual(ctx.clients[0].calls,[('hotbar',8)])

    def test_select_main_inventory_swaps_to_hotbar(self):
        ctx=self.context({'events':[], 'inventory':[{'slot':12,'item_id':'minecraft:stone','count':3}]})
        p.rich_action(ctx,'select_inventory_item','minecraft:stone')
        self.assertEqual(ctx.clients[0].calls,[('click',12,'swap',0),('hotbar',0)])

    def test_unknown_block_never_becomes_air(self):
        ctx=self.context()
        with self.assertRaisesRegex(MissingCapability,'^client:read_block_state$'):
            p.block(ctx,(0,80,0))

    def test_failed_query_is_failed_check(self):
        ctx=self.context(server=SimpleNamespace(query=lambda _: 'Actor has 4 [matrix]'))
        p.query(ctx,'scoreboard players get Actor matrix',r'has 7 \[matrix\]')
        self.assertFalse(p.oracle(ctx)[0].passed)

    def test_same_tick_triple_never_runs_sequentially(self):
        ctx = self.context()
        ctx.row['origin'] = 'triple:dupe:two_players_take_same_tick'
        with self.assertRaisesRegex(MissingCapability, '^same_tick_admission_fence$'):
            p.setup(ctx)

    def test_plugin_fixture_rewrites_only_isolated_config(self):
        from tools.harness.matrix_domains import plugins
        original = Path('/run/media/kaiserroman/games/solaris-work/scenarios/live-plugins/krasnokamen-permissions/config.toml')
        before = original.read_text()
        configured = []
        ctx = self.context(server=SimpleNamespace(
            plugins_dir=original.parent.parent,
            reconfigure=lambda overrides, **kwargs: configured.append((overrides, kwargs))))
        automation = []
        ctx.clients[0].set_auto_auth = automation.append
        ctx.clients[0].disconnect = lambda: None
        ctx.clients[0].reconnect = lambda: None
        with tempfile.TemporaryDirectory(prefix='.platform-fixture-', dir=Path(__file__).resolve().parents[2]) as directory:
            ctx.artifact_dir = Path(directory)
            target = plugins.deploy(ctx, 'krasnokamen-permissions',
                                    groups={'guest': [], 'member': ['krasnokamen.essentials.use']},
                                    default_group='guest')
            data = tomllib.loads((target/'krasnokamen-permissions/config.toml').read_text())
            self.assertEqual(data['default_group'], 'guest')
            self.assertEqual(data['groups'], [{'name':'guest','nodes':[]},
                                             {'name':'member','nodes':['krasnokamen.essentials.use']}])
            self.assertEqual(configured, [({}, {'plugins_dir': target})])
            self.assertEqual(sorted(p.name for p in target.iterdir()),
                             sorted(p.name for p in original.parent.parent.iterdir()))
        self.assertEqual(original.read_text(), before)
        self.assertEqual(automation, [])

    def test_all_actor_repeat_never_claimed_by_one_actor(self):
        ctx = self.context()
        ctx.row['steps'] = ['All 8 clients repeat the action concurrently on own targets and on one shared target']
        with self.assertRaisesRegex(MissingCapability, '^concurrent_subject_actors$'):
            p.setup(ctx)

    def test_console_context_uses_console_authority(self):
        ctx = self.context(server=SimpleNamespace(command=lambda text: 'Updated scores for 1 holders'))
        ctx.row['context']['permission'] = 'console'
        p.response(ctx, 'scoreboard players set Actor matrix 1', r'Updated scores')
        self.assertEqual(ctx.clients[0].calls, [])
        self.assertTrue(p.oracle(ctx)[0].passed)

    def test_block_ack_requires_real_token_and_consumed_sequence(self):
        ctx = self.context({'events': [], 'block_ack_sequence': 7})
        p.block_ack(ctx, 7)
        with self.assertRaises(TimeoutError):
            p.block_ack(ctx, 8)
        for token in (None, True):
            with self.assertRaisesRegex(MissingCapability, '^block_ack_sequence$'):
                p.block_ack(ctx, token)
        self.assertEqual(ctx.state['checks'], [])

    def test_inventory_selection_accepts_received_slot_zero(self):
        ctx = self.context({'events': [], 'inventory': [
            {'slot': 0, 'item_id': 'minecraft:stone', 'count': 1}]})
        p.rich_action(ctx, 'select_inventory_item', 'minecraft:stone')
        self.assertEqual(ctx.clients[0].calls, [('click', 0, 'swap', 0), ('hotbar', 0)])

    def test_scoreboard_displays_checked_on_each_client(self):
        from tools.harness.matrix_domains import scoreboard_teams
        data = {'events': [], 'scoreboard': {
            'displays': {slot:'matrix' for slot in ('sidebar', 'list', 'below_name')},
            'scores': {'Actor': {'matrix': 7}}}}
        ctx = self.context(data)
        ctx.clients.append(Client(data))
        with patch.object(p, 'admin'):
            scoreboard_teams.objectives_display(ctx)
        checks = p.oracle(ctx)
        self.assertEqual(len(checks), 12)
        self.assertEqual(sum(check.name.startswith('client 1') for check in checks), 6)

    def test_arena_leaves_automatic_market_zone(self):
        calls = []
        ctx = self.context(server=SimpleNamespace(
            ensure_loaded=lambda area: calls.append(('load', area)),
            command=lambda command: 'ok'))
        ctx.clients[0].teleport_to = lambda *pos: calls.append(('teleport', pos))
        ctx.clients[0].close_container = lambda: calls.append(('close',))
        self.assertEqual(p.arena(ctx), (64, 80, 64))
        self.assertEqual([call[0] for call in calls], ['teleport', 'close', 'load'])

    def test_absent_package_does_not_change_checkpoint_policy(self):
        from tools.harness.matrix_domains import plugins
        calls = []
        ctx = self.context(server=SimpleNamespace(
            plugins_dir=Path(__file__).resolve().parent,
            reconfigure=lambda *args, **kwargs: calls.append((args, kwargs))))
        with self.assertRaisesRegex(MissingCapability, '^fixture:plugin_package:harness-trapping-guest$'):
            plugins.deploy(ctx, 'harness-trapping-guest')
        self.assertEqual(calls, [])

    def test_auth_fixture_disables_automation_not_authentication(self):
        from tools.harness.matrix_domains import plugins
        calls = []
        ctx = self.context(server=SimpleNamespace(
            plugins_dir=Path('/run/media/kaiserroman/games/solaris-work/scenarios/live-plugins'),
            reconfigure=lambda overrides, **kwargs: calls.append(('config', overrides))))
        ctx.clients[0].disconnect = lambda: calls.append(('disconnect',))
        ctx.clients[0].set_auto_auth = lambda enabled: calls.append(('automation', enabled))
        ctx.clients[0].reconnect = lambda: calls.append(('reconnect',))
        with tempfile.TemporaryDirectory(prefix='.platform-auth-', dir=Path(__file__).resolve().parents[2]) as directory:
            ctx.artifact_dir = Path(directory)
            plugins.deploy(ctx, 'krasnokamen-auth', require_plugin_login=True)
        self.assertEqual(calls, [('disconnect',), ('config', {'auth': {'require_plugin_login': True}}),
                                 ('automation', False), ('reconnect',)])

    def test_first_block_ack_waits_for_receipt_not_capability(self):
        ctx = self.context({'events': [], 'block_ack_sequence': None})
        def receive(predicate, **kwargs):
            self.assertFalse(predicate())
            ctx.clients[0].data['block_ack_sequence'] = 5
            return predicate()
        ctx.clients[0].wait_until = receive
        p.block_ack(ctx, 5)
        self.assertEqual(ctx.state['checks'], [])

    def test_unpublished_block_ack_is_exact_capability(self):
        with self.assertRaisesRegex(MissingCapability, '^block_ack_sequence$'):
            p.block_ack(self.context(), 5)

    def test_ground_item_radius_uses_received_entities(self):
        near = {'id':1, 'type':'minecraft:item', 'position':[1,80,0]}
        far = {'id':2, 'type':'minecraft:item', 'position':[9,80,0]}
        mob = {'id':3, 'type':'minecraft:pig', 'position':[1,80,0]}
        ctx = self.context({'events':[], 'entities':[near, far, mob]})
        self.assertEqual(p.rich_action(ctx, 'item_entities_in_radius', [0,80,0], 4), [near])
        self.assertEqual(p.rich_action(ctx, 'entity_count', 'minecraft:item'), 2)

    def test_block_volume_reads_actual_states(self):
        ctx = self.context()
        ctx.clients[0].read_block_state = lambda position: {
            'block_id': 'minecraft:sculk' if position[0] == 1 else 'minecraft:stone',
            'properties': {}}
        states = p.rich_action(ctx, 'read_blocks', (0,79,0), (1,79,0))
        self.assertEqual([state['name'] for state in states],
                         ['minecraft:stone', 'minecraft:sculk'])

    def test_observer_block_state_is_not_actor_state(self):
        ctx = self.context()
        observer = Client({'events': []})
        observer.read_block_state = lambda position: {'block_id':'minecraft:air', 'properties':{}}
        ctx.clients.append(observer)
        self.assertEqual(p.block(ctx, (2,80,0), index=1)['name'], 'minecraft:air')
        with self.assertRaisesRegex(MissingCapability, '^client:read_block_state$'):
            p.block(ctx, (2,80,0))

    def test_real_shared_executor_accepts_json_evidence(self):
        from tools.harness.matrix_api import execute_adapter
        ctx = self.context()
        def action(context):
            p.check(context, 'packet bytes', b'\x00\xff', bytearray(b'\x00\xff'))
            p.test(context, 'failed observation retained', False, {'position': (1,2,3)})
        checks = execute_adapter(Adapter(p.setup, action, p.oracle), ctx)
        self.assertEqual([check.passed for check in checks], [True, False])
        self.assertIsInstance(ctx.state['checks'][0], dict)
        self.assertEqual(checks[0].detail['actual'], {'bytes_hex': '00ff'})
        self.assertEqual(checks[1].detail, {'position': [1,2,3]})

    def test_wait_block_requires_actual_received_state(self):
        ctx = self.context()
        reads = iter([None, {'block_id': 'minecraft:air', 'properties': {}}])
        def read_block(position):
            state = next(reads)
            if state is None:
                raise MissingCapability('snapshot:block_state_not_received:' + str(position))
            return state
        def wait(predicate, **kwargs):
            self.assertIsNone(predicate())
            return predicate()
        ctx.clients[0].read_block_state = read_block
        ctx.clients[0].wait_until = wait
        self.assertEqual(p.wait_block(ctx, (2,80,0), 'minecraft:air')['name'], 'minecraft:air')
        self.assertEqual(ctx.state['checks'], [])

    def test_wait_block_does_not_hide_missing_capabilities(self):
        ctx = self.context()
        with self.assertRaisesRegex(MissingCapability, '^client:read_block_state$'):
            p.wait_block(ctx, (2,80,0), 'minecraft:air')

if __name__ == '__main__':
    unittest.main()
