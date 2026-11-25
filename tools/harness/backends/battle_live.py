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
import time
import sys
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
# The row's symptom is being placed far from the fight. Measured with the same scene and pin pair:
# the fix places him 11.17 blocks from the nearest enemy file (client position vs the server image),
# the pre-fix control 30.17, and trial-7's symptom was ~230. 20 blocks sits between the two with
# ~9 and ~10 blocks of margin and leaves the 230-block jump far outside.
ENEMY_REACH_BLOCKS = 20.0
# PovRed is staged on the red side (`--live-player PovRed:red`).
HOLDER_SIDE = 1
# The spectator camera sits about a block behind the followed entity's eye.
CAMERA_FOLLOW_BLOCKS = 8.0
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


def judge(report: dict[str, Any]) -> None:
    """Set `passed` and fail the step: a profile's own report is not evidence by itself."""
    report["passed"] = bool(report["assertions"]) and all(report["assertions"].values()) \
        and not report.get("incomplete")
    if not report["passed"]:
        failed = [name for name, ok in report["assertions"].items() if not ok]
        raise RuntimeError(f"battle-live assertions failed: {failed or 'incomplete checklist'}")


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
        # The runner creates its run directory itself and refuses an existing one.
        run_dir = root / "run"
        port, dashboard = runtime.reserve_port(), runtime.reserve_port()
        address = f"127.0.0.1:{port}"
        command = [
            sys.executable, str(REPO_ROOT / "tools" / "battle" / "run-battle.py"),
            "--pin", str(inputs["server"].parent), "--client", str(inputs["recorder"]),
            "--client-data", str(inputs["recorder_data"]), "--cache", str(inputs["cache"]),
            "--run-dir", str(run_dir), "--world", str(inputs["world"]),
            "--world-seed", str(inputs["world_seed"]), "--seed", "42", "--scenario", "field-1k",
            "--count", str(PER_SIDE), "--duration", "900", "--view-distance", str(VIEW_DISTANCE),
            "--pre-stage-ticks", str(PRE_STAGE_TICKS), "--port", str(port),
            "--dashboard-port", str(dashboard), "--live-player", f"{HOLDER}:red",
            "--live-spectator", VIEWER, "--live-timeout", "600", "--console-fifo",
            "--allow-marker-gaps", "--no-phase-log",
            # The WAL publication is the witness for the respawn symptom: the server's own
            # fighter positions per tick (review wave2-13 M3, Main-approved route).
            "--crash-handoff",
        ]
        runner_log = (ctx.artifact_dir / "battle-live-runner.log").open("x")
        runner, runner_index = battle_recovery._launch(
            ctx, command, ctx.artifact_dir / "battle-live-runner.log", cwd=REPO_ROOT,
            stdout=runner_log, stderr=subprocess.STDOUT)
        console = battle_recovery.FifoConsole(run_dir)
        battle = battle_recovery._runner_module()
        battle.watch_ready(run_dir, runner, timeout=180, label="new run directory")
        runtime.wait_port(port, 300, runner, log_path=run_dir / "server.log")
        for name in (HOLDER, VIEWER):
            client = battle_recovery.Horizon(ctx, inputs, run_dir, name, display)
            clients.append(client)
            client.start(address, "live")
        online = console.wait(rf"Battle player online: {HOLDER} entity=(\d+)", 0, timeout=900)
        holder_entity = int(online.group(1))
        console.wait(rf"Battle spectator placed: {VIEWER} .*camera=({holder_entity})\b", 0,
                     timeout=600)
        console.wait(r"Battle started tick=", 0, timeout=900)
        before = [client.capture("before-death") for client in clients]
        camera_before = (before[1]["state"].get("camera") or {}).get("entity_id")
        check(report, "clients_reached_play",
              before[0]["state"].get("screen") == "playing" and before[1]["state"].get("screen") == "playing",
              [data["state"].get("screen") for data in before])
        check(report, "holder_is_the_survival_front_rank",
              before[0]["state"].get("gamemode") == "survival",
              before[0]["state"].get("gamemode"))
        check(report, "spectator_camera_bound_before_death",
              camera_before == holder_entity, {"camera": camera_before, "holder": holder_entity})
        # Contact: the telemetry's fighting count is the server's own front-line census.
        strike, rows = _wait_for_fighting(battle, dashboard, report)
        (run_dir / "before-death-telemetry.jsonl").write_text("".join(json.dumps(row) + "\n" for row in rows))
        report.setdefault("observations", {})["fighting"] = (strike.get("battle") or {}).get("fighting")
        mark = console.mark()
        console.send(f"kill {HOLDER}")
        died = console.wait(rf"Battle live died: {HOLDER}", mark, timeout=300)
        report["observations"]["died_line"] = died.group(0)
        # The client drives his own respawn: the server's live respawn answers that
        # action (battle_recovery's kit does the same on the real client).
        holder = clients[0]
        holder.driver("wait_for", condition="screen", value="dead", timeout=300)
        dead = holder.capture("dead")
        died_at = _point(dead["state"])
        holder.driver("respawn")
        holder.driver("wait_for", condition="screen", value="playing", timeout=300)
        respawned = console.wait(rf"Battle live respawned: {HOLDER}[^\n]*", mark, timeout=600)
        line = respawned.group(0)
        report["observations"]["respawn_line"] = line
        at = _parse_at(line)
        check(report, "holder_respawned", at is not None, line)
        check(report, "respawn_is_in_contact", CONTACT_TRUE.search(line) is not None, line)
        check(report, "respawn_is_not_the_side_centre", "rule=centre" not in line, line)
        after = [client.capture("after-respawn") for client in clients]
        camera_after = (after[1]["state"].get("camera") or {})
        holder_after = after[0]["state"]
        report.setdefault("observations", {}).update(
            {"camera_before": camera_before, "camera_after": camera_after.get("entity_id"),
             "respawn_at": at, "died_at": died_at})
        check(report, "holder_back_in_play_after_respawn",
              holder_after.get("screen") == "playing", holder_after.get("screen"))
        # The row's symptom: the spectator stayed on the old entity. His camera
        # must now be within a few blocks of the holder he follows.
        gap = _camera_gap(camera_after, holder_after)
        report["observations"]["camera_gap_blocks"] = gap
        check(report, "spectator_camera_followed_the_respawn",
              gap is not None and gap <= CAMERA_FOLLOW_BLOCKS, {"gap": gap, "camera": camera_after.get("entity_id")})
        # The row's symptom (an enemy in sight after the respawn) cannot be measured by
        # this gate yet: the holder's horizon-ctl exposes no entity census (`entities`
        # blocks, `minecraft_list_entities` belongs to the hotbar MCP client) and the
        # client `state` carries no entities. Recorded as the row's missing proof —
        # `contact=`/`rule=centre` are strings the fix itself writes, not a control
        # (review wave2-13 M3).
        if scenario == "battle-live-respawn-line-contact":
            report.setdefault("incomplete", []).append(
                "no client entity census through horizon-ctl: the enemy-in-reach symptom is not measured")
        # The row's symptom from the server's own published truth: the respawned holder must
        # have an enemy soldier inside weapon reach in the next battle image. Log fields are
        # not evidence here (contact=/rule=centre are strings the fix writes).
        holder_at = _point(holder_after)
        witness = _respawn_enemy_witness(run_dir, holder_at)
        report["observations"]["respawn_enemy_witness"] = witness
        check(report, "holder_sees_an_enemy_after_respawn",
              witness.get("min_enemy_distance") is not None
              and witness["min_enemy_distance"] <= ENEMY_REACH_BLOCKS, witness)
        # Where he landed against where he fell: the row names a 230-block jump.
        if at is not None and died_at is not None:
            fall = sum((a - b) ** 2 for a, b in zip(at, died_at)) ** 0.5
            report["observations"]["fall_distance_blocks"] = fall
            check(report, "respawn_near_where_he_fell", fall <= FALL_TOLERANCE_BLOCKS, fall)
        else:
            report.setdefault("incomplete", []).append(
                "no client position or server at=: the fall distance cannot be measured")
        judge(report)
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


