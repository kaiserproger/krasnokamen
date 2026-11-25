"""Ledger SUMMON-ITEMS-HORSE-CARGO: a summoned chested mount carries its vanilla `Items`.

Vanilla `AbstractHorse`/`ChestedHorse` keeps a chested horse, donkey, mule or llama cargo in
`Items` beside `ChestedHorse` (and the llama's `Strength`). The summon field set used to
answer "Entity does not support summon NBT field Items" — or `ChestedHorse` — for every
mount, so a saved or summoned mount could never be brought back with its cargo.

The scenario summons a chested donkey that already carries three diamonds and one apple in
the authored slots, then opens the donkey's own cargo menu with the client (sneak + use,
the same input `native-v2-ledger-donkey-cargo` uses) and asserts the received menu: 53
slots, three diamonds in the first cargo slot, one apple in the last cargo column's slot.
The verdicts are assertions on the menu the server sent, so a refusal (the pre-fix
behaviour) fails them instead of timing out.

Control: the fix is `ee71ee684`; its non-test hunks (the `supports` predicate and the
`prepare` mount-cargo block) reverse on that commit and the summon then refuses the field.
"""
from __future__ import annotations

from .native_p1 import _aim
from .probe_steps import ProbeSteps, container_slot

DONKEY = "minecraft:donkey"
DIAMOND = "minecraft:diamond"
APPLE = "minecraft:apple"
MENU = "net.minecraft.world.inventory.HorseInventoryMenu"
MENU_SLOTS = 53
CARGO_FIRST = 2
# A donkey's cargo is five columns of three: storage slot 14 is the last column's third row.
LAST_CARGO = CARGO_FIRST + 14


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    steps.fixture("gamerule send_command_feedback true")
    steps.fixture(
        f"summon {DONKEY} {x + 1.5} {y} {z + 2.5} "
        '{Tame:1b,NoAI:1b,ChestedHorse:1b,'
        'Items:[{Slot:0b,id:"minecraft:diamond",count:3},'
        '{Slot:14b,id:"minecraft:apple",count:1}]}')
    donkey = steps.call("minecraft_wait_for_visible_entity", {
        "entity_type": DONKEY, "radius": 12, "timeout_seconds": steps.remaining()})
    identity = {key: donkey[key] for key in ("entity_id", "entity_uuid", "entity_type")}
    steps.report["donkey"] = identity
    steps.call("minecraft_approach_entity", {
        "entity_id": identity["entity_id"], "timeout_seconds": steps.remaining()})
    steps.call("minecraft_wait_ticks", {"ticks": 10})

    # Sneak + use is the ordinary mount-menu input, but only when the donkey is in the
    # crosshair: aim at its body centre first (a plain approach does not turn the view).
    _aim(steps, {**identity, **donkey})
    steps.call("minecraft_set_hotbar_slot", {"slot": 8})
    steps.call("minecraft_press_inputs", {"keys": ["sneak", "use"], "ticks": 1})
    state = steps.wait(
        lambda state: state["container"]["container_id"] != 0
        and state["container"]["menu_class"] == MENU,
        "chested_donkey_cargo_menu_received",
        timeout_seconds=30,
    )
    steps.report["chat_after_use"] = [
        line for line in state["recent_chat"] if isinstance(line, str)][:8]
    steps.check("cargo_menu_has_53_slots", state["container"]["slot_count"] == MENU_SLOTS)
    first = container_slot(state, CARGO_FIRST)
    last = container_slot(state, LAST_CARGO)
    steps.report["authored_cargo"] = {"first": first, "last": last}
    steps.check("summoned_items_land_in_the_first_cargo_slot",
                first["item_id"] == DIAMOND and first["count"] == 3)
    steps.check("summoned_items_land_in_the_last_cargo_column",
                last["item_id"] == APPLE and last["count"] == 1)
    # Vanilla stores equipment separately from cargo: the saddle slot stays empty.
    steps.check("cargo_summon_does_not_fill_the_saddle_slot",
                container_slot(state, 0)["item_id"] == "minecraft:air")
    steps.call("minecraft_close_screen")

    steps.report["limits"] = [
        "One summoned chested donkey with two authored cargo slots, read through the client's own "
        "cargo menu; no chest attach, no deposit/withdraw, no mule or llama column check (core test "
        "covers Strength) and no export/reimport.",
    ]
