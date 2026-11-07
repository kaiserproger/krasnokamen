"""Compact received chunk palettes, with no guessed dimension origin."""
import struct
import threading
from types import SimpleNamespace
import unittest

from .matrix_api import MissingCapability
from .protocol_chunks import decode_chunk, block_state
from .protocol_matrix_client import ProtocolMatrixClient
from .wire import write_varint


def chunk(palette):
    sections = b'\0\0\0\0' + palette + b'\0\0'
    return struct.pack('>ii', -2, 3) + b'\0' + write_varint(len(sections)) + sections


def packed(bits, values):
    per_word = 64 // bits
    words = []
    for start in range(0, len(values), per_word):
        word = sum(value << (index * bits) for index, value in enumerate(values[start:start + per_word]))
        words.append(struct.pack('>Q', word))
    return b''.join(words)


class ChunkObservationTests(unittest.TestCase):
    def test_singleton_has_no_array_length(self):
        coords, sections = decode_chunk(chunk(b'\0' + write_varint(1234)))
        self.assertEqual(coords, (-2, 3))
        self.assertEqual(block_state(sections, (-17, -49, 63), -64), 1234)
        with self.assertRaisesRegex(RuntimeError, 'outside received'):
            block_state(sections, (-17, -48, 63), -64)

    def test_indirect_values_do_not_cross_long_boundaries(self):
        values = [index % 3 for index in range(4096)]
        palette = b'\x05\x03\x01\x02\x03' + packed(5, values)
        _, sections = decode_chunk(chunk(palette))
        for index in (11, 12, 255, 256, 4095):
            position = (index % 16, -64 + index // 256, (index // 16) % 16)
            self.assertEqual(block_state(sections, position, -64), index % 3 + 1)

    def test_direct_palette_and_truncation_fail_closed(self):
        values = [index for index in range(4096)]
        body = chunk(b'\x0f' + packed(15, values))
        _, sections = decode_chunk(body)
        self.assertEqual(block_state(sections, (15, -49, 15), -64), 4095)
        for bad in (body[:-1], chunk(b'\x21'), chunk(b'\x04\0')):
            with self.assertRaises(RuntimeError):
                decode_chunk(bad)

    def test_unknown_dimension_cannot_become_guessed_air(self):
        _, sections = decode_chunk(chunk(b'\0\x01'))
        client = ProtocolMatrixClient.__new__(ProtocolMatrixClient)
        client.block_states = {1: {'block_id': 'minecraft:stone'}}
        client.observer = SimpleNamespace(condition=threading.Condition(), blocks={},
                                         chunk_min_y=None, chunk_sections={(-2, 3): sections})
        with self.assertRaises(MissingCapability):
            client.read_block_state((-17, -64, 48))
        client.observer.chunk_min_y = -64
        self.assertEqual(client.read_block_state((-17, -64, 48)), {'block_id': 'minecraft:stone'})
        client.observer.blocks[(-17, -64, 48)] = 2
        client.block_states[2] = {'block_id': 'minecraft:dirt'}
        self.assertEqual(client.read_block_state((-17, -64, 48)), {'block_id': 'minecraft:dirt'})


if __name__ == '__main__':
    unittest.main()
