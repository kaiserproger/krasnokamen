"""Ledger REDSTONE-DAYLIGHT: a detector placed under settled thunder must not report full power.

The row's fix (`da5421f7b`) moves the daylight detector's 20-tick periodic recheck from
priority 0 to 1 so an explicit wake (priority 0) is no longer swallowed by the queued
recheck. The pre-image therefore keeps `power=15` until the recheck comes round; the fixed
tree resamples on the wake itself.

The gate makes that deterministic instead of racing a live ramp: `/tick freeze` stops the
loop, `/weather thunder 1000` plus `/tick step` runs the ramp to its settled value, the
detector is placed with `/setblock`, and then a single `/tick step 1` is the only tick the
detector can use. `minecraft_read_block` returns the block-state `properties`, so `power`
is read straight from the client's own chunk model — no timeout and no command feedback.

Assertions: the detector exists and carries a `power` property; after the single tick it
must not read 15 (the pre-image's stale value); and a second step must not change it back
(the settled value is stable, so the read is not an artifact of sampling).
"""
from __future__ import annotations

from typing import Any

from .probe_steps import ProbeSteps

DETECTOR = "minecraft:daylight_detector"
# `/weather thunder <duration>` is in seconds; 1000 s is longer than the whole gate.
THUNDER = 1000
# 0.01 per tick over 100 ticks settles the mixer; 200 leaves margin.
SETTLE_TICKS = 200


def _read(steps: ProbeSteps, point: dict[str, int]) -> dict[str, Any]:
    return steps.call("minecraft_read_block", dict(point))


def _power(steps: ProbeSteps, point: dict[str, int]) -> int | None:
    value = _read(steps, point).get("properties", {}).get("power")
    return None if value is None else int(value)


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    point = {"x": x + 2, "y": y + 1, "z": z + 2}
    # A frozen world advances only through `/tick step`, so the ramp and the detector's
    # own ticks land on chosen boundaries.
    steps.fixture("tick freeze")
    steps.fixture(f"weather thunder {THUNDER}")
    steps.fixture(f"tick step {SETTLE_TICKS}")
    steps.fixture(f"setblock {point['x']} {point['y']} {point['z']} {DETECTOR}")
    steps.report["placed"] = _read(steps, point)
    # One tick: the pre-image's wake is swallowed by its queued recheck, the fixed tree
    # resamples here.
    steps.fixture("tick step 1")
    first = _power(steps, point)
    steps.report["power_after_one_tick"] = first
    steps.check("daylight_detector_placed", _read(steps, point).get("block_id") == DETECTOR)
    steps.check("daylight_detector_carries_power", first is not None)
    steps.check("daylight_detector_reports_the_settled_sky_under_thunder", first != 15)

    steps.fixture("tick step 1")
    second = _power(steps, point)
    steps.report["power_after_second_tick"] = second
    steps.check("daylight_detector_power_is_stable", second == first)
    steps.fixture("tick unfreeze")

    steps.report["limits"] = [
        "One frozen world, one settled thunder ramp and one detector placed after it; the power comes "
        "from the client's own chunk-model properties. No clear-sky comparison, no sensor chain and "
        "no live (unfrozen) weather ramp.",
    ]
