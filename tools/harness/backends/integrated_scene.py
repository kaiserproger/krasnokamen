"""Private vanilla Java integrated-world scenes; no standalone server or EULA write."""
from __future__ import annotations

import re
from pathlib import Path
from typing import Any

from .. import runtime
from ..profiles import ProfileContext
from . import scene_shot
from .native_craft_drag import Pointer
from .title_connect import _TitleProbeSteps


def wait_ui(probe: _TitleProbeSteps, predicate, label: str) -> dict[str, Any]:
    # The Java bridge emits packet/lifecycle events, but not every title-screen
    # or loading-overlay transition. A bounded event wait also samples UI state
    # when that exact event is absent; all other errors remain fatal.
    while True:
        state = probe.observe()
        if predicate(state):
            probe.check(label, True)
            return state
        try:
            probe.call("minecraft_wait_for_state_change", {
                "observed_version": state["state_version"],
                "timeout_seconds": min(1.0, probe.remaining()),
            })
        except RuntimeError as error:
            if "client state event did not arrive" not in str(error):
                raise


def run(ctx: ProfileContext, scenario: dict[str, Any]) -> dict[str, Any]:
    artifact = ctx.artifact_dir
    timeout = ctx.timeout_seconds or 600.0
    if not 0 < timeout <= 3600:
        raise ValueError("timeout must be in (0, 3600]")
    require_play = scenario.get("require_play", True)
    if type(require_play) is not bool:
        raise ValueError("require_play must be boolean")
    client = client_log = mcp = xvfb = pointer = None
    try:
        xvfb, display, _ = runtime.start_xvfb(artifact)
        ctx.logs.append(str(artifact / "xvfb.log"))
        client, client_log, mcp = scene_shot.start_java(
            ctx, artifact, None, scene_shot.scene_username(scenario), timeout, display, 8, 70)
        mcp.request_timeout_seconds = min(timeout + 10, 130.0)
        window = scene_shot.find_java_window(display)
        if window is None:
            raise RuntimeError("vanilla Java opened no window on the harness Xvfb")
        owned = {"display": display, "window_id": window}
        scene_shot.resize_java_window(owned)
        mcp.p4_window = owned
        ctx.client_evidence["window_focus"] = owned
        ctx.client_evidence["oracle"] = "vanilla Java integrated singleplayer"
        pointer = Pointer(owned)
        with _TitleProbeSteps(mcp, artifact / "integrated-scene", timeout, "integrated-scene") as probe:
            initial = probe.observe()
            probe.check("integrated_title_ready", initial["screen"].get("class", "").endswith("TitleScreen"))
            wait_ui(probe, lambda s: not s.get("overlay", {}).get("open"), "integrated_title_overlay_closed")
            shots = []
            for step in scenario["steps"]:
                if not isinstance(step, dict) or len(step) != 1:
                    raise ValueError(f"invalid integrated scene step: {step!r}")
                kind, value = next(iter(step.items()))
                if kind == "ui_shot" and isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9_-]+", value):
                    shots.append(scene_shot.capture_java(probe, artifact, value))
                    probe.report.setdefault("ui_states", {})[value] = probe.observe()
                elif kind == "ui_wait" and isinstance(value, str):
                    wait_ui(probe, lambda s: s["screen"].get("class") == value, "ui_" + value.rsplit(".", 1)[-1])
                elif kind == "ui_click" and isinstance(value, dict):
                    if set(value) != {"class", "title", "x", "y"}:
                        raise ValueError("ui_click requires exact class, title and window coordinates")
                    state = probe.observe()
                    probe.check("ui_click_screen", state["screen"].get("class") == value["class"]
                                and state["screen"].get("title") == value["title"])
                    x, y = value["x"], value["y"]
                    if type(x) is not int or type(y) is not int or not (0 <= x < 1280 and 0 <= y < 720):
                        raise ValueError("ui_click coordinates outside the owned window")
                    pointer.move(x, y)
                    pointer.button(True)
                    pointer.button(False)
                else:
                    shots.extend(scene_shot.execute(probe, artifact, [step], "java"))
            state = probe.observe()
            if require_play:
                probe.check("integrated_world_playing", state.get("in_play") is True)
                log = (artifact / "client.log").read_text(errors="replace").lower()
                probe.check("integrated_server_logged", "starting integrated minecraft server" in log)
            probe.report["final_state"] = state
        scene_shot.check_java_render_distance(artifact / "game" / "options.txt", 8)
        return {"screenshots": shots, "scenario": probe.report, "oracle": "vanilla Java integrated singleplayer",
                "require_play": require_play}
    finally:
        if pointer is not None:
            pointer.close()
        if mcp is not None:
            mcp.close()
        runtime.stop_process(client)
        if client is not None:
            ctx.exits[ctx.client_evidence["command_index"]] = client.returncode
        if client_log is not None:
            client_log.close()
        runtime.stop_process(xvfb)
