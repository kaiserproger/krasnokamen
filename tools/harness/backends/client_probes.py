"""Cheap MCP client probes; the broader Java compatibility default stays separate."""
from __future__ import annotations

import hashlib
import json
import math
import os
import secrets
import subprocess
import uuid
from dataclasses import replace
from pathlib import Path
from typing import Any

from .. import compatibility, runtime
from ..mcp import McpClient
from ..profiles import ProfileContext, _server_binary
from . import (entities_probe, java_p5, light_wire, native_auth_budget, native_client, native_lifecycle,
               native_light, native_v2)
from .gameplay import cpu_usage, peak_rss_kib, plugin_directory, prepare_operator_fixture, require_tools
from .probe_steps import ProbeSteps

CORE_TOOLS = frozenset({"minecraft_observe", "minecraft_wait_for_state_change", "minecraft_send_chat"})

def world_dir_toml(artifact: Path) -> str:
    """Quote the absolute world path even when receipts live outside the repository."""
    return json.dumps(str(artifact / "world"))



def run_core_subset(client: Any, artifact_dir: Path, timeout: float) -> dict[str, Any]:
    with ProbeSteps(client, artifact_dir / "core-probe", timeout, "core-client-chat-subset") as steps:
        before = steps.observe()
        marker = f"Solaris core-client MCP {uuid.uuid4().hex[:12]}"
        steps.call("minecraft_send_chat", {"message": marker, "command": False})
        after = steps.wait(lambda state: any(marker in line for line in state["recent_chat"]),
                           "own_chat_echo_observed")
        steps.check("same_player_connection", before["player"]["entity_uuid"] == after["player"]["entity_uuid"])
        steps.check("same_dimension", before["dimension"] == after["dimension"])
        steps.report.update(scope="server-chat-only subset", marker=marker,
                            excludes=["Wasm plugin checks", "economy purchase/refund", "Loader-required rejection"])
    return steps.report




