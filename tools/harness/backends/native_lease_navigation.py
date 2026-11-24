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


def _arrived(steps, name, arguments):
    """One bounded navigation request; a client that refuses the route is this row's assertion false.

    `minecraft_navigate_to_block` reports an exhausted planner as `target is unreachable …` with
    `retryable: false` (stage `automation`). Only that refusal is the row's observable: a timeout, an
    MCP drop or any other tool error is re-raised, because it never observed the bug and must not
    become a red proving assertion."""
    try:
        result = steps.call("minecraft_navigate_to_block", arguments)
    except RuntimeError as error:
        text = str(error)
        if '"code": "target is unreachable' not in text or '"retryable": false' not in text:
            raise
        steps.report.setdefault("navigation_errors", {})[name] = text
        result = {}
    steps.check(name, result.get("arrived") is True)
    return result

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
                    "entity_id": target["entity_id"], "timeout_seconds": 150.0})
            except Exception as error:
                result["approach_error"] = str(error)
        worker = threading.Thread(target=approach)
        worker.start()
        worker.join(timeout=165)
        # The row's own claim comes first: a client that loses the controller at the idle deadline
        # never returns the bounded action, so `active_writer_survived_idle_deadline` is the
        # assertion that has to flip, and the generic completion check follows it.
        completed = not worker.is_alive()
        approach = result.get("approach") or {}
        controller_now = _controller(b)
        steps.report["idle_deadline"] = {
            "elapsed_seconds": round(time.monotonic() - started, 3),
            "completed": completed,
            "approach_entity_uuid": approach.get("entity_uuid"),
            "target_entity_uuid": target.get("entity_uuid"),
            "approach_error": result.get("approach_error"),
            "controller_active": controller_now.get("active"),
            "controller_remaining_ms": controller_now.get("remaining_ms"),
        }
        steps.check("active_writer_survived_idle_deadline",
                    time.monotonic() - started >= 125 and completed
                    and approach.get("entity_uuid") == target.get("entity_uuid")
                    and controller_now.get("active") is True)
        steps.check("long_action_completed", completed)
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
        # The cancelled writer unwinds asynchronously; a press that arrives while the client still
        # reports writer_action_busy is a harness race, not the row. Retry on the client's own state
        # (bounded), and record the last error so a permanent busy writer is still visible.
        press_error = None
        for _attempt in range(25):
            try:
                b.call_tool("minecraft_press_inputs", {"keys": ["forward"], "ticks": 1, "hold": True})
                press_error = None
                break
            except RuntimeError as error:
                press_error = str(error)
                if "writer_action_busy" not in press_error:
                    break
                b.call_tool("minecraft_observe")
        if press_error is not None:
            steps.report.setdefault("replacement_press_errors", []).append(press_error)
        replacement = _controller(b)
        worker.join(timeout=20)
        steps.check("old_action_cancelled", not worker.is_alive() and "cancel" in result.get("old_error", ""))
        current = _controller(b)
        steps.check("replacement_writer_retained", current.get("active") is True
                    and current.get("controller") == replacement.get("controller"))
        steps.check("replacement_input_retained", "forward" in b.call_tool("minecraft_observe")["inputs"]["held"])
        # A UI intent consumes the UI state's pending release (`UiState::execute`), but opening a
        # screen releases gameplay input on BOTH clients by design (restore receipt
        # 20261124T131141-native-v2-lease-navigation-oelqyuq9: the held key is gone on the fixed
        # client too), so an open/close pair cannot carry this row's control. The step was removed
        # again; the row stays open with that finding instead of a non-discriminating assertion.
        b.call_tool("minecraft_press_inputs", {"keys": ["forward"], "ticks": 1, "hold": False})
    finally:
        b.close()
        if worker is not None:
            worker.join(timeout=175)

    # The client must receive the hole and adjacent supporting block before the
    # exact straddling pose is imposed by the private operator fixture.
    _set(steps, {"x": x, "y": y - 1, "z": z}, "minecraft:air")
    _block(steps, {"x": x + 1, "y": y - 1, "z": z}, "minecraft:obsidian")
    steps.fixture(f"tp @s {x + .85} {y} {z + .5}")
    state = steps.wait(lambda s: s["player"]["on_ground"] and abs(s["player"]["x"] - (x + .85)) < .03,
                       "grounded_edge_pose")
    steps.check("edge_center_over_air", int(state["player"]["x"] // 1) == x)
    _arrived(steps, "edge_navigation_arrived", {"x": x + 4, "y": y, "z": z, "timeout_seconds": 30.0})
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
    _arrived(steps, "open_door_navigation_arrived", {"x": x, "y": y, "z": z - 5, "timeout_seconds": 60.0})
    door_after = steps.observe()["player"]
    steps.check("door_grounded_crossing", door_after["on_ground"] and door_after["z"] < z - 2)
    _echo(steps, "lease_navigation_complete")
    steps.report["scope"] = "private fixture, native MCP writer and observed navigation; not general input or route coverage"
