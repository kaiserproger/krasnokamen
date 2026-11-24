"""Read retained crash WAL evidence without treating temporary files as published.

This does not establish that a particular command was in a frame: callers must
match the actual post-image, not just compare a command's tick to the watermark.

One frame format, format 2, `crates/mc-core/src/journal.rs`:
`[magic u32][version u32][store uuid 16][palette digest 8][tick_lo u64][tick_hi u64][len u32][crc32 u32][payload]`
(little endian). The store/palette binding is the server's to verify on replay — a gate reads a
world and a copy of that same world — so this reader finds the payload and refuses everything else.
Format 1 has no store identity and this build refuses it by name; a gate that accepted it would
report a world the server cannot open as healthy.
"""
from __future__ import annotations

import hashlib
import json
import re
import struct
import zlib
from pathlib import Path

_FRAME = struct.Struct('<II16s8sQQII')
_MAGIC = 0x534F4C4A
_VERSION = 2
_NAME = re.compile(r'batch-(\d+)-(\d+)\.bin\Z')
# Offset of the payload in a published frame; callers that already validated the frame use it.
HEADER_LEN = _FRAME.size


def frame(name: str, data: bytes) -> tuple[int, int, bytes] | None:
    """`(tick_lo, tick_hi, payload)` of one published batch, or None for a torn/foreign file.

    The name must be `batch-<lo>-<hi>.bin` and must agree with the header, the payload length must
    match and its CRC must hold: a `.tmp` still being written, a truncated tail and a format-1
    frame all read as None, never as committed bytes.
    """
    found = _NAME.fullmatch(name)
    if found is None or len(data) < HEADER_LEN:
        return None
    magic, version, _store, _palette, lo, hi, length, crc = _FRAME.unpack_from(data)
    if (magic, version, lo, hi) != (_MAGIC, _VERSION, int(found[1]), int(found[2])):
        return None
    payload = data[HEADER_LEN:]
    if len(payload) != length or zlib.crc32(payload) != crc:
        return None
    return lo, hi, payload


def read_published(journal: Path, checkpoint_tick: int) -> dict:
    """Validate retained frames and the contiguous replay tail after checkpoint.

    A temporary file is reported but never decoded as committed. The checkpoint
    tick must be obtained from the actual committed checkpoint, not a guessed
    timer. Payloads are returned so an observer can identify real post-images.
    """
    if checkpoint_tick < 0:
        raise ValueError('negative checkpoint tick')
    batches, unpublished = [], []
    for path in journal.iterdir():
        match = _NAME.fullmatch(path.name)
        if match:
            batches.append((int(match[1]), int(match[2]), path))
        else:
            unpublished.append(path.name)
    frames = []
    next_tick = checkpoint_tick + 1
    for lo, hi, path in sorted(batches):
        if lo > hi:
            raise ValueError(f'{path}: invalid frame identity')
        data = path.read_bytes()
        if len(data) < HEADER_LEN:
            raise ValueError(f'{path}: truncated frame')
        decoded = frame(path.name, data)
        if decoded is None:
            raise ValueError(f'{path}: invalid frame identity, version or checksum '
                             f'(format {_VERSION} frames only)')
        if hi > checkpoint_tick:
            if lo != next_tick:
                raise ValueError(f'{path}: replay gap or overlap at tick {next_tick}')
            next_tick = hi + 1
        entries = [json.loads(line) for line in decoded[2].splitlines() if line]
        frames.append({'file': path.name, 'lo': lo, 'hi': hi,
                       'sha256': hashlib.sha256(data).hexdigest(), 'entries': entries})
    return {'checkpoint_tick': checkpoint_tick, 'published_through': next_tick - 1,
            'frames': frames, 'unpublished': sorted(unpublished)}


def classify_observation(*, acknowledged: bool, published_image: bool,
                         checkpoint_image: bool, observed_after: bool) -> str:
    """Classify a specific witnessed post-image, not a timestamp inference.

    A matching published WAL image or a matching committed checkpoint image is
    required independently of an acknowledgement. An observed image is not by
    itself proof that the mutation was acknowledged or durable before the cut.
    """
    if not acknowledged:
        return 'unacknowledged'
    if not (published_image or checkpoint_image):
        return 'unpublished'
    return 'survived' if observed_after else 'mismatch'
