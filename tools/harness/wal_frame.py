"""The tick-WAL batch frame of crates/mc-core/src/journal.rs: the one reader and writer of harness tools.

A published batch `journal/batch-<lo>-<hi>.bin` is format 3: `[magic u32][version u32][store 16]
[branch 8][palette 8][tick_lo u64][tick_hi u64][length u32][crc32 u32]` (little endian, 64 bytes)
and the payload, JSON lines of `JournalEntry`. The store id, the branch of that copy of the world
and the block-palette digest bind the batch to the world that wrote it; the server refuses any
other. Frames of format 1 (32-byte header) and 2 (no branch) are refused by name here too, never
parsed with a stale layout.
"""
from __future__ import annotations

import json
import re
import struct
import zlib
from dataclasses import dataclass
from pathlib import Path

MAGIC = 0x534F_4C4A
VERSION = 3
HEADER = struct.Struct("<II16s8s8sQQII")
HEADER_LEN = HEADER.size
NAME = re.compile(r"batch-(\d+)-(\d+)\.bin\Z")

# `(store id 16, branch 8, palette digest 8)` of one copy of one world under one build.
Identity = tuple[bytes, bytes, bytes]


class FrameError(ValueError):
    """A frame the server would refuse: not a batch, another format, or ticks other than its name's."""


class TornFrame(FrameError):
    """A frame cut short, or whose payload fails its length or CRC: a batch caught while being copied."""


@dataclass(frozen=True)
class Frame:
    store: bytes
    branch: bytes
    palette: bytes
    lo: int
    hi: int
    payload: bytes

    @property
    def identity(self) -> Identity:
        return self.store, self.branch, self.palette

    def entries(self) -> list:
        """The payload's `JournalEntry` lines, decoded."""
        return [json.loads(line) for line in self.payload.splitlines() if line]


def span(name: str) -> tuple[int, int] | None:
    """`(lo, hi)` of a published batch file name, None for any other name (`.tmp`, `.event`)."""
    match = NAME.fullmatch(name)
    return (int(match[1]), int(match[2])) if match else None


def batch_name(lo: int, hi: int) -> str:
    return f"batch-{lo:020}-{hi:020}.bin"


def _check_header(data: bytes) -> None:
    if len(data) < 8:
        raise TornFrame("truncated WAL frame header")
    magic, version = struct.unpack_from("<II", data)
    if magic != MAGIC:
        raise FrameError(f"not a WAL batch frame (magic {magic:#010x})")
    if version != VERSION:
        raise FrameError(f"WAL frame format {version}; the server writes and replays only format {VERSION}")
    if len(data) < HEADER_LEN:
        raise TornFrame("truncated WAL frame header")


def decode(data: bytes, expected: tuple[int, int] | None = None) -> Frame:
    """Decode one complete batch; `expected` is the `(lo, hi)` its file name promises."""
    _check_header(data)
    _, _, store, branch, palette, lo, hi, length, crc = HEADER.unpack_from(data)
    if lo > hi or (expected is not None and (lo, hi) != expected):
        raise FrameError(f"WAL frame ticks {lo}..={hi} differ from its name {expected}")
    payload = data[HEADER_LEN:]
    if len(payload) != length or zlib.crc32(payload) != crc:
        raise TornFrame("WAL frame payload length or CRC32 checksum mismatch")
    return Frame(store, branch, palette, lo, hi, payload)


def read(path: Path) -> Frame:
    """Decode the published batch at `path`, checking its ticks against its name."""
    expected = span(path.name)
    if expected is None:
        raise FrameError(f"{path.name} is not a published batch name")
    try:
        return decode(path.read_bytes(), expected)
    except FrameError as error:
        raise type(error)(f"{path}: {error}") from None


def encode(lo: int, hi: int, payload: bytes, identity: Identity) -> bytes:
    """One complete format-3 frame bound to `identity`."""
    store, branch, palette = identity
    if len(store) != 16 or len(branch) != 8 or len(palette) != 8:
        raise ValueError("a WAL frame needs a 16-byte store id, an 8-byte branch and an 8-byte palette digest")
    return HEADER.pack(MAGIC, VERSION, store, branch, palette, lo, hi, len(payload),
                       zlib.crc32(payload)) + payload


def header_identity(data: bytes) -> Identity | None:
    """`(store, branch, palette)` of a format-3 frame header, None for anything else."""
    try:
        _check_header(data)
    except FrameError:
        return None
    _, _, store, branch, palette, *_ = HEADER.unpack_from(data)
    return store, branch, palette


def capture_identity(world: Path) -> Identity | None:
    """The identity this build writes for `world`: read from a format-3 batch the server left behind.

    The palette digest is a property of the build (the embedded block report), so it is never
    derived from data files, and the branch belongs to this copy of the world; only a batch the
    server itself wrote carries both. Take the newest such batch, or None when the journal is empty.
    """
    directory = world / "journal"
    if not directory.is_dir():
        return None
    for batch in sorted(directory.glob("batch-*.bin"), reverse=True):
        try:
            with batch.open("rb") as handle:
                data = handle.read(HEADER_LEN)
        except OSError:
            continue
        identity = header_identity(data)
        if identity is not None:
            return identity
    return None
