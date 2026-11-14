"""Regressions for real-observation fences and per-row plugin identity."""
import json
import tempfile
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

from .backends.protocol_lifecycle import ProtocolServer
from .matrix_api import Adapter, Check, MissingCapability, execute_adapter
from .matrix_execution import copy_plugins
from . import matrix_execution
from .protocol_matrix_client import ProtocolMatrixClient
from .test_matrix_api import context


class MatrixRuntimeTests(unittest.TestCase):
    def test_non_json_adapter_state_is_rejected_before_actions(self):
        ctx=context(); action=Mock()
        def setup(c): c.state['checks']=[Check('not JSON',True)]
        with self.assertRaisesRegex(TypeError,'JSON-serializable'):
            execute_adapter(Adapter(setup,action,lambda c:[Check('ok',True)]),ctx)
        action.assert_not_called()

    def test_non_json_oracle_detail_is_rejected(self):
        with self.assertRaisesRegex(TypeError,'JSON-serializable'):
            execute_adapter(Adapter(lambda c:None,lambda c:None,
                            lambda c:[Check('bad',True,object())]),context())

    def test_plugin_copy_defaults_full_and_excludes_manifest_id(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp); source=root/'source'; source.mkdir()
            for directory,plugin_id in [('computer','krasnokamen'),('graves','graves')]:
                package=source/directory; package.mkdir()
                (package/'plugin.toml').write_text(f'id = "{plugin_id}"\n')
                (package/'plugin.wasm').write_bytes(b'fixture')
            self.assertEqual(copy_plugins(source,root/'all'),['krasnokamen','graves'])
            self.assertEqual(copy_plugins(source,root/'without',['graves']),['krasnokamen'])
            self.assertFalse((root/'without/graves').exists())
            with self.assertRaisesRegex(ValueError,'unknown excluded'):
                copy_plugins(source,root/'bad',['unknown'])

    def test_chunk_fence_never_treats_forceload_ack_as_readiness(self):
        server=ProtocolServer(binary=Path('/binary'),run_dir=Path('/unused'),port=26100,
                              vanilla_data=Path('/vanilla'),seed=81)
        server.command=Mock(return_value='Marked chunks as force loaded')
        with self.assertRaisesRegex(MissingCapability,'chunk_loaded_signal'):
            server.ensure_loaded(((0,0,0),(16,80,16)))
        server.command.assert_not_called()
        called=[]
        server.ensure_loaded_callback=called.append
        server.ensure_loaded(((0,0,0),(16,80,16)))
        self.assertEqual(called,[((0,0,0),(16,80,16))])
        server.command.assert_called_once_with('forceload add 0 0 16 16')

    def test_current_menu_suffix_overrides_stale_window_zero(self):
        client=object.__new__(ProtocolMatrixClient); client.items={1:'minecraft:stone'}
        stack={'item_id':1,'count':16,'components':{},'removed':[]}
        player=[None]*46; player[36]=stack
        menu=[None]*63; menu[0]=stack
        client.observer=SimpleNamespace(condition=threading.RLock(),position=None,events=[],health=None,
              food=None,revision=10,chunks=set(),menu_errors={},current_window=4,
              chat_components=[],chat_errors=[],merchant_offers={},merchant_errors={},
              menu_types={4:0},menu_properties={},menus={
                0:{'slots':player,'cursor':None,'state_id':1},
                4:{'slots':menu,'cursor':None,'state_id':2}})
        self.assertEqual(client.snapshot()['inventory'],[])
        self.assertEqual(client.snapshot()['menu']['slots'],[
            {'slot':0,'item_id':'minecraft:stone','count':16,'components':{},'removed':[]}])
        menu[0]=None; menu[-9]=stack
        self.assertEqual(client.snapshot()['inventory'],[
            {'slot':36,'item_id':'minecraft:stone','count':16,'components':{},'removed':[]}])
        client.static_registries = {'minecraft:potion': {'5': 'minecraft:awkward'}}
        client.observer.join = {'registries': {
            'minecraft:enchantment': {'0': 'minecraft:mending'}}}
        client.observer.join.update(uuid='00000000-0000-0000-0000-00000000002a',
                                    entity_id=42)
        snapshot = client.snapshot()
        self.assertEqual(snapshot['uuid'], client.observer.join['uuid'])
        self.assertEqual(snapshot['entity_id'], 42)
        self.assertEqual(snapshot['registries']['minecraft:enchantment'],
                         {'0': 'minecraft:mending'})
        self.assertEqual(snapshot['registry_sources'], {
            'minecraft:potion': 'vanilla_report',
            'minecraft:enchantment': 'received_configuration'})
        snapshot['registries']['minecraft:enchantment']['0'] = 'not received'
        self.assertEqual(client.snapshot()['registries']['minecraft:enchantment']['0'],
                         'minecraft:mending')

    def test_entity_snapshot_names_actual_packets_and_never_masks_lost_state(self):
        from .protocol_entities import ProtocolEntities
        from .protocol_observations import decode_stack
        from .test_protocol_entities import add_body, varint
        client = object.__new__(ProtocolMatrixClient)
        client.items = {3: 'minecraft:stone'}
        client.entity_types = {25: 'minecraft:item'}
        tracker = ProtocolEntities(decode_stack=decode_stack)
        client.observer = SimpleNamespace(
            condition=threading.RLock(), position=None, events=[], health=None,
            food=None, revision=1, chunks=set(), chat_components=[], chat_errors=[],
            menu_errors={}, merchant_errors={}, menus={}, current_window=0,
            entities=tracker)
        tracker.consume(0x01, add_body())
        tracker.consume(0x63, varint(7) + bytes([8, 7, 5, 3, 0, 0, 255]))
        entity = client.snapshot()['entities'][0]
        self.assertEqual(entity['type'], 'minecraft:item')
        self.assertEqual(entity['item']['item_id'], 'minecraft:stone')
        self.assertEqual(entity['spawn_position'], [8.5, 64.0, -3.25])
        tracker.consume(0x01, b'truncated')
        tracker.consume(0x4D, varint(0))
        snapshot = client.snapshot()
        self.assertNotIn('entities', snapshot)
        self.assertIn('entities', snapshot['observation_errors'])

    def test_unknown_received_block_state_is_not_air(self):
        client=object.__new__(ProtocolMatrixClient); client.block_states={0:{'block_id':'minecraft:air','properties':{}}}
        client.observer=SimpleNamespace(condition=threading.RLock(),blocks={})
        with self.assertRaisesRegex(MissingCapability,'not_received'):
            client.read_block_state((1,2,3))

    def test_received_event_indices_survive_reconnect_and_failed_join(self):
        client = object.__new__(ProtocolMatrixClient)
        client.name = 'Probe'; client.address = ('unused', 26100)
        client.artifact_dir = Path('/unused'); client.account_uuid = None
        client.timeout_seconds = 1; client.require_auth = False
        client.protocol = 777
        client.generation = 1; client._event_history = []
        def observer(events):
            return SimpleNamespace(
                condition=threading.RLock(), position=None, events=events, health=None,
                food=None, revision=1, chunks=set(), chat_components=[], chat_errors=[],
                menu_errors={}, merchant_errors={}, menus={}, current_window=0,
                closed=True, ready=Mock())
        client.observer = observer(['before-boundary'])
        start = len(client.snapshot()['events'])
        joined = observer(['after-boundary'])
        with patch('tools.harness.protocol_matrix_client.wire.PlayObserver',
                   side_effect=[OSError('join failed'), joined]):
            with self.assertRaisesRegex(OSError, 'join failed'):
                client.reconnect()
            client.reconnect()
        snapshot = client.snapshot()
        self.assertEqual(snapshot['events'], ['before-boundary', 'after-boundary'])
        self.assertEqual(snapshot['events'][start:], ['after-boundary'])
        self.assertEqual(snapshot['generation'], 2)

    def test_auto_auth_control_preserves_server_auth_requirement(self):
        client=object.__new__(ProtocolMatrixClient)
        client.name='Probe'; client.address=('127.0.0.1',26100)
        client.artifact_dir=Path('/unused'); client.account_uuid=None
        client.timeout_seconds=1; client.require_auth=True
        client.protocol = 777
        client.generation=0; client.observer=None
        client.command=Mock(); client.wait_until=Mock()
        observer=Mock()
        observer.events=['/register', 'Successfully logged in']
        with patch('tools.harness.protocol_matrix_client.wire.PlayObserver',return_value=observer):
            client.set_auto_auth(False)
            client.reconnect()
            self.assertTrue(client.require_auth)
            client.command.assert_not_called()
            client.wait_until.assert_not_called()
            observer.closed=True
            client.set_auto_auth(True)
            client.reconnect()
            client.command.assert_called_once_with('register MatrixProbe42 MatrixProbe42')

    def test_exact_plugin_profiles_override_adapter_default_exclusions(self):
        contracts={'death.keep':{'changed_by':['graves'],'vanilla':'without','live':'full'}}
        for profile, expected, fingerprint in [('live',set(),'full'),('vanilla',{'graves'},'without')]:
            row={'id':'bl:death.keep/context@'+profile,'subject':'death.keep',
                 'context':{'plugin_profile':profile}}
            self.assertEqual(matrix_execution.select_plugin_profile(
                row,contracts,adapter_exclusions=('graves',)),(expected,fingerprint))
        with self.assertRaisesRegex(ValueError,'contradict'):
            matrix_execution.select_plugin_profile(
                {'id':'bl:death.keep/context@live','subject':'death.keep',
                 'context':{'plugin_profile':'live'}},contracts,requested_exclusions=['graves'])
        with self.assertRaisesRegex(ValueError,'profile'):
            matrix_execution.select_plugin_profile(
                {'id':'bl:death.keep/context','subject':'death.keep','context':{}},contracts)

    def test_plugin_fingerprint_ignores_mutable_state_but_not_entry(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp); package=root/'package'; package.mkdir()
            (package/'plugin.toml').write_text('id = "sample"\nentry = "entry.wasm"\n')
            entry=package/'entry.wasm'; entry.write_bytes(b'wasm')
            before=matrix_execution.plugin_set_fingerprint(root)
            (package/'state.json').write_text('mutable')
            self.assertEqual(before,matrix_execution.plugin_set_fingerprint(root))
            entry.write_bytes(b'changed')
            self.assertNotEqual(before,matrix_execution.plugin_set_fingerprint(root))

    def test_multiplayer_requires_all_clients_on_own_and_shared_targets(self):
        def run(own, shared, *, forged=False, oracle_passed=True,
                same_tick=False, split=False, origin=None, players=8):
            ctx=context(); ctx.factors['player_count']=str(players)
            ctx.clients=[SimpleNamespace(name=f'Proto{i}') for i in range(players)]
            ctx.row['origin']=origin or 'pairwise'
            ctx.concurrent_hook=lambda c,actions,**mode:[
                {'client':client.name,'verb':verb,'tick':7+(i if split else 0),
                 'connection':i,'input':{'actual':True}}
                for i,(client,verb,kwargs) in enumerate(actions)]
            def act(c):
                if origin: c.state['composite_origin_verified']=origin
                if forged:
                    c.state['concurrent_subject_actors']={'own':True,'shared':True}
                    return
                actions=[(client,'click_slot',{'slot':0}) for client in c.clients]
                if own: c.concurrently(actions[:own],target='own',same_tick=same_tick)
                if shared: c.concurrently(actions[:shared],target='shared',same_tick=same_tick)
            adapter=Adapter(lambda c:None,act,lambda c:[Check('world oracle',oracle_passed)])
            with patch.object(matrix_execution,'load_adapter',return_value=adapter):
                return matrix_execution.run_subject(ctx)
        for own,shared in [(0,0),(8,0),(0,8),(1,8),(8,1)]:
            with self.subTest(own=own,shared=shared), self.assertRaisesRegex(
                    MissingCapability,'concurrent_subject_actors'):
                run(own,shared)
        with self.assertRaisesRegex(MissingCapability,'concurrent_subject_actors'):
            run(0,0,forged=True)
        self.assertTrue(run(8,8)[0]['passed'])
        self.assertFalse(run(0,0,oracle_passed=False)[0]['passed'])
        self.assertTrue(run(8,8,split=True)[0]['passed'])
        strict='triple:dupe:two_players_same_slot_same_tick'
        with self.assertRaisesRegex(MissingCapability,'same_tick_admission_fence'):
            run(2,2,origin=strict,players=2)
        with self.assertRaisesRegex(MissingCapability,'same_tick_admission_split'):
            run(2,2,origin=strict,same_tick=True,split=True,players=2)
        self.assertTrue(run(2,2,origin=strict,same_tick=True,players=2)[0]['passed'])
        self.assertTrue(run(0,0,origin='triple:dupe:restart_after_interaction',
                            split=True,players=2)[0]['passed'])
        self.assertTrue(run(0,0,players=2)[0]['passed'])  # Ordinary N2 is actor+observer.


if __name__=='__main__': unittest.main()
