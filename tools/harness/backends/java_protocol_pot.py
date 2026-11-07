"""Physical creative-inventory echoes of a lossy decorated-pot projection."""
from __future__ import annotations

import json
import uuid
from pathlib import Path
from typing import Any

from . import java_p5
from .light_wire import Reader
from .menu_observer import holds, observe_menu
from .native_craft_drag import Pointer
from .native_promotion_wire import read_nbt
from .probe_steps import ProbeSteps
from .village_autonomy_qa import _snbt_fields

SCENARIO = "java-protocol-pot-creative-echo"
REQUIRED_TOOLS = frozenset({
    "minecraft_observe", "minecraft_wait_for_state_change", "minecraft_send_chat",
    "minecraft_open_inventory", "minecraft_close_screen", "minecraft_screenshot",
    "minecraft_wait_for_inventory",
})
POT = "minecraft:decorated_pot"


def creative_packet_observer(report_path: Path, artifact: Path):
    """Retain the real client's creative writes, without changing its stream."""
    report = json.loads(report_path.read_text())
    creative_id = report["play"]["serverbound"]["minecraft:set_creative_mode_slot"]["protocol_id"]
    events: list[dict[str, Any]] = []

    def observe(_wire, phase: str, packet_id: int, payload: bytes) -> None:
        if phase == "play" and packet_id == creative_id:
            events.append({"slot": int.from_bytes(payload[:2], "big", signed=True),
                           "payload_hex": payload.hex()})
            (artifact / "creative-wire.json").write_text(json.dumps(events, indent=2) + "\n")

    return observe


def dropped_pot(steps: ProbeSteps) -> dict[str, Any]:
    """Query the real dropped entity; no inventory fixture supplies this result."""
    marker = "pot-drop-" + uuid.uuid4().hex
    steps.call("minecraft_send_chat", {"message": marker, "command": False})
    steps.wait(lambda state: any(marker in line for line in state["recent_chat"]),
               "drop_query_fence")
    steps.call("minecraft_send_chat", {
        "message": "data get entity @e[type=minecraft:item,limit=1,sort=nearest] Item",
        "command": True,
    })

    def reply(state):
        lines = state["recent_chat"]
        boundary = next((i for i, line in enumerate(lines) if marker in line), None)
        return boundary is not None and any("has the following entity data:" in line
                                           for line in lines[:boundary])

    state = steps.wait(reply, "dropped_entity_data")
    raw = next(line for line in state["recent_chat"] if "has the following entity data:" in line)
    steps.report["dropped_pot_snbt"] = raw
    return _snbt_fields(raw[raw.index("{"):])


