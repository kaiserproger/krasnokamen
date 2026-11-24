"""Ledger lectern custody: insert a book, open the lectern menu, take the book back.

The row `CONTAINERS-002` says the lectern's hold/open/page/take-book menu is
absent.  Every observation here is the client's own received block state, menu
class and inventory; the fixture only supplies the block and the book.
"""
from __future__ import annotations

from typing import Any

from .probe_steps import ProbeSteps, container_slot, item_count

LECTERN = "minecraft:lectern"
BOOK = "minecraft:written_book"


def _point(origin: tuple[int, int, int], dx: int, dz: int) -> dict[str, int]:
    return {"x": origin[0] + dx, "y": origin[1], "z": origin[2] + dz}


def _set(steps: ProbeSteps, point: dict[str, int], state: str) -> None:
    steps.fixture(f"setblock {point['x']} {point['y']} {point['z']} {state}")
    steps.call("minecraft_wait_for_block_state", {
        **point, "block_id": state.split("[", 1)[0], "properties": {}, "timeout_seconds": steps.remaining()})


def _block(steps: ProbeSteps, point: dict[str, int], properties: dict[str, str]) -> dict[str, Any]:
    return steps.call("minecraft_wait_for_block_state", {
        **point, "block_id": LECTERN, "properties": properties, "timeout_seconds": steps.remaining()})


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    point = _point(origin, 2, 1)
    _set(steps, point, f"{LECTERN}[facing=north]")
    steps.fixture(f"debug give {BOOK} 1 0")
    steps.call("minecraft_wait_for_inventory", {
        "item_id": BOOK, "count": 1, "timeout_seconds": steps.remaining()})
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})

    # The first use moves the held book onto the lectern.
    steps.call("minecraft_use_item_on", {**point, "face": "up", "hand": "main_hand"})
    _block(steps, point, {"has_book": "true"})
    steps.wait(lambda state: item_count(state, BOOK) == 0, "held_book_inserted_onto_the_lectern")

    # The same use on an occupied lectern opens its menu with the stored book.
    steps.call("minecraft_use_item_on", {**point, "face": "up", "hand": "main_hand"})
    state = steps.wait(
        lambda state: state["container"]["container_id"] != 0
        and state["container"]["menu_class"] == "net.minecraft.world.inventory.LecternMenu",
        "lectern_menu_received",
    )
    steps.check("lectern_menu_has_one_slot", state["container"]["slot_count"] == 1)
    stored = container_slot(state, 0)
    steps.check("lectern_slot_holds_the_book", stored["item_id"] == BOOK and stored["count"] == 1)

    # The take-book button returns the same book to the player's inventory.
    steps.call("minecraft_click_container_button", {"button_id": 3})
    state = steps.wait(lambda state: item_count(state, BOOK) == 1, "take_book_returns_it")
    if state["container"]["container_id"] != 0 and state["container"]["slot_count"] == 1:
        steps.check("lectern_slot_is_empty_after_take", container_slot(state, 0)["count"] == 0)
    _block(steps, point, {"has_book": "false"})
    steps.check("lectern_released_the_book", True)
    steps.call("minecraft_close_screen")
    steps.report["limits"] = [
        "One written book; page turning is a menu property the client does not expose, so only "
        "insert/open/take are asserted here.",
    ]