def run(ctx: ProfileContext, kind: str) -> dict[str, Any]:
    if kind not in {"inventory", "entities", "core-client", "native-lifecycle"}:
        raise ValueError(f"unknown shared client probe: {kind}")
    v2_scenario = ctx.scenario_id in native_v2.NATIVE_SCENARIO_IDS
    auth_scenario = ctx.scenario_id == native_auth_budget.SCENARIO
    p5_scenario = ctx.scenario_id in java_p5.SCENARIO_IDS
    from . import java_protocol_pot

    protocol_pot = ctx.scenario_id == java_protocol_pot.SCENARIO
    if protocol_pot and (ctx.client != "java" or kind != "inventory"):
        raise ValueError("creative pot echo requires --client java and inventory")
    if p5_scenario:
        if ctx.client != "java" or kind != "entities":
            raise ValueError("java-v2-p5 scenarios require --client java and entities")
    if v2_scenario or auth_scenario:
        if ctx.client != "native" or kind != "entities":
            raise ValueError("native-v2 scenarios require --client native and entities")
    if (v2_scenario or auth_scenario) and native_v2.scenario_catalog.SCENARIOS[ctx.scenario_id].evidence == "graphical" and not ctx.windowed:
        raise ValueError(f"{ctx.scenario_id} requires --windowed")
    soak_seconds = 1800.0
    check = kind in {"inventory", "core-client"} and ctx.extra_args == ["--check"]
    if kind == "native-lifecycle":
        if ctx.extra_args:
            if len(ctx.extra_args) != 2 or ctx.extra_args[0] != "--soak-seconds":
                raise ValueError("native-lifecycle accepts only --soak-seconds N")
            soak_seconds = float(ctx.extra_args[1])
        if not math.isfinite(soak_seconds) or soak_seconds < 0:
            raise ValueError("--soak-seconds must be finite and nonnegative")
    if (ctx.extra_args and kind != "native-lifecycle" and not check) or ctx.platform is not None:
        raise ValueError(f"{kind} probe accepts only --client, --timeout-seconds and --check, without Loader")
    default_timeout = (soak_seconds + 180.0 if kind == "native-lifecycle"
                       else 600.0 if ctx.scenario_id in (native_light.SCENARIO_ID,
                                                        "native-v2-radius-full-view",
                                                        "native-v2-p4-damage",
                                                        "native-v2-automation-storage")
                       else 1200.0 if ctx.scenario_id == "native-v2-settlement-storage"
                       else 900.0 if ctx.scenario_id in ("native-v2-destroyed-chest-race",
                                                         "native-v2-corrupt-block-entities",
                                                         "native-v2-corrupt-chunk-near-camera",
                                                         "native-v2-duplicate-login")
                       else (180.0 if v2_scenario else 120.0))
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else default_timeout
    if not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("probe --timeout-seconds must be finite and positive")
    startup_timeout = min(timeout, 120.0)
    required = {"inventory": compatibility.INVENTORY_TOOLS,
                "entities": entities_probe.REQUIRED_TOOLS,
                "core-client": CORE_TOOLS,
                "native-lifecycle": native_lifecycle.REQUIRED_TOOLS}[kind]
    if v2_scenario:
        required = native_v2.REQUIRED_TOOLS
    if auth_scenario:
        required = native_auth_budget.REQUIRED_TOOLS
    promotion_maps = (ctx.scenario_id or "").startswith("native-v2-map-")
    promotion_actor = ctx.scenario_id in (
        "native-v2-map-inventory-player-markers", "native-v2-map-pumpkin-hiding")
    if ctx.scenario_id == "native-v2-cursor-close":
        required = required | {"minecraft_open_inventory", "client_capture"}
    if ctx.scenario_id in ("native-v2-craft-drag", "native-v2-craft-drag-paced",
                           "native-v2-shift-double-click", "native-v2-shift-components",
                           "native-v2-shift-chest-correction"):
        required = required | {"minecraft_open_inventory", "client_capture"}
    if ctx.scenario_id == "native-v2-chest-drag-disconnect":
        required = required | {"minecraft_connect", "minecraft_wait_for_no_visible_item", "client_capture"}
    if ctx.scenario_id == "native-v2-settlement-storage":
        required = required | {"client_capture"}
    if ctx.scenario_id == "native-v2-custody-mount-export":
        required = required | {"minecraft_approach_entity", "minecraft_interact_entity", "client_capture"}
    if ctx.scenario_id == "native-v2-duplicate-login":
        required = required | {"minecraft_connect"}
    if p5_scenario:
        required = java_p5.REQUIRED_TOOLS
    if protocol_pot:
        required = java_protocol_pot.REQUIRED_TOOLS
    if ctx.scenario_id in ("java-v2-p5-craft-drag", "java-v2-p5-craft-drag-paced",
                           "java-v2-p5-shift-double-click"):
        required = required | {"minecraft_open_inventory"}
    if ctx.scenario_id in ("native-v2-shift-double-click", "native-v2-shift-chest-correction",
                           "java-v2-p5-shift-double-click"):
        required = required | {"minecraft_wait_for_container_slot"}
    artifact = ctx.artifact_dir
    plugin_dir = plugin_directory(ctx)
    if auth_scenario:
        # The shipping solaris-auth package behind the native login gate; the harness copies
        # the sibling checkout's package into the private plugin directory.
        root = Path(os.environ.get("SOLARIS_DEFAULT_PLUGINS_ROOT", runtime.REPO_ROOT.parent / "solaris-default-plugins"))
        package = plugin_dir / "solaris-auth"
        package.mkdir()
        for name in ("plugin.toml", "plugin.wasm"):
            (package / name).write_bytes((root / "solaris-auth" / name).read_bytes())
        ctx.client_evidence["auth_package_sha256"] = hashlib.sha256(
            (package / "plugin.wasm").read_bytes()).hexdigest()
    binary = _server_binary(ctx)
    port = runtime.reserve_port()
    address = f"127.0.0.1:{port}"
    username = {"inventory": "HarnessInventory", "entities": "HarnessEntities",
                "core-client": "HarnessCore", "native-lifecycle": "HarnessLifecycle"}[kind]
    if ctx.scenario_id == "java-v2-p5-towns":
        username = "P5Setup"
    operators = [username] if kind != "core-client" else []
    if ctx.scenario_id == "native-v2-p5-humanoid-armor" or promotion_actor:
        operators.append("P5ArmorActor")
    config = artifact / "server.toml"
    view_distance, simulation_distance = 4, 4
    if ctx.scenario_id == "native-v2-radius-full-view":
        # G5 radius split: full client view square with a smaller sim radius.
        view_distance, simulation_distance = 8, 4
    # The towns gate tests plugin protection with non-operators, not spawn protection.
    town_spawn_setting = "spawn_protection = 0\n" if ctx.scenario_id == "java-v2-p5-towns" else ""
    auth_setting = "require_plugin_login = true\n" if auth_scenario else ""
    config.write_text(f'''[server]
name = "shared-{kind}-probe"
motd = "Shared Python MCP gameplay probe"
view_distance = {view_distance}
simulation_distance = {simulation_distance}
{town_spawn_setting}[network]
bind_address = "127.0.0.1"
port = {port}
[auth]
online_mode = false
{auth_setting}[plugins]
directory = {json.dumps(str(plugin_dir))}
[data]
world_dir = {world_dir_toml(artifact)}
vanilla_data_dir = {json.dumps(os.environ.get("SOLARIS_SERVER_VANILLA_DATA_DIR", "data/vanilla"))}
seed = 81
''')
    prepare_operator_fixture(ctx, operators)
    if ctx.scenario_id == "native-v2-p2-optional-pack":
        with config.open("a") as output:
            output.write('\n[resource_pack]\nurl = "http://127.0.0.1:9/promotion-pack.zip"\nrequired = false\n')
    with binary.open("rb") as executable:
        digest = hashlib.file_digest(executable, "sha256").hexdigest()
    evidence = ctx.client_evidence
    evidence.update(server_address=address, server_binary_path=str(binary), server_binary_sha256=digest,
                    world_dir=str(artifact / "world"), seed=81, operators=operators,
                    probe_scope=kind if kind != "core-client" else "server-chat-only subset",
                    peak_client_rss_kib=None, client_rss_source="Linux /proc/PID/status VmHWM",
                    client_cpu=None, client_cpu_source="Linux /proc/PID/stat utime+stime, starttime")
    if check:
        return {"mode": "prepared", "client_evidence": evidence}
    server = client_process = xvfb = mcp = wire_trace = None
    promotion_receipt = None
    peer_process = peer_mcp = peer_log = None
    peer_evidence: dict[str, Any] = {}
    server_log = client_log = display_log = None
    try:
        server_log_path = artifact / "server.log"
        server_log = server_log_path.open("wb")
        server = subprocess.Popen([str(binary), "--config", str(config), "--no-console"],
                                  cwd=runtime.REPO_ROOT, stdout=server_log,
                                  stderr=subprocess.STDOUT, start_new_session=True)
        ctx.logs.append(str(server_log_path))
        runtime.wait_port(port, startup_timeout, server, log_path=server_log_path)
        if ctx.scenario_id in ("native-v2-shift-double-click", "native-v2-shift-chest-correction",
                               "java-v2-p5-shift-double-click"):
            from .shift_correction_wire import CorrectionWire

            data = Path(os.environ["SOLARIS_SERVER_VANILLA_DATA_DIR"])
            report = (data.parent / "26.1.2" if ctx.client == "java" else data) / "reports" / "packets.json"
            wire_trace = CorrectionWire(port, artifact, report)
            evidence["correction_wire_trace"] = str(wire_trace.receipt_path)
        if ctx.client == "native":
            display = None
            client_address = wire_trace.address if wire_trace is not None else address
            if (ctx.scenario_id == "native-v2-p1-signs" and os.environ.get("SOLARIS_P1_SIGN_WIRE_TRACE") == "1"
                    or ctx.scenario_id == "native-v2-p2-fishing"
                    or ctx.scenario_id in ("native-v2-p1-armor-stand", "native-v2-p1-helmet-equip",
                                          "native-v2-p1-player-observer")):
                from .native_p1 import SignWireTrace

                wire_trace = SignWireTrace(port, artifact, fishing=ctx.scenario_id == "native-v2-p2-fishing")
                client_address = wire_trace.address
                evidence["native_wire_trace"] = str(wire_trace.path)
            elif ctx.scenario_id in (native_light.SCENARIO_ID, "native-v2-radius-full-view"):
                wire_trace = light_wire.LightWireProxy(port, artifact)
                client_address = wire_trace.address
                evidence["native_wire_trace"] = str(wire_trace.path)
            elif ctx.scenario_id == "native-v2-p2-optional-pack":
                from .native_promotion_custody import PackWireReceipt

                pack_receipt = PackWireReceipt(
                    Path(os.environ["SOLARIS_SERVER_VANILLA_DATA_DIR"]) / "reports" / "packets.json", artifact)
                wire_trace = light_wire.LightWireProxy(
                    port, artifact, pack_receipt.server_packet, pack_receipt.client_packet)
                client_address = wire_trace.address
                evidence["native_wire_trace"] = str(pack_receipt.path)
            elif ctx.scenario_id == "native-v2-p4-damage":
                from .native_p4 import DamageWireReceipt

                report_path = Path(os.environ["SOLARIS_SERVER_VANILLA_DATA_DIR"]) / "reports" / "packets.json"
                damage_receipt = DamageWireReceipt(report_path)
                wire_trace = light_wire.LightWireProxy(port, artifact, damage_receipt.server_packet, decode_light=False)
                client_address = wire_trace.address
                evidence["native_wire_trace"] = str(wire_trace.path)
            elif promotion_maps or ctx.scenario_id == "native-v2-banner-placement":
                from .native_promotion_wire import PromotionWireReceipt

                report_path = Path(os.environ["SOLARIS_SERVER_VANILLA_DATA_DIR"]) / "reports" / "packets.json"
                promotion_receipt = PromotionWireReceipt(report_path, artifact)
                wire_trace = light_wire.LightWireProxy(port, artifact, promotion_receipt.server_packet)
                client_address = wire_trace.address
                evidence["native_wire_trace"] = str(wire_trace.path)
            if ctx.windowed:
                xvfb, display, display_log = runtime.start_xvfb(artifact)
                ctx.logs.append(str(artifact / "xvfb.log"))
            client_process, client_log, mcp = native_client.start(
                ctx, native_client.resolve_binary(ctx), client_address, username, startup_timeout,
                display=display)
            if promotion_maps or ctx.scenario_id == "native-v2-banner-placement":
                def promotion_snapshot() -> dict[str, Any]:
                    if wire_trace.errors:
                        raise RuntimeError(f"promotion wire errors: {wire_trace.errors}")
                    return wire_trace.query(promotion_receipt.snapshot)

                mcp.promotion_wire_receipt = promotion_snapshot
            if ctx.scenario_id == "native-v2-p4-damage":
                def damage_snapshot() -> list[dict[str, Any]]:
                    if wire_trace.errors:
                        raise RuntimeError(f"damage wire errors: {wire_trace.errors}")
                    return wire_trace.query(damage_receipt.snapshot)

                mcp.p4_damage_wire = damage_snapshot
            if ctx.scenario_id == "native-v2-p2-fishing":
                mcp.p2_wire_trace = wire_trace
            if ctx.scenario_id in (native_light.SCENARIO_ID, "native-v2-radius-full-view"):
                mcp.light_wire = wire_trace
            if ctx.scenario_id == "native-v2-p2-optional-pack":
                def single_connection_pack_receipt() -> dict[str, Any]:
                    if wire_trace.errors:
                        raise RuntimeError(f"optional pack wire errors: {wire_trace.errors}")
                    return pack_receipt.receipt()

                mcp.promotion_pack_receipt = single_connection_pack_receipt
            if ctx.scenario_id == "native-v2-p1-player-observer":
                peer_ctx = replace(ctx, artifact_dir=artifact / "observer-client", client_evidence=peer_evidence)
                evidence["observer_client"] = peer_evidence
                peer_process, peer_log, peer_mcp = native_client.start(
                    peer_ctx, native_client.resolve_binary(peer_ctx), client_address, "HarnessObserver",
                    startup_timeout, display=display)
                peer_mcp.request_timeout_seconds = 130.0
                peer_mcp.call_tool("minecraft_wait_for_play", {"timeout_seconds": startup_timeout})
                mcp.p1_observer = (peer_mcp, display, client_process.pid, peer_process.pid)
                mcp.p1_wire_trace = wire_trace
                from . import native_focus

                native_focus.acquire(display, client_process.pid, startup_timeout)
            if ctx.scenario_id == "native-v2-p5-humanoid-armor" or promotion_actor:
                peer_ctx = replace(ctx, artifact_dir=artifact / "headless-actor",
                                   client_evidence=peer_evidence, windowed=False)
                evidence["headless_armor_actor"] = peer_evidence
                peer_process, peer_log, peer_mcp = native_client.start(
                    peer_ctx, native_client.resolve_binary(peer_ctx), address, "P5ArmorActor",
                    startup_timeout)
                peer_mcp.request_timeout_seconds = 130.0
                peer_mcp.call_tool("minecraft_wait_for_play", {"timeout_seconds": startup_timeout})
                mcp.p5_armor_actor = peer_mcp
            if ctx.scenario_id == "native-v2-destroyed-chest-race":
                # Shared (multi-world) command execution only exists while another dimension
                # is populated: a second, headless client waits in the Nether for the scenario.
                peer_ctx = replace(ctx, artifact_dir=artifact / "nether-peer",
                                   client_evidence=peer_evidence, windowed=False)
                evidence["nether_peer_client"] = peer_evidence
                peer_process, peer_log, peer_mcp = native_client.start(
                    peer_ctx, native_client.resolve_binary(peer_ctx), address, "NetherPeer",
                    startup_timeout)
                peer_mcp.request_timeout_seconds = 130.0
                peer_mcp.call_tool("minecraft_wait_for_play", {"timeout_seconds": startup_timeout})
                mcp.nether_peer = peer_mcp
            if ctx.scenario_id == "native-v2-duplicate-login":
                def start_second_login() -> Any:
                    # Same username, hence the same offline identity, on a second client.
                    nonlocal peer_process, peer_log, peer_mcp
                    peer_ctx = replace(ctx, artifact_dir=artifact / "second-login",
                                       client_evidence=peer_evidence, windowed=False)
                    evidence["second_login_client"] = peer_evidence
                    peer_process, peer_log, peer_mcp = native_client.start(
                        peer_ctx, native_client.resolve_binary(peer_ctx), client_address, username,
                        startup_timeout)
                    peer_mcp.request_timeout_seconds = 130.0
                    return peer_mcp

                mcp.start_second_login = start_second_login
                mcp.server_address = client_address
            if ctx.scenario_id == "native-v2-settlement-storage":
                # Deliberately not an operator: the settlement's view-only visitor. Server
                # dialog buttons need presentation (minecraft_click_screen_button), so the
                # guest shares the owner's display; the owner window is then raised and
                # refocused so XTest input and the owner's keys never reach the guest. The
                # scenario hands X focus to the guest (by pid) only for its own legs.
                peer_ctx = replace(ctx, artifact_dir=artifact / "guest-client", client_evidence=peer_evidence)
                evidence["storage_guest_client"] = peer_evidence
                peer_process, peer_log, peer_mcp = native_client.start(
                    peer_ctx, native_client.resolve_binary(peer_ctx), client_address, "StorageGuest",
                    startup_timeout, display=display)
                peer_mcp.request_timeout_seconds = 130.0
                peer_mcp.call_tool("minecraft_wait_for_play", {"timeout_seconds": startup_timeout})
                mcp.storage_guest = (peer_mcp, peer_process.pid)
                if ctx.windowed:
                    from . import native_focus

                    evidence["window_focus"] = native_focus.acquire(display, client_process.pid, startup_timeout)
        else:
            client_address = wire_trace.address if wire_trace is not None else address
            if protocol_pot:
                report = Path(os.environ["SOLARIS_SERVER_VANILLA_DATA_DIR"]).parent / "26.1.2/reports/packets.json"
                wire_trace = light_wire.LightWireProxy(
                    port, artifact, client_packet_observer=java_protocol_pot.creative_packet_observer(report, artifact),
                    decode_light=False)
                client_address = wire_trace.address
                evidence["creative_wire_trace"] = str(artifact / "creative-wire.json")
            xvfb, display, display_log = runtime.start_xvfb(artifact)
            token, mcp_port = secrets.token_urlsafe(32), runtime.reserve_port()
            if p5_scenario or protocol_pot:
                game_dir = artifact / "game"
                game_dir.mkdir(parents=True, exist_ok=True)
                (game_dir / "options.txt").write_text(
                    "version:4790\nonboardAccessibility:false\nguiScale:2\nrenderDistance:4\n"
                    "simulationDistance:5\nmaxFps:30\npauseOnLostFocus:false\n")
            client_process, client_log = runtime.start_client(
                game_dir=artifact / "game", log_path=artifact / "client.log", token=token,
                mcp_port=mcp_port, username=username, display=display,
                client_profile="vanilla" if ctx.scenario_id == "java-v2-p5-shift-double-click" else None,
                window_size=(640, 480) if ctx.scenario_id == "java-v2-p5-shift-double-click" else None)
            if ctx.scenario_id == "java-v2-p5-shift-double-click":
                agent_jar = runtime._agent_jar_path()
                with agent_jar.open("rb") as source:
                    evidence.update(java_client_profile="vanilla", java_version=os.environ.get("SOLARIS_JAVA_VERSION", "26.1.2"),
                                    java_agent_jar=str(agent_jar), java_agent_sha256=hashlib.file_digest(source, "sha256").hexdigest())
            mcp = McpClient(f"http://127.0.0.1:{mcp_port}/mcp", token, startup_timeout + 10)
            mcp.initialize()
            ctx.logs.extend([str(artifact / "client.log"), str(artifact / "xvfb.log")])
        mcp.request_timeout_seconds = 130.0
        if ctx.scenario_id in ("native-v2-shift-double-click", "native-v2-shift-chest-correction",
                               "java-v2-p5-shift-double-click"):
            mcp.correction_wire = wire_trace
        tools = mcp.list_tools()
        evidence["available_tools"] = sorted(tool["name"] for tool in tools)
        require_tools(tools, required | {"minecraft_wait_for_play", "minecraft_disconnect"}, kind)
        if ctx.client == "java":
            require_tools(tools, {"minecraft_connect"}, kind)
            runtime.wait_client_ready_for_connect(mcp, startup_timeout)
            mcp.call_tool("minecraft_connect", {"server_addr": client_address})
        play = mcp.call_tool("minecraft_wait_for_play", {"timeout_seconds": startup_timeout})
        if not play.get("in_play"):
            raise RuntimeError(f"{kind} client did not enter Play: {play}")
        if p5_scenario:
            def restart_p5_server(remaining: float) -> str:
                nonlocal server, server_log
                old_pid = server.pid
                runtime.stop_process(server)
                if server.returncode != 0:
                    raise RuntimeError(f"P5 server did not stop cleanly: {server.returncode}")
                server_log.close()
                server_log = server_log_path.open("ab")
                server = subprocess.Popen([str(binary), "--config", str(config), "--no-console"],
                                          cwd=runtime.REPO_ROOT, stdout=server_log,
                                          stderr=subprocess.STDOUT, start_new_session=True)
                runtime.wait_port(port, remaining, server, log_path=server_log_path)
                evidence.setdefault("server_restarts", []).append({
                    "old_pid": old_pid, "new_pid": server.pid,
                    "same_world_dir": str(artifact / "world"), "same_binary_sha256": digest,
                })
                return address

            def attach_p5_client() -> None:
                mcp.p4_restart_server = restart_p5_server
                mcp.p5_switch_identity = switch_p5_identity
                mcp.p4_window = java_p5.acquire_window(display, startup_timeout)
                evidence.setdefault("java_windows", []).append(mcp.p4_window)

            def switch_p5_identity(name: str, remaining: float) -> Any:
                nonlocal mcp, client_process, client_log
                mcp.call_tool("minecraft_disconnect")
                mcp.close()
                runtime.stop_process(client_process)
                old_returncode = client_process.returncode
                client_log.close()
                session_dir = artifact / ("identity-" + name)
                game_dir = session_dir / "game"
                game_dir.mkdir(parents=True, exist_ok=True)
                (game_dir / "options.txt").write_text(
                    "version:4790\nonboardAccessibility:false\nguiScale:2\nrenderDistance:4\n"
                    "simulationDistance:5\nmaxFps:30\npauseOnLostFocus:false\n")
                token, mcp_port = secrets.token_urlsafe(32), runtime.reserve_port()
                log_path = session_dir / "client.log"
                client_process, client_log = runtime.start_client(
                    game_dir=game_dir, log_path=log_path, token=token, mcp_port=mcp_port,
                    username=name, display=display)
                ctx.logs.append(str(log_path))
                mcp = McpClient(f"http://127.0.0.1:{mcp_port}/mcp", token, remaining + 10)
                mcp.initialize()
                mcp.request_timeout_seconds = 130.0
                require_tools(mcp.list_tools(), required | {"minecraft_connect", "minecraft_wait_for_play"}, kind)
                runtime.wait_client_ready_for_connect(mcp, remaining)
                mcp.call_tool("minecraft_connect", {"server_addr": address})
                entered = mcp.call_tool("minecraft_wait_for_play", {"timeout_seconds": remaining})
                if entered.get("in_play") is not True:
                    raise RuntimeError(f"P5 identity {name} did not enter Play: {entered}")
                evidence.setdefault("identity_switches", []).append({
                    "username": name, "previous_launcher_exit": old_returncode,
                    "new_launcher_pid": client_process.pid, "previous_stopped_before_next_start": True,
                })
                attach_p5_client()
                return mcp

            attach_p5_client()
        if protocol_pot:
            mcp.p4_window = java_p5.acquire_window(display, startup_timeout)
            evidence["java_window"] = mcp.p4_window
            result = java_protocol_pot.run(mcp, artifact, timeout)
        elif kind == "inventory":
            with ProbeSteps(mcp, artifact / "inventory-fixture", startup_timeout, "inventory-operator-fixture") as setup:
                setup.fixture("debug give minecraft:apple 2 1")
                setup.call("minecraft_wait_for_inventory", {
                    "item_id": "minecraft:apple", "count": 2, "timeout_seconds": setup.remaining(),
                })
            result = compatibility.run_inventory_probe(mcp, artifact, timeout)
        elif auth_scenario:
            result = native_auth_budget.run(mcp, artifact, timeout, address, server.pid)
        elif v2_scenario:
            from . import native_p4
            mcp.p4_window = evidence.get("window_focus")
            mcp.server_address = client_address

            mcp.server_log_path = server_log_path
            if (ctx.scenario_id in native_p4.RESTART_SCENARIOS
                    or ctx.scenario_id in (native_light.SCENARIO_ID, "native-v2-end-restart",
                                           "native-v2-custody-mount-export",
                                           "native-v2-corrupt-block-entities",
                                           "native-v2-corrupt-chunk-near-camera")):
                def restart_server(remaining: float) -> str:
                    nonlocal server, server_log
                    old_pid = server.pid
                    runtime.stop_process(server)
                    if server.returncode != 0:
                        raise RuntimeError(f"P4 server did not stop cleanly: {server.returncode}")
                    server_log.close()
                    if (ctx.scenario_id == "native-v2-custody-mount-export"
                            or (ctx.scenario_id in ("native-v2-corrupt-block-entities",
                                                    "native-v2-corrupt-chunk-near-camera")
                                and getattr(mcp, "corrupt_fixture", None))):
                        from .native_promotion_custody import convert_world

                        convert_world(ctx, binary, config, evidence,
                                      corrupt=getattr(mcp, "corrupt_fixture", None))
                    server_log = server_log_path.open("ab")
                    server = subprocess.Popen([str(binary), "--config", str(config), "--no-console"],
                                              cwd=runtime.REPO_ROOT, stdout=server_log,
                                              stderr=subprocess.STDOUT, start_new_session=True)
                    runtime.wait_port(port, remaining, server, log_path=server_log_path)
                    evidence.setdefault("server_restarts", []).append({
                        "old_pid": old_pid, "new_pid": server.pid, "world_dir": evidence["world_dir"],
                        "same_binary_sha256": digest,
                    })
                    return address

                mcp.p4_restart_server = restart_server
                if ctx.scenario_id == "native-v2-custody-mount-export":
                    def export_reimport(remaining: float) -> dict[str, str]:
                        address = restart_server(remaining)
                        return {"address": address, **evidence["world_conversions"][-1]}

                    mcp.promotion_mount_export_reimport = export_reimport
            result = native_v2.run(mcp, artifact, timeout, ctx.scenario_id)
        elif p5_scenario:
            from . import java_craft_drag, shift_matrix

            runners = {
                "java-v2-p5-towns": java_p5.run_towns,
                "java-v2-p5-parity": java_p5.run_parity,
                "java-v2-p5-cursor": java_p5.run_cursor,
                "java-v2-p5-craft-drag": java_craft_drag.run,
                "java-v2-p5-craft-drag-paced": java_craft_drag.run_paced,
                "java-v2-p5-shift-double-click": shift_matrix.run_java,
            }
            result = runners[ctx.scenario_id](mcp, artifact, timeout)
        elif kind == "entities":
            result = entities_probe.run(mcp, artifact / "entities-probe", timeout)
        elif kind == "native-lifecycle":
            result = native_lifecycle.run(mcp, artifact, timeout, address, client_process.pid, soak_seconds)
        else:
            result = run_core_subset(mcp, artifact, timeout)
        if result.get("passed") is not True:
            raise RuntimeError(f"{kind} probe failed: {result}")
        if ctx.windowed and ctx.client == "native":
            evidence["capture"] = native_client.capture(mcp, artifact)
        if ctx.client == "native":
            rss = peak_rss_kib(client_process.pid)
            if rss is None:
                raise RuntimeError("native client peak RSS unavailable after probe")
            evidence["peak_client_rss_kib"] = rss
        mcp.call_tool("minecraft_disconnect")
        return {"scenario": result}
    finally:
        if ctx.client == "native" and client_process is not None:
            rss, cpu = peak_rss_kib(client_process.pid), cpu_usage(client_process.pid)
            if rss is not None:
                evidence["peak_client_rss_kib"] = rss
            if cpu is not None:
                evidence["client_cpu"] = cpu
        if peer_mcp is not None:
            try:
                peer_mcp.close()
            except Exception:
                pass
        runtime.stop_process(peer_process)
        if peer_process is not None:
            ctx.exits[peer_evidence["command_index"]] = peer_process.returncode
        if mcp is not None:
            try:
                mcp.close()
            except Exception:
                pass
        runtime.stop_process(client_process)
        if ctx.client == "native" and client_process is not None:
            ctx.exits[evidence["command_index"]] = client_process.returncode
        if wire_trace is not None:
            wire_trace.close()
        runtime.stop_process(server, interrupt=True)
        runtime.stop_process(xvfb)
        for log in (server_log, client_log, peer_log, display_log):
            if log is not None:
                log.close()
