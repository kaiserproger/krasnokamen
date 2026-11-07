"""Native swap-hands, milk-cow and keep-inventory custody restart probes.

Operator fixture setup is explicit. Ordinary actions and received conservation
checks are separate; only canonical runtime receipts establish a passed cell.
No sequential-Q, partial-inventory or air-use-only armor subset substitutes for
the complete matrix subjects. Unsupported cases fail closed.
"""
from __future__ import annotations

import math
import uuid
from typing import Any

from .probe_steps import item_count

SUBJECTS = frozenset({
    "inventory.swap_hands",
    "death_respawn.keep_inventory_true",
    "farming.milk_cow",
})


def _name(lane: Any) -> str:
    return lane.names[0]


def _fence(steps: Any, label: str) -> Any:
    """Same-connection chat echo proving the server ordered prior commands."""
    marker = "custody-fence-" + uuid.uuid4().hex
    steps.call("minecraft_send_chat", {"message": marker, "command": False})
    return steps.wait(
        lambda state: any(marker in line for line in state.get("recent_chat", [])),
        label,
    )


def _empty(state: Any) -> bool:
    return all(stack.get("count") == 0 for stack in state.get("inventory", [])
               if stack.get("slot") not in (41, 42))


def _multiset(state: Any) -> dict[str, int]:
    totals: dict[str, int] = {}
    for stack in state.get("inventory", []):
        if stack.get("slot") in (41, 42):
            continue
        if stack.get("item_id") is None or stack.get("count") is None:
            raise RuntimeError("ordinary player inventory remains unknown")
        item_id = stack["item_id"]
        totals[item_id] = totals.get(item_id, 0) + int(stack["count"])
    return totals


def _slot(state: Any, slot: int) -> Any | None:
    return next(
        (stack for stack in state.get("inventory", []) if stack.get("slot") == slot),
        None,
    )


def _origin(state: Any) -> tuple[int, int, int]:
    player = state["player"]
    return (
        math.floor(player["x"]),
        math.floor(player["y"]),
        math.floor(player["z"]),
    )


def _select(steps: Any, item_id: str, count: int) -> None:
    steps.call(
        "minecraft_select_hotbar_item",
        {"item_id": item_id, "count": count, "timeout_seconds": steps.remaining()},
    )


def _respawn_alive(steps: Any) -> Any:
    respawn = steps.call("minecraft_respawn", {"timeout_seconds": steps.remaining()})
    steps.report["respawn_result"] = respawn
    alive = steps.call("minecraft_observe")
    while not (
        alive.get("in_play") is True
        and (alive.get("player") or {}).get("health") is not None
        and alive["player"]["health"] > 0
        and alive.get("screen", {}).get("open") is False
    ):
        steps.call(
            "minecraft_wait_for_state_change",
            {
                "observed_version": alive["state_version"],
                "timeout_seconds": steps.remaining(),
            },
        )
        alive = steps.call("minecraft_observe")
    steps.check("respawn_alive_screen_closed", True)
    return alive


def _grant_levels(lane: Any, steps: Any, levels: int) -> None:
    name = _name(lane)
    lane.console(f"xp add {name} {levels} levels")
    state = steps.observe()
    if (state.get("player") or {}).get("experience_level") != levels:
        _fence(steps, "xp_grant_ordered")
        state = steps.observe()
    if (state.get("player") or {}).get("experience_level") != levels:
        raise NotImplementedError(
            "xp_grant_console: server console 'xp add <player> <n> levels' produced "
            f"no {levels} receiving level; XP-grant fixture unavailable"
        )
    steps.check("fixture_xp_levels_received", True)


def _no_ground_copy(steps: Any, items: set[str], label: str) -> None:
    rows=steps.call('minecraft_list_entities',{'radius':32,'limit':512})
    steps.check(label,not rows['truncated'] and not any(
        row.get('item',{}).get('item_id') in items for row in rows['entities']))


