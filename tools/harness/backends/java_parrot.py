"""Observe vanilla Java's parrot state after real jukebox events."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from . import java_p5
from .native_v2 import _block, _echo, _give, _point, _prepare, _set, _use
from .probe_steps import ProbeSteps


def _dance_is(row: dict[str, Any] | None, expected: bool) -> bool:
    return row is not None and row.get("parrot_dancing") is expected


def run(client: Any, artifact: Path, timeout: float) -> dict[str, Any]:
    with ProbeSteps(client, artifact / "java-parrot", timeout, "java-parrot-jukebox") as steps:
        x, y, z = origin = _prepare(steps)
        jukebox = _point(origin, dz=2)
        _set(steps, jukebox, "minecraft:jukebox")
        steps.fixture(f'summon minecraft:parrot {x + 1.5} {y} {z + 2.5} {{NoAI:1b,CustomName:"QAParrot"}}')
        _echo(steps, "parrot_fixture_ordered")
        parrot = steps.call("minecraft_wait_for_visible_entity", {
            "entity_type": "minecraft:parrot", "radius": 12,
            "timeout_seconds": steps.remaining(),
        })
        identity = parrot["entity_uuid"]
        steps.report["parrot_uuid"] = identity

        def row() -> dict[str, Any] | None:
            return next((entry for entry in steps.call("minecraft_list_entities", {
                "radius": 32, "limit": 128,
            })["entities"] if entry["entity_uuid"] == identity), None)

        def expect(dancing: bool, label: str) -> None:
            steps.wait(lambda _: _dance_is(row(), dancing), label)

        expect(False, "initially_not_dancing")
        _give(steps, "minecraft:music_disc_13")
        steps.call("minecraft_set_hotbar_slot", {"slot": 0})
        _use(steps, jukebox)
        _block(steps, jukebox, "minecraft:jukebox", {"has_record": "true"})
        expect(True, "record_start_dancing")
        java_p5.capture(steps, "parrot-record-playing")
        steps.call("minecraft_set_hotbar_slot", {"slot": 8})
        _use(steps, jukebox)
        expect(False, "record_stop_stops_dancing")
        _give(steps, "minecraft:music_disc_13")
        steps.call("minecraft_set_hotbar_slot", {"slot": 0})
        steps.call("minecraft_wait_ticks", {"ticks": 2})
        _use(steps, jukebox)
        expect(True, "record_restart_dancing")
        steps.fixture(f"tp @e[type=minecraft:parrot,name=QAParrot,limit=1] {x + 5.5} {y} {z + 2.5}")
        _echo(steps, "parrot_move_ordered")
        steps.wait(lambda _: (item := row()) is not None and item["x"] > x + 5.0,
                   "parrot_teleported_beyond_radius")
        _block(steps, jukebox, "minecraft:jukebox", {"has_record": "true"})
        steps.call("minecraft_wait_ticks", {"ticks": 2})
        expect(False, "out_of_range_stops_dancing")
        java_p5.capture(steps, "parrot-out-of-range")
    return steps.report
