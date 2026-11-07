"""Physical right-button and outside-release inventory drag observations."""
from __future__ import annotations

from typing import Any, Callable

from .menu_observer import log_custody, observe_menu
from .native_craft_drag import Pointer
from .native_v2 import _echo
from .probe_steps import ProbeSteps


def run(steps: ProbeSteps, pointer: Pointer, coordinates: Callable[[dict[str, Any], int], tuple[int, int]],
        capture: Callable[[str], Any], *, client: str) -> None:
    for variant, button, outside in (("right", 3, False), ("outside", 1, True)):
        label = f"{client}_drag_{variant}"
        steps.fixture("clear @s")
        steps.fixture("give @s minecraft:oak_log 4")
        _echo(steps, label + "_fixture")
        steps.call("minecraft_open_inventory")
        state = steps.wait(lambda s: s["screen"]["open"] and s["container"]["container_id"] == 0,
                           label + "_open")
        sources = [row for row in state["container"]["slots"] if 36 <= row["slot"] <= 44
                   and row.get("item_id") == "minecraft:oak_log" and row.get("count") == 4]
        steps.check(label + "_fixture_stack", len(sources) == 1)
        pointer.move(*coordinates(state, sources[0]["slot"]))
        pointer.button(True, button)
        held = 2 if button == 3 else 4
        steps.wait(lambda s: s["container"]["carried"].get("item_id") == "minecraft:oak_log"
                   and s["container"]["carried"].get("count") == held, label + "_pickup")
        for phase in ("press", "move", "release"):
            if phase == "move":
                pointer.move(*(25, 25) if outside else coordinates(state, 1))
            elif phase == "release":
                pointer.button(False, button)
            capture(label + "_" + phase)
            image = observe_menu(steps, label + "_" + phase)
            steps.check(label + "_" + phase + "_custody", log_custody(image) == 4)
            if phase == "release":
                steps.check(label + "_release_keeps_cursor",
                            image["carried"]["item"] == "minecraft:oak_log"
                            and image["carried"]["count"] == held)
        steps.call("minecraft_close_screen")
        _echo(steps, label + "_close_ordered")
        image = observe_menu(steps, label + "_close")
        steps.check(label + "_close_custody", log_custody(image) == 4 and image["carried"]["count"] == 0)
        capture(label + "_close")
