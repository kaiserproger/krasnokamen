"""The forged WAL batch is the current frame format and carries the world's own bindings.

The frame layout lives in `wal_frame` (format 3: `[magic][version][store 16][branch 8]
[palette 8][tick_lo][tick_hi][len][crc]`). Older frames are refused by name before
replay, so `publish_opaque_batch` takes the identity from a batch the server wrote
and refuses to invent one.
"""
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from . import wal_frame
from .backends.wal_forge import marker_store_uuid, publish_opaque_batch

STORE = bytes(range(1, 17))
BRANCH = bytes(range(51, 59))
PALETTE = bytes(range(101, 109))


class WalForgeTests(unittest.TestCase):
    def test_a_forged_batch_round_trips_the_captured_identity(self):
        with TemporaryDirectory() as directory:
            world = Path(directory) / "world"
            journal = world / "journal"
            journal.mkdir(parents=True)
            (journal / wal_frame.batch_name(42, 42)).write_bytes(
                wal_frame.encode(42, 42, b'{"Tick":{}}\n', (STORE, BRANCH, PALETTE)))
            self.assertEqual(wal_frame.capture_identity(world), (STORE, BRANCH, PALETTE))
            forged = publish_opaque_batch(world, 42, {(1, 64, 2): b"\x0a\x00"},
                                          wal_frame.capture_identity(world))
            self.assertEqual(forged.name, "batch-00000000000000000043-00000000000000000043.bin")
            frame = wal_frame.read(forged)
            self.assertEqual(frame.identity, (STORE, BRANCH, PALETTE))
            self.assertEqual((frame.lo, frame.hi), (43, 43))
            self.assertEqual(frame.entries(), [{"OpaqueBlockEntity": {"pos": [1, 64, 2], "nbt": [10, 0]}}])

    def test_a_forged_batch_refuses_to_invent_an_identity(self):
        with TemporaryDirectory() as directory:
            world = Path(directory) / "world"
            with self.assertRaisesRegex(RuntimeError, "store id and branch and this build's palette"):
                publish_opaque_batch(world, 7, {(0, 0, 0): b""}, None)

    def test_the_store_id_reads_from_both_marker_layouts(self):
        with TemporaryDirectory() as directory:
            world = Path(directory)
            (world / "checkpoint.marker").write_bytes(b"KCMARK\x01\x00" + STORE + bytes(40))
            self.assertEqual(marker_store_uuid(world), STORE)
            (world / "checkpoint.marker").write_bytes(b"KCMARK\x02\x00" + STORE + BRANCH + bytes(40))
            self.assertEqual(marker_store_uuid(world), STORE)


if __name__ == "__main__":
    unittest.main()