def run(client: Any, artifact: Path, timeout: float) -> dict[str, Any]:
    with ProbeSteps(client, artifact / SCENARIO, timeout, SCENARIO) as steps:
        steps.fixture("gamemode creative @s")
        steps.fixture("clear @s")
        steps.fixture('debug give minecraft:decorated_pot['
                      'minecraft:pot_decorations={back:{id:"minecraft:brick",count:3,'
                      'components:{"minecraft:custom_name":{text:"Nested Protocol Pot"}}}},'
                      'minecraft:custom_name={text:"Protocol Pot"}] 1 0')
        steps.call("minecraft_wait_for_inventory", {
            "item_id": POT, "count": 1, "timeout_seconds": steps.remaining(),
        })
        initial = observe_menu(steps, "before_creative_pickup")
        canonical = initial["slots"][36]
        steps.check("one_authoritative_fixture_pot", holds(canonical, POT, 1))
        components = canonical["components"]
        steps.check("fixture_outer_custom_name",
                    components["custom_name"] == "Protocol Pot")
        preserved = components["components"]["preserved_components"]
        decorations = next(row for row in preserved if row["name"] == "minecraft:pot_decorations")
        reader = Reader(bytes(decorations["value"]))
        templates = read_nbt(reader)
        steps.check("fixture_missing_sides_nested_count_and_name", templates == {
            "back": {"id": "minecraft:brick", "count": 3,
                     "components": {"minecraft:custom_name": {"text": "Nested Protocol Pot"}}},
        })
        steps.report["canonical_pot"] = canonical
        steps.call("minecraft_open_inventory")
        opened = steps.wait(lambda state: state["screen"]["class"].endswith(".CreativeModeInventoryScreen"),
                            "actual_creative_screen_open")
        steps.check("empty_initial_client_cursor", opened["container"]["carried"]["count"] == 0)
        before = Path(java_p5.capture(steps, "protocol-pot-before"))
        from PIL import Image
        with Image.open(before) as image:
            width, height = image.size
        # Local 26.1.2 javap: CreativeModeInventoryScreen's image is 195x136;
        # ItemPickerMenu.addInventoryHotbarSlots(inventory, 9, 112), stride 18.
        # The shared Java runner fixes guiScale=2 before opening this screen.
        scale = 2
        left, top = (width - 195 * scale) // 2, (height - 136 * scale) // 2
        pointer = Pointer(client.p4_window)
        try:
            for source, target in ((0, 1), (1, 0)):
                label = f"hotbar_{source}_to_{target}"
                pointer.move(left + (17 + 18 * source) * scale, top + 120 * scale)
                pointer.button(True)
                steps.wait(lambda state: state["container"]["carried"].get("item_id") == POT
                           and state["container"]["carried"]["count"] == 1
                           and not any(row["slot"] == source for row in state["inventory"]),
                           label + "_physical_pickup")
                pointer.button(False)
                picked = observe_menu(steps, label + "_picked_up")
                steps.check(label + "_server_source_empty", picked["slots"][36 + source]["count"] == 0)
                java_p5.capture(steps, "protocol-pot-" + label + "-carried")
                pointer.move(left + (17 + 18 * target) * scale, top + 120 * scale)
                pointer.button(True)
                pointer.button(False)
                steps.wait(lambda state: state["container"]["carried"]["count"] == 0
                           and any(row["slot"] == target and row["item_id"] == POT
                                   for row in state["inventory"]),
                           label + "_physical_return")
                returned = observe_menu(steps, label + "_returned")
                java_p5.capture(steps, "protocol-pot-" + label + "-returned")
                steps.check(label + "_authoritative_components_unchanged",
                            returned["slots"][36 + target] == canonical)
                steps.check(label + "_only_one_authoritative_pot",
                            sum(row["count"] for row in returned["slots"] if row["item"] == POT) == 1
                            and returned["carried"]["count"] == 0)
            # Drop the physically carried projection outside the native menu.
            pointer.move(left + 17 * scale, top + 120 * scale)
            pointer.button(True)
            steps.wait(lambda state: state["container"]["carried"]["count"] == 1
                       and not any(row["slot"] == 0 for row in state["inventory"]),
                       "drop_physical_pickup")
            pointer.button(False)
            pointer.move(25, 25)
            pointer.button(True)
            pointer.button(False)
            steps.wait(lambda state: state["container"]["carried"]["count"] == 0,
                       "physical_outside_drop")
            dropped = dropped_pot(steps)
            java_p5.capture(steps, "protocol-pot-dropped")
            dropped_components = _snbt_fields(dropped["components"])
            dropped_sides = _snbt_fields(dropped_components["minecraft:pot_decorations"])
            back = _snbt_fields(dropped_sides["back"])
            nested_components = _snbt_fields(back["components"])
            nested_name = _snbt_fields(nested_components["minecraft:custom_name"])
            steps.check("drop_restores_canonical_nested_stack",
                        json.loads(dropped["id"]) == POT and dropped["count"] == "1"
                        and set(dropped_sides) == {"back"}
                        and json.loads(back["id"]) == "minecraft:brick" and back["count"] == "3"
                        and json.loads(nested_name["text"]) == "Nested Protocol Pot")
            # The same old-client projection without hidden nested data must
            # not reuse the consumed drop provenance during creative echoes.
            steps.fixture('debug give minecraft:decorated_pot['
                          'minecraft:custom_name={text:"Protocol Pot"}] 1 0')
            steps.call("minecraft_wait_for_inventory", {
                "item_id": POT, "count": 1, "timeout_seconds": steps.remaining(),
            })
            plain = observe_menu(steps, "plain_pot_after_drop")["slots"][36]
            steps.check("drop_does_not_restore_stale_nested_components",
                        holds(plain, POT, 1) and plain["components"]["custom_name"] == "Protocol Pot"
                        and plain != canonical)
            # Re-seed only the next branch, then close with the real cursor full.
            steps.fixture('debug give minecraft:decorated_pot['
                          'minecraft:pot_decorations={back:{id:"minecraft:brick",count:3,'
                          'components:{"minecraft:custom_name":{text:"Nested Protocol Pot"}}}},'
                          'minecraft:custom_name={text:"Protocol Pot"}] 1 0')
            steps.wait(lambda state: any(row["slot"] == 0 and row["item_id"] == POT
                                        for row in state["inventory"]), "close_fixture_received")
            before_close = observe_menu(steps, "before_close_with_carried")
            steps.check("close_fixture_canonical_received", before_close["slots"][36] == canonical)
            pointer.move(left + 17 * scale, top + 120 * scale)
            pointer.button(True)
            steps.wait(lambda state: state["container"]["carried"]["count"] == 1
                       and not any(row["slot"] == 0 for row in state["inventory"]),
                       "close_physical_pickup")
            pointer.button(False)
            java_p5.capture(steps, "protocol-pot-close-carried")
            steps.call("minecraft_close_screen")
            closed = observe_menu(steps, "after_creative_close")
            steps.check("creative_close_does_not_invent_server_cursor_custody",
                        closed["slots"][36]["count"] == 0 and closed["carried"]["count"] == 0)
            steps.fixture('debug give minecraft:decorated_pot['
                          'minecraft:custom_name={text:"Protocol Pot"}] 1 0')
            steps.call("minecraft_wait_for_inventory", {
                "item_id": POT, "count": 1, "timeout_seconds": steps.remaining(),
            })
            steps.call("minecraft_open_inventory")
            steps.wait(lambda state: state["screen"]["class"].endswith(".CreativeModeInventoryScreen"),
                       "reopen_after_carried_close")
            reopened = observe_menu(steps, "plain_pot_after_close")
            steps.check("close_does_not_restore_stale_nested_components",
                        reopened["slots"][36] == plain and reopened["carried"]["count"] == 0)
            steps.call("minecraft_close_screen")
            java_p5.capture(steps, "protocol-pot-after-close")
        finally:
            pointer.close()
    return steps.report
