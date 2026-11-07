"""Graphical singleplayer recovery and world-lease regressions."""
from __future__ import annotations

import hashlib
import math
import os
from pathlib import Path
import select
import shutil
import signal
import subprocess
import time
from concurrent.futures import ThreadPoolExecutor

from .. import runtime
from ..profiles import ProfileContext
from . import native_client
from . import singleplayer_slow
from .title_connect import TITLE_CLASS, _TitleProbeSteps

SELECT = "net.minecraft.client.gui.screens.worldselection.SelectWorldScreen"
CREATE = "net.minecraft.client.gui.screens.worldselection.CreateWorldScreen"
FIRST = "native-world-first-start-interrupted"
LEASE = "native-sp-relaunch-lease-race"
SLOW = "native-sp-slow-machine"
WEAK = "native-sp-weak-hardware"


def _click(probe, state, label):
    probe.call("minecraft_click_screen_button", {
        "expected_screen_class": state["screen"]["class"],
        "expected_title": state["screen"]["title"], "button_label": label,
    })


def _server_child(client):
    children = set()
    for path in Path(f"/proc/{client.pid}/task").glob("*/children"):
        try:
            children.update(path.read_text().split())
        except FileNotFoundError:
            continue
    servers = []
    for child in children:
        try:
            if Path(os.readlink(f"/proc/{child}/exe")).name == "solaris-v2":
                servers.append(int(child))
        except FileNotFoundError:
            continue
    if len(servers) != 1:
        raise RuntimeError(f"expected one owned singleplayer server, got {servers}")
    return servers[0]


def _wait_path(path, timeout):
    if not path.parent.exists():
        _wait_path(path.parent, timeout)
    runtime.wait_for_path_condition(path, timeout, lambda: (path.exists(), None))


def _select_world(probe, state):
    probe.call("client_pointer", {"expected_screen_class": SELECT,
        "expected_title": state["screen"]["title"], "action": "click", "x": 250, "y": 140})


def _play(probe, state):
    _click(probe, state, "Play Selected World")
    result = probe.call("minecraft_wait_for_play", {"timeout_seconds": min(probe.remaining(), 120)})
    probe.check("entered_world", result.get("in_play") is True)
    probe.check("entered_overworld", result.get("dimension") == "minecraft:overworld")
    return result


def validate_args(ctx: ProfileContext) -> float:
    if ctx.client != "native" or not ctx.windowed:
        raise ValueError("singleplayer regressions require --client native --windowed")
    if ctx.scenario_id not in (FIRST, LEASE, SLOW, WEAK):
        raise ValueError("select a registered singleplayer regression scenario")
    if ctx.extra_args or ctx.platform:
        raise ValueError("singleplayer regressions do not accept extra arguments or platform")
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else 900.0
    if not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be positive")
    return timeout


