"""Graphical, causal probes for End checkpoint recovery and projectile compaction."""
from __future__ import annotations

import uuid

from .native_p1 import _aim, _capture, _rows
from .native_p4 import _entity_at, _restart
from .native_v2 import _echo
from .probe_steps import ProbeSteps


def _identity(row: dict) -> dict:
    return {key: row[key] for key in ("entity_id", "entity_uuid", "entity_type")}


def _console_shutdown(steps: ProbeSteps) -> None:
    if not callable(getattr(steps.client, "p4_restart_server", None)):
        raise RuntimeError("console shutdown requires the console restart hook")
    _echo(steps, "console_shutdown_initial_chat")
    _restart(steps)
    log = steps.client.console_shutdown_log
    steps.report["console_shutdown_log"] = log
    for label, message in (
        ("tick_owner_stopped", "core tick owner stopping"),
        ("journal_owner_stopped", "core journal owner stopped"),
        ("checkpoint_writer_stopped", "native checkpoint writer stopped"),
        ("server_loop_stopped", "server stopping"),
        ("shutdown_checkpoint_committed", "checkpoint committed"),
    ):
        steps.check(label, message in log)
    steps.check("shutdown_has_no_error", "ERROR" not in log)


def _end_restart(steps: ProbeSteps) -> None:
    if not callable(getattr(steps.client, "p4_restart_server", None)):
        raise RuntimeError("End restart requires the clean p4_restart_server hook")
    player = steps.observe()["player"]["entity_uuid"]
    steps.fixture("difficulty hard")
    _echo(steps, "initial_hard_difficulty_ordered")
    steps.fixture("gamemode creative")
    steps.fixture("execute in the_end run tp @s 8 100 8")
    arrival = steps.wait(lambda state: state["dimension"] == "minecraft:the_end",
                         "end_arrival", wait_for_play=True)
    steps.check("end_arrival_same_player", arrival["player"]["entity_uuid"] == player)
    # A fixed platform makes the existing restart helper's grounded fence meaningful.
    steps.fixture("fill 5 99 5 11 99 11 minecraft:obsidian")
    _echo(steps, "end_platform_ordered")
    dragon = steps.call("minecraft_wait_for_visible_entity", {
        "entity_type": "minecraft:ender_dragon", "radius": 128,
        "timeout_seconds": steps.remaining(),
    })
    steps.check("live_dragon_uuid_present", bool(dragon.get("entity_uuid")))
    steps.report["dragon_before_restart"] = _identity(dragon)
    _aim(steps, dragon)
    _capture(steps, "end-live-dragon-before-restart")
    steps.fixture("difficulty peaceful")
    _echo(steps, "peaceful_command_ordered")
    marker = f"end difficulty {uuid.uuid4().hex}"
    steps.call("minecraft_send_chat", {"message": marker, "command": False})
    steps.wait(lambda state: any(marker in line for line in state["recent_chat"]),
               "end_difficulty_query_marker")
    steps.fixture("difficulty")
    def peaceful_after_marker(state: dict) -> bool:
        for line in state["recent_chat"]:
            if marker in line:
                return False
            if "peaceful" in line.lower():
                return True
        return False
    steps.wait(peaceful_after_marker, "end_peaceful_difficulty_received")
    steps.call("minecraft_wait_ticks", {"ticks": 20})
    _restart(steps)
    after = steps.observe()
    steps.check("restart_same_end_world", after["dimension"] == "minecraft:the_end")
    steps.check("restart_same_end_player", after["player"]["entity_uuid"] == player)
    # Wait for a received dragon packet, then fence its persistent UUID rather
    # than its restart-local protocol entity ID.
    restored = steps.call("minecraft_wait_for_visible_entity", {
        "entity_type": "minecraft:ender_dragon", "radius": 128,
        "timeout_seconds": steps.remaining(),
    })
    steps.check("restart_retains_exact_dragon_uuid", restored["entity_uuid"] == dragon["entity_uuid"])
    steps.report["dragon_after_restart"] = _identity(restored)
    _aim(steps, restored)
    _capture(steps, "end-live-dragon-after-restart")
    steps.report["limits"] = ["live peaceful End fight only; dead-dragon checkpoint not exercised",
                              "screenshots require independent visual review"]


def _gallery(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    player = steps.observe()["player"]["entity_uuid"]
    steps.fixture("tick freeze")
    _echo(steps, "gallery_freeze_ordered")
    frozen = True
    try:
        # The first projectile hits the floor and is removed during flight. The
        # second still processes the hittable snapshot after swap-removing the
        # first; the armor stand is deliberately the final entity allocation.
        fixtures = (
            ("cow", x + 5.5, y, z + 4.5, "{NoAI:1b}"),
            ("snowball", x + 3.5, y + 1, z + 4.5, "{Motion:[0.0d,-1.0d,0.0d]}"),
            ("arrow", x - 2.5, y + 3, z + 4.5, "{}"),
            ("armor_stand", x + 0.5, y, z + 4.5, "{}"),
        )
        rows = []
        for kind, px, py, pz, nbt in fixtures:
            steps.fixture(f"summon minecraft:{kind} {px} {py} {pz} {nbt}")
            _echo(steps, f"gallery_{kind}_summon_ordered")
            rows.append(_entity_at(steps, f"minecraft:{kind}", px, pz))
        steps.check("gallery_distinct_entity_uuids", len({row["entity_uuid"] for row in rows}) == 4)
        steps.report["gallery_before"] = [_identity(row) for row in rows]
        _aim(steps, rows[3])
        _capture(steps, "gallery-frozen-before-compaction")
        steps.fixture("tick unfreeze")
        frozen = False
        steps.call("minecraft_wait_for_entity_removed", {
            **_identity(rows[1]), "timeout_seconds": steps.remaining(),
        })
        after = _echo(steps, "gallery_post_compaction_ordered_echo")
        steps.check("gallery_same_live_player", after["player"]["entity_uuid"] == player
                    and after["player"]["health"] is not None and after["player"]["health"] > 0)
        remaining = _rows(steps, 32)
        for label, row in (("cow", rows[0]), ("arrow", rows[2]), ("armor_stand", rows[3])):
            steps.check(f"gallery_{label}_same_uuid_after_compaction", any(
                found["entity_uuid"] == row["entity_uuid"] and found["entity_type"] == row["entity_type"]
                for found in remaining))
        steps.report["gallery_after"] = [_identity(row) for row in remaining]
        _capture(steps, "gallery-after-compaction")
    finally:
        if frozen:
            steps.fixture("tick unfreeze")
    steps.report["limits"] = ["minimal mixed gallery with projectile floor impact and swap-removal",
                              "screenshots require independent visual review"]


def run(steps: ProbeSteps, origin: tuple[int, int, int], kind: str) -> None:
    if kind not in ("end-restart", "entity-gallery", "console-shutdown"):
        raise ValueError(f"unknown world promotion probe: {kind}")
    if not getattr(steps.client, "p4_window", None):
        raise RuntimeError("world promotion probe requires the harness-owned windowed Xvfb client")
    if kind == "console-shutdown":
        _console_shutdown(steps)
    elif kind == "end-restart":
        _end_restart(steps)
    else:
        _gallery(steps, origin)
