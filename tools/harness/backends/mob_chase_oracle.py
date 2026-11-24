"""One long-window chase reading from named client entity samples.

A server sends an entity position every few ticks, so a short two-segment window measures that
sampling period, not the mob's pace (REVIEW-wave2-12 M1: every sampled displacement of the zombie
chase was a multiple of three ticks). This oracle reads one rate over the whole window between the
first and the last sample, and it also records whether a chase really happened: the mob's client-side
aggression flag and the distance it closed on the player.
"""
from __future__ import annotations

import math
from typing import Any


def _only(sample: dict[str, Any], kind: str) -> dict[str, Any]:
    rows = [row for row in sample["snapshot"]["entities"] if row.get("entity_type") == kind]
    if len(rows) != 1:
        raise ValueError(f"expected exactly one {kind}, got {len(rows)}")
    return rows[0]


def _flags(row: dict[str, Any]) -> int:
    """The mob-flags byte (`EntityDataValue::Byte` at index 15) of a received entity, or 0."""
    metadata = row.get("received_metadata") or {}
    for index in ("15", 15):
        entry = metadata.get(index) if isinstance(metadata, dict) else None
        if isinstance(entry, dict) and entry.get("kind") == "byte":
            return int(entry.get("value") or 0)
    return 0


def measure(samples: list[dict[str, Any]], kind: str) -> dict[str, Any]:
    """Horizontal pace over `samples[0] -> samples[-1]`, its approach and mob flags."""
    if len(samples) < 2:
        raise ValueError("a chase window needs at least two samples")
    rows = [_only(sample, kind) for sample in samples]
    elapsed = int(samples[-1]["game_time"]) - int(samples[0]["game_time"])
    if elapsed <= 0:
        raise ValueError(f"{kind} client game time did not advance")
    first, last = rows[0], rows[-1]
    displacement = math.hypot(float(last["x"]) - float(first["x"]), float(last["z"]) - float(first["z"]))
    if not math.isfinite(displacement):
        raise ValueError(f"{kind} has non-finite position")
    for row in rows:
        if not math.isfinite(float(row.get("distance", float("nan")))):
            raise ValueError(f"{kind} sample records no distance to the player")
    flags = [_flags(row) for row in rows]
    return {
        "kind": kind,
        "entity_uuid": first["entity_uuid"],
        "identity_constant": len({row["entity_uuid"] for row in rows}) == 1,
        "samples": len(rows),
        "elapsed_client_ticks": elapsed,
        "displacement_blocks": displacement,
        "blocks_per_client_tick": displacement / elapsed,
        "distance_first": float(first["distance"]),
        "distance_last": float(last["distance"]),
        "approach_blocks": float(first["distance"]) - float(last["distance"]),
        "flags_first": flags[0],
        "flags_last": flags[-1],
    }
