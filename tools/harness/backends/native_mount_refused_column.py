"""A mounted prediction beside the column the server quarantined must not abort.

The refused fixture column is the one `native-v2-corrupt-chunk-near-camera` uses: the chunk
column beside the player's own, its last block corrupted in an exported Anvil copy and the world
imported again, so the server refuses that chunk and never sends it. On foot the client resolves
an unheld cell as one solid cube (`collision::gather`) and `walk_to_refused_column` proves the
body stops at the column's face instead of freezing.

A mounted client used to abort its whole prediction at the first unheld scan cell
(`unloaded_mount_collision`/`unloaded_mount_fluid`/`unloaded_boat_water`/`unloaded_boat_ground`),
so the boat froze at the border with `movement_blocked` naming the scan and no predicted step.
The row's carrier is that `movement_blocked` stays null while the step is predicted: `corrections
== 0` alone is not evidence, because the frozen control also reports 0.
"""
from __future__ import annotations

import math
from typing import Any

from .native_corrupt_block_entities import FIXTURES, _prepare, _server_log
from .native_p1 import _rows
from .native_p4 import _restart, walk_to_refused_column
from .native_v2 import _echo
from .probe_steps import ProbeSteps

SCENARIO = "native-v2-mount-beside-refused-column"


def _refused_column(origin: tuple[int, int, int]) -> tuple[int, int]:
    """(the player's own chunk column, the column beside it the scene refuses)."""
    camera = origin[0] + 0.5
    floor_ = math.floor(camera * 0.25 - 0.5)
    own = origin[0] // 16
    demanded = [column for column in range((floor_ - 2) // 4, (floor_ + 3) // 4 + 1) if column != own]
    if not demanded:
        raise RuntimeError("no chunk column beside the player to refuse")
    return own, demanded[0]


def _entity_rows(steps: ProbeSteps) -> list[dict[str, Any]]:
    """The client's own entity rows (they carry `passengers`/`vehicle`; the tool rows do not)."""
    return ((steps.observe().get("entities") or {}).get("entities")) or []


def _boat(steps: ProbeSteps, x: float, z: float, radius: float = 3.0) -> dict[str, Any] | None:
    return next((row for row in _rows(steps, 24)
                 if row.get("entity_type") == "minecraft:oak_boat"
                 and abs(row["x"] - x) < radius and abs(row["z"] - z) < radius), None)


def _identity(row: dict[str, Any]) -> dict[str, Any]:
    return {key: row[key] for key in ("entity_id", "entity_uuid", "entity_type")}


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    own, chunk_x = _refused_column(origin)
    points = [{"x": chunk_x * 16 + 1, "y": origin[1], "z": origin[2]}]
    steps.report["mount_refused_column"] = {
        "player_chunk": own, "refused_chunk": chunk_x,
        "block_offset_from_player": points[0]["x"] - origin[0],
    }
    _prepare(steps, origin, FIXTURES[:1], points)
    _restart(steps)
    steps.check("server_never_panicked", "panicked" not in _server_log(steps))
    state = steps.observe()
    steps.check("mount_beside_refused_column_player_grounded",
                state["player"].get("on_ground") is True)
    # The on-foot stop, shared with the loading-screen scene: it leaves the body at the column's
    # face, one half width from the refused cell the boat's scans sample.
    walk_to_refused_column(steps, origin, chunk_x)
    state = steps.observe()
    x, y, z = state["player"]["x"], state["player"]["y"], state["player"]["z"]
    # Away from the refused column, so approaching the boat never walks the body into it.
    # Away from the refused column: east when it lies west of the player, west when it lies east.
    side = 1.0 if chunk_x < own else -1.0
    steps.fixture(f"summon minecraft:oak_boat {x + side * 2.0} {y} {z}")
    steps.call("minecraft_wait_ticks", {"ticks": 4})
    receipt = _echo(steps, "mount_refused_column_boat_summon")
    listing = steps.call("minecraft_list_entities", {"radius": 32, "limit": 64})
    steps.report["mount_refused_column_summon"] = {
        "at": [x + side * 2.0, y, z],
        "chat": (receipt or {}).get("recent_chat", [])[-4:],
        "types": sorted({row.get("entity_type") for row in listing.get("entities") or []}),
        "count": len(listing.get("entities") or []),
    }
    steps.wait(lambda _: _boat(steps, x + side * 2.0, z) is not None,
               "mount_refused_column_boat_received", timeout_seconds=min(60.0, steps.remaining()))
    row = _boat(steps, x + side * 2.0, z)
    steps.report["mount_refused_column_boat"] = {
        key: row.get(key) for key in ("entity_id", "entity_uuid", "x", "y", "z")}
    # The approach tool declares only entity_id (its own schema has no entity_type).
    steps.call("minecraft_approach_entity", {"entity_id": row["entity_id"]})
    steps.call("minecraft_interact_entity", {**_identity(row), "hand": "main_hand"})
    steps.call("minecraft_wait_ticks", {"ticks": 3})
    rows = _entity_rows(steps)
    boat_rows = [entry for entry in rows if entry.get("type") == "minecraft:oak_boat"]
    boarded = [entry for entry in boat_rows if entry.get("passengers")]
    yaw = 0 if chunk_x < own else 180
    # A boat moves only when its rider drives it, so a short press that moves the boat is the
    # riding proof (the observation's rows carry no vehicle field for the local player).
    probe_before = _boat(steps, row["x"], row["z"], radius=6.0)
    steps.call("minecraft_look", {"yaw_deg": yaw, "pitch_deg": 0})
    steps.call("minecraft_press_inputs", {"keys": ["right"], "ticks": 8})
    steps.call("minecraft_wait_ticks", {"ticks": 3})
    probe_after = _boat(steps, row["x"], row["z"], radius=6.0)
    boat_moved = (probe_before is not None and probe_after is not None
                  and abs(probe_after["x"] - probe_before["x"]) > 0.05)
    steps.report["mount_refused_column_riding"] = {
        "boats": [{key: entry.get(key) for key in ("network_id", "passengers", "vehicle")}
                  for entry in boat_rows],
        "boat_before": (probe_before or {}).get("x"),
        "boat_after": (probe_after or {}).get("x"),
    }
    steps.check("mount_beside_refused_column_riding", bool(boarded) or boat_moved)
    before = steps.observe()
    # The client's own key mapping: at yaw 0 "right" walks toward -x (see walk_to_refused_column).
    steps.call("minecraft_press_inputs", {"keys": ["right"], "ticks": 40})
    # Ordered receipt: the read below must be causally after the drive, not a race with it.
    steps.call("minecraft_wait_ticks", {"ticks": 2})
    after = steps.observe()
    face = chunk_x * 16 + 16 if chunk_x < own else chunk_x * 16
    steps.report["mount_refused_column_drive"] = {
        "refused_chunk": chunk_x, "face": face,
        "before": [before["player"].get("x"), before["player"].get("z")],
        "after": [after["player"].get("x"), after["player"].get("z")],
        "movement_blocked": after["player"].get("movement_blocked"),
        "corrections": after["player"].get("corrections"),
        "on_ground": after["player"].get("on_ground"),
        "velocity": after["player"].get("velocity"),
    }
    steps.check("mount_beside_refused_column_not_blocked",
                after["player"].get("movement_blocked") is None)
    steps.check("mount_beside_refused_column_step_made_no_correction",
                after["player"].get("corrections") == 0)
    moved = after["player"].get("x")
    steps.check("mount_beside_refused_column_step_predicted",
                isinstance(moved, (int, float))
                and (moved - before["player"]["x"]) * (-side) > 0.2)
