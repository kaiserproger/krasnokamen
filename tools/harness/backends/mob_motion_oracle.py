"""Bounded displacement evidence from three client-visible entity samples."""
from __future__ import annotations

import math
from typing import Any


def measure(samples: list[dict[str, Any]], entity_types: list[str]) -> dict[str, Any]:
    if len(samples) != 3:
        raise ValueError("mob motion requires exactly three samples")
    result: dict[str, Any] = {}
    for kind in entity_types:
        entities = []
        for sample in samples:
            rows = [row for row in sample["snapshot"]["entities"] if row.get("entity_type") == kind]
            if len(rows) != 1:
                raise ValueError(f"expected exactly one {kind}, got {len(rows)}")
            entities.append(rows[0])
        if len({row["entity_uuid"] for row in entities}) != 1:
            raise ValueError(f"{kind} identity changed across samples")
        segments = []
        for index in range(2):
            first, last = samples[index:index + 2]
            elapsed_ticks = last["game_time"] - first["game_time"]
            if elapsed_ticks <= 0:
                raise ValueError(f"{kind} client game time did not advance")
            a, b = entities[index:index + 2]
            displacement = math.hypot(float(b["x"]) - float(a["x"]), float(b["z"]) - float(a["z"]))
            if not math.isfinite(displacement):
                raise ValueError(f"{kind} has non-finite position")
            segments.append({"displacement_blocks": displacement,
                             "elapsed_client_ticks": elapsed_ticks,
                             "blocks_per_client_tick": displacement / elapsed_ticks})
        result[kind] = {"entity_uuid": entities[0]["entity_uuid"], "segments": segments,
                        "total_displacement_blocks": sum(segment["displacement_blocks"] for segment in segments)}
    return result