def _swap_hands(lane: Any, scenario: dict[str, Any]) -> None:
    _ = scenario
    steps = lane.steps
    name = _name(lane)
    lane.console(f"clear {name}")
    steps.wait(_empty, "fixture_inventory_empty")
    lane.console(f"give {name} minecraft:torch 1")
    steps.wait(lambda s: item_count(s, "minecraft:torch") == 1, "fixture_torch_received")
    _select(steps, "minecraft:torch", 1)
    steps.call("minecraft_press_inputs", {"keys": ["swap_offhand"], "ticks": 1})
    steps.wait(
        lambda s: any(
            stack.get("slot") == 40 and stack.get("item_id") == "minecraft:torch"
            for stack in s.get("inventory", [])
        ),
        "torch_rests_in_offhand",
    )
    lane.console(f"give {name} minecraft:stick 1")
    steps.wait(lambda s: item_count(s, "minecraft:stick") == 1, "fixture_stick_received")
    _select(steps, "minecraft:stick", 1)
    steps.call("minecraft_press_inputs", {"keys": ["swap_offhand"], "ticks": 1})
    swapped = steps.wait(
        lambda s: s.get("selected_item", {}).get("item_id") == "minecraft:torch"
        and any(
            stack.get("slot") == 40 and stack.get("item_id") == "minecraft:stick"
            for stack in s.get("inventory", [])
        ),
        "hands_swapped_reference",
    )
    steps.check("main_holds_torch", True)
    steps.check("offhand_holds_stick", True)
    pre = _multiset(swapped)
    steps.check(
        "pre_restart_exact_pair",
        pre.get("minecraft:torch") == 1 and pre.get("minecraft:stick") == 1,
    )
    lane.custody_boundary()
    state = steps.observe(wait_for_play=True)
    steps.check(
        "post_restart_main_still_torch",
        state.get("selected_item", {}).get("item_id") == "minecraft:torch",
    )
    off = _slot(state, 40)
    steps.check(
        "post_restart_offhand_still_stick",
        off is not None
        and off.get("item_id") == "minecraft:stick"
        and int(off.get("count", 0)) == 1,
    )
    steps.check("conservation_exact_across_restart", _multiset(state) == pre)
    _no_ground_copy(steps,{'minecraft:torch','minecraft:stick'},'no_ground_hand_copy_after_restart')








def _interact(lane: Any, steps: Any, identity: dict[str, Any]) -> None:
    keys = ("entity_id", "entity_uuid", "entity_type")
    target = {key: identity[key] for key in keys}
    approach = steps.call(
        "minecraft_approach_entity",
        {"entity_id": target["entity_id"], "timeout_seconds": steps.remaining()},
    )
    steps.check("fixture_mob_in_reach", approach.get("in_reach") is True)
    steps.call("minecraft_interact_entity", {**target, "hand": "main_hand"})


def _milk_cow(lane: Any, scenario: dict[str, Any]) -> None:
    steps = lane.steps
    name = _name(lane)
    lane.console(f"clear {name}")
    steps.wait(_empty, "fixture_inventory_empty")
    bx, by, bz = _origin(steps.observe())
    lane.console(f"summon minecraft:cow {bx + 2.5} {by} {bz + 0.5}")
    lane.console(f"summon minecraft:mooshroom {bx - 2.5} {by} {bz + 0.5}")
    cow = steps.call(
        "minecraft_wait_for_visible_entity",
        {"entity_type": "minecraft:cow", "radius": 16, "timeout_seconds": steps.remaining()},
    )
    steps.report["cow"] = {k: cow[k] for k in ("entity_id", "entity_uuid", "entity_type")}
    shroom = steps.call(
        "minecraft_wait_for_visible_entity",
        {
            "entity_type": "minecraft:mooshroom",
            "radius": 16,
            "timeout_seconds": steps.remaining(),
        },
    )
    steps.report["mooshroom"] = {
        k: shroom[k] for k in ("entity_id", "entity_uuid", "entity_type")
    }
    lane.console(f"give {name} minecraft:bucket 2")
    steps.wait(lambda s: item_count(s, "minecraft:bucket") == 2, "fixture_buckets_received")
    lane.console(f"give {name} minecraft:bowl 1")
    steps.wait(lambda s: item_count(s, "minecraft:bowl") == 1, "fixture_bowl_received")
    _select(steps, "minecraft:bucket", 2)
    _interact(lane, steps, cow)
    steps.wait(lambda s: item_count(s, "minecraft:milk_bucket") == 1, "cow_milked_once")
    steps.check("first_bucket_consumed", item_count(steps.observe(), "minecraft:bucket") == 1)
    _select(steps, "minecraft:bucket", 1)
    _interact(lane, steps, shroom)
    steps.wait(lambda s: item_count(s, "minecraft:milk_bucket") == 2, "mooshroom_milked_once")
    steps.check("second_bucket_consumed", item_count(steps.observe(), "minecraft:bucket") == 0)
    _select(steps, "minecraft:bowl", 1)
    _interact(lane, steps, shroom)
    steps.wait(
        lambda s: item_count(s, "minecraft:mushroom_stew") == 1, "mooshroom_bowled_stew"
    )
    steps.check("bowl_consumed", item_count(steps.observe(), "minecraft:bowl") == 0)
    pre = _multiset(steps.observe())
    steps.check(
        "pre_restart_dairy_exact",
        pre.get("minecraft:milk_bucket") == 2
        and pre.get("minecraft:mushroom_stew") == 1,
    )
    lane.custody_boundary()
    state = steps.observe(wait_for_play=True)
    steps.check(
        "post_restart_dairy_exact",
        item_count(state, "minecraft:milk_bucket") == 2
        and item_count(state, "minecraft:mushroom_stew") == 1
        and item_count(state, "minecraft:bucket") == 0
        and item_count(state, "minecraft:bowl") == 0,
    )
    steps.check("conservation_exact_across_restart", _multiset(state) == pre)
    _no_ground_copy(steps,{'minecraft:milk_bucket','minecraft:mushroom_stew'},
                    'no_ground_dairy_copy_after_restart')


