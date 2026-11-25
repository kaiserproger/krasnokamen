"""Ledger HS-2: a hopper pulling from a container vehicle refreshes its open menu.

The row says a vehicle-to-hopper transfer leaves an already-open hopper menu stale.
The scenario opens the hopper's own menu, summons a chest minecart carrying one
diamond in the block above it, waits a bounded number of client ticks and reads the
open menu: the pulled diamond must arrive as an ordinary menu-slot update.  Closing
and reopening the menu then shows the same diamond, so the transfer itself is proven
on both trees and a red above is the stale menu, not a missing ingress.
"""
from __future__ import annotations

from typing import Any

from .native_v2 import _block, _open, _point
from .probe_steps import ProbeSteps, container_slot, item_count

HOPPER = "minecraft:hopper"
DIAMOND = "minecraft:diamond"
HOPPER_MENU = "net.minecraft.world.inventory.HopperMenu"
MENU_SLOTS = 41  # 5 hopper slots + 36 player slots
# A hopper pulls on its 8-tick cooldown; ten client-tick steps cover several
# cooldowns without owning the verdict (the labelled assertion does).
WAIT_STEPS = 12
TICKS_PER_STEP = 10


def _hopper_menu(state: dict[str, Any]) -> bool:
    return (state["container"]["container_id"] != 0
            and state["container"]["menu_class"] == HOPPER_MENU
            and state["container"]["slot_count"] == MENU_SLOTS)


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    hopper = _point(origin, dx=2, dz=1)
    for level in (hopper["y"], hopper["y"] + 1):
        steps.fixture(f"setblock {hopper['x']} {level} {hopper['z']} minecraft:air")
    steps.fixture(f"setblock {hopper['x']} {hopper['y']} {hopper['z']} {HOPPER}")
    _block(steps, hopper, HOPPER)

    state = _open(steps, hopper, "HopperMenu", MENU_SLOTS)
    steps.check("open_hopper_menu_received", _hopper_menu(state))
    steps.check("hopper_menu_starts_empty", container_slot(state, 0)["count"] == 0)

    # The vehicle's cargo arrives as vanilla `Items` NBT; the hopper pulls from the
    # container directly above it.
    cart = {**hopper, "y": hopper["y"] + 1}
    steps.fixture(
        f"summon minecraft:chest_minecart {cart['x']} {cart['y']} {cart['z']} "
        f'{{Items:[{{Slot:0b,id:"{DIAMOND}",count:1}}],NoGravity:1b,Motion:[0.0,0.0,0.0]}}')
    steps.call("minecraft_wait_for_visible_entity", {
        "entity_type": "minecraft:chest_minecart", "radius": 12,
        "timeout_seconds": steps.remaining()})

    # Bounded client-tick wait; the verdict below is an assertion on the open menu.
    received = False
    for _ in range(WAIT_STEPS):
        state = steps.observe()
        if container_slot(state, 0)["item_id"] == DIAMOND:
            received = True
            break
        steps.call("minecraft_wait_ticks", {"ticks": TICKS_PER_STEP})
    steps.check("open_hopper_menu_receives_the_pulled_item", received)

    state = steps.observe()
    steps.check("open_hopper_menu_holds_exactly_one_diamond",
                container_slot(state, 0)["count"] == 1)
    steps.check("no_duplicate_in_the_player_inventory", item_count(state, DIAMOND) == 0)

    # The transfer is not the fix: closing and reopening shows the diamond on both
    # trees, so the assertions above are about the open menu alone.
    steps.call("minecraft_close_screen")
    state = steps.observe()
    steps.check("hopper_menu_closed", state["container"]["container_id"] == 0)
    state = _open(steps, hopper, "HopperMenu", MENU_SLOTS)
    steps.check("transfer_reached_the_hopper", container_slot(state, 0)["item_id"] == DIAMOND)
    steps.report["limits"] = [
        "One chest minecart over one hopper, one diamond and one open menu; no chains, "
        "hopper-to-chest push, restart or offline export.  The cart is NoGravity so it "
        "stays in the hopper's suction cell.",
    ]
