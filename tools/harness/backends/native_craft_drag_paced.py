"""Frame-paced physical craft drag using the native harness-owned window."""
from __future__ import annotations

import time
from pathlib import Path
from typing import Any

from . import native_client
from .menu_observer import log_custody, observe_menu
from .native_craft_drag import Pointer, center, count
from .native_v2 import _echo
from .probe_steps import ProbeSteps, container_slot


def run(steps: ProbeSteps, artifact: Path, owned_window: dict[str, Any]) -> None:
    initial = steps.observe()
    sources = [row for row in initial["container"]["slots"]
               if 36 <= row["slot"] <= 44
               and row.get("item_id") == "minecraft:oak_log" and row.get("count") == 1]
    steps.check("native_unique_hotbar_log", len(sources) == 1)
    steps.check("native_empty_grid_cursor", all(container_slot(initial, slot)["count"] == 0
                                                for slot in range(5))
                and initial["container"]["carried"]["count"] == 0)
    pointer = Pointer(owned_window)
    try:
        pointer.move(*center(initial, sources[0]["slot"]))
        started = time.monotonic()
        pointer.button(True)
        steps.wait(lambda s: s["container"]["carried"].get("item_id") == "minecraft:oak_log"
                            and s["container"]["carried"].get("origin") == "server_observed",
                            "native_drag_pickup_received")
        down_server = observe_menu(steps, "native_drag_down")
        steps.check("native_drag_down_server_conserves_log", log_custody(down_server) == 1)
        down = native_client.capture(steps.client, artifact, "native-craft-drag-down")
        steps.call("minecraft_wait_ticks", {"ticks": 1})
        pointer.move(*center(initial, 1))
        move = native_client.capture(steps.client, artifact, "native-craft-drag-move")
        move_server = observe_menu(steps, "native_drag_move")
        steps.check("native_drag_move_server_conserves_log", log_custody(move_server) == 1)
        steps.call("minecraft_wait_ticks", {"ticks": 1})
        pointer.button(False)
        steps.report["gesture_ms"] = (time.monotonic() - started) * 1000
        up = native_client.capture(steps.client, artifact, "native-craft-drag-up")
        up_server = observe_menu(steps, "native_drag_up")
        steps.check("native_drag_up_server_conserves_log", log_custody(up_server) == 1)
        steps.report["frame_ids"] = [down["frame_id"], move["frame_id"], up["frame_id"]]
        steps.check("native_paced_distinct_frames", len(set(steps.report["frame_ids"])) == 3)
        moved = _echo(steps, "native_craft_drag_ordered")
        carried = (moved["container"]["carried"].get("item_id") == "minecraft:oak_log"
                   and moved["container"]["carried"].get("count") == 1
                   and all(container_slot(moved, slot)["count"] == 0 for slot in range(5)))
        placed = (moved["container"]["carried"]["count"] == 0
                  and container_slot(moved, 1).get("item_id") == "minecraft:oak_log"
                  and container_slot(moved, 1).get("count") == 1
                  and container_slot(moved, 0).get("item_id") == "minecraft:oak_planks"
                  and container_slot(moved, 0).get("count") == 4)
        steps.report["drag_outcome"] = "carried" if carried else "placed" if placed else "other"
        steps.check("native_drag_server_custody", (carried or placed)
                    and moved["container"]["carried"].get("origin") == "server_observed"
                    and count(moved, "minecraft:oak_log") == 1)
        steps.call("minecraft_close_screen")
        _echo(steps, "native_craft_drag_closed_ordered")
        steps.call("minecraft_wait_for_inventory", {
            "item_id": "minecraft:oak_log", "count": 1,
            "timeout_seconds": steps.remaining(),
        })
        closed_server = observe_menu(steps, "native_drag_closed")
        steps.check("native_drag_close_server_conserves_log",
                    log_custody(closed_server) == 1
                    and closed_server["carried"]["count"] == 0)
        steps.check("native_drag_matches_vanilla_cursor", carried)
        steps.check("native_drag_up_server_keeps_cursor",
                    up_server["carried"] == {"item": "minecraft:oak_log", "count": 1})
        from . import drag_variants

        drag_variants.run(steps, pointer, center,
                          lambda name: native_client.capture(steps.client, artifact, name),
                          client="native")
        steps.report["craft_drag_scope"] = {
            "input": "Xvfb down/move/up; ordered server probes, captured frames and client ticks between edges",
            "fixture": "operator-granted oak log", "visual_review_required": True,
        }
    finally:
        pointer.close()
