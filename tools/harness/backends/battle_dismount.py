"""Battle dismount gate: a rider whose horse is killed mid-charge must walk afterwards, not skate.

Row ``BATTLE-DISMOUNTED-RIDER-SKATES-TO-SWEEP-POINT`` (docs/bugs/rows): ``battle/cavalry.rs``'s
``sweep_drives`` used to write the raw sweep heading — a displacement in blocks — as the drive of every
untargeted steady member of a sweeping mounted regiment, so a rider whose horse died kept a charge-sized
drive on foot and his wire positions jumped up to 10.9 blocks per packet.

The scene runs ``field-1k`` through ``tools/battle/run-battle.py`` with a live holder and a recorder
(the runner owns the server, the world and the WAL publication), waits for a *mounted* member whose
``vel`` shows a charge, kills that horse through the console fifo (``kill <uuid>``, the same surface
``battle_rearm_recovery`` uses), and then reads the published per-tick entity records for the next 100
ticks: the rider's own ground speed must stay at or below the walk speed the server gives a foot
soldier. Every input is fail-closed; a battle that never charges is incomplete, never a pass.
"""
from __future__ import annotations

import json
import sys
import time
from pathlib import Path
from typing import Any

from .. import runtime
from ..profiles import ProfileContext
from ..runtime import REPO_ROOT
from . import battle_recovery
from .. import wal_frame

HOLDER, VIEWER = battle_recovery.HOLDER, battle_recovery.VIEWER
PER_SIDE = 200
PRE_STAGE_TICKS = 120
CHARGE_SPEED = 5.0          # blocks per second a mounted member must show to count as charging
WALK_SPEED = 4.317          # vanilla walking speed, blocks per second
TICK_SPEED = WALK_SPEED / 20.0
TICKS_AFTER_KILL = 100
TICKS_BOUND = 12_000        # give the charge 600 s of battle before calling the scene incomplete


class DismountObserver:
    """Fold the published entity records into per-entity position, velocity and mount riders.

    A horse carries its riders as uuids in `state.entity_life.mount.riders`; a rider's own `vehicle`
    field is the *cargo* mount enum (minecart, boat), never his horse, so the link is read from the
    horse's side."""

    def __init__(self, archive: Path, horse_type: int):
        self.archive, self.horse_type = archive, horse_type
        self.last_tick = -1
        self.entities: dict[int, dict[str, Any]] = {}
        self.by_uuid: dict[str, int] = {}

    def update(self) -> None:
        for path in sorted(self.archive.glob("batch-*.bin")):
            span = wal_frame.span(path.name)
            if span is None or span[1] <= self.last_tick:
                continue
            try:
                frame = wal_frame.decode(path.read_bytes(), span)
            except wal_frame.TornFrame:
                break
            for line in frame.payload.splitlines():
                if b'"Entity"' not in line:
                    continue
                entry = json.loads(line)["Entity"]
                life = (entry.get("state") or {}).get("entity_life") or {}
                self.entities[entry["id"]] = {
                    "type_id": entry["type_id"],
                    "uuid": entry["uuid"],
                    "pos": list(entry["pos"]),
                    "vel": list(entry.get("vel") or [0.0, 0.0, 0.0]),
                    "alive": entry["alive"],
                    "riders": list((life.get("mount") or {}).get("riders") or []),
                }
                self.by_uuid[entry["uuid"]] = entry["id"]
            self.last_tick = frame.hi

    def charging_rider(self) -> dict[str, Any] | None:
        """A rider mounted on a horse and moving at charge speed: the sweep_drives row's subject."""
        for horse in self.entities.values():
            if horse["type_id"] != self.horse_type or not horse["alive"] or not horse["riders"]:
                continue
            # The horse carries the rider, so the charge speed is the horse's own velocity; the
            # rider's record follows it and, after the kill, carries his foot drive alone.
            speed = sum(component * component for component in horse["vel"]) ** 0.5
            for uuid in horse["riders"]:
                rider = self.entities.get(self.by_uuid.get(uuid, -1))
                if rider is None or not rider["alive"]:
                    continue
                if speed >= CHARGE_SPEED:
                    return {**rider, "speed": speed, "mount_uuid": horse["uuid"], "mount_id": horse["id"]}
        return None

    def rider_by_uuid(self, uuid: str) -> dict[str, Any] | None:
        return self.entities.get(self.by_uuid.get(uuid, -1))

    def wait_for(self, condition, label: str, timeout_seconds: float = 900.0, interval: float = 0.5):
        """Poll the archive: the runner publishes batches continuously and has no single marker file
        the harness could watch, so this is a bounded re-read, not an inotify wait."""
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            self.update()
            found = condition()
            if found:
                return found
            time.sleep(interval)
        raise RuntimeError(f"{label}: the published battle did not reach the scene's condition in "
                           f"{timeout_seconds:.0f}s")


