"""The native-only held screenshot: press the keys, capture, release.

The Java client's ``minecraft_press_inputs`` schema declares ``keys`` and ``ticks`` only (its
``press_inputs`` command holds for an exact tick count and returns after it), so a held frame
cannot be requested there and ``validate_steps`` refuses it. Keeping the call in this module
also keeps the Java source scan honest: ``preflight`` validates every MCP call in the scanned
scene-shot modules against the client's own schema, and a native-only ``hold`` literal in one of
them rejected every Java scene-shot run (2026-11-25).
"""

from __future__ import annotations

import json
from typing import Any, Callable


def mcp_calls(step: dict[str, Any]):
    """The native press call a held screenshot issues before launch."""
    yield "minecraft_press_inputs", {"keys": step["hold_keys"], "ticks": 1, "hold": True}


def shoot(probe: Any, step: dict[str, Any], capture: Callable[..., str], artifact: Any) -> str:
    """Press `hold_keys`, wait for the tab list when it is the one asked for, capture, release."""
    name, keys = step["shot"], step["hold_keys"]
    args = {"keys": keys, "ticks": 1, "hold": True}
    probe.call("minecraft_press_inputs", args)
    try:
        if "key.keyboard.tab" in keys:
            probe.wait(lambda state: any(str(node.get("id", "")).startswith("tab/player/")
                                         for node in state.get("ui_nodes", [])),
                       "tab_list_visible")
        return capture(probe, artifact, name)
    finally:
        probe.call("minecraft_press_inputs", {**args, "hold": False})


def use_phases(probe: Any, value: dict[str, Any], capture: Callable[..., str], artifact: Any) -> list[str]:
    """Hold use for `value['ticks']`, proving it starts, stays held and releases on the native client."""
    name = value["name"]
    before = probe.observe()
    probe.check(f"{name}_initially_idle", before.get("using") is None)
    args = {"keys": ["use"], "ticks": 1, "hold": True}
    probe.call("minecraft_press_inputs", args)
    shots = []
    try:
        probe.call("minecraft_wait_ticks", {"ticks": 1})
        started = probe.observe()
        probe.check(f"{name}_started", started.get("using") is not None)
        (artifact / f"{name}-start.json").write_text(json.dumps(started, indent=2) + "\n")
        shots.append(capture(probe, artifact, f"{name}-start"))
        probe.call("minecraft_wait_ticks", {"ticks": value["ticks"]})
        held = probe.observe()
        probe.check(f"{name}_held", held.get("using") is not None)
        (artifact / f"{name}-hold.json").write_text(json.dumps(held, indent=2) + "\n")
        shots.append(capture(probe, artifact, f"{name}-hold"))
    finally:
        probe.call("minecraft_press_inputs", {**args, "hold": False})
    probe.call("minecraft_wait_ticks", {"ticks": 1})
    released = probe.observe()
    probe.check(f"{name}_released", released.get("using") is None)
    (artifact / f"{name}-release.json").write_text(json.dumps(released, indent=2) + "\n")
    shots.append(capture(probe, artifact, f"{name}-release"))
    return shots
