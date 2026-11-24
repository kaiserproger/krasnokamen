"""Battle-live gate: one small staged battle with a live holder and a bound spectator.

Rows (docs/bugs/rows):
- BATTLE-LIVE-RESPAWN-FAR-FROM-FIGHT: a respawned front-rank player must land in a file of his
  line that is in contact (nearest where he fell), not at his side's centre file far from the fight.
- BATTLE-LIVE-SPECTATOR-LEFT-BEHIND: a spectator bound to the holder must follow him across his
  death and respawn (the camera rebinds), not stay on the old entity.

The run is one staged battle through ``tools/battle/run-battle.py`` (the runner owns the server, the
world and the recorder; the harness owns the two real Horizon clients and the console fifo). It is
deliberately small: ``field-1k`` with a few hundred a side reaches contact in a bounded time. The
gate kills the holder once the telemetry reports fighting, reads the server's own
``Battle live respawned`` line (``contact=``/``rule=``) and each client's received state.

Everything fail-closed: missing inputs refuse, a battle that never reaches contact is incomplete (never
a pass), and the spectator assertion needs the holder's *new* entity id from his own client.
"""
from __future__ import annotations

import json
import re
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

from .. import runtime
from ..profiles import ProfileContext
from ..runtime import REPO_ROOT
from . import battle_recovery

HOLDER, VIEWER = battle_recovery.HOLDER, battle_recovery.VIEWER
PER_SIDE = 200
VIEW_DISTANCE = 8
PRE_STAGE_TICKS = 120
BATTLE_TICKS_BOUND = 1200
FALL_TOLERANCE_BLOCKS = 40.0
RESPAWN_MARKERS = (
    re.compile(rf"Battle live respawned: {HOLDER} .*?at=(-?[\d.]+),(-?[\d.]+),(-?[\d.]+)"),
)
CONTACT_TRUE = re.compile(r"contact=true")


def scenario_ids() -> tuple[str, ...]:
    return ("battle-live-respawn-line-contact", "battle-live-spectator-follows-respawn")


def check(report: dict[str, Any], name: str, condition: bool, detail: Any = None) -> None:
    report["assertions"][name] = bool(condition)
    if detail is not None:
        report.setdefault("observations", {})[name] = detail


def _parse_at(line: str) -> list[float] | None:
    for pattern in RESPAWN_MARKERS:
        found = pattern.search(line)
        if found:
            return [float(value) for value in found.groups()]
    return None


