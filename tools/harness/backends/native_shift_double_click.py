"""Physical Shift-double-click with an observable vanilla bulk-transfer result."""
from __future__ import annotations

import ctypes as c
import time
from pathlib import Path
from typing import Any

from .menu_observer import observe_menu
from .native_craft_drag import Pointer
from .native_v2 import _echo
from .probe_steps import ProbeSteps


def run(steps: ProbeSteps, artifact: Path, owned_window: dict[str, Any], *, java: bool = False) -> None:
    # The full hotbar prevents either initial quick-move from emptying slot 9.
    # The release scan then quick-moves the matching hotbar stacks into the
    # available main inventory, which makes server custody distinguish old code.
    for slot in range(9):
        steps.fixture(f"debug give minecraft:stone 64 {slot}")
    steps.fixture("give @s minecraft:stone 10")
    _echo(steps, "shift_fixture_ordered")
    steps.call("minecraft_open_inventory")
    state = steps.wait(lambda s: s["screen"]["open"] and s["container"]["container_id"] == 0,
                       "shift_inventory_open")
    before = observe_menu(steps, "shift_before")
    def count(slot: int) -> int:
        item = before["slots"][slot]
        return item["count"] if item is not None and item.get("item") == "minecraft:stone" else 0
    steps.check("shift_fixture_hotbar_full", all(count(i) == 64 for i in range(36, 45)))
    steps.check("shift_fixture_source_partial", count(9) == 10)
    steps.check("shift_fixture_spare_main", all(before["slots"][i]["count"] == 0 for i in range(10, 36)))
    from .shift_matrix import capture, slot_center
    capture(steps, artifact, "shift-before", java)
    tick_before = steps.observe().get("client_tick", 0)
    pointer = Pointer(owned_window)
    x = pointer.x
    xt = pointer.xt
    x.XKeysymToKeycode.argtypes = [c.c_void_p, c.c_ulong]
    x.XKeysymToKeycode.restype = c.c_ubyte
    xt.XTestFakeKeyEvent.argtypes = [c.c_void_p, c.c_uint, c.c_int, c.c_ulong]
    key = x.XKeysymToKeycode(pointer.display, 0xffe1)  # Shift_L
    if not key:
        pointer.close()
        raise RuntimeError("owned Xvfb has no Shift_L key")
    try:
        pointer.move(*slot_center(steps, state, 9, artifact, java))
        if not xt.XTestFakeKeyEvent(pointer.display, key, 1, 0):
            raise RuntimeError("owned Xvfb refused shift press")
        x.XSync(pointer.display, 0)
        started = time.monotonic_ns()
        pointer.button(True)
        pointer.button(False)
        pointer.button(True)
        elapsed_ms = (time.monotonic_ns() - started) / 1_000_000
        pointer.button(False)
        steps.check("shift_second_press_within_250ms", elapsed_ms < 250)
    finally:
        xt.XTestFakeKeyEvent(pointer.display, key, 0, 0)
        x.XSync(pointer.display, 0)
        pointer.close()
    steps.report["shift_press_interval_upper_bound_ms"] = elapsed_ms
    # Native input preempts the MCP writer. Wait for subsequent client ticks
    # before reacquiring a writer so queued physical input cannot revoke it.
    if java:
        steps.call("minecraft_wait_ticks", {"ticks": 2})
    else:
        steps.wait(lambda state: state["client_tick"] >= tick_before + 2,
                   "shift_input_processed")
    _echo(steps, "shift_transfer_ordered")
    after = observe_menu(steps, "shift_after")
    capture(steps, artifact, "shift-after", java)
    stacks = [stack for stack in after["slots"][1:] if stack and stack.get("item") == "minecraft:stone"]
    steps.check("shift_stone_conserved", sum(stack["count"] for stack in stacks) == 586)
    steps.check("shift_hotbar_first_stack_moved", after["slots"][36]["count"] == 0)
