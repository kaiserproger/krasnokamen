"""Bounded displacement and facing evidence from client-visible entity samples."""
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


def shortest_arc(degrees: float) -> float:
    """The signed turn in (-180, 180] between two body-yaw bearings."""
    return (degrees + 180.0) % 360.0 - 180.0


def bearing(dx: float, dz: float) -> float:
    """Body-yaw degrees pointing along (dx, dz), the client's own convention (`yaw_delta`)."""
    return math.degrees(math.atan2(-dx, dz))


def chase_facing(samples: list[dict[str, Any]], entity_type: str,
                 player_position: list[float], *, min_step_speed: float = 0.01) -> dict[str, Any]:
    """Body yaw against the wanted direction (the chaser's drive to the player) and against
    the resolved step (the displacement the client actually received between samples).

    Vanilla `MoveControl` turns the body toward the wanted position; the row's build turned
    it toward the post-collision velocity, so on a deflected approach the body yaw sits on
    the step bearing and away from the wanted one. The step is measured from received
    positions: the wire carries an entity's resolved movement, not its velocity field.
    """
    if len(samples) < 3:
        raise ValueError("chase facing requires at least three samples")
    if len(player_position) != 3 or not all(math.isfinite(float(value)) for value in player_position):
        raise ValueError("chase facing needs a finite player position")
    entities: list[dict[str, Any]] = []
    rows: list[dict[str, Any]] = []
    yaw_rates: list[float] = []
    previous: tuple[int, float] | None = None
    for sample in samples:
        matches = [row for row in sample["snapshot"]["entities"] if row.get("entity_type") == entity_type]
        if len(matches) != 1:
            raise ValueError(f"expected exactly one {entity_type}, got {len(matches)}")
        entity = matches[0]
        tick = sample["game_time"]
        if type(tick) is not int:
            raise ValueError("chase facing sample lacks a client game_time")
        yaw = float(entity["yaw"])
        wanted = bearing(float(player_position[0]) - float(entity["x"]),
                         float(player_position[2]) - float(entity["z"]))
        if previous is not None:
            last_tick, last_yaw = previous
            elapsed = tick - last_tick
            if elapsed <= 0:
                raise ValueError(f"{entity_type} client game time did not advance")
            yaw_rates.append(abs(shortest_arc(yaw - last_yaw)) / elapsed)
        previous = (tick, yaw)
        entities.append({"game_time": tick, "entity_uuid": entity["entity_uuid"],
                         "x": float(entity["x"]), "z": float(entity["z"]), "yaw": yaw,
                         "wanted_bearing_deg": wanted,
                         "gap_to_wanted_deg": abs(shortest_arc(yaw - wanted))})
        rows.append(dict(entities[-1]))
    steps = []
    for first, last, row in zip(entities, entities[1:], rows[:-1]):
        elapsed = last["game_time"] - first["game_time"]
        dx, dz = last["x"] - first["x"], last["z"] - first["z"]
        speed = math.hypot(dx, dz) / elapsed
        if speed <= min_step_speed:
            continue
        step = bearing(dx, dz)
        steps.append({"from_tick": first["game_time"], "to_tick": last["game_time"],
                      "speed": speed, "step_bearing_deg": step,
                      "yaw_at_first_deg": row["yaw"],
                      "gap_to_step_deg": abs(shortest_arc(row["yaw"] - step)),
                      "step_deflection_deg": abs(shortest_arc(row["wanted_bearing_deg"] - step))})
    return {"entity_type": entity_type, "samples": rows, "steps": steps,
            "moving_samples": len(steps),
            "max_gap_to_wanted_deg": max(row["gap_to_wanted_deg"] for row in rows),
            "max_gap_to_step_deg": max((step["gap_to_step_deg"] for step in steps), default=0.0),
            "max_step_deflection_deg": max((step["step_deflection_deg"] for step in steps), default=0.0),
            "max_yaw_change_deg_per_tick": max(yaw_rates, default=0.0)}
