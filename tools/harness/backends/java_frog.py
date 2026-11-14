""Observe server-driven frog tongue metadata in a real vanilla Java client."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from . import java_p5
from .native_v2 import _echo, _prepare
from .probe_steps import ProbeSteps


def run(client: Any, artifact: Path, timeout: float) -> dict[str, Any]:
    with ProbeSteps(client, artifact / "java-frog", timeout, "java-frog-tongue") as steps:
        x, y, z = _prepare(steps)
        steps.call("minecraft_look", {"yaw_deg": 0, "pitch_deg": 15})
        steps.fixture(f'summon minecraft:frog {x + 0.5} {y} {z + 3.5} {{CustomName:"QAFrog"}}')
        _echo(steps, "frog_fixture_ordered")
        frog = steps.call("minecraft_wait_for_visible_entity", {
            "entity_type": "minecraft:frog", "radius": 12, "timeout_seconds": steps.remaining(),
        })
        steps.fixture(f'summon minecraft:slime {x + 1.5} {y} {z + 3.5} {{Size:0,NoAI:1b,Health:200f,CustomName:"QASlime"}}')
        _echo(steps, "frog_prey_ordered")
        slime = steps.call("minecraft_wait_for_visible_entity", {
            "entity_type": "minecraft:slime", "radius": 12, "timeout_seconds": steps.remaining(),
        })
        frog_id, slime_id = frog["entity_uuid"], slime["entity_uuid"]
        steps.report.update(frog_uuid=frog_id, prey_uuid=slime_id)
        observed = None
        for _ in range(160):
            rows = steps.call("minecraft_list_entities", {"radius": 16, "limit": 128})["entities"]
            frog = next((row for row in rows if row["entity_uuid"] == frog_id), None)
            if frog and frog.get("pose") == "USING_TONGUE" and frog.get("tongue_target_uuid") == slime_id:
                observed = frog
                break
            steps.call("minecraft_wait_ticks", {"ticks": 1})
        steps.check("frog_tongue_targets_real_slime", observed is not None)
        steps.report["tongue_observation"] = observed
        java_p5.capture(steps, "frog-tongue-active")
        steps.fixture("kill @e[type=minecraft:slime,name=QASlime,limit=1]")
        _echo(steps, "frog_prey_removed")
        def cleared(_: dict[str, Any]) -> bool:
            rows = steps.call("minecraft_list_entities", {"radius": 16, "limit": 128})["entities"]
            frog = next((row for row in rows if row["entity_uuid"] == frog_id), None)
            return bool(frog and frog.get("pose") != "USING_TONGUE" and not frog.get("tongue_target_uuid"))
        steps.wait(cleared, "frog_target_and_pose_clear")
        java_p5.capture(steps, "frog-tongue-cleared")
    return steps.report
