"""Hermetic unit regression tests for protocol_input_verbs.ProtocolInputVerbs.

No server is started: packet writes are captured by a fake observer and
checked byte-for-byte against the exact vectors from the local mc-protocol
sources. Run with:
python3 -m unittest tools.harness.test_protocol_input_verbs
"""
from __future__ import annotations

import struct
import unittest

from . import protocol_input_verbs as verbs
from . import wire
from .matrix_api import MissingCapability


class _Observer:
    def __init__(self, game_time=0):
        self.sent = []
        self.sequence = 0
        self.game_time = game_time
        self.block_ack_sequence = 0

    def send(self, packet_id, body=b''):
        if packet_id == verbs.USE_ITEM_ID:
            self.block_ack_sequence = self.sequence
        self.sent.append((packet_id, bytes(body)))


class _Client(verbs.ProtocolInputVerbs):
    def __init__(self, observer):
        self.observer = observer
        self.timeout_seconds = 30

    def wait_until(self, predicate, *, description, timeout_seconds=30):
        # Simulate SetTime-driven ticks without sleeps: advance game_time
        # until the predicate holds, mirroring the event-driven wait.
        for _ in range(10000):
            if predicate():
                return True
            if self.observer.game_time is None:
                raise AssertionError(description)
            self.observer.game_time += 1
        raise AssertionError(description)


def _packed(position):
    return wire.pack_block_position(position)


