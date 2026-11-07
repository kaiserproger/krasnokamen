"""Destroyed-open-chest custody before the next ordinary simulation commit."""
from __future__ import annotations

from typing import Any

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


ATTEMPTS = 6


def destroyed_chest_race(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    """HS-1 on the wire: the stale click follows the destroying command without waiting for it.

    The client's menu is still open when the click is issued, so the click packet leaves right
    behind the command packet. Which of the two the server handles first depends on the tick
    boundary, so each branch repeats the race; every outcome must keep the one diamond exactly
    once (inventory or ground): extraction may not duplicate it, insertion may not discard it.
    The unfixed server loses custody whenever the click lands after the command, so a branch
    that never sent a click fails instead of passing.
    """
    steps.check("graphical_client_required", bool(getattr(steps.client, "p4_window", None)))
    peer = getattr(steps.client, "nether_peer", None)
    steps.check("nether_peer_client_present", peer is not None)
    point = _point(origin)
    # Shared command execution (the HS-1 window) exists only while another dimension is populated.
    steps.fixture("gamemode creative NetherPeer")
    steps.fixture("execute in minecraft:the_nether run tp NetherPeer 8 100 8")
    while True:
        seen = peer.call_tool("minecraft_observe")
        if seen.get("dimension") == "minecraft:the_nether" and seen.get("in_play") is True:
            break
        peer.call_tool("minecraft_wait_for_state_change", {
            "observed_version": seen["state_version"], "timeout_seconds": steps.remaining()})
    steps.check("peer_populates_the_nether", True)
    _echo(steps, "nether_peer_ordered_receipt")
    steps.fixture("tick freeze")
    _echo(steps, "freeze_ordered_receipt")
    destroy = f"setblock {point['x']} {point['y']} {point['z']} minecraft:air destroy"
    try:
        for inserting in (False, True):
            label = "insertion" if inserting else "extraction"
            outcomes: list[dict[str, Any]] = []
            steps.report.setdefault("race", {})[label] = outcomes
            for attempt in range(ATTEMPTS):
                _set(steps, point, "minecraft:chest")
                _give(steps, "minecraft:diamond")
                _open(steps, point, "ChestMenu", 63)
                if not inserting:
                    _insert(steps, "minecraft:diamond", 0)
                state = steps.observe()
                stale_slot = 0 if not inserting else next(
                    stack["slot"] for stack in state["container"]["slots"]
                    if stack["slot"] >= 27 and stack["item_id"] == "minecraft:diamond")
                steps.report["fixture_commands"].append(destroy)
                steps.call("minecraft_send_chat", {"message": destroy, "command": True})
                sent = True
                try:
                    steps.call("minecraft_quick_move_container_slot", {"slot": stale_slot, "timeout_seconds": 3})
                except (RuntimeError, TimeoutError) as error:
                    sent = "no visible container" not in str(error)
                _echo(steps, f"{label}_{attempt}_destroy_ordered_receipt")
                _block(steps, point, "minecraft:air")
                state = steps.observe()
                steps.check(f"{label}_{attempt}_menu_closed", state["container"]["container_id"] == 0)
                steps.check(f"{label}_{attempt}_cursor_empty", state["container"]["carried"]["count"] == 0)
                rows = _rows(steps, 8)
                ground = sum(row.get("item", {}).get("count", 0) for row in rows
                             if row.get("item", {}).get("item_id") == "minecraft:diamond")
                held = item_count(state, "minecraft:diamond")
                outcomes.append({"click_sent": sent, "held": held, "ground": ground})
                steps.check(f"{label}_{attempt}_exactly_one_diamond_remains", held + ground == 1)
                if attempt == 0:
                    _capture(steps, label + "-after-racing-click")
                steps.fixture("kill @e[type=minecraft:item]")
                steps.fixture("clear @s")
                _echo(steps, f"{label}_{attempt}_cleanup_ordered_receipt")
            steps.check(label + "_click_raced_the_destroy", any(entry["click_sent"] for entry in outcomes))
    finally:
        steps.fixture("tick unfreeze")
    steps.report["limits"] = ["command destruction while simulation frozen; click issued right behind the command packet"]
