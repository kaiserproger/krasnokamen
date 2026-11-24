"""Publish one forged tick-WAL batch into a stopped native world.

A clean stop leaves an empty `<world>/journal/` and a checkpoint at tick T. The
next batch must be `batch-<T+1>-<T+1>.bin` (recovery refuses a hole) and replays
over that checkpoint when the server starts. The frame is format 3,
`[magic u32][version u32][store 16][branch 8][palette 8][tick_lo u64][tick_hi u64]
[len u32][crc32 u32][payload]` (little endian), the payload JSON lines of
`JournalEntry` (crates/mc-core/src/journal.rs).

The batch carries `OpaqueBlockEntity` post-images whose record id is not one the
replay validator checks, so a record the typed hydration owner (crafter, shelf)
cannot decode reaches a resident chunk: the route the hydration policy must refuse.
"""
from __future__ import annotations

import json
import re
import struct
import zlib
from pathlib import Path

from .anvil_corrupt import COMPOUND, _payload

BATCH_MAGIC = 0x534F_4C4A
BATCH_VERSION = 3
# `[magic][version][store 16][branch 8][palette 8][tick_lo][tick_hi][len][crc32]`,
# all little endian.
HEADER = struct.Struct("<II16s8s8sQQII")
HEADER_LEN = HEADER.size
MARKER_FILE = "checkpoint.marker"
# A created world's marker (root branch) and a restored copy's (with its branch);
# both carry the store id at offset 8..24.
MARKER_MAGICS = (b"KCMARK\x01\x00", b"KCMARK\x02\x00")


def network_compound(fields: list) -> bytes:
    """Network-format NBT: an anonymous compound root."""
    return struct.pack(">B", COMPOUND) + _payload(COMPOUND, fields)


def committed_tick(log: str) -> int:
    """Tick of the last committed checkpoint in a server log."""
    ticks = re.findall(r"checkpoint committed tick=(\d+)", log)
    if not ticks:
        raise RuntimeError("no committed checkpoint in the server log; the WAL tick to follow is unknown")
    return int(ticks[-1])


def _header_identity(data: bytes) -> tuple[bytes, bytes, bytes] | None:
    """`(store, branch, palette)` of a format-3 batch header, or None for anything else."""
    if len(data) < HEADER_LEN:
        return None
    magic, version, store, branch, palette, _, _, _, _ = HEADER.unpack_from(data)
    if magic != BATCH_MAGIC or version != BATCH_VERSION:
        return None
    return store, branch, palette


def capture_identity(world: Path) -> tuple[bytes, bytes, bytes] | None:
    """The identity this build writes: read from a format-3 batch it left behind.

    The palette digest is a property of the build (the embedded block report),
    so it is never derived from data files, and the branch belongs to this copy
    of the world; only a batch the server itself wrote carries both. Take the
    newest such batch, or None when the journal is empty.
    """
    directory = world / "journal"
    if not directory.is_dir():
        return None
    batches = sorted(directory.glob("batch-*.bin"), reverse=True)
    for batch in batches:
        try:
            with batch.open("rb") as handle:
                data = handle.read(HEADER_LEN)
        except OSError:
            continue
        identity = _header_identity(data)
        if identity is not None:
            return identity
    return None


def marker_store_uuid(world: Path) -> bytes | None:
    """The world's immutable store id from `checkpoint.marker` (offset 8..24)."""
    try:
        data = (world / MARKER_FILE).read_bytes()
    except OSError:
        return None
    if len(data) < 8 + 16 or data[:8] not in MARKER_MAGICS:
        return None
    return data[8:24]


def publish_opaque_batch(
    world: Path,
    after_tick: int,
    records: dict[tuple[int, int, int], bytes],
    identity: tuple[bytes, bytes, bytes] | None = None,
) -> Path:
    """Write the one-tick format-3 batch that follows `after_tick`; returns its path.

    The batch must belong to this copy of the world under this build's palette,
    so `identity` comes from a batch the server wrote (`capture_identity`) or,
    when a test supplies it explicitly, from that test. Nothing is invented here:
    a missing identity is an error, and the frame then fails the reader's binding
    check by name rather than replaying into the wrong world.
    """
    store, branch, palette = identity if identity is not None else (b"", b"", b"")
    if (len(store) != 16 or len(branch) != 8 or len(palette) != 8
            or (store == bytes(16) and palette == bytes(8))):
        raise RuntimeError(
            "forged WAL batch needs the world's store id and branch and this build's palette: "
            "capture them from a format-3 batch the server wrote (wal_forge.capture_identity)"
        )
    tick = after_tick + 1
    payload = "".join(
        json.dumps({"OpaqueBlockEntity": {"pos": list(pos), "nbt": list(raw)}}, separators=(",", ":")) + "\n"
        for pos, raw in sorted(records.items())
    ).encode()
    header = HEADER.pack(BATCH_MAGIC, BATCH_VERSION, store, branch, palette, tick, tick, len(payload),
                         zlib.crc32(payload))
    directory = world / "journal"
    directory.mkdir(exist_ok=True)
    path = directory / f"batch-{tick:020}-{tick:020}.bin"
    path.write_bytes(header + payload)
    return path
