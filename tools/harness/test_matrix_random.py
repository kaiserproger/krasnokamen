"""Observed random-matrix regressions and retained game-state contracts."""
import copy
import json
from pathlib import Path
import struct
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import zlib

from . import wal_frame
from .quirk_parity import player
from .matrix_api import MissingCapability

from .matrix_domains.random import (invariant_checks, stock, setup, recovery_matches,
                                    _mine, _exact_slots, _action, _fixture_item_stock, _published)
from .quirk_parity.seeded import player_scenario


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
            input_pending = [0]
            def click(slot,**kwargs):
                if slot==36:
                    send(0x14,'0002002400'); send(0x60,'01a3010000')
                elif slot==1:
                    send(0x14,'00040000043f0000')
                    input_pending[0] = 2
                elif slot==0:
                    send(0x14,'0006000000')
                else:
                    raise AssertionError(slot)
            client.click_slot=click
            deliver=[True]
            def wait(predicate,*,description,timeout_seconds=30):
                if input_pending[0] == 2:
                    # The click acknowledgement is only the early result-slot
                    # packet. A second wait must fence input AND cursor receipt.
                    input_pending[0] = 1
                    observer.wait(lambda p: p.menus[0]['state_id'] == 4,
                                  description='early craft result received')
                elif input_pending[0] == 1:
                    input_pending[0] = 0
                    self.assertFalse(predicate())
                    send(0x14,'0005000101a3010000')
                    observer.wait(lambda p: p.menus[0]['state_id'] == 5,
                                  description='craft input received without cursor correction')
                    self.assertFalse(predicate(), 'craft accepted input while the received cursor still holds the log')
                    if deliver[0]:
                        send(0x60,'00')
                return observer.wait(lambda _:predicate(),description=description,timeout=0.1)
            client.wait_until=wait
            ctx=SimpleNamespace(clients=[client])
            with patch('tools.harness.matrix_domains.random._supply'), \
                 patch('tools.harness.matrix_domains.random._close'):
                try:
                    _action(ctx,{'actor':0,'verb':'craft','count':2,'target':0})
                except TimeoutError as error:
                    self.fail(f"seed 19 crafting failed to consume delivered transitions: {error}")
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

    def test_vanilla_craft_cursor_correction_uses_transmitted_prediction(self):
        import socket
        import threading
        from . import wire
        from .protocol_matrix_client import ProtocolMatrixClient

        local, remote = socket.socketpair()
        stopped = threading.Event()
        errors = []
        clicks = []
        observer = None
        worker = None

        def serve():
            try:
                for expected_slot in (36, 1, 0):
                    packet, body = wire.read_compressed_frame(remote)
                    while packet == 0x0D:
                        packet, body = wire.read_compressed_frame(remote)
                    self.assertEqual(packet, 0x12)
                    window, offset = wire.read_varint_bytes(body)
                    _, offset = wire.read_varint_bytes(body, offset)
                    slot, button = struct.unpack_from(">hb", body, offset)
                    mode, offset = wire.read_varint_bytes(body, offset + 3)
                    changed, offset = wire.read_varint_bytes(body, offset)
                    self.assertEqual((window, slot, changed), (0, expected_slot, 0))
                    prediction = body[offset:]
                    clicks.append((slot, button, mode, prediction.hex()))
                    if slot == 36:
                        wire.send_compressed_frame(remote, 0x14, bytes.fromhex("0002002400"))
                        actual_cursor = bytes.fromhex("01a3010000")
                    elif slot == 1:
                        self.assertEqual((button, mode), (1, 0))
                        wire.send_compressed_frame(remote, 0x14, bytes.fromhex("00040000043f0000"))
                        wire.send_compressed_frame(remote, 0x14, bytes.fromhex("0005000101a3010000"))
                        actual_cursor = b"\x00"
                    else:
                        self.assertEqual(mode, 1)
                        wire.send_compressed_frame(remote, 0x14, bytes.fromhex("0006000000"))
                        actual_cursor = b"\x00"
                    # Vanilla sends a correction only when the transmitted
                    # prediction differs. No observer/wait hook injects it.
                    if prediction != actual_cursor:
                        wire.send_compressed_frame(remote, 0x60, actual_cursor)
            except Exception as error:
                if not stopped.is_set():
                    errors.append(error)

        try:
            with patch.object(wire, "join_offline_socket", return_value=(local, {})):
                observer = wire.PlayObserver(("unused", 0), "Probe", account_uuid=None, timeout=1)
            client = object.__new__(ProtocolMatrixClient)
            client.observer = observer
            client.items = {163: "minecraft:oak_log", 63: "minecraft:oak_planks"}
            client.teleport_to = lambda *args: None
            client.open_inventory = lambda: None
            client.wait_until = lambda predicate, **kwargs: observer.wait(
                lambda _: predicate(), description=kwargs["description"], timeout=1)
            slots = [b"\x00"] * 46
            slots[36] = bytes.fromhex("01a3010000")
            wire.send_compressed_frame(remote, 0x12, bytes([0, 1, 46]) + b"".join(slots) + b"\x00")
            observer.wait(lambda p: 0 in p.menus, description="initial player menu")
            worker = threading.Thread(target=serve, daemon=True)
            worker.start()

            def craft(matrix):
                _action(matrix, {"actor": 0, "verb": "craft", "count": 2, "target": 0})
                matrix.state["final"] = client.snapshot()
                return [{"passed": True}]

            with tempfile.TemporaryDirectory() as directory:
                fixture = Path(directory)
                jar = fixture / "server.jar"
                jar.write_bytes(b"socket fixture")
                dimension = fixture / "data/minecraft/dimension_type/overworld.json"
                dimension.parent.mkdir(parents=True)
                dimension.write_text('{"min_y": -64}')
                ctx = SimpleNamespace(artifact_dir=fixture, client_evidence={}, logs=[])
                with patch.object(player, "VanillaMatrixServer") as server, \
                     patch.object(player, "ProtocolMatrixClient", return_value=client), \
                     patch.object(player, "run_subject", side_effect=craft), \
                     patch("tools.harness.matrix_domains.random._supply"), \
                     patch("tools.harness.matrix_domains.random._close"):
                    server.return_value.log_path = fixture / "server.log"
                    try:
                        final = player.run_player_script(ctx, player_scenario(19, 1, 2), jar, fixture)
                    except TimeoutError as error:
                        self.fail(f"vanilla craft retained its old cursor without a wire correction: {error}")
                self.assertIsNone(final["cursor"])
                self.assertEqual(final["menu"]["state_id"], 6)
            self.assertEqual([click[0] for click in clicks], [36, 1, 0])
            self.assertFalse(errors, errors)
        finally:
            stopped.set()
            if observer is not None:
                observer.close()
            else:
                local.close()
            remote.close()
            if worker is not None:
                worker.join(timeout=2)
                self.assertFalse(worker.is_alive(), "wire fixture thread did not stop")

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

    def test_explicit_resource_units_preserve_crafting_inputs_and_outputs(self):
        self.assertEqual(stock([{'item_id': 'minecraft:oak_log', 'count': 2}]),
                         stock([{'item_id': 'minecraft:oak_planks', 'count': 8}]))
        self.assertNotEqual(stock([{'item_id': 'minecraft:oak_log', 'count': 2}]),
                            stock([{'item_id': 'minecraft:oak_planks', 'count': 9}]))
        self.assertEqual(stock([{'item_id': 'minecraft:diamond', 'count': 1}]),
                         {'minecraft:diamond': 1})

    def test_kill9_fence_reads_the_current_wal_frame_and_refuses_an_older_one(self):
        with tempfile.TemporaryDirectory() as tmp:
            journal = Path(tmp) / 'journal'
            journal.mkdir()
            ctx = SimpleNamespace(server=SimpleNamespace(world_dir=Path(tmp)))
            identity = (bytes(range(1, 17)), bytes(range(51, 59)), bytes(range(101, 109)))
            batch = journal / wal_frame.batch_name(7, 9)
            batch.write_bytes(wal_frame.encode(7, 9, b'{}\n', identity))
            self.assertEqual(_published(ctx, 8), {'path': str(batch), 'low': 7, 'high': 9})
            # The server refuses a format-1 frame by name, so the fence never counts one as published.
            (journal / wal_frame.batch_name(10, 10)).write_bytes(
                struct.pack('<IIQQII', wal_frame.MAGIC, 1, 10, 10, 3, zlib.crc32(b'{}\n')) + b'{}\n')
            with self.assertRaisesRegex(AssertionError, 'format 1'):
                _published(ctx, 10)


if __name__ == '__main__':
    unittest.main()
