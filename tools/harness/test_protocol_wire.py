"""Protocol matrix observations fail closed and preserve actual wire values."""
import socket
import struct
import unittest
import uuid
from unittest.mock import Mock, patch
from . import wire
from .protocol_observations import decode_menu


class ProtocolWireTests(unittest.TestCase):
    def test_received_container_suffix_survives_menu_closure(self):
        observer = wire.PlayObserver.__new__(wire.PlayObserver)
        observer.menus = {0: {'slots': [None] * 46},
                          4: {'slots': [None] * 63}}
        observer.menus[4]['slots'][27] = {'item_id': 1, 'count': 4}
        observer.menus[4]['slots'][62] = {'item_id': 2, 'count': 7}
        observer._cache_player_suffix(4)
        self.assertEqual(observer.menus[0]['slots'][9], {'item_id': 1, 'count': 4})
        self.assertEqual(observer.menus[0]['slots'][44], {'item_id': 2, 'count': 7})
        self.assertIsNone(observer.menus[0]['slots'][0])

    def test_plain_menu_decodes_slots_and_cursor(self):
        # window 4, state 7, 3 slots: empty, item 42 x 16, empty; cursor 42 x 2.
        body = bytes([4, 7, 3, 0, 16, 42, 0, 0, 0, 2, 42, 0, 0])
        self.assertEqual(decode_menu(body),
                         {'window': 4, 'state_id': 7, 'slots': [None,
                          {'item_id':42,'count':16,'components':{},'removed':[]}, None],
                          'cursor': {'item_id':42,'count':2,'components':{},'removed':[]}})

    def test_component_menu_is_not_silently_treated_as_plain(self):
        with self.assertRaisesRegex(wire.UnsupportedObservation, 'component'):
            decode_menu(bytes([0, 1, 1, 1, 42, 1, 0, 1, 0, 0]))

    def test_trailing_and_truncated_menu_rejected(self):
        for body in (bytes([0, 0, 0, 0, 99]), bytes([0, 0, 1])):
            with self.subTest(body=body), self.assertRaises(RuntimeError):
                decode_menu(body)

    def test_block_position_signed_coordinates(self):
        self.assertEqual(wire.pack_block_position((-1, -64, -2)),
                         struct.pack('>Q', ((2**26-1) << 38) | ((2**26-2) << 12) | (4096-64)))

    def test_unknown_click_mode_rejected_before_write(self):
        with self.assertRaises(ValueError):
            wire.click_body(1, 0, 0, 0, 7)

    def test_click_wire_has_empty_prediction_not_item_stack(self):
        self.assertEqual(wire.click_body(4, 7, 12, 0, 1), bytes([4, 7, 0, 12, 0, 1, 0, 0]))

    def test_compressed_frame_roundtrip(self):
        a, b = socket.socketpair()
        try:
            wire.send_compressed_frame(a, 0x12, b'hello')
            self.assertEqual(wire.read_compressed_frame(b), (0x12, b'hello'))
        finally:
            a.close()
            b.close()

    def test_section_block_updates_preserve_negative_sections_and_state_ids(self):
        section = ((-2 & 0x3fffff) << 42) | ((3 & 0x3fffff) << 20) | (-4 & 0xfffff)
        def varlong(value):
            out=bytearray()
            while value > 127:
                out.append((value & 127) | 128); value >>= 7
            out.append(value)
            return bytes(out)
        body=struct.pack('>Q',section)+wire.write_varint(2)
        body+=varlong((131072 << 12) | (15 << 8) | (2 << 4) | 1)
        body+=varlong(0)
        self.assertEqual(wire.decode_section_blocks(body),
                         {(-17,-63,50):131072,(-32,-64,48):0})
        for malformed in (body[:-1],body+b'\0',struct.pack('>Q',section)+wire.write_varint(4097)):
            with self.assertRaises(RuntimeError):
                wire.decode_section_blocks(malformed)

    def test_join_preserves_received_configuration_registry_names(self):
        registry = (wire.mc_string('minecraft:enchantment') + b'\x02'
                    + wire.mc_string('minecraft:mending') + b'\x00'
                    + wire.mc_string('minecraft:sharpness') + b'\x00')
        sock = Mock()
        accepted_uuid = uuid.UUID(int=42)
        login = accepted_uuid.bytes + wire.mc_string('RegistryProbe') + b'\x00'
        packets = [(wire.LOGIN_CLIENTBOUND_SUCCESS, login),
                   (wire.CONFIG_CLIENTBOUND_KNOWN_PACKS, b''),
                   (0x07, registry), (wire.CONFIG_CLIENTBOUND_FINISH, b''),
                   (wire.PLAY_CLIENTBOUND_LOGIN, struct.pack('>i', 42))]
        with patch.object(wire.socket, 'create_connection', return_value=sock), \
             patch.object(wire, 'read_frame', return_value=(
                 wire.LOGIN_CLIENTBOUND_SET_COMPRESSION, b'\x00')), \
             patch.object(wire, 'read_compressed_frame', side_effect=packets):
            actual_sock, joined = wire.join_offline_socket(
                ('unused', 26100), 'RegistryProbe', account_uuid=uuid.UUID(int=1))
        self.assertIs(actual_sock, sock)
        self.assertEqual(joined['entity_id'], 42)
        self.assertEqual(joined['uuid'], str(accepted_uuid))
        self.assertEqual(joined['registries']['minecraft:enchantment'], {
            '0': 'minecraft:mending', '1': 'minecraft:sharpness'})

    def test_observer_tracks_actual_block_ack_not_generic_revision(self):
        a,b=socket.socketpair()
        observer=None
        try:
            with patch.object(wire,'join_offline_socket',return_value=(a,{})):
                observer=wire.PlayObserver(('unused',0),'Probe',account_uuid=None,timeout=1)
            self.assertIsNone(observer.block_ack_sequence)
            wire.send_compressed_frame(b,0x71,struct.pack('>q',100))
            observer.wait(lambda p:p.game_time==100,description='actual time packet')
            self.assertIsNone(observer.block_ack_sequence)
            wire.send_compressed_frame(b,0x04,wire.write_varint(7))
            observer.wait(lambda p:p.block_ack_sequence==7,description='exact sequence ACK')
        finally:
            if observer is not None: observer.close()
            else: a.close()
            b.close()

    def test_received_event_indices_do_not_roll_over_at_256(self):
        from .test_protocol_observations import text_nbt
        a, b = socket.socketpair()
        observer = None
        try:
            with patch.object(wire, 'join_offline_socket', return_value=(a, {})):
                observer = wire.PlayObserver(('unused', 0), 'Probe',
                                             account_uuid=None, timeout=1)
            for index in range(300):
                wire.send_compressed_frame(b, 0x79, text_nbt(f'event-{index}') + b'\x00')
            wire.send_compressed_frame(b, 0x71, struct.pack('>q', 100))
            observer.wait(lambda p: p.game_time == 100, description='chat stream barrier')
            self.assertEqual(len(observer.events), 300)
            self.assertEqual(observer.events[255:257], ['event-255', 'event-256'])
        finally:
            if observer is not None: observer.close()
            else: a.close()
            b.close()


    def test_263_packet_insertions_never_enter_legacy_decoders(self):
        for new_id, decoder_id in ((36, 36), (38, 37), (82, 81),
                                   (84, 82), (122, 120), (124, 121)):
            self.assertEqual(wire.play_packet_id(new_id, inbound=True, protocol=777), decoder_id)
        for new_id in (37, 83, 123):
            self.assertEqual(wire.play_packet_id(new_id, inbound=True, protocol=777), -1)
        self.assertEqual(wire.play_packet_id(53, inbound=False, protocol=777), 54)
        with self.assertRaises(wire.UnsupportedObservation):
            wire.play_packet_id(1, inbound=True, protocol=999)

    def test_263_teleport_confirmation_contains_resulting_pose(self):
        a, b = socket.socketpair()
        b.settimeout(1)
        observer = None
        try:
            with patch.object(wire, 'join_offline_socket', return_value=(a, {})):
                observer = wire.PlayObserver(('unused', 0), 'Probe',
                                             account_uuid=None, timeout=1, protocol=777)
            body = wire.write_varint(7) + struct.pack('>ddddddffi', 1, 80, 2, 0, 0, 0, 45, 10, 0)
            wire.send_compressed_frame(b, 0x49, body)
            packet, confirmation = wire.read_compressed_frame(b)
            self.assertEqual(packet, 0)
            self.assertEqual(confirmation, wire.write_varint(7) + struct.pack('>dddff', 1, 80, 2, 45, 10))
        finally:
            if observer is not None:
                observer.close()
            else:
                a.close()
            b.close()


if __name__ == '__main__':
    unittest.main()
