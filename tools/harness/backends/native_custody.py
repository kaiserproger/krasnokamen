"""Destroyed-open-chest custody before the next ordinary simulation commit."""
from __future__ import annotations

import os
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
    if os.environ.get("BREAK_HS1_ATTACK"):
        return _hs1_attack(steps, origin, os.environ["BREAK_HS1_ATTACK"])
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


def _hs1_attack(steps: ProbeSteps, origin: tuple[int, int, int], variant: str) -> None:
    """BREAK_HS1_ATTACK breaker glue (not for merge): HS-1 race on other storages and destroyers."""
    peer = steps.client.nether_peer
    point, other = _point(origin), _point(origin, dx=1)
    p = f"{point['x']} {point['y']} {point['z']}"
    o = f"{other['x']} {other['y']} {other['z']}"
    steps.fixture("gamemode creative NetherPeer")
    steps.fixture("execute in minecraft:the_nether run tp NetherPeer 8 100 8")
    while True:
        seen = peer.call_tool("minecraft_observe")
        if seen.get("dimension") == "minecraft:the_nether" and seen.get("in_play") is True:
            break
        peer.call_tool("minecraft_wait_for_state_change", {
            "observed_version": seen["state_version"], "timeout_seconds": steps.remaining()})
    if variant in ("crossdim", "feedback", "twoview"):
        steps.fixture("op NetherPeer")
        _echo(steps, "peer_op_ordered_receipt")
    if variant == "feedback":
        return _feedback_attack(steps, peer)
    if variant in ("twoview", "tnt"):
        return _hs1_dynamic(steps, origin, peer, variant)
    steps.fixture("tick freeze")
    _echo(steps, "freeze_ordered_receipt")
    destroy = {"fill": f"fill {p} {p} minecraft:air destroy",
               "stone": f"setblock {p} minecraft:stone destroy"}.get(variant, f"setblock {p} minecraft:air destroy")
    items = ["minecraft:diamond"] + (["minecraft:emerald"] if variant == "double" else [])
    outcomes: list[dict[str, Any]] = []
    steps.report["attack"] = {"variant": variant, "destroy": destroy, "outcomes": outcomes}
    try:
        for inserting in (False, True):
            for attempt in range(int(os.environ.get("BREAK_HS1_ATTEMPTS", "3"))):
                label = f"{'insertion' if inserting else 'extraction'}_{attempt}"
                if variant == "double":
                    steps.fixture(f"setblock {p} minecraft:chest[facing=north,type=left]")
                    steps.fixture(f"setblock {o} minecraft:chest[facing=north,type=right]")
                    _block(steps, point, "minecraft:chest", {"type": "left"})
                    _block(steps, other, "minecraft:chest", {"type": "right"})
                    menu, slots = "ChestMenu", 90
                elif variant == "shulker":
                    _set(steps, point, "minecraft:shulker_box")
                    menu, slots = "ShulkerBoxMenu", 63
                else:
                    _set(steps, point, "minecraft:barrel" if variant == "barrel" else "minecraft:chest")
                    menu, slots = "ChestMenu", 63
                container = slots - 36
                for index, item in enumerate(items):
                    _give(steps, item, index)
                _open(steps, point, menu, slots)
                if not inserting:
                    _insert(steps, "minecraft:diamond", 0)
                    if variant == "double":
                        _insert(steps, "minecraft:emerald", 27)
                state = steps.observe()
                stale = [0, 27] if not inserting and variant == "double" else [0] if not inserting else [
                    stack["slot"] for stack in state["container"]["slots"]
                    if stack["slot"] >= container and stack["item_id"] in items]
                steps.report["fixture_commands"].append(destroy)
                if variant == "crossdim":
                    peer.call_tool("minecraft_send_chat", {
                        "message": f"execute in minecraft:overworld run {destroy}", "command": True})
                else:
                    steps.call("minecraft_send_chat", {"message": destroy, "command": True})
                sent = []
                for slot in stale:
                    try:
                        steps.call("minecraft_quick_move_container_slot", {"slot": slot, "timeout_seconds": 3})
                        sent.append(True)
                    except (RuntimeError, TimeoutError) as error:
                        sent.append("no visible container" not in str(error))
                _echo(steps, label + "_destroy_ordered_receipt")
                _block(steps, point, "minecraft:stone" if variant == "stone" else "minecraft:air")
                state = steps.observe()
                record: dict[str, Any] = {"label": label, "click_sent": sent,
                                          "menu_closed": state["container"]["container_id"] == 0}
                carried = state["container"]["carried"]
                rows = _rows(steps, 8)
                record["ground_rows"] = [row.get("item") for row in rows if row.get("item")]
                inside: dict[str, int] = {}
                if variant == "shulker":
                    steps.call("minecraft_send_chat", {
                        "message": "data get entity @e[type=minecraft:item,limit=1,sort=nearest] Item", "command": True})
                    echoed = _echo(steps, label + "_shulker_data_receipt")
                    lines = [line for line in echoed.get("recent_chat", []) if "shulker_box" in line]
                    record["shulker_data"] = lines[:1]
                    inside["minecraft:diamond"] = int(bool(lines) and "minecraft:diamond" in lines[0])
                remaining: dict[str, int] = {}
                if variant == "double":
                    if not record["menu_closed"]:
                        steps.call("minecraft_close_screen")
                    opened = _open(steps, other, "ChestMenu", 63)
                    for item in items:
                        remaining[item] = sum(stack["count"] for stack in opened["container"]["slots"]
                                              if stack["slot"] < 27 and stack["item_id"] == item)
                    steps.call("minecraft_close_screen")
                    steps.fixture(f"setblock {o} minecraft:air")
                    state = steps.observe()
                totals = {}
                for item in items:
                    held = item_count(state, item) + (carried["count"] if carried.get("item_id") == item else 0)
                    ground = sum(row.get("item", {}).get("count", 0) for row in rows
                                 if row.get("item", {}).get("item_id") == item)
                    totals[item] = {"held": held, "ground": ground, "remaining": remaining.get(item, 0),
                                    "inside_shulker": inside.get(item, 0)}
                record["totals"] = totals
                record["conserved"] = all(sum(parts.values()) == 1 for parts in totals.values())
                outcomes.append(record)
                if attempt == 0:
                    _capture(steps, label + "-after-attack")
                steps.fixture("kill @e[type=minecraft:item]")
                steps.fixture("clear @s")
                _echo(steps, label + "_cleanup_ordered_receipt")
    finally:
        steps.fixture("tick unfreeze")
    steps.check("attack_menu_closed_every_attempt", all(entry["menu_closed"] for entry in outcomes))
    steps.check("attack_conserved_every_attempt", all(entry["conserved"] for entry in outcomes))


