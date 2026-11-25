"""One drop per Q press, counted as item stacks on the ground (own scenario).

CLIENT-HELD-DROP-KEY-DOUBLE-DROP: vanilla drains `keyDrop` with `consumeClick`, so one press is one
drop; the pre-fix client also dispatched the held-key tick edge and dropped twice. The hold goes
through the client's own `minecraft_press_inputs` with `hold: true`, the route the green lease scene
uses, not an XTest key-down.
"""
from __future__ import annotations

from .menu_observer import holds, observe_menu
from .native_craft_drag import center
from .native_player_input import _echo, _input, _open_inventory, _seed_main
from .probe_steps import ProbeSteps

HOLD_TICKS = 3
DROP_KEY = "key.keyboard.q"


def _ground_sum(steps: ProbeSteps) -> int:
    """Dropped items merge, so the ground count is the sum of the stacks' counts."""
    rows = steps.call("minecraft_list_entities", {"radius": 24.0, "limit": 128})["entities"]
    return sum(int((row.get("item") or {}).get("count") or 0)
               for row in rows if row.get("entity_type") == "minecraft:item")


def run(steps: ProbeSteps, origin: tuple[int, int, int] | None = None) -> None:
    steps.wait(lambda s: s["player"]["on_ground"], "held_drop_grounded")
    steps.fixture("gamemode survival @s")
    steps.fixture("clear @s")
    steps.fixture("kill @e[type=minecraft:item]")
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
    steps.call("minecraft_press_inputs", {"keys": [DROP_KEY], "ticks": 1, "hold": True})
    steps.call("minecraft_wait_ticks", {"ticks": HOLD_TICKS})
    steps.call("minecraft_press_inputs", {"keys": [DROP_KEY], "ticks": 1, "hold": False})
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
    steps.check("held_drop_once", previous == 1)
