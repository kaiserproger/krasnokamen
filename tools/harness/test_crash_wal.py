import json
import struct
import tempfile
import unittest
import zlib
from pathlib import Path

from tools.harness.backends.wal_forge import BATCH_MAGIC, BATCH_VERSION, HEADER, HEADER_LEN
from tools.harness.crash_wal import classify_observation, read_published


STORE, PALETTE = bytes(range(16)), bytes(range(8))


def frame(directory, lo, hi, entries, *, version=BATCH_VERSION):
    """One published batch packed with the *writer's* struct (`crates/mc-core/src/journal.rs`);
    `version` for the refusals. A fixture that copies the reader's own struct string would only
    prove the reader agrees with itself (review wave2-8 m4)."""
    payload = b''.join(json.dumps(entry).encode() + b'\n' for entry in entries)
    data = HEADER.pack(BATCH_MAGIC, version, STORE, PALETTE, lo, hi,
                       len(payload), zlib.crc32(payload)) + payload
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

    def test_the_frame_offsets_are_the_writer_s(self):
        # journal.rs: HEADER_LEN 56; lo@32, hi@40, len@48, crc@52 (review wave2-8 m4).
        self.assertEqual(HEADER_LEN, 56)
        packed = HEADER.pack(BATCH_MAGIC, BATCH_VERSION, STORE, PALETTE, 11, 12, 99, 77)
        self.assertEqual(struct.unpack_from('<Q', packed, 32)[0], 11)
        self.assertEqual(struct.unpack_from('<Q', packed, 40)[0], 12)
        self.assertEqual(struct.unpack_from('<I', packed, 48)[0], 99)
        self.assertEqual(struct.unpack_from('<I', packed, 52)[0], 77)

    def test_a_format_one_frame_is_refused_not_replayed(self):
        # The server refuses format 1 by name (no store identity); a gate that read it would call a
        # world this build cannot open healthy.
        with tempfile.TemporaryDirectory() as temp:
            journal = Path(temp)
            frame(journal, 5, 5, [{'Player': {'inventory': ['marker']}}], version=1)
            with self.assertRaisesRegex(ValueError, 'format 2'):
                read_published(journal, 4)
        # The real v1 layout is 32 bytes: it must say "format 1", not "truncated".
        with tempfile.TemporaryDirectory() as temp:
            journal = Path(temp)
            (journal / 'batch-00000000000000000006-00000000000000000006.bin').write_bytes(
                struct.pack('<IIQQII', BATCH_MAGIC, 1, 6, 6, 0, 0))
            with self.assertRaisesRegex(ValueError, 'format-1 frame'):
                read_published(journal, 4)


if __name__ == '__main__':
    unittest.main()