def _feedback_attack(steps: ProbeSteps, peer: Any) -> None:
    """BREAK_HS1_ATTACK=feedback glue (not for merge): send_command_feedback with a second operator
    (vanilla broadcastToAdmins) and with an operator in the Nether."""
    def wait_peer(text: str) -> list[str]:
        while True:
            seen = peer.call_tool("minecraft_observe")
            if any(text in line for line in seen["recent_chat"]):
                return seen["recent_chat"]
            peer.call_tool("minecraft_wait_for_state_change", {
                "observed_version": seen["state_version"], "timeout_seconds": min(steps.remaining(), 60)})

    log: list[dict[str, Any]] = []
    steps.report["feedback_attack"] = log

    def case(name: str, actor: str, commands: list[str]) -> None:
        for command in commands:
            if actor == "main":
                steps.call("minecraft_send_chat", {"message": command, "command": True})
            else:
                peer.call_tool("minecraft_send_chat", {"message": command, "command": True})
        fence = f"Fence{name}"
        if actor == "main":
            steps.call("minecraft_send_chat", {"message": f"say {fence}", "command": True})
        else:
            peer.call_tool("minecraft_send_chat", {"message": f"say {fence}", "command": True})
        main_chat = steps.wait(lambda state: any(fence in line for line in state["recent_chat"]), name + "_main")["recent_chat"]
        peer_chat = wait_peer(fence)
        log.append({"case": name, "actor": actor, "commands": commands, "main_chat": main_chat, "peer_chat": peer_chat})

    steps.fixture("gamerule send_command_feedback true")
    case("A_true_main_time", "main", ["time set 1111"])
    steps.fixture("gamerule send_command_feedback false")
    case("B_false_main_time", "main", ["time set 2222", "give @s minecraft:dirt 1"])
    case("C_false_peer_nether", "peer", ["time set 3333", "give @s minecraft:dirt 1", "weather rain"])
    case("D_false_execute_as_peer", "main", ["execute as NetherPeer run time set 4444",
                                              "execute as NetherPeer at @s run give @s minecraft:stick 1"])
    steps.fixture("gamerule send_command_feedback true")
    case("E_true_peer_nether", "peer", ["time set 5555"])
    steps.fixture("gamerule send_command_feedback false")
    case("F_false_list_peer", "peer", ["list"])


def _peer_state(peer: Any, steps: ProbeSteps, done: Any, label: str, limit: float = 60) -> dict[str, Any]:
    import time
    deadline = time.monotonic() + min(steps.remaining(), limit)
    while True:
        seen = peer.call_tool("minecraft_observe")
        if done(seen):
            return seen
        left = deadline - time.monotonic()
        if left <= 0:
            raise TimeoutError(f"{label}: peer state not reached")
        peer.call_tool("minecraft_wait_for_state_change", {
            "observed_version": seen["state_version"], "timeout_seconds": max(1.0, min(left, 120.0))})


