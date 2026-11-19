"""Regressions for observed matrix failures and retained reconnect evidence."""
import json
import tempfile
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

from .backends.protocol_lifecycle import ProtocolServer
from .protocol_matrix_client import ProtocolMatrixClient


class MatrixRuntimeTests(unittest.TestCase):
    def test_console_fence_is_private_and_requires_fresh_query_result(self):
        from .matrix_domains import settlements
        with tempfile.TemporaryDirectory() as temp:
            log=Path(temp)/'server.log'
            log.write_text('')
            server=ProtocolServer(binary=Path('/binary'),run_dir=Path(temp),port=26100,
                                  vanilla_data=Path('/vanilla'),seed=81)
            server.log_path=log
            stdin=Mock()
            server._process=SimpleNamespace(stdin=stdin,poll=lambda:None)
            def wait(path, timeout, predicate):
                marker=stdin.write.call_args.args[0].decode().split('scoreboard players list ')[1].strip()
                fence=f'Scores for {marker}: {{}}'
                before=log.read_text()
                log.write_text(before + '[Player] ' + fence + '\n')
                self.assertFalse(predicate()[0])
                town='OtherTown' if stdin.write.call_count == 3 else 'MatrixTown'
                log.write_text(before + f'Settlements: {town}.\n' + fence + '\n')
                ok, fresh=predicate()
                self.assertTrue(ok)
                return fresh
            ctx=SimpleNamespace(server=server)
            with patch('tools.harness.runtime.wait_for_path_condition',side_effect=wait):
                first=settlements.query(ctx,'settlement list')
                second=settlements.query(ctx,'settlement list')
                changed=settlements.query(ctx,'settlement list')
            self.assertEqual(first,second)
            self.assertEqual(first,'Settlements: MatrixTown.')
            self.assertNotEqual(first,changed)
            self.assertNotEqual(stdin.write.call_args_list[0],stdin.write.call_args_list[1])

    def test_closed_menu_uses_newer_received_player_inventory(self):
        client=object.__new__(ProtocolMatrixClient)
        client.items={1:'minecraft:arrow',2:'minecraft:bow'}
        arrow=lambda n:{'item_id':1,'count':n,'components':{},'removed':[]}
        bow={'item_id':2,'count':1,'components':{'minecraft:damage':1},'removed':[]}
        player=[None]*46
        player[36],player[37]=arrow(3),bow
        menu=[None]*63
        menu[-9]=arrow(4)
        client.observer=SimpleNamespace(condition=threading.RLock(),position=None,events=[],health=None,
            food=None,revision=3,chunks=set(),menu_errors={},current_window=0,
            chat_components=[],chat_errors=[],merchant_offers={},merchant_errors={},
            menu_types={},menu_properties={},menus={
                0:{'slots':player,'cursor':None,'state_id':3},
                1:{'slots':menu,'cursor':None,'state_id':2}})
        inventory=client.snapshot()['inventory']
        self.assertEqual([(s['item_id'],s['count']) for s in inventory],
                         [('minecraft:arrow',3),('minecraft:bow',1)])
        self.assertEqual(inventory[1]['components']['minecraft:damage'],1)

    def test_closed_menu_does_not_override_newer_player_inventory(self):
        client = object.__new__(ProtocolMatrixClient)
        client.items = {1: 'minecraft:arrow', 2: 'minecraft:bow'}
        arrow4 = {'item_id': 1, 'count': 4, 'components': {}, 'removed': []}
        arrow3 = {'item_id': 1, 'count': 3, 'components': {}, 'removed': []}
        bow = {'item_id': 2, 'count': 1, 'components': {'minecraft:damage': 1}, 'removed': []}
        player, stale_menu = [None] * 46, [None] * 63
        player[36], player[37], stale_menu[-9] = arrow3, bow, arrow4
        client.observer = SimpleNamespace(
            condition=threading.RLock(), position=None, events=[], health=None, food=None,
            revision=10, chunks=set(), menu_errors={}, current_window=0, chat_components=[],
            chat_errors=[], merchant_offers={}, merchant_errors={}, menu_types={}, menu_properties={},
            menus={0: {'slots': player, 'cursor': None, 'state_id': 2},
                   1: {'slots': stale_menu, 'cursor': None, 'state_id': 1}})
        inventory = client.snapshot()['inventory']
        self.assertEqual([(stack['item_id'], stack['count']) for stack in inventory],
                         [('minecraft:arrow', 3), ('minecraft:bow', 1)])
        self.assertEqual(inventory[1]['components']['minecraft:damage'], 1)

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
        tracker.consume(0x4D, varint(1) + varint(7))
        history=client.snapshot()['entity_lifecycle']
        self.assertEqual([event['event'] for event in history], ['spawn','remove'])
        self.assertEqual(history[0]['uuid'],history[1]['uuid'])
        tracker.consume(0x01, b'truncated')
        tracker.consume(0x4D, varint(0))
        snapshot = client.snapshot()
        self.assertNotIn('entities', snapshot)
        self.assertIn('entities', snapshot['observation_errors'])

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


