"""Original cursor-close and placed-banner repros through the native client."""
from __future__ import annotations

import math

from .native_p1 import _capture
from .native_v2 import _block, _click, _echo, _open, _point, _set, _use
from .probe_steps import ProbeSteps, item_count


def cursor_close(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    point = _point(origin)
    _set(steps, point, "minecraft:crafting_table")
    steps.fixture("give @s minecraft:apple 3")
    steps.wait(lambda state: item_count(state, "minecraft:apple") == 3,
               "cursor_fixture_three_apples_received")
    for menu in ("inventory", "crafting_table"):
        if menu == "inventory":
            steps.call("minecraft_open_inventory")
            opened = steps.wait(lambda state: state["screen"]["open"]
                                and state["container"]["container_id"] == 0,
                                "cursor_inventory_open_received")
        else:
            opened = _open(steps, point, "CraftingMenu", 46)
        sources = [stack for stack in opened["container"]["slots"]
                   if stack["item_id"] == "minecraft:apple" and stack["count"] == 3]
        steps.check(f"{menu}_unique_three_apple_source", len(sources) == 1)
        _click(steps, sources[0]["slot"])
        carried = steps.wait(lambda state: state["container"]["carried"]["item_id"] == "minecraft:apple"
                             and state["container"]["carried"]["count"] == 3,
                             f"{menu}_three_apples_on_cursor_received")
        steps.check(f"{menu}_no_inventory_duplicate_before_close",
                    item_count(carried, "minecraft:apple") == 0)
        _capture(steps, f"cursor-{menu}-before-close")
        steps.call("minecraft_close_screen")
        # The chat fence orders the close response, not a repair inventory click.
        closed = _echo(steps, f"{menu}_close_ordered")
        steps.check(f"{menu}_close_returned_three_exactly_once",
                    item_count(closed, "minecraft:apple") == 3)
        if menu == "inventory":
            steps.call("minecraft_open_inventory")
            reopened = steps.wait(lambda state: state["screen"]["open"]
                                  and state["container"]["container_id"] == 0,
                                  "cursor_inventory_reopened_received")
        else:
            reopened = _open(steps, point, "CraftingMenu", 46)
        # Capture even the pre-fix phantom, before checking/failing or clicking.
        _capture(steps, f"cursor-{menu}-reopened-before-any-click")
        steps.check(f"{menu}_reopen_cursor_empty_before_any_click",
                    reopened["container"]["carried"]["count"] == 0)
        steps.check(f"{menu}_reopen_three_apples_conserved",
                    item_count(reopened, "minecraft:apple") == 3)
        steps.call("minecraft_close_screen")
    steps.report["limits"] = ["original three-apple inventory and crafting-table repros; no overflow branch"]


def banner_placement(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    snapshot = getattr(steps.client, "promotion_wire_receipt", None)
    if not callable(snapshot):
        raise RuntimeError("placement probe requires its unmodified graphical wire receipt")
    layers = [{"pattern": "minecraft:cross", "color": "red"},
              {"pattern": "minecraft:stripe_top", "color": "blue"}]
    cases = (
        ("patterned", "white_banner",
         'banner_patterns=[{pattern:"minecraft:cross",color:"red"},{pattern:"minecraft:stripe_top",color:"blue"}],custom_name={text:"Named banner"}',
         {"patterns": layers, "CustomName": {"text": "Named banner"}}),
        ("named_plain", "white_banner", 'custom_name={text:"Plain named banner"}',
         {"CustomName": {"text": "Plain named banner"}}),
    )
    for index, (label, item, components, expected) in enumerate(cases):
        point = _point(origin, dx=index - 2)
        support = {**point, "y": point["y"] - 1}
        _set(steps, point, "minecraft:air")
        steps.fixture(f"debug give minecraft:{item}[{components}] 1 0")
        steps.call("minecraft_set_hotbar_slot", {"slot": 0})
        _echo(steps, f"placement_{label}_fixture_ordered")
        stack = steps.call("minecraft_read_item_components", {"source": "selected"})
        steps.check(f"placement_{label}_source_received",
                    stack.get("item_id") == f"minecraft:{item}" and stack.get("count") == 1)
        steps.report.setdefault("placement_source_stacks", {})[label] = stack
        cursor = len(snapshot()["events"])
        _use(steps, support)
        _block(steps, point, f"minecraft:{item}")
        _echo(steps, f"placement_{label}_use_ordered")
        player = steps.observe()["player"]
        dx, dz = point["x"] + .5 - player["x"], point["z"] + .5 - player["z"]
        steps.call("minecraft_look", {
            "yaw_deg": round(math.degrees(math.atan2(-dx, dz))),
            "pitch_deg": round(math.degrees(math.atan2(player["y"] + 1.62 - point["y"] - .6,
                                                       math.hypot(dx, dz)))),
        })
        _capture(steps, f"placement-{label}-received")
        position = [point[key] for key in ("x", "y", "z")]

        def transferred(_state: dict) -> bool:
            rows = [row for row in snapshot()["events"][cursor:]
                    if row["packet"] == "block_entity_data" and row["position"] == position]
            return bool(rows) and all(rows[-1]["nbt"].get(key) == value for key, value in expected.items())

        steps.wait(transferred, f"placement_{label}_item_components_received_as_block_entity",
                   timeout_seconds=10)
        steps.wait(lambda state: item_count(state, f"minecraft:{item}") == 0,
                   f"placement_{label}_ordinary_use_consumed_one")
    steps.report["placement_wire"] = snapshot()
    steps.report["visual_review_required"] = [
        "Patterned banner shows red cross and blue top stripe; plain named banner remains plain.",
    ]
    steps.report["limits"] = ["patterned and named banner transfer only; profiled heads and decorated pots have separate open rows"]