def run(ctx: ProfileContext) -> dict[str, Any]:
    scenario = ctx.scenario_id
    inputs = battle_recovery.resolve_inputs(ctx, scenario)
    report: dict[str, Any] = {"scenario": scenario, "assertions": {}}
    ctx.failure_scenario = report
    display_process, display, display_log = runtime.start_xvfb(ctx.artifact_dir)
    session = None
    try:
        registry = json.loads((inputs["cache"] / "reports" / "registries.json").read_text())
        horse_type = registry["minecraft:entity_type"]["entries"]["minecraft:horse"]["protocol_id"]
        root = ctx.artifact_dir / "battle-dismount"
        root.mkdir(parents=True, exist_ok=True)
        session = battle_recovery.RunnerSession(ctx, inputs, root / "run", display,
                                               paused_ticks=0, console_fifo=True)
        session.start()
        # The runner is driven through its console fifo (the same surface battle_live uses); the
        # stdin Console only exists for the recovery path.
        console = battle_recovery.FifoConsole(session.root)
        console.wait(r"Battle live placed: " + HOLDER, 0, timeout=900)
        console.wait(r"Battle started tick=", 0, timeout=900)
        observer = DismountObserver(session.root / "wal-archive", horse_type)
        (session.root / "wal-archive").mkdir(parents=True, exist_ok=True)
        mark = console.mark()
        console.send("say A11DismountWatching")
        console.wait(r"\[Server\] A11DismountWatching", mark)
        target = observer.wait_for(observer.charging_rider, "a charging rider")
        report["observations"] = {"charging_rider": target}
        mark = console.mark()
        console.send(f"kill {target['mount_uuid']}")
        console.send("say A11DismountHorseKilled")
        console.wait(r"\[Server\] A11DismountHorseKilled", mark, timeout=120)
        kill_reply = console.text_since(mark)
        report["observations"]["kill_reply"] = kill_reply
        battle_recovery.check(report, "horse_killed_under_the_rider", "Killed " in kill_reply, kill_reply)
        # 100 published ticks of the rider's own position: the fix keeps him at foot speed.
        rider_uuid = target["uuid"]
        samples: list[dict[str, Any]] = []
        deadline = observer.last_tick + TICKS_AFTER_KILL

        def hundred_ticks():
            observer.update()
            entity = observer.rider_by_uuid(rider_uuid)
            horse = observer.entities.get(target["mount_id"])
            if entity is not None and entity["alive"] and horse is not None and not horse["alive"]:
                samples.append({"tick": observer.last_tick, "pos": entity["pos"], "vel": entity["vel"]})
            return observer.last_tick >= deadline
        observer.wait_for(hundred_ticks, "100 published ticks after the kill")
        steps = []
        for before, after in zip(samples, samples[1:]):
            if after["tick"] - before["tick"] != 1:
                continue
            steps.append(sum((b - a) ** 2 for a, b in zip(before["pos"], after["pos"])) ** 0.5)
        report["observations"].update({"samples": len(samples), "steps": len(steps),
                                       "max_step": max(steps) if steps else None,
                                       "mean_step": (sum(steps) / len(steps)) if steps else None})
        battle_recovery.check(report, "rider_sample_window_complete", len(steps) >= TICKS_AFTER_KILL // 2,
                              report["observations"])
        battle_recovery.check(report, "dismounted_rider_keeps_walk_speed",
                              bool(steps) and max(steps) <= TICK_SPEED * 1.35,
                              {"max_step": report["observations"]["max_step"],
                               "walk_step": TICK_SPEED})
        battle_live_judge(report, scenario)
        return {"scenario": report}
    finally:
        if session is not None:
            session.close()
        runtime.stop_process(display_process)
        display_log.close()


def battle_live_judge(report: dict[str, Any], scenario: str) -> None:
    from . import battle_live
    battle_live.judge(report)