def _hs1_dynamic(steps: ProbeSteps, origin: tuple[int, int, int], peer: Any, variant: str) -> None:
    """twoview: a second viewer clicks too (overworld, non-shared path); tnt: an explosion destroys the
    open chest during an ordinary tick while the viewer clicks."""
    point = _point(origin)
    p = f"{point['x']} {point['y']} {point['z']}"
    diamond = "minecraft:diamond"
    outcomes: list[dict[str, Any]] = []
    steps.report["attack"] = {"variant": variant, "outcomes": outcomes}
    if variant == "twoview":
        steps.fixture(f"execute in minecraft:overworld run tp NetherPeer {point['x'] + 2} {point['y']} {point['z']}")
        _peer_state(peer, steps, lambda s: s.get("dimension") == "minecraft:overworld" and s.get("in_play") is True,
                    "peer_in_overworld")
        pass
    else:
        steps.fixture("gamemode creative @s")
    _echo(steps, "dynamic_setup_receipt")
    try:
        for attempt in range(3):
            label = f"{variant}_{attempt}"
            _set(steps, point, "minecraft:chest")
            _give(steps, diamond)
            if variant == "twoview":
                peer.call_tool("minecraft_set_hotbar_slot", {"slot": 8})
                peer.call_tool("minecraft_use_item_on", {**point, "face": "up", "hand": "main_hand"})
                try:
                    _peer_state(peer, steps, lambda s: s["container"]["container_id"] != 0, "peer_opened", 20)
                except TimeoutError:
                    seen = peer.call_tool("minecraft_observe")
                    steps.report["attack"]["peer_open_failed"] = {
                        "peer": {k: seen["player"].get(k) for k in ("x", "y", "z", "yaw", "pitch")},
                        "dimension": seen.get("dimension"), "container": seen["container"].get("container_id")}
                    raise
            _open(steps, point, "ChestMenu", 63)
            _insert(steps, diamond, 0)
            if variant == "twoview":
                destroy = f"setblock {p} minecraft:air destroy"
            else:
                # `{fuse:0}` is refused by this server ("Unknown or unsupported summon NBT field fuse",
                # tnt-probe-1); the default 80-tick fuse explodes in an ordinary tick.
                destroy = f"summon minecraft:tnt {point['x'] + 0.5} {point['y']} {point['z'] + 0.5}"
            steps.report["fixture_commands"].append(destroy)
            steps.call("minecraft_send_chat", {"message": destroy, "command": True})
            if variant == "tnt":
                _block(steps, point, "minecraft:air")
            sent = []
            for actor in ([steps.client, peer] if variant == "twoview" else [steps.client]):
                try:
                    actor.call_tool("minecraft_quick_move_container_slot", {"slot": 0, "timeout_seconds": 3})
                    sent.append(True)
                except Exception as error:  # noqa: BLE001 - refusal after closure is expected
                    sent.append("no visible container" not in str(error))
            _block(steps, point, "minecraft:air")
            _echo(steps, label + "_destroyed_receipt")
            main = steps.observe()
            rows = _rows(steps, 16)
            ground = sum(row.get("item", {}).get("count", 0) for row in rows
                         if row.get("item", {}).get("item_id") == diamond)
            record: dict[str, Any] = {"label": label, "click_sent": sent,
                                      "main_menu_closed": main["container"]["container_id"] == 0,
                                      "main_held": item_count(main, diamond) + (main["container"]["carried"]["count"]
                                      if main["container"]["carried"].get("item_id") == diamond else 0),
                                      "ground": ground, "ground_rows": [row.get("item") for row in rows if row.get("item")]}
            if variant == "twoview":
                other = peer.call_tool("minecraft_observe")
                record["peer_menu_closed"] = other["container"]["container_id"] == 0
                record["peer_held"] = item_count(other, diamond) + (other["container"]["carried"]["count"]
                                      if other["container"]["carried"].get("item_id") == diamond else 0)
            record["conserved"] = record["main_held"] + record.get("peer_held", 0) + ground == 1
            outcomes.append(record)
            if attempt == 0:
                _capture(steps, label + "-after")
            steps.fixture("kill @e[type=minecraft:item]")
            steps.fixture("clear @s")
            if variant == "twoview":
                steps.fixture("clear NetherPeer")
            else:
                x, y, z = origin
                steps.fixture(f"fill {x - 5} {y - 1} {z - 4} {x + 5} {y - 1} {z + 6} minecraft:obsidian")
            _echo(steps, label + "_cleanup_receipt")
    finally:
        if variant == "twoview":
            steps.fixture("tick unfreeze")
    steps.check("dynamic_conserved_every_attempt", all(entry["conserved"] for entry in outcomes))
