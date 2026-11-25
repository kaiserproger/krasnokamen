"""Ledger MAP-FRAME-AUTO-EXPLORE: a map carried in a pocket must not explore terrain.

Vanilla samples terrain only for a map (or a clone) in a player's hand. The pre-image
collected every carried map as a viewer and treated "any carried copy is not framed" as
the exploration gate, so a map sitting in a *pocket* — not selected, not off-hand — still
painted pixels as its carrier walked. The fix (`33812b466`) builds the explorer set from
`selected`/`OFF_HAND` only.

The scenario gives the client an empty map, fills it with its own use (the server allocates
a record with zero explored pixels), moves it to a pocket slot with the client's own
inventory drag, walks a short leg and asserts through the `maps` observation that the
record's explored pixels never grow. A framed copy is *not* a discriminator (a wall map
alone never explored before the fix either), so none is placed.

Control: the row's scoped patch (`docs/bugs/promotions/MAP-FRAME-AUTO-EXPLORE.patch`) puts
the pre-image gate back; with it applied the pocket map paints and the assertion reds.
"""
from __future__ import annotations

from typing import Any

from .native_p1 import _aim
from .native_v2 import _give, _use
from .probe_steps import ProbeSteps, item_count

MAP = "minecraft:map"
# A slot outside the hotbar and the off hand: inventory slot 20 (row 2, column 3).
POCKET = 20
WALK_STEPS = 6
TICKS_PER_STEP = 10


def _maps(steps: ProbeSteps) -> dict[str, Any]:
    return {row["id"]: row for row in steps.observe().get("maps") or []}


def _explored(steps: ProbeSteps, map_id: int) -> int | None:
    row = _maps(steps).get(str(map_id))
    return None if row is None else row.get("visible_pixels")


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    steps.fixture("gamerule send_command_feedback true")
    _give(steps, MAP, 0)
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    # A held use allocates the map record on the server (maps.rs `allocate`).
    _use(steps, {"x": x + 2, "y": y, "z": z + 2}, face="up")
    steps.call("minecraft_wait_ticks", {"ticks": 10})
    before = _maps(steps)
    steps.report["maps_after_filling"] = before
    allocated = [int(key) for key, row in before.items() if row.get("scale") == 0]
    steps.check("a_scale_zero_map_record_exists", bool(allocated))
    if not allocated:
        return
    map_id = allocated[0]
    steps.check("fresh_map_starts_unexplored", (_explored(steps, map_id) or 0) == 0)

    # The client's own drag puts the map in a pocket: not selected, not off-hand.
    state = steps.call("minecraft_open_inventory")
    slot = next((stack["slot"] for stack in state["container"]["slots"]
                 if stack["item_id"] == MAP and stack["count"] == 1), None)
    steps.check("map_is_carried_in_the_inventory", slot is not None)
    if slot is None:
        return
    steps.call("minecraft_click_container_slot", {"slot": slot, "button": "primary"})
    steps.call("minecraft_click_container_slot", {"slot": POCKET, "button": "primary"})
    steps.call("minecraft_close_screen")
    carried = steps.observe()
    steps.report["pocket_slot"] = POCKET
    steps.check("map_moved_out_of_the_hand",
                item_count(carried, MAP) == 1 and carried["player"]["selected_hotbar_slot"] != 0)

    # Walk a short leg away from the pad; only a held map may paint.
    _aim(steps, {"x": x, "z": z - 8, "bounding_box": {"min_y": y, "max_y": y + 2}})
    start = _explored(steps, map_id) or 0
    samples: list[int | None] = []
    for _ in range(WALK_STEPS):
        steps.call("minecraft_press_inputs", {"keys": ["forward"], "ticks": TICKS_PER_STEP})
        samples.append(_explored(steps, map_id))
    steps.report["explored_samples"] = samples
    steps.check("pocket_map_never_explores", all(value in (None, start) for value in samples))
    steps.check("pocket_map_still_unexplored", (_explored(steps, map_id) or 0) == 0)

    steps.report["limits"] = [
        "One allocated scale-zero map carried in inventory slot 20 while the player walks six short "
        "legs; explored pixels come from the client's own `maps` observation. No wall map, no clone, "
        "no cartography table and no deposit/withdraw.",
    ]
