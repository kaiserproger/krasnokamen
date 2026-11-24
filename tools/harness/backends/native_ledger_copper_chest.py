"""Ledger COPPER-CHEST-OPEN-WEATHERING: an open copper chest waits for its menu close.

The row says a copper chest oxidizes while a client still holds its menu.  This
scenario puts an opened chest and two unopened controls on one pad, raises the
random-tick rate so the oxidation sampler rolls, and waits for BOTH controls to
reach the final oxidation stage.  The opened chest must keep its placed stage and
its menu throughout, and it advances only after the menu closes.

Two controls carry the discrimination: on a tree without the guard the opened chest
advances at the same random rate as the controls, so the gate is green only when it
happens to lag all six control advances, one run in seven.

Tools used: read_block, wait_for_block_state, use_item_on, close_screen, observe.
"""
from __future__ import annotations

from typing import Any

from .native_v2 import _block, _open, _point
from .probe_steps import ProbeSteps

CHEST = "minecraft:copper_chest"
EXPOSED = "minecraft:exposed_copper_chest"
WEATHERED = "minecraft:weathered_copper_chest"
OXIDIZED = "minecraft:oxidized_copper_chest"
MENU_SLOTS = 63  # 27 copper-chest slots + 36 player slots
# Oxidation needs a random-tick draw with the ~5.7 % change-over-time gate; the
# default 3 draws per ticking section would need hours for six control stages.
SPEED = 400


def _id(steps: ProbeSteps, point: dict[str, int]) -> str:
    return str(steps.call("minecraft_read_block", point).get("block_id", ""))


def _menu(state: dict[str, Any]) -> bool:
    return (state["container"]["container_id"] != 0
            and state["container"]["menu_class"] == "net.minecraft.world.inventory.ChestMenu"
            and state["container"]["slot_count"] == MENU_SLOTS)


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    # The Manhattan-4 oxidation scan returns false on a younger neighbour, so the three
    # chests sit at least five blocks apart and each advances only by its own draws.
    opened = _point(origin, dx=-4, dz=2)
    controls = (_point(origin, dx=4, dz=2), _point(origin, dx=4, dz=-3))
    for point in (opened, *controls):
        steps.fixture(f"setblock {point['x']} {point['y']} {point['z']} {CHEST}")
        _block(steps, point, CHEST)

    state = _open(steps, opened, "ChestMenu", MENU_SLOTS)
    steps.check("open_copper_chest_menu_received", _menu(state))
    # The sampler starts only after the menu is open, so no chest can advance during setup.
    steps.fixture(f"gamerule randomTickSpeed {SPEED}")

    # Both unopened controls reaching the final stage prove the sampler is rolling here.
    for index, control in enumerate(controls, 1):
        steps.call("minecraft_wait_for_block_state", {
            **control, "block_id": OXIDIZED, "properties": {}, "timeout_seconds": 115})
        state = steps.observe()
        steps.check(f"open_copper_chest_stays_placed_while_control_{index}_oxidizes",
                    _id(steps, opened) == CHEST)
        steps.check(f"open_copper_chest_menu_survives_control_{index}", _menu(state))

    # Closing the menu releases custody: the chest must now weather like the controls.
    # It may pass more than one stage before the client read, so any advance counts.
    steps.call("minecraft_close_screen")
    steps.call("minecraft_wait_for_block_state", {
        **opened, "block_id": EXPOSED, "properties": {}, "timeout_seconds": 115})
    resumed = _id(steps, opened)
    steps.report["closed_copper_chest_stage"] = resumed
    steps.check("closed_copper_chest_resumes_weathering",
                resumed in (EXPOSED, WEATHERED, OXIDIZED))
    steps.report["limits"] = [
        "One opened and two unopened single chests on one pad, stages 0..3; no double chest, "
        "waxed neighbour, death/respawn opener release or restart.  The controls carry the "
        "roll evidence at randomTickSpeed %d; the opened chest stays at its placed stage "
        "through six control advances, so a guard-free tree passes only when its own draws "
        "lag all of them (about one run in seven), which is why this gate needs repeats." % SPEED,
    ]