class SubjectProbeChunkTests(unittest.TestCase):
    def test_subject_probe_resolves_fixture_blocks_from_received_chunks(self):
        # wire.py drops observer.blocks entries when a LevelChunkWithLight arrives,
        # so read_block_state can only resolve chunk-data blocks when the client knows
        # its dimension minimum. Subject probes must get it too, not only random mode.
        from .backends import protocol_matrix
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            dim_dir = root / 'vanilla' / 'data' / 'minecraft' / 'dimension_type'
            dim_dir.mkdir(parents=True)
            (dim_dir / 'overworld.json').write_text(json.dumps({'min_y': -64}))
            server_bin = root / 'server-bin'
            server_bin.write_bytes(b'fake-server')
            artifact = root / 'artifact'
            artifact.mkdir()
            ctx = SimpleNamespace(server_bin=server_bin, artifact_dir=artifact,
                                  plugins_dir=None,
                                  extra_args=['--subject', 'combat.mob_targets_player',
                                              '--vanilla-data', str(root / 'vanilla')],
                                  client_evidence={}, commands=[], cwds=[], exits=[], logs=[])
            server = Mock()
            server.config_path = artifact / 'runproto.toml'
            server.log_path = artifact / 'server.log'
            server.running = False
            client = Mock()
            client.observer = SimpleNamespace()
            client.snapshot.return_value = {}
            with patch.object(protocol_matrix, 'ProtocolServer', return_value=server), \
                 patch.object(protocol_matrix, 'ProtocolMatrixClient', return_value=client), \
                 patch.object(protocol_matrix, 'prepare_access'), \
                 patch.object(protocol_matrix, 'select_plugin_profile', return_value=([], None)), \
                 patch.object(protocol_matrix, 'run_subject',
                              return_value=[{'passed': True, 'name': 'x', 'detail': {}}]):
                result = protocol_matrix.run(ctx)
        self.assertEqual(result['protocol_matrix']['status'], 'pass')
        self.assertEqual(client.chunk_min_y, -64)
        self.assertEqual(client.observer.chunk_min_y, -64)

    def test_dimension_min_y_reads_namespaced_and_plain_ids_and_fails_closed(self):
        from .backends.protocol_matrix import dimension_min_y
        with tempfile.TemporaryDirectory() as tmp:
            vanilla = Path(tmp)
            dim_dir = vanilla / 'data' / 'minecraft' / 'dimension_type'
            dim_dir.mkdir(parents=True)
            (dim_dir / 'the_nether.json').write_text(json.dumps({'min_y': 0}))
            self.assertEqual(dimension_min_y(vanilla, 'the_nether'), 0)
            self.assertEqual(dimension_min_y(vanilla, 'minecraft:the_nether'), 0)
            with self.assertRaises((OSError, KeyError, ValueError)):
                dimension_min_y(vanilla, 'overworld')


if __name__=='__main__': unittest.main()
