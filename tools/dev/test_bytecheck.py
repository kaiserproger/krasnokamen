"""WAL v2 framing and X1/X2/X3 classifier tests, not producer byte proofs."""
import base64
import copy
import json
import struct
import unittest
import zlib

from tools.dev import bytecheck


def frame(entries):
    payload = b''.join(json.dumps(entry, separators=(',', ':')).encode() + b'\n'
                       for entry in entries)
    header = struct.pack('<II16s8sQQII', 0x534F4C4A, 2, bytes(range(16)),
                         bytes(range(8)), 2, 2, len(payload), zlib.crc32(payload))
    return header + payload


def artifact(raw):
    return {'batch-00000000000000000002-00000000000000000002.bin':
            base64.b64encode(raw).decode()}


def control_images(control):
    stable = 'global/transit/stable'
    base = {name: {} for name in bytecheck.ARTIFACTS}
    base['_stable_transit'] = stable
    for name in ('A', 'C-live', 'C-offline'):
        base[name] = {key: {'payload': 'before'} for key in
                      [stable, 'global/transit/gone', 'entity/1',
                       'global/settlement/1', 'global/settlement/2',
                       'global/settlement/3']}
    entries = [{'Dimension': {'dimension': 'Overworld', 'entries': [
        {'Settlement': {'owner_name': 'same', 'anchor': [0, 80, 0]}},
        {'Entity': {'mining_progress': 1, 'lumber_progress': 2}},
        {'Transit': {'record': '{"mining_progress":1,"lumber_progress":2}'}},
    ]}}]
    base['B'] = artifact(frame(entries))
    candidate = copy.deepcopy(base)
    changed = {
        'X1': {'A': ['global/settlement/1', 'global/settlement/2', 'global/settlement/3'],
               'C-live': ['global/settlement/1', 'global/settlement/3'],
               'C-offline': ['global/settlement/1']},
        'X2': {'A': [stable, 'global/transit/gone'], 'C-live': [stable]},
        'X3': {name: ['entity/1'] for name in ('A', 'C-live', 'C-offline')},
    }[control]
    for stage, keys in changed.items():
        for key in keys:
            candidate[stage][key]['payload'] = 'after'
    images = entries[0]['Dimension']['entries']
    if control == 'X1':
        images[0]['Settlement'] = {'anchor': [0, 80, 0], 'owner_name': 'same'}
    elif control == 'X2':
        images[1]['Entity'] = {'lumber_progress': 2, 'mining_progress': 1}
        images[2]['Transit']['record'] = '{"lumber_progress":2,"mining_progress":1}'
    else:
        candidate['D-export'] = {'entities/r.0.0.mca': 'changed NBT bytes'}
    candidate['B'] = artifact(frame(entries))
    return base, candidate


class WalClassificationTests(unittest.TestCase):
    def test_x1_x2_x3_expected_control_classification(self):
        for control, variants in [('X1', ['Settlement']), ('X2', ['Entity', 'Transit']),
                                  ('X3', [])]:
            with self.subTest(control=control):
                base, candidate = control_images(control)
                verdict = bytecheck.compare(base, candidate, control)
                self.assertEqual(verdict['control']['result'], 'RED')
                self.assertEqual(verdict['control']['wal_variants'], variants)

    def test_store_uuid_bytes_still_count_in_whole_frame_comparison(self):
        base, candidate = control_images('X3')
        raw = bytearray(base64.b64decode(next(iter(candidate['B'].values()))))
        raw[8] ^= 1
        candidate['B'] = artifact(raw)
        self.assertEqual(bytecheck.wal_lines(base['B']), bytecheck.wal_lines(candidate['B']))
        with self.assertRaisesRegex(SystemExit, 'changed artifacts'):
            bytecheck.compare(base, candidate, 'X3')

    def test_incomplete_headers_refuse(self):
        raw = frame([{'Entity': {'id': 1}}])
        for length in range(56):
            with self.subTest(length=length):
                with self.assertRaisesRegex(SystemExit, 'incomplete WAL v2'):
                    bytecheck.wal_lines(artifact(raw[:length]))

    def test_non_v2_versions_refuse(self):
        for version in (0, 1, 3):
            with self.subTest(version=version):
                raw = bytearray(frame([{'Entity': {'id': 1}}]))
                struct.pack_into('<I', raw, 4, version)
                with self.assertRaisesRegex(SystemExit, 'unsupported WAL frame'):
                    bytecheck.wal_lines(artifact(raw))

    def test_wrong_magic_refuses(self):
        raw = bytearray(frame([{'Entity': {'id': 1}}]))
        struct.pack_into('<I', raw, 0, 0)
        with self.assertRaisesRegex(SystemExit, 'unsupported WAL frame'):
            bytecheck.wal_lines(artifact(raw))

    def test_truncated_and_trailing_payloads_refuse(self):
        raw = frame([{'Entity': {'id': 1}}])
        for invalid in (raw[:-1], raw + b'\n'):
            with self.subTest(length=len(invalid)):
                with self.assertRaisesRegex(SystemExit, 'WAL frame length mismatch'):
                    bytecheck.wal_lines(artifact(invalid))

    def test_wrong_v2_length_refuses(self):
        raw = bytearray(frame([{'Entity': {'id': 1}}]))
        struct.pack_into('<I', raw, 48, len(raw))
        with self.assertRaisesRegex(SystemExit, 'WAL frame length mismatch'):
            bytecheck.wal_lines(artifact(raw))


if __name__ == '__main__':
    unittest.main()
