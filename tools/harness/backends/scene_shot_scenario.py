"""Scene-shot profile entry: the scenario file contract and the run that launches the
client, executes the steps and keeps the frames.

The client/report seams live on ``scene_shot`` (the tests patch them there), so this
module reaches them as ``_qa.<name>`` and never binds them.
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

from .. import runtime
from ..profiles import ProfileContext
from . import native_client
from .gameplay import prepare_operator_fixture
from .probe_steps import ProbeSteps
from . import scene_shot as _qa
from .scene_shot_checks import (
    check_263_entity_rigs, check_decorated_pot, check_dynamic_context, check_elytra, check_entity_visuals,
    check_gait_spider, check_item_bars, check_placed_head, check_player_head, check_player_head_registry,
    check_shade_override, check_shelf_clock, check_toasts,
    player_head_skin_pack,
)
from .scene_shot_client import (
    LatencyProxy, NATIVE_FOV, NATIVE_GUI_SCALE, WINDOW, _validate_java_display,
    check_java_render_distance, find_java_window, scene_username, stage_plugins, start_java,
    wait_play,
)
from .scene_shot_execute import execute
from .scene_shot_steps import validate_steps


def validate_scenario(ctx: ProfileContext, scenario: dict[str, Any] | None = None):
    """Load and validate exactly the inputs used by the scene runner."""
    if ctx.client not in ("native", "java") or not ctx.windowed or ctx.platform is not None:
        raise ValueError("scene-shot requires --client native|java --windowed")
    named_fixtures = {
        "java-matrix-system-chat": "java_matrix_system_chat_scene.json",
        "native-263-entity-rigs": "entity_263_rigs_scene.json",
        "native-shelf-clock": "shelf_clock_scene.json",
        "native-entity-visuals": "entity_visuals_scene.json",
        "native-elytra": "elytra_scene.json",
        "native-toast": "toast_scene.json",
        "native-item-bars": "item_bars_scene.json",
        "native-shade-override": "shade_override_scene.json",
        "native-decorated-pot": "decorated_pot_scene.json",
        "native-player-head": "player_head_scene.json",
        "native-player-head-registry": "player_head_registry_scene.json",
        "native-placed-head": "player_head_scene.json",
        "native-dynamic-context": "dynamic_context_scene.json",
        "native-gait-spider": "gait_phase_scene.json",
        "native-death-tilt": "death_tilt_scene.json",
        "native-command-feedback": "command_feedback_scene.json",
        "native-low-roof-house": "low_roof_house_scene.json",
        "native-falling-block-prediction": "falling_block_scene.json",
        "native-frozen-game-age": "frozen_game_age_scene.json",
        "native-unloaded-eye": "unloaded_eye_scene.json",
        "native-village-idle-stroll": "village_idle_stroll_scene.json",
        "native-village-night-door": "village_night_door_scene.json",
        "native-village-late-job": "village_late_job_scene.json",
        "native-village-distant-job": "village_distant_job_scene.json",
        "native-village-deep-water": "village_deep_water_scene.json",
        "native-village-mob-doors": "village_mob_doors_scene.json",
        "native-village-after-hours-job": "village_after_hours_job_scene.json",
        "native-village-breeding": "village_breeding_scene.json",
        "native-village-moved-workstation": "village_moved_workstation_scene.json",
        "native-village-occupied-bed": "village_occupied_bed_scene.json",
        "native-village-open-air-home": "village_open_air_home_scene.json",
        "native-village-stair-penetration": "village_stair_penetration_scene.json",
        "native-village-exit-foliage": "village_exit_foliage_scene.json",
        "native-kelp-ocean-floor": "kelp_ocean_floor_scene.json",
        "native-settlement-radius-growth": "settlement_radius_scene.json",
        "native-settlement-buildings-upgrade": "settlement_buildings_scene.json",
        "issue1-01-villager-leg": "issue1_01_villager_leg_scene.json",
        "issue1-18-sheep-goat": "issue1_18_sheep_goat_scene.json",
        "issue1-19-hostile-motion": "issue1_19_hostile_motion_scene.json",
    }
    if ctx.scenario_id == "java-gait-sidecar":
        if ctx.client != "java" or scenario is not None or ctx.extra_args or os.environ.get("SCENE_SHOT_SCENARIO"):
            raise ValueError("java-gait-sidecar requires its registered Java fixture without overrides")
        scenario = json.loads((runtime.REPO_ROOT / "tools/harness/gait_phase_scene.json").read_text())
        scenario["java_plain"] = True
    if ctx.scenario_id in named_fixtures:
        client = "java" if ctx.scenario_id.startswith("java-") else "native"
        if ctx.client != client or scenario is not None or ctx.extra_args or os.environ.get("SCENE_SHOT_SCENARIO"):
            raise ValueError(f"{ctx.scenario_id} requires its registered {client} fixture without overrides")
        scenario = json.loads((runtime.REPO_ROOT / "tools/harness" / named_fixtures[ctx.scenario_id]).read_text())
    elif scenario is None:
        args = ctx.extra_args
        if args and args[0] == "--":
            args = args[1:]
        if len(args) > 1:
            raise ValueError("scene-shot accepts one scenario JSON path")
        scenario_path = args[0] if args else os.environ.get("SCENE_SHOT_SCENARIO")
        if not scenario_path:
            raise ValueError("set SCENE_SHOT_SCENARIO or pass a scenario JSON path")
        scenario = json.loads(Path(scenario_path).read_text())
    if not isinstance(scenario, dict) or not isinstance(scenario.get("steps"), list):
        raise ValueError("scenario must contain a steps array")
    if type(scenario.get("java_plain", False)) is not bool or (scenario.get("java_plain") and ctx.client != "java"):
        raise ValueError("java_plain requires --client java and a boolean value")
    if type(scenario.get("java_hide_chat", False)) is not bool or (scenario.get("java_hide_chat") and ctx.client != "java"):
        raise ValueError("java_hide_chat requires --client java and a boolean value")
    if scenario.get("vanilla_singleplayer") is True:
        if ctx.client != "java":
            raise ValueError("vanilla singleplayer scenes require --client java")
        from . import integrated_scene

        return integrated_scene.run(ctx, scenario)
    seed = scenario.get("seed", 100)
    if type(seed) is not int:
        raise ValueError("seed must be an integer")
    server_view = scenario.get("server_view_distance", 4)
    if type(server_view) is not int or not 2 <= server_view <= 32:
        raise ValueError("server_view_distance must be in 2..32")
    spawn_protection = scenario.get("spawn_protection", 0)
    if type(spawn_protection) is not int or spawn_protection < 0:
        raise ValueError("spawn_protection must be a non-negative integer")
    operator = scenario.get("operator", True)
    if type(operator) is not bool:
        raise ValueError("operator must be true or false")
    if scenario.get("operator_level", 4) not in (1, 2, 3, 4):
        raise ValueError("operator_level must be 1..4")
    if type(scenario.get("server_console", False)) is not bool:
        raise ValueError("server_console must be true or false")
    if "lod_enabled" in scenario and type(scenario["lod_enabled"]) is not bool:
        raise ValueError("lod_enabled must be true or false")
    if "lod_share_terrain_keys" in scenario:
        if type(scenario["lod_share_terrain_keys"]) is not bool or scenario.get("lod_enabled") is not True:
            raise ValueError("lod_share_terrain_keys requires lod_enabled=true and a boolean value")
    username = scene_username(scenario)
    client_latency_ms = scenario.get("client_latency_ms", 0)
    if type(client_latency_ms) is not int or not 0 <= client_latency_ms <= 5000:
        raise ValueError("client_latency_ms must be an integer in 0..5000")
    settings = scenario.get("client_settings", {})
    if not isinstance(settings, dict) or set(settings) - {"graphics_preset", "render_distance", "fov", "gui_scale", "language", "screen_size"}:
        raise ValueError("unsupported client settings")
    if {"gui_scale", "screen_size"} & settings.keys() and ctx.client != "java":
        raise ValueError("gui_scale and screen_size are Java-only scene settings")
    if {"gui_scale", "language", "screen_size"} & settings.keys():
        # The language reaches the native client as KRASNOKAMEN_CLIENT_LANGUAGE, the Java client
        # through options.txt; both accept exactly the pinned locales.
        _validate_java_display(settings.get("gui_scale", NATIVE_GUI_SCALE), settings.get("language", "en_us"),
                               settings.get("screen_size", WINDOW))
    if "fov" in settings and (type(settings["fov"]) is not int or not 30 <= settings["fov"] <= 110):
        raise ValueError("fov must be an integer in 30..110")
    if "graphics_preset" in settings:
        preset = settings["graphics_preset"]
        if not isinstance(preset, str) or preset.lower() not in ("low", "medium", "high"):
            raise ValueError("graphics_preset must be Low, Medium or High")
        settings["graphics_preset"] = preset.lower()
    if "render_distance" in settings and (type(settings["render_distance"]) is not int or not 2 <= settings["render_distance"] <= 32):
        raise ValueError("render_distance must be in 2..32")
    validate_steps(scenario["steps"], ctx.client)
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else 600.0
    if not 0 < timeout <= 3600:
        raise ValueError("timeout must be in (0, 3600]")
    if ctx.client == "java" and "graphics_preset" in settings:
        raise ValueError("graphics_preset is a native-client setting; the Java client keeps its default")
    return scenario, seed, server_view, spawn_protection, operator, username, client_latency_ms, settings, timeout


def run(
    ctx: ProfileContext,
    scenario: dict[str, Any] | None = None,
    script: Callable[[ProbeSteps, Path, list[dict[str, Any]], str], list[str]] = execute,
) -> dict[str, Any]:
    """Run one scene; other profiles can supply a scenario and Python script."""
    (scenario, seed, server_view, spawn_protection, operator, username,
     client_latency_ms, settings, timeout) = validate_scenario(ctx, scenario)
    binary_text = os.environ.get("KRASNOKAMEN_SERVER_BINARY")
    if not binary_text:
        raise RuntimeError("scene-shot requires KRASNOKAMEN_SERVER_BINARY")
    binary = Path(binary_text).expanduser().resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise RuntimeError(f"missing executable server binary: {binary}")
    artifact = ctx.artifact_dir
    port = runtime.reserve_port()
    config = artifact / "scene-shot.toml"
    source = (runtime.REPO_ROOT / "playable.toml").read_text()
    # BREAK_CMD_BLOCK breaker glue (not for merge): vanilla enable-command-block for command-block feedback.
    command_block = "\nenable_command_block = true" if os.environ.get("BREAK_CMD_BLOCK") else ""
    replacements = (("[server]", f"[server]\nspawn_protection = {spawn_protection}{command_block}"),
                    ("view_distance = 4", f"view_distance = {server_view}"),
                    ("port = 25565", f"port = {port}"),
                    ('world_dir = ".analysis/test-world-v12"', f'world_dir = {json.dumps(str(artifact / "world"))}'),
                    ('directory = "plugins"', f'directory = {json.dumps(str(artifact / "plugins"))}'),
                    ("seed = 0", f"seed = {seed}"))
    for old, new in replacements:
        if source.count(old) != 1:
            raise RuntimeError(f"expected one config token: {old}")
        source = source.replace(old, new, 1)
    if "lod_enabled" in scenario:
        if "[lod]" in source:
            raise RuntimeError("playable.toml already contains a lod section")
        source += f'\n[lod]\nenabled = {str(scenario["lod_enabled"]).lower()}\n'
        if "lod_share_terrain_keys" in scenario:
            source += f'share_terrain_keys = {str(scenario["lod_share_terrain_keys"]).lower()}\n'
    config.write_text(source)
    if scenario.get("world"):
        shutil.copytree(Path(scenario["world"]).expanduser().resolve(), artifact / "world")
    stage_plugins(ctx.plugins_dir, artifact / "plugins")
    if operator:
        _qa.prepare_operator_fixture(ctx, [username], scenario.get("operator_level", 4))
    xdg = artifact / "xdg"
    settings_path = xdg / "krasnokamen-client" / "client-settings.json"
    settings_path.parent.mkdir(parents=True)
    if ctx.client == "native":
        from ..preflight import CLIENT_CONTENT_DIRS, client_template

        # Link the read-only content of the data root the env names (the one
        # preflight checked). Settings are written below; caches (far-terrain),
        # saved runs and window state must not leak between runs.
        source_root = client_template()
        for name in CLIENT_CONTENT_DIRS:
            entry = source_root / name
            if entry.exists():
                (settings_path.parent / name).symlink_to(entry.resolve(), target_is_directory=entry.is_dir())
    # Only this scene's own settings reach the client: an inherited file (the operator's
    # environment or their data root) would make a reference comparison depend on the
    # machine, and a client test that saved a rebinding would silently change the gate.
    # The native client takes its language from KRASNOKAMEN_CLIENT_LANGUAGE (below), not the file.
    base = {"fov": NATIVE_FOV, **{key: value for key, value in settings.items()
                                  if not (key == "language" and ctx.client != "java")}}
    if ctx.client == "java" and "graphics_preset" in settings:
        raise ValueError("graphics_preset is a native-client setting; the Java client keeps its default")
    settings_path.write_text(json.dumps(base, indent=2) + "\n")
    server = client = xvfb = mcp = log = client_log = proxy = probe = None
    server_index = len(ctx.commands)
    # `server_console`: the server reads operator commands from RUN_DIR/console.fifo (the battle runner's
    # mechanism, `battle_recovery.FifoConsole`), so privileged fixtures never depend on the player's own op level.
    console_fd = None
    if scenario.get("server_console", False):
        os.mkfifo(artifact / "console.fifo")
        console_fd = os.open(artifact / "console.fifo", os.O_RDWR)  # read+write: never blocks, never sees EOF
    server_command = [str(binary), "--config", str(config)] + ([] if console_fd is not None else ["--no-console"])
    ctx.commands.append(server_command)
    ctx.cwds.append(str(runtime.REPO_ROOT))
    ctx.exits.append(None)
    try:
        (artifact / "lifecycle-progress.json").write_text(json.dumps({"phase": "server_start", "timeout_seconds": timeout, "started_unix": time.time()}) + "\n")
        log = (artifact / "server.log").open("wb")
        ctx.logs.append(str(artifact / "server.log"))
        with binary.open("rb") as server_executable:
            server_hash = hashlib.file_digest(server_executable, "sha256").hexdigest()
        ctx.client_evidence.update(server_binary_path=str(binary), server_binary_sha256=server_hash)
        server = runtime.start_server(server_command, cwd=runtime.REPO_ROOT, stdin=console_fd, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        runtime.wait_port(port, timeout, server, log_path=artifact / "server.log")
        client_port = port
        (artifact / "lifecycle-progress.json").write_text(json.dumps({"phase": "proxy_start", "timeout_seconds": 8, "started_unix": time.time()}) + "\n")
        if ctx.scenario_id == "native-unloaded-eye":
            from .unloaded_eye import UnloadedEyeProxy, execute as unloaded_eye_script

            proxy = UnloadedEyeProxy(port, artifact)
            client_port = proxy.start()
            script = unloaded_eye_script
        elif client_latency_ms or any(step.get("server_eof") is True for step in scenario["steps"]):
            proxy = LatencyProxy(port, client_latency_ms)
            client_port = proxy.start()
        xvfb, display, _ = runtime.start_xvfb(artifact, tuple(settings.get("screen_size", WINDOW)))
        ctx.logs.append(str(artifact / "xvfb.log"))
        old_xdg = os.environ.get("XDG_DATA_HOME")
        old_packs = os.environ.get("KRASNOKAMEN_CLIENT_RESOURCE_PACKS")
        old_language = os.environ.get("KRASNOKAMEN_CLIENT_LANGUAGE")
        try:
            os.environ["XDG_DATA_HOME"] = str(xdg)
            if ctx.client != "java" and "language" in settings:
                os.environ["KRASNOKAMEN_CLIENT_LANGUAGE"] = settings["language"]
            if ctx.scenario_id in ("native-player-head", "native-placed-head"):
                pack = player_head_skin_pack(artifact)
                os.environ["KRASNOKAMEN_CLIENT_RESOURCE_PACKS"] = os.pathsep.join(
                    [old_packs, str(pack)] if old_packs else [str(pack)])
                ctx.client_evidence["head_skin_pack"] = str(pack)
            if ctx.client == "java":
                client, client_log, mcp = start_java(
                    ctx, artifact, f"127.0.0.1:{client_port}", username, timeout, display,
                    base.get("render_distance", 12), base["fov"], plain=scenario.get("java_plain", False),
                    gui_scale=settings.get("gui_scale", NATIVE_GUI_SCALE), language=settings.get("language", "en_us"),
                    screen_size=tuple(settings.get("screen_size", WINDOW)),
                    hide_chat=scenario.get("java_hide_chat", False))
            else:
                client, client_log, mcp = native_client.start(ctx, native_client.resolve_binary(ctx), f"127.0.0.1:{client_port}", username, timeout, display=display, settings=settings_path)
        finally:
            for key, old in (("XDG_DATA_HOME", old_xdg),
                             ("KRASNOKAMEN_CLIENT_RESOURCE_PACKS", old_packs),
                             ("KRASNOKAMEN_CLIENT_LANGUAGE", old_language)):
                if old is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = old
        mcp.request_timeout_seconds = min(timeout + 10, 130.0)
        mcp.scene_address = f"127.0.0.1:{client_port}"
        mcp.scene_window_size = tuple(settings.get("screen_size", WINDOW))
        mcp.scene_proxy = proxy
        (artifact / "lifecycle-progress.json").write_text(json.dumps({"phase": "scenario", "timeout_seconds": timeout, "started_unix": time.time()}) + "\n")
        probe_id = ctx.scenario_id if ctx.scenario_id in (
            "native-unloaded-eye", "native-gait-spider", "java-gait-sidecar") else "scene-shot"
        with ProbeSteps(mcp, artifact / "scene-shot", timeout, probe_id) as probe:
            wait_play(probe)
            if ctx.client == "java":
                window_id = find_java_window(display)
                if window_id is None:
                    raise RuntimeError(f"Java client opened no window on the harness Xvfb {display}; refusing to run")
                ctx.client_evidence["window_focus"] = mcp.p4_window = {"display": display, "window_id": window_id}
                _qa.resize_window(mcp.p4_window, mcp.scene_window_size)
                probe.call("minecraft_wait_ticks", {"ticks": 10})
            else:
                from . import native_focus
                ctx.client_evidence["window_focus"] = mcp.p4_window = native_focus.acquire(display, client.pid, probe.remaining())
            before = probe.observe()
            probe.call("minecraft_wait_for_state_change", {
                "observed_version": before["state_version"], "timeout_seconds": probe.remaining(),
            })
            shots = script(probe, artifact, scenario["steps"], ctx.client)
            if ctx.scenario_id == "java-matrix-system-chat":
                from .java_system_chat import check
                check(probe, artifact)
            elif ctx.scenario_id == "native-263-entity-rigs":
                check_263_entity_rigs(probe, artifact)
            elif ctx.scenario_id == "native-shelf-clock":
                check_shelf_clock(probe, artifact)
            elif ctx.scenario_id == "native-elytra":
                check_elytra(probe, artifact)
            elif ctx.scenario_id == "native-toast":
                check_toasts(probe, artifact)
            elif ctx.scenario_id == "native-entity-visuals":
                check_entity_visuals(probe, artifact)
            elif ctx.scenario_id == "native-item-bars":
                check_item_bars(probe, artifact)
            elif ctx.scenario_id == "native-shade-override":
                check_shade_override(probe, artifact)
            elif ctx.scenario_id == "native-decorated-pot":
                check_decorated_pot(probe, artifact)
            elif ctx.scenario_id == "native-player-head":
                check_player_head(probe, artifact)
            elif ctx.scenario_id == "native-player-head-registry":
                check_player_head_registry(probe, artifact)
            elif ctx.scenario_id == "native-placed-head":
                check_placed_head(probe, artifact)
            elif ctx.scenario_id == "native-dynamic-context":
                check_dynamic_context(probe, artifact)
            elif ctx.scenario_id == "native-gait-spider":
                check_gait_spider(probe, artifact)
        result = {"screenshots": shots, "scenario": probe.report, "seed": seed, "settings": settings, "client_latency_ms": client_latency_ms}
    except Exception as error:
        # Keep the scenario's own report for the failure receipt: `assertions` says whether an
        # assertion failed, which a promotion control must tell apart from a timeout or a crash.
        if probe is not None:
            ctx.failure_scenario = {"scenario": dict(probe.report), "error": str(error)}
        raise
    finally:
        (artifact / "lifecycle-progress.json").write_text(json.dumps({"phase": "teardown", "started_unix": time.time()}) + "\n")
        (artifact / "teardown-progress.json").write_text(json.dumps({"component": "mcp", "timeout_seconds": getattr(mcp, "request_timeout_seconds", 0), "started_unix": time.time()}) + "\n")
        if mcp is not None:
            mcp.close()
        (artifact / "teardown-progress.json").write_text(json.dumps({"component": "client", "timeout_seconds": 13, "started_unix": time.time()}) + "\n")
        runtime.stop_process(client)
        if client is not None:
            ctx.exits[ctx.client_evidence["command_index"]] = client.returncode
        if client_log is not None:
            client_log.close()
        (artifact / "teardown-progress.json").write_text(json.dumps({"component": "proxy", "timeout_seconds": 8, "started_unix": time.time()}) + "\n")
        if proxy is not None:
            proxy.close()
        (artifact / "teardown-progress.json").write_text(json.dumps({"component": "server", "timeout_seconds": 13, "started_unix": time.time()}) + "\n")
        runtime.stop_process(server, interrupt=True)
        if server is not None:
            ctx.exits[server_index] = server.returncode
        if console_fd is not None:
            os.close(console_fd)
        (artifact / "teardown-progress.json").write_text(json.dumps({"component": "display", "timeout_seconds": 13, "started_unix": time.time()}) + "\n")
        runtime.stop_process(xvfb)
        if log is not None:
            log.close()
    if ctx.client == "java":
        check_java_render_distance(artifact / "game" / "options.txt", base.get("render_distance", 12))
    return result
