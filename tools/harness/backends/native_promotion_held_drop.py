"""Held drop-key parity on the harness-owned native window (own scenario, not registered yet).

CLIENT-HELD-DROP-KEY-DOUBLE-DROP: vanilla dispatches the drop key once per press
(`Minecraft#handleKeybinds` drains `consumeClick`), so a physical Q held across client ticks must
leave exactly one item on the ground. The pre-fix client also dispatched the held-key tick edge, so
it drops twice.

Run it through `tools.harness run scenario native-v2-held-drop-key` only after the scenario is
registered; until then it is exercised with the same `ProbeSteps` wrapper the other input scenes use.
"""
from __future__ import annotations

import math

from . import native_client  # noqa: F401  (the runner binds the client the same way)
from .menu_observer import holds, observe_menu
from .native_craft_drag import center
from .native_player_input import _echo, _hold_key, _input, _open_inventory, _seed_main
from .probe_steps import ProbeSteps

HOLD_TICKS = 3


def _ground_sum(steps: ProbeSteps) -> int:
    """Dropped items merge: item entities carry stacks, so the count is the stack sum."""
    rows = steps.call("minecraft_list_entities", {"radius": 24.0, "limit": 128})["entities"]
    return sum(int(row.get("item", {}).get("count") or 0)
               for row in rows if row.get("entity_type") == "minecraft:item")


def run(steps: ProbeSteps, origin: tuple[int, int, int] | None = None) -> None:
    state = steps.wait(lambda s: s["player"]["on_ground"], "held_drop_grounded")
    origin = tuple(math.floor(state["player"][key]) for key in ("x", "y", "z"))
    steps.fixture("gamemode survival @s")
    steps.fixture("clear @s")
    steps.fixture("kill @e[type=minecraft:item]")
    # The server spawns nothing while the tick loop is frozen, so this scene never freezes: the
    # symptom is the number of item entities the server actually creates.
    steps.wait(lambda _: _ground_sum(steps) == 0, "held_drop_ground_clear", timeout_seconds=90)
    _seed_main(steps, "minecraft:diamond", 5, 9)
    _echo(steps, "held_drop_fixture")
    state = _open_inventory(steps, False)
    _input(steps, key=ord("1"), point=center(state, 9))
    swapped = observe_menu(steps, "held_drop_exchanged")
    steps.check("held_drop_fixture_selected", holds(swapped["slots"][36], "minecraft:diamond", 5))
    steps.call("minecraft_close_screen")
    steps.call("minecraft_wait_ticks", {"ticks": 5})
    steps.report["held_drop_hold_ticks"] = HOLD_TICKS
    _hold_key(steps, ord("q"), HOLD_TICKS)
    # Settle: wait for the first item, then let the second drop packet (if any) land too.
    steps.wait(lambda _: _ground_sum(steps) >= 1, "held_drop_landed", timeout_seconds=90)
    stable, previous = 0, -1
    for _ in range(6):
        steps.call("minecraft_wait_ticks", {"ticks": 5})
        current = _ground_sum(steps)
        stable = stable + 1 if current == previous else 0
        previous = current
        if stable >= 2:
            break
    steps.report["held_drop_items_after"] = previous
    steps.report["held_drop_origin"] = origin
    steps.check("held_drop_once", previous == 1)