def _point(state: dict[str, Any]) -> list[float] | None:
    """The client's own position, when its state exposes one."""
    player = state.get("player") or {}
    block_pos = player.get("block_pos")
    if isinstance(block_pos, list) and len(block_pos) == 3:
        return [float(value) for value in block_pos]
    if all(isinstance(player.get(key), (int, float)) for key in ("x", "y", "z")):
        return [float(player["x"]), float(player["y"]), float(player["z"])]
    return None


def _respawn_enemy_witness(run_dir: Path, holder_at: list[float] | None,
                          holder: str = HOLDER) -> dict[str, Any]:
    """The respawned holder's distance to the nearest soldier of another faction.

    Reads the runner's `wal-archive/` (enabled with `--crash-handoff`) and folds the battle images:
    the hold carries the holder's own position and regiment, the member anchors carry the line, and
    `regiments[i].spec.faction` separates the sides. That is the server's own truth about who stood
    within weapon reach of the respawn — no log string, no client feed (review wave2-13 M3).
    """
    archive = run_dir / "wal-archive"
    result: dict[str, Any] = {"archive_exists": archive.is_dir(), "images": 0,
                              "holder": None, "holder_at": None,
                              "enemy_fighters": 0, "min_enemy_distance": None}
    if not archive.is_dir():
        return result
    fold = None
    for path in sorted(archive.glob("batch-*.bin")):
        decoded = battle_recovery.wal_frame(path.name, path.read_bytes())
        if decoded is None:
            continue
        for line in decoded[2].splitlines():
            if b'"Battle' not in line:
                continue
            for dimension, entry in battle_recovery._visit(json.loads(line)):
                if dimension != "minecraft:overworld":
                    continue
                folded = battle_recovery.fold_battle(fold, entry)
                if folded is not None:
                    fold = folded
                    result["images"] += 1
    if fold is None:
        return result
    battle = fold["battle"]
    hold = next((row for row in battle["live"]["holds"] if row.get("name") == holder), None)
    if hold is None or holder_at is None:
        return result
    # The hold's own `at` lags the respawn teleport, so where he stands comes from his client;
    # the server image supplies only the enemy side (measured, HANDOFF-20).
    at = [float(value) for value in holder_at]
    result["holder"], result["holder_at"] = hold.get("name"), at
    regiments = battle["formation"]["regiments"]
    own = regiments[hold["regiment"]]["spec"]["faction"]
    distances = []
    for member in battle["formation"]["members"]:
        faction = regiments[member["regiment"]]["spec"]["faction"]
        if faction == own or not member.get("anchor"):
            continue
        anchor = member["anchor"]
        distances.append((((anchor[0] - at[0]) ** 2 + (anchor[1] - at[2]) ** 2) ** 0.5, member["id"]))
    result["enemy_fighters"] = len(distances)
    # The last image is the freshest reading; a fighter row may vanish on death, so keep the min.
    result["min_enemy_distance"] = round(min(d[0] for d in distances), 2) if distances else None
    return result


def _camera_gap(camera: dict[str, Any], holder: dict[str, Any]) -> float | None:
    """Distance from the spectator's camera eye to the holder's own eye."""
    entity = camera.get("entity") or {}
    eye = entity.get("position")
    if not (isinstance(eye, list) and len(eye) == 3):
        return None
    target = (holder.get("player") or {}).get("eye")
    if not (isinstance(target, list) and len(target) == 3):
        point = _point(holder)
        if point is None:
            return None
        target = [point[0], point[1] + 1.62, point[2]]
    return sum((float(a) - float(b)) ** 2 for a, b in zip(eye, target)) ** 0.5
