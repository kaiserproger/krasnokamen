"""Publish one forged tick-WAL batch into a stopped native world.

A clean stop leaves an empty `<world>/journal/` and a checkpoint at tick T. The
next batch must be `batch-<T+1>-<T+1>.bin` (recovery refuses a hole) and replays
over that checkpoint when the server starts. The frame is
`[magic u32][version u32][tick_lo u64][tick_hi u64][len u32][crc32 u32][payload]`
(little endian), the payload JSON lines of `JournalEntry` (crates/mc-core/src/journal.rs).

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
BATCH_VERSION = 1


def network_compound(fields: list) -> bytes:
    """Network-format NBT: an anonymous compound root."""
    return struct.pack(">B", COMPOUND) + _payload(COMPOUND, fields)


def committed_tick(log: str) -> int:
    """Tick of the last committed checkpoint in a server log."""
    ticks = re.findall(r"checkpoint committed tick=(\d+)", log)
    if not ticks:
        raise RuntimeError("no committed checkpoint in the server log; the WAL tick to follow is unknown")
    return int(ticks[-1])


def publish_opaque_batch(world: Path, after_tick: int, records: dict[tuple[int, int, int], bytes]) -> Path:
    """Write the one-tick batch that follows `after_tick`; returns its path."""
    tick = after_tick + 1
    payload = "".join(
        json.dumps({"OpaqueBlockEntity": {"pos": list(pos), "nbt": list(raw)}}, separators=(",", ":")) + "\n"
        for pos, raw in sorted(records.items())
    ).encode()
    header = struct.pack("<IIQQII", BATCH_MAGIC, BATCH_VERSION, tick, tick, len(payload), zlib.crc32(payload))
    directory = world / "journal"
    directory.mkdir(exist_ok=True)
    path = directory / f"batch-{tick:020}-{tick:020}.bin"
    path.write_bytes(header + payload)
    return path
