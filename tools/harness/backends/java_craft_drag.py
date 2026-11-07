"""Physical player-inventory craft drag through the private vanilla Java client."""
from __future__ import annotations

from pathlib import Path
import time
from typing import Any

from . import java_p5
from .native_craft_drag import Pointer
from .menu_observer import holds, log_custody, observe_menu
from .native_v2 import _echo, _prepare
from .probe_steps import ProbeSteps, container_slot, item_count


def run(client: Any, artifact: Path, timeout: float, *, paced: bool = False) -> dict[str, Any]:
    scenario = "java-v2-p5-craft-drag-paced" if paced else "java-v2-p5-craft-drag"
    with ProbeSteps(client, artifact / scenario, timeout, scenario) as steps:
        _prepare(steps)
        steps.fixture("give @s minecraft:oak_log 1")
        steps.call("minecraft_wait_for_inventory", {
            "item_id": "minecraft:oak_log", "count": 1,
            "timeout_seconds": steps.remaining(),
        })
        _echo(steps, "java_drag_fixture_ordered")
        steps.call("minecraft_open_inventory")
        initial = steps.wait(lambda state: state["screen"]["open"]
                             and state["container"]["container_id"] == 0
                             and state["container"]["slot_count"] == 46,
                             "java_player_inventory_open")
        sources = [row for row in initial["container"]["slots"]
                   if 36 <= row["slot"] <= 44
                   and row.get("item_id") == "minecraft:oak_log" and row.get("count") == 1]
        steps.check("java_unique_hotbar_log", len(sources) == 1)
        steps.check("java_empty_grid_cursor", all(container_slot(initial, slot)["count"] == 0
                                                  for slot in range(5))
                    and initial["container"]["carried"]["count"] == 0)
        before = Path(java_p5.capture(steps, "java-craft-drag-before"))
        from PIL import Image
        with Image.open(before) as image:
            width, height = image.size
        # P5 sets guiScale=2. Vanilla player inventory is 176x166 GUI pixels;
        # the first crafting slot is at (98,18), hotbar slots at (8+18*n,142).
        left, top, scale = (width - 352) // 2, (height - 332) // 2, 2
        hotbar = sources[0]["slot"] - 36
        pointer = Pointer(client.p4_window)
        try:
            if not paced:
                # Preserve a genuinely unfenced fast gesture as a separate
                # observation; server snapshots below necessarily fence input.
                pointer.move(left + (16 + 18 * hotbar) * scale, top + 150 * scale)
                fast_start = time.monotonic()
                pointer.button(True)
                pointer.move(left + 106 * scale, top + 26 * scale)
                pointer.button(False)
                steps.report["unfenced_fast_gesture_ms"] = (time.monotonic() - fast_start) * 1000
                fast = _echo(steps, "java_unfenced_drag_ordered")
                java_p5.capture(steps, "java-craft-drag-unfenced")
                steps.check("java_unfenced_drag_keeps_cursor",
                            fast["container"]["carried"].get("item_id") == "minecraft:oak_log"
                            and fast["container"]["carried"].get("count") == 1
                            and all(container_slot(fast, slot)["count"] == 0 for slot in range(5)))
                fast_server = observe_menu(steps, "java_unfenced_drag_up")
                steps.check("java_unfenced_server_conserves_log",
                            log_custody(fast_server) == 1
                            and holds(fast_server["carried"], "minecraft:oak_log", 1))
                steps.call("minecraft_close_screen")
                _echo(steps, "java_unfenced_closed_ordered")
                steps.fixture("clear @s")
                steps.fixture("give @s minecraft:oak_log 1")
                _echo(steps, "java_instrumented_fixture_ordered")
                steps.call("minecraft_open_inventory")
                initial = steps.wait(lambda state: state["screen"]["open"]
                                     and state["container"]["container_id"] == 0
                                     and state["container"]["slot_count"] == 46,
                                     "java_instrumented_inventory_open")
                sources = [row for row in initial["container"]["slots"]
                           if 36 <= row["slot"] <= 44
                           and row.get("item_id") == "minecraft:oak_log" and row.get("count") == 1]
                steps.check("java_instrumented_unique_hotbar_log", len(sources) == 1)
                hotbar = sources[0]["slot"] - 36
            pointer.move(left + (16 + 18 * hotbar) * scale, top + 150 * scale)
            started = time.monotonic()
            pointer.button(True)
            steps.wait(lambda s: s["container"]["carried"].get("item_id") == "minecraft:oak_log",
                       "java_drag_pickup_observed")
            down_server = observe_menu(steps, "java_drag_down")
            steps.check("java_drag_down_server_conserves_log", log_custody(down_server) == 1)
            if paced:
                java_p5.capture(steps, "java-craft-drag-down")
                steps.call("minecraft_wait_ticks", {"ticks": 1})
            pointer.move(left + 106 * scale, top + 26 * scale)
            java_p5.capture(steps, "java-craft-drag-move")
            move_server = observe_menu(steps, "java_drag_move")
            steps.check("java_drag_move_server_conserves_log", log_custody(move_server) == 1)
            if paced:
                steps.call("minecraft_wait_ticks", {"ticks": 1})
            pointer.button(False)
            steps.report["gesture_ms"] = (time.monotonic() - started) * 1000
            java_p5.capture(steps, "java-craft-drag-up")
            up_server = observe_menu(steps, "java_drag_up")
            steps.check("java_drag_up_server_conserves_log", log_custody(up_server) == 1)
            steps.check("java_drag_up_server_keeps_cursor",
                        holds(up_server["carried"], "minecraft:oak_log", 1))
            moved = _echo(steps, "java_craft_drag_ordered")
            java_p5.capture(steps, "java-craft-drag-after")
            carried = (moved["container"]["carried"].get("item_id") == "minecraft:oak_log"
                       and moved["container"]["carried"].get("count") == 1
                       and all(container_slot(moved, slot)["count"] == 0 for slot in range(5)))
            placed = (moved["container"]["carried"]["count"] == 0
                      and container_slot(moved, 1).get("item_id") == "minecraft:oak_log"
                      and container_slot(moved, 1).get("count") == 1
                      and container_slot(moved, 0).get("item_id") == "minecraft:oak_planks"
                      and container_slot(moved, 0).get("count") == 4)
            steps.report["drag_outcome"] = "carried" if carried else "placed" if placed else "other"
            steps.check("java_drag_custody", (carried or placed)
                        and item_count(moved, "minecraft:oak_planks") == 0)
            steps.check("java_drag_keeps_log_on_cursor", carried)
            steps.call("minecraft_close_screen")
            _echo(steps, "java_craft_drag_closed_ordered")
            steps.call("minecraft_wait_for_inventory", {
                "item_id": "minecraft:oak_log", "count": 1,
                "timeout_seconds": steps.remaining(),
            })
            final = steps.observe()
            steps.check("java_drag_close_keeps_log", item_count(final, "minecraft:oak_log") == 1)
            closed_server = observe_menu(steps, "java_drag_closed")
            steps.check("java_drag_close_server_conserves_log",
                        log_custody(closed_server) == 1
                        and closed_server["carried"]["count"] == 0)
            from . import drag_variants

            def coordinates(_state: dict[str, Any], slot: int) -> tuple[int, int]:
                if slot == 1:
                    return left + 106 * scale, top + 26 * scale
                return left + (16 + 18 * (slot - 36)) * scale, top + 150 * scale

            drag_variants.run(steps, pointer, coordinates,
                              lambda name: java_p5.capture(steps, name), client="java")
            steps.report["craft_drag_scope"] = {
                "input": "Xvfb down/move/up; ordered server probes, captured frames and client ticks between edges" if paced
                         else "Xvfb down/move/up with ordered server probes between edges",
                "fixture": "operator-granted oak log", "gui_scale": scale,
                "visual_review_required": True,
            }
        finally:
            pointer.close()
    return steps.report


def run_paced(client: Any, artifact: Path, timeout: float) -> dict[str, Any]:
    return run(client, artifact, timeout, paced=True)
