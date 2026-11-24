"""Ledger HS-2: a hopper pulling from a container vehicle refreshes its open menu.

The row says a vehicle-to-hopper transfer leaves an already-open hopper menu stale.
This scenario opens the hopper's own menu first, then summons the chest minecart
above it, so the pull happens while the menu is open; the client must receive the
pulled diamond in its menu slot without reopening the menu.

Tools used: use_item_on, wait_for_visible_entity, close_screen, observe.
"""
from __future__ import annotations

from .native_v2 import _block, _open, _point
from .probe_steps import ProbeSteps, container_slot, item_count

HOPPER = "minecraft:hopper"
CART = "minecraft:chest_minecart"
DIAMOND = "minecraft:diamond"
# 5 hopper slots + 36 player slots.
MENU_SLOTS = 41


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    hopper = _point(origin, dx=2, dz=1)
    steps.fixture(f"setblock {hopper['x']} {hopper['y']} {hopper['z']} {HOPPER}[facing=down]")
    _block(steps, hopper, HOPPER)

    # The client opens the hopper's own menu before any vehicle exists.
    state = _open(steps, hopper, "HopperMenu", MENU_SLOTS)
    steps.check("hopper_menu_starts_empty", container_slot(state, 0)["count"] == 0)

    steps.fixture(
        f"summon {CART} {hopper['x'] + 0.5} {hopper['y'] + 1.0} {hopper['z'] + 0.5} "
        f'{{Items:[{{Slot:0b,id:"{DIAMOND}",count:1}}]}}')
    steps.call("minecraft_wait_for_visible_entity", {
        "entity_type": CART, "radius": 12, "timeout_seconds": steps.remaining()})
    state = steps.wait(
        lambda current: container_slot(current, 0)["item_id"] == DIAMOND,
        "open_hopper_menu_receives_the_pulled_item", timeout_seconds=60)
    steps.check("pulled_exactly_one_diamond", container_slot(state, 0)["count"] == 1)
    steps.check("no_duplicate_in_the_player_inventory", item_count(state, DIAMOND) == 0)
    steps.call("minecraft_close_screen")
    steps.report["limits"] = [
        "One chest minecart above one hopper, one diamond and an already-open menu; no hopper "
        "chains, deposit path or restart.",
    ]
