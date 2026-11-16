import json
import struct
import tempfile
import unittest
import zlib
from pathlib import Path

from tools.harness.crash_wal import classify_observation, read_published


def frame(directory, lo, hi, entries):
    payload = b''.join(json.dumps(entry).encode() + b'\n' for entry in entries)
    data = struct.pack('<IIQQII', 0x534F4C4A, 1, lo, hi, len(payload), zlib.crc32(payload)) + payload
    path = directory / f'batch-{lo:020}-{hi:020}.bin'
    path.write_bytes(data)
    return path


class CrashWalTest(unittest.TestCase):
    def test_classification_requires_ack_and_matching_durable_image(self):
        for ack in (False, True):
            for wal in (False, True):
                for checkpoint in (False, True):
                    for after in (False, True):
                        expected = ('unacknowledged' if not ack else
                                    'unpublished' if not (wal or checkpoint) else
                                    'survived' if after else 'mismatch')
                        self.assertEqual(classify_observation(acknowledged=ack, published_image=wal,
                                                             checkpoint_image=checkpoint,
                                                             observed_after=after), expected)

    def test_published_tail_and_unpublished_file(self):
        with tempfile.TemporaryDirectory() as temp:
            journal = Path(temp)
            frame(journal, 5, 5, [{'Player': {'inventory': ['marker']}}])
            (journal / 'batch-00000000000000000006-00000000000000000006.bin.tmp').write_bytes(b'partial')
            result = read_published(journal, 4)
            self.assertEqual(result['published_through'], 5)
            self.assertEqual(result['frames'][0]['entries'][0]['Player']['inventory'], ['marker'])
            self.assertEqual(len(result['unpublished']), 1)

    def test_gap_and_damaged_frame_fail_closed(self):
        with tempfile.TemporaryDirectory() as temp:
            journal = Path(temp)
            path = frame(journal, 7, 7, [])
            with self.assertRaisesRegex(ValueError, 'gap'):
                read_published(journal, 5)
            path.write_bytes(path.read_bytes() + b'x')
            with self.assertRaisesRegex(ValueError, 'checksum'):
                read_published(journal, 6)


if __name__ == '__main__':
    unittest.main()
