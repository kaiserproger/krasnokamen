"""Physical creative-inventory echoes of a lossy decorated-pot projection."""
from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from . import java_p5
from .light_wire import Reader
from .menu_observer import holds, observe_menu
from .native_craft_drag import Pointer
from .native_promotion_wire import read_nbt
from .probe_steps import ProbeSteps

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
            steps.call("minecraft_close_screen")
            closed = observe_menu(steps, "after_creative_close")
            steps.check("close_keeps_authoritative_components", closed["slots"][36] == canonical)
            java_p5.capture(steps, "protocol-pot-after-close")
        finally:
            pointer.close()
    return steps.report