class EncodingTests(unittest.TestCase):
    def test_select_hotbar_uses_263_set_carried_item_packet(self):
        from .protocol_matrix_client import ProtocolMatrixClient
        client = object.__new__(ProtocolMatrixClient)
        client.observer = _Observer()
        client.select_hotbar(3)
        self.assertEqual(client.observer.sent, [(0x36, b'\x00\x03'), (0x0D, b'')])

    def test_break_block_sends_start_then_stop_with_sequence(self):
        observer = _Observer()
        self.assertEqual(_Client(observer).break_block((10, -60, -7), face='up'), 2)
        position = _packed((10, -60, -7))
        self.assertEqual(
            observer.sent,
            [(0x29, b'\x00' + position + b'\x01\x01'), (0x0D, b''),
             (0x29, b'\x03' + position + b'\x01\x02'), (0x0D, b'')])

    def test_break_block_rejects_fabricated_drop_proof(self):
        observer = _Observer()
        client = _Client(observer)
        with self.assertRaises(MissingCapability):
            client.break_block((0, 64, 0), expected_drop_item_id='minecraft:cobblestone')
        with self.assertRaises(MissingCapability):
            client.break_block((0, 64, 0), expected_drop_count=1)
        self.assertEqual(observer.sent, [])

    def test_break_block_phases_send_exact_actions(self):
        observer = _Observer()
        client = _Client(observer)
        self.assertEqual(client.break_block((10, -60, -7), face='up', phase='start'), 1)
        self.assertEqual(client.break_block((10, -60, -7), face='up', phase='abort'), 2)
        position = _packed((10, -60, -7))
        self.assertEqual(
            observer.sent,
            [(0x29, b'\x00' + position + b'\x01\x01'), (0x0D, b''),
             (0x29, b'\x02' + position + b'\x01\x02'), (0x0D, b'')])
        with self.assertRaises(ValueError):
            client.break_block((0, 64, 0), phase='half')

    def test_break_block_rejects_bad_face_and_position(self):
        client = _Client(_Observer())
        with self.assertRaises(ValueError):
            client.break_block((0, 64, 0), face='sideways')
        with self.assertRaises(ValueError):
            client.break_block((1 << 26, 64, 0))

    def test_use_item_matches_javap_layout(self):
        observer = _Observer(game_time=50)
        _Client(observer).use_item(ticks=3)
        # Exact known source: main hand, sequence 1, zero rotation; compare to
        # the pinned 300/90/-15 vector shape (hand, sequence, yRot, xRot).
        self.assertEqual(observer.sent,
                         [(0x43, b'\x00\x01' + struct.pack('>ff', 0.0, 0.0)), (0x0D, b'')])
        self.assertEqual(observer.game_time, 54)
        with self.assertRaises(ValueError):
            _Client(_Observer(game_time=1)).use_item(hand='third')
        with self.assertRaises(ValueError):
            _Client(_Observer(game_time=1)).use_item(ticks=0)

    def test_use_hold_starts_after_admission_and_new_time_sample(self):
        class DelayedAckObserver(_Observer):
            def send(self, packet_id, body=b''):
                self.sent.append((packet_id, bytes(body)))
        observer = DelayedAckObserver(game_time=332)
        class CoarseClock(_Client):
            def wait_until(self, predicate, *, description, timeout_seconds=30):
                if 'admission' in description:
                    if predicate():
                        raise AssertionError('accepted a stale block ACK')
                    observer.block_ack_sequence = observer.sequence
                    self.asserted_admission = True
                else:
                    for _ in range(10):
                        if predicate():
                            return True
                        observer.game_time += 20
                if not predicate():
                    raise AssertionError(description)
                return True
        client = CoarseClock(observer)
        client.use_item(ticks=20)
        self.assertTrue(getattr(client, 'asserted_admission', False))
        self.assertGreaterEqual(observer.game_time, 372)

    def test_release_and_swap_use_player_action_ids(self):
        observer = _Observer()
        client = _Client(observer)
        client.release_use_item()
        client.swap_hands()
        zero = _packed((0, 0, 0))
        self.assertEqual(observer.sent,
                         [(0x29, b'\x06' + zero + b'\x00\x01'), (0x0D, b''),
                          (0x29, b'\x07' + zero + b'\x00\x02'), (0x0D, b'')])

    def test_respawn_sends_perform_respawn(self):
        observer = _Observer()
        _Client(observer).respawn()
        self.assertEqual(observer.sent, [(0x0C, b'\x00'), (0x0D, b'')])

    def test_interact_entity_matches_local_vector(self):
        observer = _Observer()
        _Client(observer).interact_entity(123)
        # Exact known source: [0x7B, 0x00, 0x00, 0x00] = entity 123, main
        # hand, zero LP Vec3, not secondary.
        self.assertEqual(observer.sent, [(0x1A, bytes([0x7B, 0x00, 0x00, 0x00])), (0x0D, b'')])
        with self.assertRaises(ValueError):
            _Client(_Observer()).interact_entity(-1)
        with self.assertRaises(ValueError):
            _Client(_Observer()).interact_entity(1, location=(0.0, float('nan'), 0.0))
        secondary = _Observer()
        _Client(secondary).interact_entity(1, hand='off', secondary=True)
        self.assertEqual(secondary.sent, [(0x1A, b'\x01\x01\x00\x01'), (0x0D, b'')])

    def test_move_to_sends_position_packet(self):
        observer = _Observer()
        _Client(observer).move_to(1.5, 80.0, -2.5, on_ground=False)
        self.assertEqual(observer.sent,
                         [(0x1E, struct.pack('>dddB', 1.5, 80.0, -2.5, 0)), (0x0D, b'')])
        with self.assertRaises(ValueError):
            _Client(_Observer()).move_to(float('nan'), 0, 0)

    def test_press_inputs_flag_bits_match_javap(self):
        observer = _Observer()
        _Client(observer).press_inputs(forward=True, right=True, jump=True,
                                       sprint=True, ticks=2)
        # Exact known source: [0x59] = forward+right+jump+sprint.
        self.assertEqual(observer.sent,
                         [(0x2B, b'\x59'), (0x0D, b'')])
        self.assertEqual(observer.game_time, 2)
        with self.assertRaises(ValueError):
            _Client(_Observer()).press_inputs(ticks=0)

    def test_press_inputs_sneak_bit_and_shortcuts(self):
        observer = _Observer()
        client = _Client(observer)
        client.sneak(True)
        client.sprint(False)
        client.jump(ticks=2)
        self.assertEqual(observer.sent,
                         [(0x2B, b'\x20'), (0x0D, b''),
                          (0x2B, b'\x00'), (0x0D, b''),
                          (0x2B, b'\x10'), (0x0D, b'')])

    def test_move_is_explicit_capability_failure(self):
        with self.assertRaisesRegex(MissingCapability, r'^client:move_physics$'):
            _Client(_Observer()).move(1.0, 0.0, 5)

    def test_wait_ticks_uses_observed_game_time(self):
        observer = _Observer(game_time=100)
        self.assertEqual(_Client(observer).wait_ticks(10), 110)
        self.assertEqual(observer.sent, [])
        self.assertEqual(_Client(_Observer(game_time=7)).wait_ticks(0), 7)
        with self.assertRaises(AssertionError):
            _Client(_Observer(game_time=None)).wait_ticks(1)
        with self.assertRaises(ValueError):
            _Client(observer).wait_ticks(-1)

    def test_update_sign_layout_matches_stream_codec(self):
        observer = _Observer()
        _Client(observer).update_sign((1, 65, -2), ['one', 'two', 'three', 'four'],
                                      front=False)
        expected = (_packed((1, 65, -2)) + b'\x00'
                    + wire.mc_string('one') + wire.mc_string('two')
                    + wire.mc_string('three') + wire.mc_string('four'))
        self.assertEqual(observer.sent, [(0x3D, expected), (0x0D, b'')])
        client = _Client(_Observer())
        with self.assertRaises(ValueError):
            client.update_sign((0, 0, 0), ['only'])
        with self.assertRaises(ValueError):
            client.update_sign((0, 0, 0), ['x' * 385, '', '', ''])

    def test_edit_book_matches_exact_vector_shape(self):
        observer = _Observer()
        _Client(observer).edit_book(40, ['one'], title='title')
        # Exact known source vector: 28 01 03 one 01 05 title.
        self.assertEqual(observer.sent,
                         [(24, b'\x28\x01\x03one\x01\x05title'), (0x0D, b'')])
        observer = _Observer()
        _Client(observer).edit_book(0, [])
        self.assertEqual(observer.sent, [(24, b'\x00\x00\x00'), (0x0D, b'')])
        client = _Client(_Observer())
        with self.assertRaises(ValueError):
            client.edit_book(0, ['x'] * 101)
        with self.assertRaises(ValueError):
            client.edit_book(0, ['ok'], title='x' * 33)

    def test_client_information_matches_ru_layout(self):
        observer = _Observer()
        client = _Client(observer)
        client.set_client_information(view_distance=8, locale='ru_ru')
        client.configure_chat('hidden')
        # Exact known source layout: 05 ru_ru, 08, 02, 00, 55, 00, 01, 00, 02
        # with the default model/hand/filtering/listing/particle fields.
        first = (b'\x05ru_ru' + bytes([8, 0, 1, 0x7F, 1, 0, 1, 0]))
        second = (b'\x05ru_ru' + bytes([8, 2, 1, 0x7F, 1, 0, 1, 0]))
        self.assertEqual(observer.sent,
                         [(0x0E, first), (0x0D, b''), (0x0E, second), (0x0D, b'')])
        with self.assertRaises(ValueError):
            client.set_client_information()
        with self.assertRaises(ValueError):
            client.set_client_information(view_distance=64)
        with self.assertRaises(ValueError):
            client.configure_chat('sometimes')


if __name__ == '__main__':
    unittest.main()
