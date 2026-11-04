"""Destroyed-open-chest custody before the next ordinary simulation commit."""
from __future__ import annotations

from .native_p1 import _capture, _rows
from .native_v2 import _block, _echo, _give, _insert, _open, _point, _set
from .probe_steps import ProbeSteps, item_count


def destroyed_chest(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    steps.check("graphical_client_required", bool(getattr(steps.client, "p4_window", None)))
    point = _point(origin)
    # Freeze the simulation, not the connection/command pump. The destruction must
    # retire the menu itself; waiting for an ordinary tick would miss HS-1.
    steps.fixture("tick freeze")
    _echo(steps, "freeze_ordered_receipt")
    try:
        for inserting in (False, True):
            label = "insertion" if inserting else "extraction"
            _set(steps, point, "minecraft:chest")
            _give(steps, "minecraft:diamond")
            _open(steps, point, "ChestMenu", 63)
            if not inserting:
                _insert(steps, "minecraft:diamond", 0)
            _capture(steps, label + "-open-before-destroy")
            steps.fixture(f"setblock {point['x']} {point['y']} {point['z']} minecraft:air destroy")
            _echo(steps, label + "_destroy_ordered_receipt")
            _block(steps, point, "minecraft:air")
            state = steps.observe()
            # A closed menu cannot accept the stale extraction/deposit click.
            # Assert before unfreezing so a later commit cannot hide the race.
            steps.check(label + "_destroy_closes_before_ordinary_commit",
                        state["container"]["container_id"] == 0)
            steps.check(label + "_cursor_empty", state["container"]["carried"]["count"] == 0)
            rows = _rows(steps, 8)
            ground = sum(row.get("item", {}).get("count", 0) for row in rows
                         if row.get("item", {}).get("item_id") == "minecraft:diamond")
            stale_slot = next((stack["slot"] for stack in state["container"]["slots"]
                               if stack["item_id"] == "minecraft:diamond"), 0)
            try:
                steps.call("minecraft_quick_move_container_slot", {
                    "slot": stale_slot, "timeout_seconds": steps.remaining()})
            except RuntimeError as error:
                steps.check(label + "_stale_click_refused_for_closed_menu", "no visible container" in str(error))
            else:
                steps.check(label + "_stale_click_refused_for_closed_menu", False)
            held = item_count(state, "minecraft:diamond")
            steps.check(label + "_exact_custody_before_commit", held + ground == 1)
            steps.check(label + "_deposit_not_discarded" if inserting else label + "_no_early_extraction",
                        held == int(inserting))
            _capture(steps, label + "-closed-after-destroy")
            steps.report.setdefault("custody", {})[label] = {"held": held, "ground": ground}
            # Explicit cleanup between the two independent input branches.
            steps.fixture("kill @e[type=minecraft:item]")
            steps.fixture("clear @s")
            _echo(steps, label + "_cleanup_ordered_receipt")
    finally:
        steps.fixture("tick unfreeze")
    steps.report["limits"] = ["command destruction while simulation frozen; stale click is prevented by authoritative menu closure"]
