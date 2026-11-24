"""The forged WAL batch is format 3 and carries the world's own bindings.

A batch frame is `[magic][version][store 16][branch 8][palette 8][tick_lo][tick_hi]
[len][crc]` (version 3, little endian). Older frames are refused by name before
replay, so `publish_opaque_batch` takes the identity from a batch the server
wrote and refuses to invent one.
"""
import struct
import unittest
import zlib
from pathlib import Path
from tempfile import TemporaryDirectory

from .backends.wal_forge import (
    BATCH_MAGIC,
    BATCH_VERSION,
    HEADER,
    capture_identity,
    marker_store_uuid,
    publish_opaque_batch,
)

STORE = bytes(range(1, 17))
BRANCH = bytes(range(51, 59))
PALETTE = bytes(range(101, 109))


def write_batch(path: Path, tick: int, payload: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    header = HEADER.pack(BATCH_MAGIC, BATCH_VERSION, STORE, BRANCH, PALETTE, tick, tick,
                         len(payload), zlib.crc32(payload))
    path.write_bytes(header + payload)


class WalForgeTests(unittest.TestCase):
    def test_a_forged_batch_round_trips_the_captured_identity(self):
        with TemporaryDirectory() as directory:
            world = Path(directory) / "world"
            write_batch(world / "journal" / "batch-00000000000000000042-00000000000000000042.bin",
                        42, b'{"Tick":{}}\n')
            self.assertEqual(capture_identity(world), (STORE, BRANCH, PALETTE))
            forged = publish_opaque_batch(world, 42, {(1, 64, 2): b"\x0a\x00"},
                                          capture_identity(world))
            data = forged.read_bytes()
            magic, version, store, branch, palette, lo, hi, length, crc = HEADER.unpack_from(data)
            self.assertEqual((magic, version), (BATCH_MAGIC, BATCH_VERSION))
            self.assertEqual((store, branch, palette), (STORE, BRANCH, PALETTE))
            self.assertEqual((lo, hi), (43, 43))
            payload = data[HEADER.size:]
            self.assertEqual(length, len(payload))
            self.assertEqual(crc, zlib.crc32(payload))
            self.assertEqual(forged.name, "batch-00000000000000000043-00000000000000000043.bin")

    def test_a_forged_batch_refuses_to_invent_an_identity(self):
        with TemporaryDirectory() as directory:
            world = Path(directory) / "world"
            with self.assertRaisesRegex(RuntimeError, "store id and branch and this build's palette"):
                publish_opaque_batch(world, 7, {(0, 0, 0): b""}, None)

    def test_an_old_format_batch_is_not_an_identity_source(self):
        with TemporaryDirectory() as directory:
            world = Path(directory) / "world"
            journal = world / "journal"
            journal.mkdir(parents=True)
            # Format 1 and the store-bound format 2 (no branch) are both refused by
            # the server, so neither may lend its fields to a forged batch.
            (journal / "batch-00000000000000000001-00000000000000000001.bin").write_bytes(
                struct.pack("<IIQQII", BATCH_MAGIC, 1, 1, 1, 0, 0))
            (journal / "batch-00000000000000000002-00000000000000000002.bin").write_bytes(
                struct.pack("<II16s8sQQII", BATCH_MAGIC, 2, STORE, PALETTE, 2, 2, 0, 0))
            self.assertIsNone(capture_identity(world))

    def test_the_store_id_reads_from_both_marker_layouts(self):
        with TemporaryDirectory() as directory:
            world = Path(directory)
            (world / "checkpoint.marker").write_bytes(b"KCMARK\x01\x00" + STORE + bytes(40))
            self.assertEqual(marker_store_uuid(world), STORE)
            (world / "checkpoint.marker").write_bytes(b"KCMARK\x02\x00" + STORE + BRANCH + bytes(40))
            self.assertEqual(marker_store_uuid(world), STORE)


if __name__ == "__main__":
    unittest.main()
