"""Ledger CONTAINERS-001: a chested donkey's cargo menu opened and used by the client.

The row says the chest-equipped donkey/mule cargo menu is absent.  This scenario
attaches a chest with the client's own interaction, opens the cargo menu with a
sneaking use, and moves seven diamonds in and out through the received menu — all
with the ordinary client tools (the registered custody scenario needs an offline
export/reimport hook the client does not implement).

Tools used: wait_for_visible_entity, approach_entity, set_hotbar_slot,
interact_entity, press_inputs, quick_move_container_slot, wait_for_inventory.
"""
from __future__ import annotations

from typing import Any

from .probe_steps import ProbeSteps, container_slot, item_count

DONKEY = "minecraft:donkey"
CHEST = "minecraft:chest"
DIAMOND = "minecraft:diamond"
MENU = "net.minecraft.world.inventory.HorseInventoryMenu"
# 2 mount slots + 15 cargo slots + 36 player slots.
MENU_SLOTS = 53
CARGO_FIRST = 2
PLAYER_FIRST = 17


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    steps.fixture("gamerule send_command_feedback true")
    steps.fixture(f"summon {DONKEY} {x + 1.5} {y} {z + 2.5} {{Tame:1b,NoAI:1b}}")
    donkey = steps.call("minecraft_wait_for_visible_entity", {
        "entity_type": DONKEY, "radius": 12, "timeout_seconds": steps.remaining()})
    identity = {key: donkey[key] for key in ("entity_id", "entity_uuid", "entity_type")}
    steps.report["donkey"] = identity
    steps.call("minecraft_approach_entity", {
        "entity_id": identity["entity_id"], "timeout_seconds": steps.remaining()})
    steps.call("minecraft_wait_ticks", {"ticks": 10})

    # The client's own interaction attaches the chest and debits it.
    steps.fixture(f"debug give {CHEST} 1 0")
    steps.call("minecraft_wait_for_inventory", {
        "item_id": CHEST, "count": 1, "timeout_seconds": steps.remaining()})
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    steps.call("minecraft_interact_entity", identity)
    steps.wait(lambda state: item_count(state, CHEST) == 0, "chest_attached_and_debited")

    # Opening the cargo menu: sneak + use is the ordinary mount-menu input.
    steps.call("minecraft_set_hotbar_slot", {"slot": 8})
    steps.call("minecraft_press_inputs", {"keys": ["sneak", "use"], "ticks": 1})
    state = steps.wait(
        lambda state: state["container"]["container_id"] != 0
        and state["container"]["menu_class"] == MENU,
        "chested_donkey_cargo_menu_received",
    )
    steps.check("cargo_menu_has_53_slots", state["container"]["slot_count"] == MENU_SLOTS)

    # Deposit seven diamonds into the first cargo slot through the menu.
    steps.fixture(f"debug give {DIAMOND} 7 1")
    steps.call("minecraft_wait_for_inventory", {
        "item_id": DIAMOND, "count": 7, "timeout_seconds": steps.remaining()})
    steps.call("minecraft_set_hotbar_slot", {"slot": 1})
    state = steps.observe()
    sources = [stack for stack in state["container"]["slots"]
               if stack["slot"] >= PLAYER_FIRST and stack["item_id"] == DIAMOND
               and stack["count"] == 7]
    steps.check("one_diamond_stack_in_the_player_part", len(sources) == 1)
    steps.call("minecraft_quick_move_container_slot", {
        "slot": sources[0]["slot"], "timeout_seconds": steps.remaining()})
    state = steps.wait(
        lambda state: container_slot(state, CARGO_FIRST)["item_id"] == DIAMOND
        and container_slot(state, CARGO_FIRST)["count"] == 7
        and item_count(state, DIAMOND) == 0,
        "seven_diamonds_into_the_cargo",
    )
    steps.check("deposit_conserves_seven", container_slot(state, CARGO_FIRST)["count"] == 7)

    # Withdraw the same stack; the cargo slot empties and the player holds seven.
    steps.call("minecraft_quick_move_container_slot", {
        "slot": CARGO_FIRST, "timeout_seconds": steps.remaining()})
    state = steps.wait(
        lambda state: container_slot(state, CARGO_FIRST)["count"] == 0
        and item_count(state, DIAMOND) == 7,
        "seven_diamonds_back_in_the_inventory",
    )
    steps.check("withdrawal_conserves_seven", item_count(state, DIAMOND) == 7)
    steps.call("minecraft_close_screen")
    steps.report["limits"] = [
        "One tamed NoAI donkey and one chest; the mule comparison, death drop and llama columns rest on "
        "the cargo test. No offline export/reimport (the registered custody scenario needs a client hook "
        "that does not exist).",
    ]
