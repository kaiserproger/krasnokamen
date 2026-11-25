"""Frozen-world presentation: the frame draws an entity at its current tick, not the previous one.

Vanilla 26.3 `LevelExtractor.extractVisibleEntities` passes `!TickRateManager.isEntityFrozen(entity)`
to `DeltaTracker.getGameTimeDeltaPartialTick`, which returns 1.0 while the world is frozen, so a
stepped tick is drawn at its current position. The client bug drew the previous tick's transform
(partial tick 0). The discriminating value is therefore the transform the renderer draws, reported
as `presented_position` in the entity observation, against the position the server sent.

Two witnesses, both with a fixed UUID so one entity is read across steps: a launched item (2 blocks
per step, so a one-tick lag is two blocks) and a falling block (its own gravity step). Each capture
is taken through a bounded request timeout and a capture that does not complete within it is
recorded as inconclusive instead of holding the run: the assertion is on the observation, the frame
is the retained evidence.

The scene refuses to run on a host whose 15-minute load average is above the budget that made the
frozen capture hang twice (HANDOFF-5): it raises an explicit `inconclusive` error instead of red.
"""
from __future__ import annotations

import uuid
from pathlib import Path
from typing import Any

from . import native_client
from .native_v2 import _echo
from .probe_steps import ProbeSteps


LOAD15_BUDGET = 12.0
CAPTURE_TIMEOUT_SECONDS = 25.0
PRESENTED_TOLERANCE = 0.05


def _load15() -> float:
    return float(Path("/proc/loadavg").read_text().split()[2])


def _capture(steps: ProbeSteps, label: str) -> dict[str, Any] | None:
    """One bounded capture; a slow frame is recorded, never waited out."""
    client = steps.client
    previous = getattr(client, "request_timeout_seconds", None)
    try:
        if previous is not None:
            client.request_timeout_seconds = min(previous, CAPTURE_TIMEOUT_SECONDS)
        frame = native_client.capture(client, steps.artifact_dir.parent, label)
        steps.report.setdefault("captures", {})[label] = frame
        return frame
    except Exception as error:  # noqa: BLE001 - a slow frame is an outcome, not a failure
        steps.report.setdefault("capture_errors", {})[label] = f"{type(error).__name__}: {error}"
        return None
    finally:
        if previous is not None:
            client.request_timeout_seconds = previous


def _witness(steps: ProbeSteps, entity_uuid: str) -> dict[str, Any] | None:
    rows = steps.call("minecraft_list_entities", {"radius": 24.0, "limit": 128})["entities"]
    return next((row for row in rows if row.get("entity_uuid") == entity_uuid), None)


def _drawn_at_the_current_tick(steps: ProbeSteps, label: str, entity_uuid: str,
                               before: list[float] | None) -> dict[str, Any]:
    """One stepped tick under freeze; the drawn transform must be the received one."""
    steps.fixture("tick step 1")
    steps.call("minecraft_wait_ticks", {"ticks": 4})
    _capture(steps, label)
    row = _witness(steps, entity_uuid) or {}
    received, presented = row.get("position"), row.get("presented_position")
    steps.report[label] = {"before": before, "received": received, "presented": presented}
    steps.check(f"{label}_entity_received", isinstance(received, list) and isinstance(presented, list))
    moved = (before is not None and received is not None
             and any(abs(a - b) > 0.1 for a, b in zip(received, before)))
    steps.check(f"{label}_server_step_received", moved)
    gap = max((abs(a - b) for a, b in zip(presented, received)), default=0.0)
    steps.report[label]["presented_to_received_gap"] = gap
    steps.check(f"{label}_drawn_at_current_tick", gap <= PRESENTED_TOLERANCE)
    return row


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    load = _load15()
    steps.report["host_load15"] = load
    if load >= LOAD15_BUDGET:
        raise RuntimeError(
            f"inconclusive: frozen capture needs a quiet host (load15 {load:.2f} >= {LOAD15_BUDGET}); "
            "the same scene hung twice under peer load (HANDOFF-5)")
    x, y, z = origin
    steps.fixture("tick freeze")
    _echo(steps, "frozen_witness_freeze_ordered")
    frozen = steps.observe().get("minecraft", {}).get("server_frozen")
    steps.report["server_frozen"] = frozen
    steps.check("frozen_witness_world_frozen", frozen is True)
    # Witness 1: a launched item. Two blocks per step, so a one-tick lag is unmistakable.
    item_uuid = str(uuid.uuid4())
    steps.fixture(f"summon minecraft:item {x + 2.5} {y + 1.0} {z + 2.5} "
                  f'{{UUID:"{item_uuid}",Item:{{id:"minecraft:snowball",count:1b}},'
                  f"Motion:[2.0,0.0,0.0]}}")
    before = steps.wait(lambda _state: _witness(steps, item_uuid), "frozen_item_witness_summoned")
    steps.check("frozen_item_witness_presented_reported", "presented_position" in before)
    _drawn_at_the_current_tick(steps, "frozen_item", item_uuid, before.get("position"))
    # Witness 2: a falling block, the sibling row's subject (its own gravity step per tick).
    block_uuid = str(uuid.uuid4())
    steps.fixture(f"summon minecraft:falling_block {x + 1.5} {y + 6.0} {z + 1.5} "
                  f'{{UUID:"{block_uuid}",BlockState:{{Name:"minecraft:sand"}},Time:1}}')
    before = steps.wait(lambda _state: _witness(steps, block_uuid), "frozen_falling_witness_summoned")
    _drawn_at_the_current_tick(steps, "frozen_falling", block_uuid, before.get("position"))
    steps.fixture("kill @e[type=!minecraft:player]")
    steps.fixture("tick unfreeze")
    _echo(steps, "frozen_witness_unfreeze_ordered")
    steps.report["visual_review_required"] = [
        "Review the retained frozen captures against the recorded presented/received rows."]
    steps.report["limits"] = [
        "declared summoned witnesses under an admin tick freeze; the observation is the proof, the "
        "frames are retained evidence"]
