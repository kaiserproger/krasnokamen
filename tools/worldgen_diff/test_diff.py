import unittest
from pathlib import Path
from unittest.mock import patch

from tools.worldgen_diff import diff


def _pack(indices, bits):
    per = 64 // bits
    longs = []
    for start in range(0, len(indices), per):
        word = 0
        for slot, value in enumerate(indices[start:start + per]):
            word |= value << (slot * bits)
        longs.append(word - (1 << 64) if word >= 1 << 63 else word)
    return longs


def _chunk(palette, indices, y=0):
    return {'sections': [{'Y': y, 'block_states': {
        'palette': [{'Name': n, **({'Properties': p} if p else {})} for n, p in palette],
        'data': _pack(indices, max(4, (len(palette) - 1).bit_length()))}}]}


class DecodeTests(unittest.TestCase):
    def test_section_indices_follow_yzx_order_and_properties(self):
        registry = diff.Registry()
        indices = [0] * 4096
        indices[1] = 1  # x=1, y=0, z=0
        indices[256] = 2  # y=1
        chunk = _chunk([('minecraft:stone', None), ('minecraft:grass_block', {'snowy': 'false'}),
                        ('minecraft:water', {'level': '0'})], indices)
        blocks, _, _ = diff.decode_chunk(chunk, registry)
        base = (0 - diff.MIN_SECTION) * 4096
        self.assertEqual(registry.states[blocks[base + 1]], 'minecraft:grass_block[snowy=false]')
        self.assertEqual(registry.states[blocks[base + 256]], 'minecraft:water[level=0]')
        self.assertEqual(registry.states[blocks[0]], 'minecraft:air')

    def test_missing_chunks_fail_instead_of_reporting_partial_parity(self):
        with patch.object(diff, 'load_chunks', return_value={}):
            with self.assertRaisesRegex(ValueError, 'missing'):
                diff.compare(Path('vanilla'), Path('krasnokamen'), (0, 0), (0, 0))

    def test_unfinished_oracle_chunks_fail(self):
        unfinished = {'Status': 'minecraft:features', 'sections': []}
        with patch.object(diff, 'load_chunks', return_value={(0, 0): unfinished}):
            with self.assertRaisesRegex(ValueError, 'FULL'):
                diff.compare(Path('vanilla'), Path('krasnokamen'), (0, 0), (0, 0))

    def test_exact_properties_and_decoration_metrics(self):
        indices = [0] * 4096
        indices[0] = 1
        vanilla = _chunk([('minecraft:air', None),
                          ('minecraft:poppy', None)], indices)
        krasnokamen = _chunk([('minecraft:air', None),
                          ('minecraft:air', None)], indices)
        vanilla['Status'] = krasnokamen['Status'] = 'minecraft:full'
        vanilla['sections'][0]['biomes'] = {'palette': ['minecraft:plains']}
        krasnokamen['sections'][0]['biomes'] = {'palette': ['minecraft:forest']}
        with patch.object(diff, 'load_chunks',
                          side_effect=[{(0, 0): vanilla}, {(0, 0): krasnokamen}]):
            result, _ = diff.compare(Path('vanilla'), Path('krasnokamen'), (0, 0), (0, 0))
        self.assertEqual(result['exact_state_match_nonair_pct'], 0)
        self.assertEqual(result['metrics']['no_decor']['match_pct'], 100)
        self.assertEqual(result['name_mismatch_cells'], 1)
        self.assertEqual(result['metrics']['no_decor_state']['match_pct'], 100)
        self.assertEqual(result['exact_state_mismatch_cells'], 1)
        self.assertEqual(result['biome_cells'], 64)
        self.assertEqual(result['biome_mismatch_cells'], 64)

        vanilla = _chunk([('minecraft:water', {'level': '0'})], [0] * 4096)
        krasnokamen = _chunk([('minecraft:water', {'level': '1'})], [0] * 4096)
        vanilla['Status'] = krasnokamen['Status'] = 'minecraft:full'
        with patch.object(diff, 'load_chunks',
                          side_effect=[{(0, 0): vanilla}, {(0, 0): krasnokamen}]):
            result, _ = diff.compare(Path('vanilla'), Path('krasnokamen'), (0, 0), (0, 0))
        self.assertEqual(result['exact_state_match_nonair_pct'], 0)
        self.assertEqual(result['metrics']['name']['match_nonair_pct'], 100)
        self.assertEqual(result['metrics']['no_decor_state']['match_nonair_pct'], 0)


if __name__ == '__main__':
    unittest.main()