def _die_and_record_drops(lane: Any, steps: Any, drop_ids: list[str]) -> tuple[int, int, int]:
    name = _name(lane)
    lane.console(f"kill {name}")
    steps.call(
        "minecraft_wait_for_health_below",
        {"health": 0.001, "timeout_seconds": steps.remaining()},
    )
    grave = steps.observe()
    dx, dy, dz = _origin(grave)
    for item_id in drop_ids:
        steps.call(
            "minecraft_wait_for_visible_item",
            {
                "item_id": item_id,
                "x": dx,
                "y": dy,
                "z": dz,
                "timeout_seconds": steps.remaining(),
            },
        )
    steps.check("death_drops_at_death_spot", True)
    return dx, dy, dz




def _keep_inventory_true(lane: Any, scenario: dict[str, Any]) -> None:
    steps = lane.steps
    name = _name(lane)
    lane.console("gamerule keepInventory true")
    lane.console(f"clear {name}")
    steps.wait(_empty, "fixture_inventory_empty")
    lane.console(f"give {name} minecraft:diamond 2")
    steps.wait(lambda s: item_count(s, "minecraft:diamond") == 2, "fixture_diamonds_received")
    _grant_levels(lane, steps, 20)
    pre = _multiset(steps.observe())
    dx, dy, dz = _die_and_record_drops(lane, steps, [])
    lane.custody_boundary()
    steps.observe(wait_for_play=True)
    alive = _respawn_alive(steps)
    steps.check(
        "inventory_kept_across_death",
        item_count(alive, "minecraft:diamond") == 2,
    )
    steps.check(
        "levels_kept_across_death",
        (alive.get("player") or {}).get("experience_level") == 20,
    )
    steps.check("conservation_exact_across_restart", _multiset(alive) == pre)
    _ = _origin(alive)  # respawn point recorded in receipt; oracle fences the death spot
    steps.call(
        "minecraft_wait_for_no_visible_item",
        {
            "item_id": "minecraft:diamond",
            "x": dx,
            "y": dy,
            "z": dz,
            "timeout_seconds": steps.remaining(),
        },
    )
    steps.check("nothing_dropped", True)






_HANDLERS = {
    "inventory.swap_hands": _swap_hands,
    "death_respawn.keep_inventory_true": _keep_inventory_true,
    "farming.milk_cow": _milk_cow,
}


def run(lane: Any, scenario: dict[str, Any]) -> None:
    """Run one custody scenario; see module docstring for scope and blockers."""
    subject = scenario.get("subject", "")
    if subject not in SUBJECTS:
        raise NotImplementedError(f"uncatalogued_subject: no custody probe for {subject!r}")
    context = scenario.get("context", {})
    count = context.get("player_count", "1")
    if count != "1":
        raise NotImplementedError(
            f"multi_client_{count}: same-tick contention needs extra actors; "
            "shared multi-client probe owns it"
        )
    mode = context.get("game_mode", "survival")
    if mode != "survival":
        raise NotImplementedError(
            f"game_mode_{mode}: refusal/creative oracle differs from the survival "
            "custody conservation probe"
        )
    lane.steps.wait(lambda state: state["player"]["on_ground"], "initial_grounded")
    network = context.get("network_condition", "normal")
    if network in {"slow_reader_backpressure", "malformed_or_stale_packet"}:
        raise NotImplementedError(
            f"{network}: no socket-reader control/raw-packet injection in ordinary "
            "native MCP"
        )
    second = ""
    steps_list = scenario.get("steps", [])
    if len(steps_list) > 1:
        second = steps_list[1]
    if "Two players" in second:
        raise NotImplementedError(
            "multi_client_same_tick: two-actor same-tick contention needs a second "
            "actor; shared multi-client probe owns it"
        )
    if "Destroy the holder" in second:
        raise NotImplementedError(
            "destroy_during_interaction: tick-synchronized holder destruction is "
            "not exposed by ordinary native MCP"
        )
    handler = _HANDLERS.get(subject)
    if handler is None:
        raise NotImplementedError(f"uncatalogued_subject: no custody probe for {subject!r}")
    handler(lane, scenario)
