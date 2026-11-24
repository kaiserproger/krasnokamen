"""Ledger COPPER-CHEST-OPEN-WEATHERING: an open copper chest waits for its menu close.

The row says a copper chest oxidizes while a client still holds its menu.  This
scenario puts an unopened control chest and an opened one on the same pad, raises
the random-tick rate so the oxidation sampler rolls, and waits for the control to
advance two weathering stages.  The opened chest must stay at its placed stage with
its menu intact, and it advances only after the menu closes.

Tools used: read_block, wait_for_block_state, use_item_on, close_screen, observe.
"""
from __future__ import annotations

from typing import Any

from .native_v2 import _block, _open, _point
from .probe_steps import ProbeSteps

CHEST = "minecraft:copper_chest"
EXPOSED = "minecraft:exposed_copper_chest"
WEATHERED = "minecraft:weathered_copper_chest"
MENU_SLOTS = 63  # 27 copper-chest slots + 36 player slots
# Oxidation needs a random-tick draw with the ~5.7 % change-over-time gate; the
# default 3 draws per ticking section would need hours for two control stages.
SPEED = 200


def _id(steps: ProbeSteps, point: dict[str, int]) -> str:
    return str(steps.call("minecraft_read_block", point).get("block_id", ""))


def _menu(state: dict[str, Any]) -> bool:
    return (state["container"]["container_id"] != 0
            and state["container"]["menu_class"] == "net.minecraft.world.inventory.ChestMenu"
            and state["container"]["slot_count"] == MENU_SLOTS)


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    # The Manhattan-4 oxidation scan returns false on a younger neighbour, so the two
    # chests sit eight blocks apart: one chest can then advance while the other stays
    # at its placed age.
    opened = _point(origin, dx=-4, dz=2)
    control = _point(origin, dx=4, dz=2)
    for point in (opened, control):
        steps.fixture(f"setblock {point['x']} {point['y']} {point['z']} {CHEST}")
        _block(steps, point, CHEST)

    state = _open(steps, opened, "ChestMenu", MENU_SLOTS)
    steps.check("open_copper_chest_menu_received", _menu(state))
    # The sampler starts only after the menu is open, so the control cannot advance
    # past a stage during setup.
    steps.fixture(f"gamerule randomTickSpeed {SPEED}")

    # The unopened control advancing twice proves the sampler is rolling on this pad.
    steps.call("minecraft_wait_for_block_state", {
        **control, "block_id": EXPOSED, "properties": {}, "timeout_seconds": 110})
    state = steps.observe()
    steps.check("open_copper_chest_stays_unaffected_while_control_reaches_exposed",
                _id(steps, opened) == CHEST)
    steps.check("open_copper_chest_menu_survives_first_control_stage", _menu(state))

    steps.call("minecraft_wait_for_block_state", {
        **control, "block_id": WEATHERED, "properties": {}, "timeout_seconds": 110})
    state = steps.observe()
    steps.check("open_copper_chest_stays_unaffected_while_control_reaches_weathered",
                _id(steps, opened) == CHEST)
    steps.check("open_copper_chest_menu_survives_second_control_stage", _menu(state))

    # Closing the menu releases custody: the chest must now weather like the control.
    steps.call("minecraft_close_screen")
    steps.call("minecraft_wait_for_block_state", {
        **opened, "block_id": EXPOSED, "properties": {}, "timeout_seconds": 110})
    steps.check("closed_copper_chest_resumes_weathering", _id(steps, opened) == EXPOSED)
    steps.report["limits"] = [
        "One opened and one unopened single chest on one pad, ages 0..2; no double chest, "
        "waxed neighbour, death/respawn opener release or restart.  The control carries the "
        "roll evidence at randomTickSpeed %d." % SPEED,
    ]