def run(ctx: ProfileContext) -> dict:
    timeout = validate_args(ctx)
    server = Path(os.environ.get("SOLARIS_SERVER_BINARY", "")).resolve()
    if not server.is_file():
        raise RuntimeError("SOLARIS_SERVER_BINARY must name the release server")
    with server.open("rb") as executable:
        server_hash = hashlib.file_digest(executable, "sha256").hexdigest()
    source_root = native_client._client_root()
    client_binary = native_client.resolve_binary(ctx)
    artifact = ctx.artifact_dir
    stage = artifact / "bin"
    stage.mkdir()
    client_copy = stage / "solaris-client"
    shutil.copy2(client_binary, client_copy)
    server_link = stage / "solaris-v2"
    pidfile = unit = None
    if ctx.scenario_id in (SLOW, WEAK):
        pidfile, unit = singleplayer_slow.stage_server(server, stage, artifact, slow=ctx.scenario_id == SLOW)
    else:
        server_link.symlink_to(server)
    root = artifact / "data" / "solaris-client"
    content = root / "servers" / "content"
    content.mkdir(parents=True)
    # The server's sidecar validation rejects directory symlinks.
    shutil.copytree((runtime.REPO_ROOT / "data" / "vanilla").resolve(), content / "26.3")
    for name in ("home", "cache", "config"):
        (artifact / name).mkdir()
    for name in ("facts", "mojang", "resources", "assets", "data"):
        source = source_root / name
        if source.exists():
            (root / name).symlink_to(source, target_is_directory=source.is_dir())
    env = {"SOLARIS_CLIENT_ROOT": str(root), "XDG_DATA_HOME": str(root.parent),
           "HOME": str(artifact / "home"), "XDG_CACHE_HOME": str(artifact / "cache"),
           "XDG_CONFIG_HOME": str(artifact / "config"), "ALSOFT_DRIVERS": "null"}
    if ctx.scenario_id in (SLOW, WEAK):
        env.update({"LIBGL_ALWAYS_SOFTWARE": "1", "VK_ICD_FILENAMES": "/usr/share/vulkan/icd.d/lvp_icd.json",
                    "VK_DRIVER_FILES": "/usr/share/vulkan/icd.d/lvp_icd.json"})
    previous = {name: os.environ.get(name) for name in env}
    os.environ.update(env)
    ctx.client_evidence.update(server_binary_path=str(server), server_binary_sha256=server_hash)
    xvfb = client = mcp = None
    holder = None
    holder_fd = None
    world = None
    try:
        xvfb, display, _ = runtime.start_xvfb(artifact)
        ctx.logs.append(str(artifact / "xvfb.log"))
        client, _, mcp = native_client.start(ctx, client_copy, None, None, min(timeout, 180), display=display)
        mcp.request_timeout_seconds = timeout + 10
        with _TitleProbeSteps(mcp, artifact / "singleplayer", timeout, ctx.scenario_id) as probe:
            title = probe.call("minecraft_observe")
            probe.check("title_open", title["screen"].get("class") == TITLE_CLASS)
            _click(probe, title, "Singleplayer")
            state = probe.wait(lambda s: s["screen"].get("class") == SELECT, "select_world")
            _click(probe, state, "Create New World")
            state = probe.wait(lambda s: s["screen"].get("class") == CREATE, "create_world")
            if ctx.scenario_id in (SLOW, WEAK):
                probe.call("minecraft_set_screen_input", {
                    "expected_screen_class": CREATE, "expected_title": state["screen"]["title"],
                    "input_label": "Seed", "value": "42",
                })
                probe.report["seed"] = 42
            _click(probe, state, "Create New World")
            state = probe.wait(lambda s: s["screen"].get("class") == SELECT, "world_created")
            native_client.capture(mcp, artifact, "singleplayer-created")
            worlds = list((root / "worlds").glob("*/solaris-world.json"))
            if len(worlds) != 1:
                raise RuntimeError(f"expected one fresh world, found {len(worlds)}")
            world = worlds[0].parent
            if ctx.scenario_id in (SLOW, WEAK):
                singleplayer_slow.capture(probe, artifact, "slow-before-play")
                started = time.monotonic()
                _click(probe, state, "Play Selected World")
                if ctx.scenario_id == SLOW:
                    starting = probe.wait(
                        lambda s: s["screen"].get("class") == "net.minecraft.client.gui.screens.GenericMessageScreen",
                        "slow_start_screen")
                    probe.report["slow_start_observation"] = starting
                    singleplayer_slow.capture(probe, artifact, "slow-starting")
                log = world / "logs" / "server.log"
                try:
                    def startup_progress(text):
                        probe.report["server_startup_progress_log"] = text
                        probe.wait(
                            lambda s: s["state_version"] > starting["state_version"],
                            "startup_progress_published")
                        singleplayer_slow.capture(probe, artifact, "slow-server-progress")

                    result = singleplayer_slow.wait_server(
                        pidfile, log, probe.remaining(limit=None),
                        on_progress=startup_progress if ctx.scenario_id == SLOW else None)
                    probe.report["server_outcome"] = result
                    probe.report["server_wait_seconds"] = time.monotonic() - started
                    quota_path = artifact / "slow-server-cpu.max"
                    quota = quota_path.read_text().split() if quota_path.is_file() else []
                    probe.report["server_cpu_max"] = quota
                    if ctx.scenario_id == SLOW:
                        probe.check("server_cpu_quota_1_percent",
                                    len(quota) == 2 and all(part.isdigit() for part in quota)
                                    and int(quota[0]) * 100 == int(quota[1]))
                        if result == "listening":
                            probe.check("startup_crossed_former_deadline",
                                        probe.report["server_wait_seconds"] > 180)
                            singleplayer_slow.release_startup_quota(unit)
                            probe.report["startup_quota_released_after_listening"] = True
                    if result == "listening":
                        # This canonical wait follows the connection epoch across
                        # title-to-Play; a plain observe can race that handoff.
                        probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
                    final = probe.wait(
                        lambda s: (s.get("in_play") is True and s["screen"].get("open") is False)
                        or s["screen"].get("class") == SELECT,
                        "slow_world_play_or_failure")
                    probe.report["slow_final_observation"] = final
                    probe.check("slow_server_listened", result == "listening")
                    probe.check("slow_entered_world", final.get("in_play") is True)
                    probe.report["play_to_first_view_seconds"] = time.monotonic() - started
                    probe.check("terrain_overlay_cleared", final["screen"].get("open") is False)
                    player = final["player"]
                    yaw = 10 if abs(player["yaw"] - 10) > 1 else 0
                    probe.call("minecraft_look", {"yaw_deg": yaw, "pitch_deg": 0})
                    probe.wait(lambda s: abs(s["player"]["yaw"] - yaw) < 0.1, "ordinary_look_controls_player")
                    probe.report["play_to_control_seconds"] = time.monotonic() - started
                    probe.report["rss"] = singleplayer_slow.rss(
                        client.pid, int((artifact / "server-process.pid").read_text()))
                    singleplayer_slow.capture(probe, artifact, "slow-final")
                except BaseException:
                    try:
                        probe.report["slow_failure_observation"] = probe.call("minecraft_observe")
                        singleplayer_slow.capture(probe, artifact, "slow-failure")
                    except Exception as capture_error:
                        probe.report["slow_failure_capture_error"] = str(capture_error)
                    raise
            elif ctx.scenario_id == FIRST:
                database = world / "world" / "native" / "state.redb"
                marker = world / "world" / ".solaris-first-start"
                def interrupt():
                    _wait_path(database, min(timeout, 180))
                    pid = _server_child(client)
                    if not marker.is_file():
                        raise RuntimeError("first-start marker absent when database appeared")
                    os.kill(pid, signal.SIGKILL)
                    return {"pid": pid, "database": str(database), "marker_present": True}
                with ThreadPoolExecutor(max_workers=1) as executor:
                    future = executor.submit(interrupt)
                    _click(probe, state, "Play Selected World")
                    probe.report["interruption"] = future.result(timeout=min(timeout, 180) + 5)
                recovery = os.environ.get("SOLARIS_FIRST_START_RECOVERY_BINARY")
                if recovery:
                    replacement = Path(recovery).resolve()
                    if not replacement.is_file():
                        raise RuntimeError(f"missing recovery control server: {replacement}")
                    server_link.unlink()
                    server_link.symlink_to(replacement)
                    probe.report["recovery_server"] = str(replacement)
                    with replacement.open("rb") as executable:
                        probe.report["recovery_server_sha256"] = hashlib.file_digest(executable, "sha256").hexdigest()
                state = probe.wait(lambda s: s["screen"].get("class") in (SELECT, TITLE_CLASS), "interrupted_menu")
                if state["screen"]["class"] == TITLE_CLASS:
                    _click(probe, state, "Singleplayer")
                    state = probe.wait(lambda s: s["screen"].get("class") == SELECT, "reopen_world_list")
                _select_world(probe, state)
                _play(probe, state)
            else:
                _play(probe, state)
                native_client.capture(mcp, artifact, "singleplayer-before-relaunch")
                holder = _server_child(client)
                holder_fd = os.pidfd_open(holder)
                signal.pidfd_send_signal(holder_fd, signal.SIGSTOP)
                probe.report["holder_pid"] = holder
                probe.call("minecraft_press_inputs", {"keys": ["key.keyboard.escape"], "ticks": 1})
                pause = probe.wait(lambda s: s["screen"].get("class", "").endswith("PauseScreen"), "pause")
                _click(probe, pause, "Save & Quit to Title")
                probe.wait(lambda s: s["screen"].get("class") == TITLE_CLASS, "save_quit")
                signal.pidfd_send_signal(holder_fd, signal.SIGTERM)
                mcp.close()
                mcp = None
                client.terminate()
                client.wait(timeout=60)
                if select.select([holder_fd], [], [], 0)[0]:
                    raise RuntimeError("old server exited before lease contention")
                client, _, mcp = native_client.start(ctx, client_copy, None, None, min(timeout, 180), display=display)
                mcp.request_timeout_seconds = timeout + 10
                probe.client = mcp
                title = probe.wait(lambda s: s["screen"].get("class") == TITLE_CLASS, "relaunched_title")
                _click(probe, title, "Singleplayer")
                state = probe.wait(lambda s: s["screen"].get("class") == SELECT, "relaunched_world_list")
                _select_world(probe, state)
                log = world / "logs" / "server.log"
                def release():
                    def logged():
                        text = log.read_text(errors="replace") if log.exists() else ""
                        return ("waiting for its holder to exit" in text or "already leased" in text, text)
                    text = runtime.wait_for_path_condition(log, min(timeout, 90), logged)
                    (artifact / "lease-wait.log").write_text(text)
                    signal.pidfd_send_signal(holder_fd, signal.SIGCONT)
                    if "waiting for its holder to exit" not in text:
                        raise RuntimeError("lease_wait_logged assertion failed: server refused held world")
                with ThreadPoolExecutor(max_workers=1) as executor:
                    future = executor.submit(release)
                    try:
                        _play(probe, state)
                    finally:
                        future.result(timeout=min(timeout, 90) + 5)
                probe.check("lease_wait_logged", True)
            native_client.capture(mcp, artifact, "singleplayer-ingame")
            probe.call("minecraft_disconnect", {})
        return {"scenario": probe.report}
    finally:
        if holder_fd is not None:
            try:
                signal.pidfd_send_signal(holder_fd, signal.SIGCONT)
                if not select.select([holder_fd], [], [], 15)[0]:
                    signal.pidfd_send_signal(holder_fd, signal.SIGKILL)
            except ProcessLookupError:
                pass
            finally:
                os.close(holder_fd)
        if mcp is not None:
            mcp.close()
        runtime.stop_process(client)
        if unit is not None:
            subprocess.run(["systemctl", "--user", "stop", unit + ".scope"],
                           check=False, capture_output=True, timeout=30)
        if world is not None:
            log = world / "logs" / "server.log"
            if log.is_file():
                shutil.copy2(log, artifact / "singleplayer-server.log")
        runtime.stop_process(xvfb)
        for name, value in previous.items():
            if value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = value
