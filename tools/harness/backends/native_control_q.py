"""Ctrl+Q at the world level: vanilla `player.drop(Screen.hasControlDown())`.

Without Ctrl one press drops one item; with Ctrl held the whole selected stack leaves the slot in
one act. The client does not predict a drop (interaction.rs sends only PLAYER_ACTION), so the scene
asserts what the *server* did: the inventory the client received and the dropped entities' counts.
A stack of N must arrive on the ground as ONE item entity carrying N, never as N entities of one and
never as a single item.
"""
from __future__ import annotations

import time
from typing import Any

from .native_v2 import _echo
from .probe_steps import ProbeSteps, item_count


DIAMOND = "minecraft:diamond"
STACK = 5


def _selected(steps: ProbeSteps) -> dict[str, Any]:
    """The selected hotbar stack as the server sent it (`_prepare` leaves the rest empty)."""
    return steps.call("minecraft_read_item_components", {"source": "selected"})


def _ground(steps: ProbeSteps) -> list[int]:
    """Counts of the dropped diamonds on the ground, one row per item entity."""
    rows = steps.client.call_tool("minecraft_list_entities", {"radius": 8, "limit": 128})["entities"]
    return [row["item"]["count"] for row in rows
            if row.get("item", {}).get("item_id") == DIAMOND]


def _ground_settled(steps: ProbeSteps, timeout_seconds: float, empty: bool = False) -> list[int]:
    """Block on server ticks until two successive reads of the ground agree.

    `minecraft_wait_ticks` waits on the client's own tick loop, so this is an event wait at tick
    granularity, not a wall-clock sleep."""
    deadline = time.monotonic() + timeout_seconds
    previous: list[int] | None = None
    while True:
        counts = _ground(steps)
        wanted = counts == [] if empty else counts != []
        if wanted and counts == previous:
            return counts
        if time.monotonic() >= deadline:
            return counts
        previous = counts
        steps.call("minecraft_wait_ticks", {"ticks": 4})


def _press(steps: ProbeSteps, keys: list[str], ticks: int = 2) -> Any:
    return steps.call("minecraft_press_inputs", {"keys": keys, "ticks": ticks})


def _set_stack(steps: ProbeSteps, count: int = STACK) -> dict[str, Any]:
    """`debug give` sets the hotbar slot outright (it does not add)."""
    steps.fixture(f"debug give {DIAMOND} {count} 0")
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    return steps.wait(lambda state: item_count(state, DIAMOND) == count,
                      f"control_q_inventory_holds_{count}", timeout_seconds=15)


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    state = _set_stack(steps)
    steps.check("control_q_no_screen_open", state["screen"].get("open") is False)
    steps.report["control_q_selected_stack"] = _selected(steps).get("count")
    steps.check("control_q_selected_stack_holds_five", _selected(steps).get("count") == STACK)
    # Baseline on the same key path: one press without Ctrl drops exactly one item.
    _press(steps, ["key.keyboard.q"])
    state = steps.wait(lambda state: item_count(state, DIAMOND) == STACK - 1,
                       "control_q_plain_press_drops_one_item", timeout_seconds=20)
    plain = _ground_settled(steps, 20)
    steps.report["control_q_plain"] = {"selected": _selected(steps).get("count"), "ground": plain}
    steps.check("control_q_plain_press_leaves_the_stack_short_one", plain == [1])
    # A fresh stack of five, and no leftovers from the baseline on the ground.
    steps.fixture("kill @e[type=minecraft:item]")
    steps.call("minecraft_wait_ticks", {"ticks": 5})
    steps.check("control_q_baseline_ground_cleared", _ground_settled(steps, 10, empty=True) == [])
    steps.check("control_q_stack_refilled", item_count(_set_stack(steps), DIAMOND) == STACK)
    # Ctrl is reported to the session as its own modifier state; both keys go down in one native
    # keyboard call, so Q is pressed while Ctrl is held (vanilla Ctrl+Q in the world).
    _press(steps, ["key.keyboard.left.control", "key.keyboard.q"])
    # Wait for the server's own slot update (either outcome): the assertion below, not a timeout,
    # is what a pre-fix control has to fail.
    steps.wait(lambda state: item_count(state, DIAMOND) < STACK,
               "control_q_drop_applied_by_the_server", timeout_seconds=20)
    steps.call("minecraft_wait_ticks", {"ticks": 5})
    ground = _ground_settled(steps, 20)
    steps.report["control_q_ctrl"] = {"selected": _selected(steps).get("count"), "ground": ground}
    steps.check("control_q_whole_stack_dropped", ground == [STACK])
    steps.check("control_q_whole_stack_is_one_entity", len(ground) == 1)
    steps.check("control_q_whole_stack_leaves_the_slot",
                item_count(steps.observe(), DIAMOND) == 0)
    _echo(steps, "control_q_ordered_chat")
