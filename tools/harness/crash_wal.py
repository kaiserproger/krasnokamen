"""Read retained crash WAL evidence without treating temporary files as published.

This does not establish that a particular command was in a frame: callers must
match the actual post-image, not just compare a command's tick to the watermark.
"""
from __future__ import annotations

import hashlib
import json
import re
import struct
import zlib
from pathlib import Path

_HEADER = struct.Struct('<IIQQII')
_NAME = re.compile(r'batch-(\d+)-(\d+)\.bin\Z')
_MAGIC = 0x534F4C4A


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
        data = path.read_bytes()
        if len(data) < _HEADER.size:
            raise ValueError(f'{path}: truncated frame')
        magic, version, header_lo, header_hi, length, crc = _HEADER.unpack_from(data)
        payload = data[_HEADER.size:]
        if (magic, version, header_lo, header_hi) != (_MAGIC, 1, lo, hi) or lo > hi:
            raise ValueError(f'{path}: invalid frame identity')
        if len(payload) != length or zlib.crc32(payload) != crc:
            raise ValueError(f'{path}: invalid frame length or checksum')
        if hi > checkpoint_tick:
            if lo != next_tick:
                raise ValueError(f'{path}: replay gap or overlap at tick {next_tick}')
            next_tick = hi + 1
        entries = [json.loads(line) for line in payload.splitlines() if line]
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
