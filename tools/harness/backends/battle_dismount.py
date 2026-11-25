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
    """Fold the published entity records into per-entity position, velocity and vehicle state."""

    def __init__(self, archive: Path, horse_type: int):
        self.archive, self.horse_type = archive, horse_type
        self.last_tick = -1
        self.entities: dict[int, dict[str, Any]] = {}

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
                self.entities[entry["id"]] = {
                    "type_id": entry["type_id"],
                    "uuid": entry["uuid"],
                    "pos": list(entry["pos"]),
                    "vel": list(entry.get("vel") or [0.0, 0.0, 0.0]),
                    "alive": entry["alive"],
                    "vehicle": ((entry.get("state") or {}).get("entity_life") or {}).get("vehicle"),
                }
            self.last_tick = frame.hi

    def charging_rider(self) -> dict[str, Any] | None:
        """A mounted member moving at charge speed on a horse: the sweep_drives row's subject."""
        for entity in self.entities.values():
            if not entity["alive"] or entity["vehicle"] is None:
                continue
            mount = self.entities.get(entity["vehicle"])
            if mount is None or mount["type_id"] != self.horse_type:
                continue
            speed = sum(component * component for component in entity["vel"]) ** 0.5
            if speed >= CHARGE_SPEED:
                return {**entity, "speed": speed, "mount_uuid": mount["uuid"]}
        return None

    def wait_for(self, condition, timeout_ticks: int = TICKS_BOUND):
        def ready():
            self.update()
            return condition(), None
        return runtime.wait_for_path_condition(self.archive / "published", 600, ready)


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
        target = observer.wait_for(observer.charging_rider)
        report["observations"] = {"charging_rider": target}
        mark = console.mark()
        console.send(f"kill {target['mount_uuid']}")
        console.send("say A11DismountHorseKilled")
        console.wait(r"\[Server\] A11DismountHorseKilled", mark, timeout=120)
        kill_reply = console.text_since(mark)
        report["observations"]["kill_reply"] = kill_reply
        battle_recovery.check(report, "horse_killed_under_the_rider", "Killed " in kill_reply, kill_reply)
        # 100 published ticks of the rider's own position: the fix keeps him at foot speed.
        rider = target["id"]
        samples: list[dict[str, Any]] = []
        deadline = observer.last_tick + TICKS_AFTER_KILL

        def hundred_ticks():
            observer.update()
            entity = observer.entities.get(rider)
            if entity is not None and entity["alive"] and entity["vehicle"] is None:
                samples.append({"tick": observer.last_tick, "pos": entity["pos"], "vel": entity["vel"]})
            return observer.last_tick >= deadline
        observer.wait_for(hundred_ticks)
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
