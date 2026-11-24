"""Read retained crash WAL evidence without treating temporary files as published.

This does not establish that a particular command was in a frame: callers must
match the actual post-image, not just compare a command's tick to the watermark.
"""
from __future__ import annotations

import hashlib
from pathlib import Path

from . import wal_frame


def read_published(journal: Path, checkpoint_tick: int) -> dict:
    """Validate retained frames and the contiguous replay tail after checkpoint.

    A temporary file is reported but never decoded as committed. The checkpoint
    tick must be obtained from the actual committed checkpoint, not a guessed
    timer. Payloads are returned so an observer can identify real post-images.
    Frames are the server's current format (`wal_frame`); a damaged frame or one
    of another format raises `wal_frame.FrameError` (a `ValueError`).
    """
    if checkpoint_tick < 0:
        raise ValueError('negative checkpoint tick')
    batches, unpublished = [], []
    for path in journal.iterdir():
        span = wal_frame.span(path.name)
        if span:
            batches.append((*span, path))
        else:
            unpublished.append(path.name)
    frames = []
    next_tick = checkpoint_tick + 1
    for lo, hi, path in sorted(batches):
        data = path.read_bytes()
        try:
            frame = wal_frame.decode(data, (lo, hi))
        except wal_frame.FrameError as error:
            raise type(error)(f'{path}: {error}') from None
        if hi > checkpoint_tick:
            if lo != next_tick:
                raise ValueError(f'{path}: replay gap or overlap at tick {next_tick}')
            next_tick = hi + 1
        frames.append({'file': path.name, 'lo': lo, 'hi': hi,
                       'sha256': hashlib.sha256(data).hexdigest(), 'entries': frame.entries()})
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