def run(ctx: ProfileContext) -> dict[str, Any]:
    scenario = ctx.scenario_id
    if scenario not in scenario_ids() or ctx.extra_args:
        raise ValueError(f"battle-live runs only its named scenarios ({', '.join(scenario_ids())})")
    inputs = battle_recovery.resolve_inputs(ctx, scenario)
    report: dict[str, Any] = {"scenario": scenario, "assertions": {}}
    ctx.failure_scenario = report
    display_process, display, display_log = runtime.start_xvfb(ctx.artifact_dir)
    clients: list[battle_recovery.Horizon] = []
    runner = None
    runner_log = None
    try:
        root = ctx.artifact_dir / "battle-live"
        root.mkdir(parents=True, exist_ok=True)
        port, dashboard = runtime.reserve_port(), runtime.reserve_port()
        address = f"127.0.0.1:{port}"
        command = [
            sys.executable, str(REPO_ROOT / "tools" / "battle" / "run-battle.py"),
            "--pin", str(inputs["server"].parent), "--client", str(inputs["recorder"]),
            "--client-data", str(inputs["recorder_data"]), "--cache", str(inputs["cache"]),
            "--run-dir", str(root), "--world", str(inputs["world"]),
            "--world-seed", str(inputs["world_seed"]), "--seed", "42", "--scenario", "field-1k",
            "--count", str(PER_SIDE), "--duration", "900", "--view-distance", str(VIEW_DISTANCE),
            "--pre-stage-ticks", str(PRE_STAGE_TICKS), "--port", str(port),
            "--dashboard-port", str(dashboard), "--live-player", f"{HOLDER}:red",
            "--live-spectator", VIEWER, "--live-timeout", "600", "--console-fifo",
            "--allow-marker-gaps", "--no-phase-log",
        ]
        runner_log = (ctx.artifact_dir / "battle-live-runner.log").open("x")
        runner, runner_index = battle_recovery._launch(
            ctx, command, ctx.artifact_dir / "battle-live-runner.log", cwd=REPO_ROOT,
            stdout=runner_log, stderr=subprocess.STDOUT)
        console = battle_recovery.FifoConsole(root)
        battle = battle_recovery._runner_module()
        battle.watch_ready(root, runner, timeout=180, label="new run directory")
        runtime.wait_port(port, 300, runner, log_path=root / "server.log")
        for name in (HOLDER, VIEWER):
            client = battle_recovery.Horizon(ctx, inputs, root, name, display)
            clients.append(client)
            client.start(address, "live")
        console.wait(rf"Battle player online: {HOLDER} entity=(\d+)", 0, timeout=600)
        console.wait(rf"Battle spectator placed: {VIEWER} .*camera=(\d+)", 0, timeout=600)
        console.wait(r"Battle started tick=", 0, timeout=600)
        before = [client.capture("before-death") for client in clients]
        entity_before = (before[0]["state"].get("player") or {}).get("entity_id")
        camera_before = (before[1]["state"].get("camera") or {}).get("entity_id")
        check(report, "clients_reached_play",
              before[0]["state"].get("screen") == "playing" and before[1]["state"].get("screen") == "playing",
              [data["state"].get("screen") for data in before])
        check(report, "holder_is_the_survival_front_rank",
              before[0]["state"].get("gamemode") == "survival" and isinstance(entity_before, int),
              before[0]["state"].get("gamemode"))
        check(report, "spectator_camera_bound_before_death",
              isinstance(camera_before, int) and camera_before == entity_before,
              {"camera": camera_before, "holder": entity_before})
        # Contact: the telemetry's fighting count is the server's own front-line census.
        strike, rows = _wait_for_fighting(battle, dashboard, report)
        (root / "before-death-telemetry.jsonl").write_text("".join(json.dumps(row) + "\n" for row in rows))
        report.setdefault("observations", {})["fighting"] = (strike.get("battle") or {}).get("fighting")
        mark = console.mark()
        console.send(f"kill {HOLDER}")
        died = console.wait(rf"Battle live died: {HOLDER}", mark, timeout=120)
        report["observations"]["died_line"] = died.group(0)
        mark = console.mark()
        respawned = console.wait(rf"Battle live respawned: {HOLDER}", mark, timeout=120)
        line = respawned.group(0)
        report["observations"]["respawn_line"] = line
        at = _parse_at(line)
        check(report, "holder_respawned", at is not None, line)
        check(report, "respawn_is_in_contact", CONTACT_TRUE.search(line) is not None, line)
        # The client's own received state after the respawn: alive, in play, and a new entity id.
        deadline = time.monotonic() + 120
        after: list[dict[str, Any]] = []
        while time.monotonic() < deadline:
            after = [client.capture("after-respawn") for client in clients]
            player = after[0]["state"].get("player") or {}
            if after[0]["state"].get("screen") == "playing" and isinstance(player.get("entity_id"), int) \
                    and player.get("entity_id") != entity_before:
                break
            time.sleep(0.5)
        entity_after = (after[0]["state"].get("player") or {}).get("entity_id")
        camera_after = (after[1]["state"].get("camera") or {}).get("entity_id")
        report.setdefault("observations", {}).update(
            {"entity_before": entity_before, "entity_after": entity_after,
             "camera_before": camera_before, "camera_after": camera_after,
             "respawn_at": at})
        check(report, "holder_back_in_play_after_respawn",
              after[0]["state"].get("screen") == "playing"
              and isinstance(entity_after, int) and entity_after != entity_before,
              {"screen": after[0]["state"].get("screen"), "entity": entity_after})
        if scenario == "battle-live-spectator-follows-respawn":
            check(report, "spectator_camera_followed_the_respawn",
                  isinstance(camera_after, int) and isinstance(entity_after, int)
                  and camera_after == entity_after,
                  {"camera": camera_after, "holder": entity_after})
        # Fall distance, when the clients expose coordinates: the row names a 230-block jump.
        fall = _fall_distance(before[0]["state"], after[0]["state"])
        if fall is not None:
            report["observations"]["fall_distance_blocks"] = fall
            check(report, "respawn_near_where_he_fell", fall <= FALL_TOLERANCE_BLOCKS, fall)
        else:
            report.setdefault("incomplete", []).append(
                "client state exposes no player position: the fall distance is only the server line's contact flag")
        report["passed"] = all(report["assertions"].values()) and not report.get("incomplete")
    finally:
        for client in clients:
            client.stop()
        if runner is not None:
            battle_recovery._record_exit(ctx, runner_index, runner)
            runtime.stop_process(runner)
        if runner_log is not None:
            runner_log.close()
        runtime.stop_process(display_process)
        display_log.close()
    return {"scenario": report}


def _wait_for_fighting(battle: Any, dashboard: int, report: dict[str, Any],
                       bound: int = BATTLE_TICKS_BOUND) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """Wait (event-driven, on the runner's telemetry) until the battle reports fighting."""
    after = None
    seen: list[dict[str, Any]] = []
    started_at = None
    while True:
        text = battle.telemetry_get(dashboard, after, 1)
        rows = [json.loads(line) for line in text.splitlines() if line]
        if not rows:
            raise RuntimeError("battle telemetry produced no rows")
        seen.extend(rows)
        after = rows[-1]["tick"]
        if started_at is None:
            started_at = after
        for row in rows:
            if (row.get("battle") or {}).get("fighting", 0) > 0:
                return row, seen
        if after - started_at > bound:
            report["passed"] = False
            raise RuntimeError(f"battle produced no fighting within {bound} ticks")
        if len(seen) > bound:
            seen = seen[-bound:]


def _fall_distance(before: dict[str, Any], after: dict[str, Any]) -> float | None:
    """Distance between the two captures' own player positions, if the client exposes them."""
    def point(state: dict[str, Any]) -> list[float] | None:
        player = state.get("player") or {}
        if all(isinstance(player.get(key), (int, float)) for key in ("x", "y", "z")):
            return [float(player["x"]), float(player["y"]), float(player["z"])]
        return None

    first, last = point(before), point(after)
    if first is None or last is None:
        return None
    return sum((a - b) ** 2 for a, b in zip(first, last)) ** 0.5
