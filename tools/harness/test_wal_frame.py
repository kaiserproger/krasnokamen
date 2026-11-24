"""The harness's one WAL frame reader and writer matches the server's format 3 and refuses older ones."""
import struct
import unittest
import zlib
from pathlib import Path
from tempfile import TemporaryDirectory

from . import wal_frame

STORE = bytes(range(1, 17))
BRANCH = bytes(range(51, 59))
PALETTE = bytes(range(101, 109))
IDENTITY = (STORE, BRANCH, PALETTE)


def format_1(lo: int, hi: int, payload: bytes) -> bytes:
    """The retired 32-byte-header frame `[magic][1][tick_lo][tick_hi][len][crc]`."""
    return struct.pack("<IIQQII", wal_frame.MAGIC, 1, lo, hi, len(payload), zlib.crc32(payload)) + payload


def format_2(lo: int, hi: int, payload: bytes) -> bytes:
    """The retired store-bound frame without a branch."""
    return struct.pack("<II16s8sQQII", wal_frame.MAGIC, 2, STORE, PALETTE, lo, hi, len(payload),
                       zlib.crc32(payload)) + payload


class WalFrameTests(unittest.TestCase):
    def test_the_layout_is_the_servers(self):
        # crates/mc-core/src/journal.rs: a 64-byte header, the branch at offset 24, the length at 56.
        data = wal_frame.encode(7, 9, b'{"Tick":{}}\n', IDENTITY)
        self.assertEqual(wal_frame.HEADER_LEN, 64)
        self.assertEqual(struct.unpack_from("<II", data), (0x534F4C4A, 3))
        self.assertEqual((data[8:24], data[24:32], data[32:40]), IDENTITY)
        self.assertEqual(struct.unpack_from("<QQII", data, 40), (7, 9, 12, zlib.crc32(b'{"Tick":{}}\n')))

    def test_a_frame_round_trips_its_identity_ticks_and_entries(self):
        frame = wal_frame.decode(wal_frame.encode(7, 9, b'{"Tick":{}}\n\n{"A":1}\n', IDENTITY), (7, 9))
        self.assertEqual((frame.identity, frame.lo, frame.hi), (IDENTITY, 7, 9))
        self.assertEqual(frame.entries(), [{"Tick": {}}, {"A": 1}])

    def test_older_formats_and_foreign_files_are_refused_by_name_not_as_torn(self):
        for data, reason in ((format_1(1, 1, b"{}\n"), "format 1"), (format_2(1, 1, b"{}\n"), "format 2"),
                             (b"\0" * 64, "not a WAL batch")):
            with self.subTest(reason=reason):
                with self.assertRaisesRegex(wal_frame.FrameError, reason) as caught:
                    wal_frame.decode(data, (1, 1))
                self.assertNotIsInstance(caught.exception, wal_frame.TornFrame)

    def test_a_cut_or_damaged_frame_is_torn(self):
        data = wal_frame.encode(1, 1, b'{"Tick":{}}\n', IDENTITY)
        for damaged in (data[:5], data[:40], data[:-1], data[:-1] + b"x"):
            with self.subTest(length=len(damaged)):
                with self.assertRaises(wal_frame.TornFrame):
                    wal_frame.decode(damaged, (1, 1))

    def test_ticks_must_match_the_name(self):
        data = wal_frame.encode(1, 2, b"", IDENTITY)
        with self.assertRaisesRegex(wal_frame.FrameError, "differ from its name"):
            wal_frame.decode(data, (1, 1))
        with TemporaryDirectory() as directory:
            path = Path(directory) / wal_frame.batch_name(1, 1)
            path.write_bytes(data)
            with self.assertRaisesRegex(wal_frame.FrameError, str(path)):
                wal_frame.read(path)

    def test_names_and_identity_sizes(self):
        self.assertEqual(wal_frame.batch_name(3, 4), "batch-00000000000000000003-00000000000000000004.bin")
        self.assertEqual(wal_frame.span("batch-25-25.bin"), (25, 25))
        self.assertIsNone(wal_frame.span("batch-1-1.bin.tmp"))
        with self.assertRaisesRegex(ValueError, "8-byte branch"):
            wal_frame.encode(1, 1, b"", (STORE, b"", PALETTE))

    def test_only_a_current_frame_lends_its_identity(self):
        with TemporaryDirectory() as directory:
            world = Path(directory)
            self.assertIsNone(wal_frame.capture_identity(world))
            journal = world / "journal"
            journal.mkdir()
            (journal / wal_frame.batch_name(1, 1)).write_bytes(wal_frame.encode(1, 1, b"", IDENTITY))
            # Newer names first, but neither retired format may lend its fields.
            (journal / wal_frame.batch_name(2, 2)).write_bytes(format_1(2, 2, b""))
            (journal / wal_frame.batch_name(3, 3)).write_bytes(format_2(3, 3, b""))
            self.assertEqual(wal_frame.capture_identity(world), IDENTITY)


if __name__ == "__main__":
    unittest.main()
