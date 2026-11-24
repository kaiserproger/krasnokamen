"""Publish one forged tick-WAL batch into a stopped native world.

A clean stop leaves an empty `<world>/journal/` and a checkpoint at tick T. The
next batch must be `batch-<T+1>-<T+1>.bin` (recovery refuses a hole) and replays
over that checkpoint when the server starts. The frame is the current format
of `tools/harness/wal_frame.py`, bound to the world's store id and branch and to
this build's palette; the payload is JSON lines of `JournalEntry`
(crates/mc-core/src/journal.rs).

The batch carries `OpaqueBlockEntity` post-images whose record id is not one the
replay validator checks, so a record the typed hydration owner (crafter, shelf)
cannot decode reaches a resident chunk: the route the hydration policy must refuse.
"""
from __future__ import annotations

import json
import re
import struct
from pathlib import Path

from .. import wal_frame
from .anvil_corrupt import COMPOUND, _payload

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
    identity: wal_frame.Identity | None = None,
) -> Path:
    """Write the one-tick batch that follows `after_tick`; returns its path.

    The batch must belong to this copy of the world under this build's palette,
    so `identity` comes from a batch the server wrote (`wal_frame.capture_identity`)
    or, when a test supplies it explicitly, from that test. Nothing is invented
    here: a missing identity is an error, and the frame then fails the reader's
    binding check by name rather than replaying into the wrong world.
    """
    store, branch, palette = identity if identity is not None else (b"", b"", b"")
    if (len(store) != 16 or len(branch) != 8 or len(palette) != 8
            or (store == bytes(16) and palette == bytes(8))):
        raise RuntimeError(
            "forged WAL batch needs the world's store id and branch and this build's palette: "
            "capture them from a batch the server wrote (wal_frame.capture_identity)"
        )
    tick = after_tick + 1
    payload = "".join(
        json.dumps({"OpaqueBlockEntity": {"pos": list(pos), "nbt": list(raw)}}, separators=(",", ":")) + "\n"
        for pos, raw in sorted(records.items())
    ).encode()
    directory = world / "journal"
    directory.mkdir(exist_ok=True)
    path = directory / wal_frame.batch_name(tick, tick)
    path.write_bytes(wal_frame.encode(tick, tick, payload, (store, branch, palette)))
    return path
