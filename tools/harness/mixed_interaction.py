"""Two actual Fabric clients in one world, with client-observed handoff."""
from __future__ import annotations

import contextlib
import json
import os
import secrets
import re
import time
from pathlib import Path
from typing import Any

from . import loader
from .java_clients import _chat_lines, _confirm_exact_permission, _wait_fresh_chat
from .java_matrix_client import JavaMatrixClient
from .mcp import McpClient


def admitted_protocols(server_log: Path) -> dict[str, dict[str, Any]]:
    expected = {"GateFabric": (777, "26.3"), "MixedVersion": (775, "26.1.2")}
    result = {}
    for line in server_log.read_text(errors="replace").splitlines():
        if "login admitted" not in line:
            continue
        fields = dict(re.findall(r'\b(peer|connection|name|protocol|release)=("[^"]*"|\S+)', line))
        fields = {key: value.strip('"') for key, value in fields.items()}
        name = fields.get("name")
        if name not in expected:
            continue
        protocol, release = expected[name]
        if (name in result or fields.get("protocol") != str(protocol)
                or fields.get("release") != release or not fields.get("peer")
                or not fields.get("connection", "").isdigit()):
            raise AssertionError(f"invalid admitted connection for {name}: {line}")
        result[name] = fields
    if set(result) != set(expected) or len({row["connection"] for row in result.values()}) != 2:
        raise AssertionError(f"missing or non-unique admitted protocol records: {result}")
    return result

ITEM = "minecraft:diamond"


def run_candidate(ctx: Any, timeout: float) -> dict[str, Any]:
    from .backends.screens_voice_qa import java_endpoint

    with java_endpoint("screens"):
        return {"scenario": loader.run("fabric", timeout, ctx.artifact_dir, {"run": run},
                                       server_bin=ctx.server_bin, plugins_dir=ctx.plugins_dir,
                                       client_ready_timeout_seconds=None)}


def run(first: Any, runtime: Any, artifact_dir: Path, server_log: Path) -> dict[str, Any]:
    second_version = "26.3" if os.environ.get("A8_NEGATIVE_CONTROL") == "same-version" else "26.1.2"
    report: dict[str, Any] = {"passed": False, "versions": ["26.3", second_version], "steps": {}}
    output = artifact_dir / "mixed-interaction.json"
    second_root = artifact_dir / f"fabric-{second_version}"
    second_root.mkdir()
    previous = os.environ.get("KRASNOKAMEN_JAVA_VERSION")
    try:
        os.environ["KRASNOKAMEN_JAVA_VERSION"] = second_version
        with contextlib.ExitStack() as stack:
            xvfb, display, xlog = runtime.start_xvfb(second_root)
            stack.callback(xlog.close)
            stack.callback(runtime.stop_process, xvfb)
            token, port = secrets.token_urlsafe(32), runtime.reserve_port()
            process, log = runtime.start_client(game_dir=second_root / "game", log_path=second_root / "client.log",
                token=token, mcp_port=port, username="MixedVersion", display=display,
                platform="fabric", ready_timeout_seconds=None)
            stack.callback(log.close)
            stack.callback(runtime.stop_process, process)
            second = McpClient(f"http://127.0.0.1:{port}/mcp", token, 130)
            stack.callback(second.close)
            second.initialize()
            runtime.wait_client_ready_for_connect(second, 120)
            import tomllib
            config = tomllib.loads((artifact_dir / "playable.toml").read_text())
            address = loader.server_address(config["network"]["port"])
            second.call_tool("minecraft_connect", {"server_addr": address})
            _confirm_exact_permission(second, address, 90)
            if not second.call_tool("minecraft_wait_for_play", {"timeout_seconds": 120}).get("in_play"):
                raise AssertionError(f"{second_version} client did not enter Play")
            report["steps"]["admitted_protocols"] = admitted_protocols(server_log)
            a = JavaMatrixClient("GateFabric", first, address, artifact_dir)
            b = JavaMatrixClient("MixedVersion", second, address, artifact_dir)
            report["steps"]["initial"] = [a.call("minecraft_observe"), b.call("minecraft_observe")]
            # Face each other at close range so screenshots and the thrown item are meaningful.
            pose = a.call("minecraft_observe")["player"]
            a.call("minecraft_send_chat", {"message": f"tp MixedVersion {pose['x']} {pose['y']} {pose['z'] + 2}", "command": True})
            a.call("minecraft_look", {"yaw_deg": 0, "pitch_deg": 0})
            b.call("minecraft_look", {"yaw_deg": 180, "pitch_deg": 0})
            b.wait_until(lambda: abs(b.call("minecraft_observe")["player"]["z"] - (pose["z"] + 2)) < 0.5,
                         description="recipient teleported to handoff position")
            for observer, target in ((a, b), (b, a)):
                target_uuid = target.call("minecraft_observe")["player"]["entity_uuid"]
                def visible_target() -> Any:
                    rows = observer.call("minecraft_list_entities", {"radius": 64, "limit": 512})["entities"]
                    return rows if any(x.get("entity_uuid") == target_uuid and
                                       x.get("entity_type") == "minecraft:player" for x in rows) else None
                visible = observer.wait_until(visible_target, description=f"{observer.name} sees {target.name}")
                report["steps"][f"{observer.name}_visible"] = visible
            for sender, receiver in ((a, b), (b, a)):
                baseline = _chat_lines(receiver.call("minecraft_observe"))
                marker = "a8-" + secrets.token_hex(8)
                sender.call("minecraft_send_chat", {"message": marker})
                report["steps"][f"chat_{sender.name}_to_{receiver.name}"] = _wait_fresh_chat(
                    receiver.mcp, baseline, marker, time.monotonic() + 30)
            for player in (a, b):
                loader.capture_ui(player.mcp, artifact_dir, player.name + "-interaction")
            before_a, before_b = a.call("minecraft_observe"), b.call("minecraft_observe")
            def count(state: dict[str, Any]) -> int:
                return sum(x.get("count", 0) for x in state["inventory"] if x.get("item_id") == ITEM)
            if count(before_a) or count(before_b):
                raise AssertionError("diamond must be absent from both initial inventories")
            a.call("minecraft_send_chat", {"message": "give GateFabric minecraft:diamond 1", "command": True})
            a.call("minecraft_wait_for_inventory", {"item_id": ITEM, "count": 1, "timeout_seconds": 30})
            a.call("minecraft_select_hotbar_item", {"item_id": ITEM, "count": 1, "timeout_seconds": 30})
            report["steps"]["before_transfer"] = [a.call("minecraft_observe"), b.call("minecraft_observe")]
            if os.environ.get("A8_NEGATIVE_CONTROL") != "no-drop":
                a.call("minecraft_press_inputs", {"keys": ["key.keyboard.q"], "ticks": 1})
            b.call("minecraft_wait_for_inventory", {"item_id": ITEM, "count": 1, "timeout_seconds": 30})
            after_a, after_b = a.call("minecraft_observe"), b.call("minecraft_observe")
            report["steps"]["after_transfer"] = [after_a, after_b]
            if count(after_a) != 0 or count(after_b) != 1:
                raise AssertionError("handoff must debit donor and credit recipient exactly once")
            report["passed"] = True
            return report
    finally:
        if previous is None:
            os.environ.pop("KRASNOKAMEN_JAVA_VERSION", None)
        else:
            os.environ["KRASNOKAMEN_JAVA_VERSION"] = previous
        output.write_text(json.dumps(report, indent=2) + "\n")
