"""Windowed native writer lifetime, handoff and navigation regressions."""
from __future__ import annotations

import json
import threading
import time

from ..mcp import McpClient
from .native_v2 import _block, _echo, _set
from .probe_steps import ProbeSteps


def _controller(client):
    resource = client._request("resources/read", {"uri": "krasnokamen-client://capabilities"})
    return json.loads(resource["contents"][0]["text"])["controller"]

def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    a = steps.client
    b = McpClient(a.endpoint, a.token, 140)
    b.initialize()
    worker = None
    result = {}
    try:
        x, y, z = origin
        _set(steps, {"x": x, "y": y + 11, "z": z}, "minecraft:stone")
        steps.fixture(f"summon minecraft:armor_stand {x + .5} {y + 12} {z + .5}")
        target = {}
        def received_target(_state):
            rows = steps.call("minecraft_list_entities", {
                "entity_type": "minecraft:armor_stand", "radius": 32, "limit": 64})["entities"]
            matches = [row for row in rows if abs(row["x"] - (x + .5)) < .1
                       and abs(row["y"] - (y + 12)) < .1 and abs(row["z"] - (z + .5)) < .1]
            if len(matches) != 1:
                return False
            target.update(matches[0])
            return True
        steps.wait(received_target, "stationary_out_of_reach_target", timeout_seconds=30)
        started = time.monotonic()
        def approach():
            try:
                result["approach"] = a.call_tool("minecraft_approach_entity", {
                    "entity_id": target["entity_id"], "timeout_seconds": 120.0})
            except Exception as error:
                result["approach_error"] = str(error)
        worker = threading.Thread(target=approach)
        worker.start()
        worker.join(timeout=135)
        steps.check("long_action_completed", not worker.is_alive())
        steps.check("active_writer_survived_idle_deadline", time.monotonic() - started >= 119
                    and result.get("approach", {}).get("entity_uuid") == target["entity_uuid"]
                    and _controller(b).get("active") is True)
        steps.check("long_action_released_input", not b.call_tool("minecraft_observe")["inputs"]["held"])
        result.clear()
        def old_action():
            try:
                result["old"] = a.call_tool("minecraft_press_inputs", {"keys": ["sneak"], "ticks": 255})
            except Exception as error:
                result["old_error"] = str(error)
        worker = threading.Thread(target=old_action)
        worker.start()
        observer = ProbeSteps(b, steps.artifact_dir, 35, "replacement-observer")
        observer.wait(lambda state: "sneak" in state["inputs"]["held"], "old_writer_sneak", timeout_seconds=20)
        steps.check("old_action_in_flight", worker.is_alive())
        a._post({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {
            "requestId": a.request_id, "reason": "handoff regression"}}, expect_json=False)
        b.call_tool("minecraft_press_inputs", {"keys": ["forward"], "ticks": 1, "hold": True})
        replacement = _controller(b)
        worker.join(timeout=20)
        steps.check("old_action_cancelled", not worker.is_alive() and "cancel" in result.get("old_error", ""))
        current = _controller(b)
        steps.check("replacement_writer_retained", current.get("active") is True
                    and current.get("controller") == replacement.get("controller"))
        steps.check("replacement_input_retained", "forward" in b.call_tool("minecraft_observe")["inputs"]["held"])
        b.call_tool("minecraft_press_inputs", {"keys": ["forward"], "ticks": 1, "hold": False})
    finally:
        b.close()
        if worker is not None:
            worker.join(timeout=140)

    # The client must receive the hole and adjacent supporting block before the
    # exact straddling pose is imposed by the private operator fixture.
    _set(steps, {"x": x, "y": y - 1, "z": z}, "minecraft:air")
    _block(steps, {"x": x + 1, "y": y - 1, "z": z}, "minecraft:obsidian")
    steps.fixture(f"tp @s {x + .85} {y} {z + .5}")
    state = steps.wait(lambda s: s["player"]["on_ground"] and abs(s["player"]["x"] - (x + .85)) < .03,
                       "grounded_edge_pose")
    steps.check("edge_center_over_air", int(state["player"]["x"] // 1) == x)
    edge = steps.call("minecraft_navigate_to_block", {"x": x + 4, "y": y, "z": z, "timeout_seconds": 30.0})
    steps.check("edge_navigation_arrived", edge.get("arrived") is True)
    edge_after = steps.observe()["player"]
    steps.check("edge_grounded_movement", edge_after["on_ground"] and edge_after["x"] > x + 2)

    # Force the observed offset that fell inside both old waypoint deadzones;
    # the only route to the target is through the open one-block doorway.
    _set(steps, {"x": x, "y": y - 1, "z": z}, "minecraft:obsidian")
    steps.fixture(f"fill {x - 5} {y} {z - 2} {x + 5} {y + 2} {z - 2} minecraft:obsidian")
    for dy in (0, 1):
        half = "lower" if dy == 0 else "upper"
        _set(steps, {"x": x, "y": y + dy, "z": z - 2},
             f"minecraft:oak_door[facing=north,half={half},hinge=left,open=true]")
        _block(steps, {"x": x, "y": y + dy, "z": z - 2}, "minecraft:oak_door",
               {"half": half, "open": "true"})
    steps.fixture(f"tp @s {x + .416} {y} {z - .498}")
    steps.wait(lambda s: s["player"]["on_ground"] and abs(s["player"]["x"] - (x + .416)) < .03
               and abs(s["player"]["z"] - (z - .498)) < .03, "door_offset_pose")
    route = steps.call("minecraft_navigate_to_block", {"x": x, "y": y, "z": z - 5, "timeout_seconds": 60.0})
    steps.check("open_door_navigation_arrived", route.get("arrived") is True)
    door_after = steps.observe()["player"]
    steps.check("door_grounded_crossing", door_after["on_ground"] and door_after["z"] < z - 2)
    _echo(steps, "lease_navigation_complete")
    steps.report["scope"] = "private fixture, native MCP writer and observed navigation; not general input or route coverage"
