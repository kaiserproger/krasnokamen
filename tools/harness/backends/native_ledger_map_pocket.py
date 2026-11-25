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
from .native_v2 import _give
from .probe_steps import ProbeSteps, item_count

MAP = "minecraft:map"
FILLED = "minecraft:filled_map"
# A slot outside the hotbar and the off hand: inventory slot 20 (row 2, column 3).
POCKET = 20
# Vanilla off-hand slot index in the native inventory.
OFF_HAND = 40
WALK_STEPS = 6
TICKS_PER_STEP = 10


def _maps(steps: ProbeSteps) -> dict[int, Any]:
    return {int(row["id"]): row for row in steps.observe().get("maps") or []}


def _explored(steps: ProbeSteps, map_id: int) -> int | None:
    row = _maps(steps).get(map_id)
    return None if row is None else row.get("visible_pixels")


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    steps.fixture("gamerule send_command_feedback true")
    _give(steps, MAP, 0)
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    # A plain held use turns the empty map into a filled one and makes the server
    # allocate the record (maps.rs `allocate`); the map id is the item component.
    steps.call("minecraft_press_inputs", {"keys": ["use"], "ticks": 1})
    steps.wait(lambda state: item_count(state, FILLED) == 1,
               "empty_map_used_into_a_filled_map", timeout_seconds=30)
    stack = steps.call("minecraft_read_item_components", {"source": "selected"})
    map_id = stack.get("components", {}).get("minecraft:map_id")
    steps.report["filled_map_components"] = stack
    steps.check("map_received_a_real_map_id_component", type(map_id) is int and map_id >= 0)
    if type(map_id) is not int:
        return
    steps.wait(lambda _state: _explored(steps, map_id) is not None,
               "map_record_received", timeout_seconds=30)
    steps.report["maps_after_filling"] = {str(key): row for key, row in _maps(steps).items()}
    steps.check("held_map_explores_before_the_pocket_move", (_explored(steps, map_id) or 0) > 0)

    # The client's own drag puts the map in a pocket: hotbar slot 0 sits at menu slot 36 in
    # the player inventory, and slot 20 is outside the hotbar and the off hand.
    steps.call("minecraft_open_inventory")
    steps.wait(lambda state: state["screen"]["open"] and state["container"]["container_id"] == 0,
               "inventory_open", timeout_seconds=30)
    steps.call("minecraft_click_container_slot", {"slot": 36, "button": "primary"})
    steps.call("minecraft_click_container_slot", {"slot": POCKET, "button": "primary"})
    steps.wait(lambda state: any(stack["slot"] == POCKET and stack["item_id"] == FILLED
                                 for stack in state["inventory"])
               and state["container"]["carried"]["count"] == 0,
               "map_moved_into_the_pocket", timeout_seconds=30)
    steps.call("minecraft_close_screen")
    carried = steps.observe()
    steps.report["pocket_slot"] = POCKET
    # "In hand" is a hotbar slot (0..8) or the off hand (40); the hotbar selection itself
    # stays where it was, so it cannot carry the check.
    steps.check("map_left_the_hand", not any(stack["item_id"] == FILLED
                                            and (stack["slot"] < 9 or stack["slot"] == OFF_HAND)
                                            for stack in carried["inventory"]))

    # Walk a short leg away from the pad; only a held map may paint.
    _aim(steps, {"x": x, "z": z - 8, "bounding_box": {"min_y": y, "max_y": y + 2}})
    # While held the map legitimately painted pixels; from here on it is a pocket copy, so
    # the count must not move again.
    start = _explored(steps, map_id) or 0
    steps.report["explored_when_pocketed"] = start
    samples: list[int | None] = []
    for _ in range(WALK_STEPS):
        steps.call("minecraft_press_inputs", {"keys": ["forward"], "ticks": TICKS_PER_STEP})
        samples.append(_explored(steps, map_id))
    steps.report["explored_samples"] = samples
    steps.check("pocket_map_never_explores", all(value in (None, start) for value in samples))
    steps.check("pocket_map_keeps_the_pixels_it_had", (_explored(steps, map_id) or 0) == start)

    steps.report["limits"] = [
        "One allocated scale-zero map carried in inventory slot 20 while the player walks six short "
        "legs; explored pixels come from the client's own `maps` observation. No wall map, no clone, "
        "no cartography table and no deposit/withdraw.",
    ]
