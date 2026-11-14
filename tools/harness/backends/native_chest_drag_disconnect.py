"""Physical chest drag and disconnect while the authoritative cursor holds an item."""
from pathlib import Path

from . import native_client
from .menu_observer import holds, observe_menu
from .native_craft_drag import Pointer, center
from .native_v2 import _echo, _give, _open, _point, _set
from .probe_steps import ProbeSteps, item_count


def run(steps: ProbeSteps, origin: tuple[int, int, int], artifact: Path) -> None:
    window = getattr(steps.client, "p4_window", None)
    address = getattr(steps.client, "server_address", None)
    if not window or not address:
        raise RuntimeError("chest drag requires a harness-owned window and reconnect address")
    point = _point(origin)
    _set(steps, point, "minecraft:chest")
    _give(steps, "minecraft:oak_log", 0)
    _echo(steps, "chest_drag_fixture_ordered")
    state = _open(steps, point, "ChestMenu", 63)
    sources = [row for row in state["container"]["slots"] if row["slot"] >= 27
               and row.get("item_id") == "minecraft:oak_log" and row.get("count") == 1]
    steps.check("chest_drag_unique_source", len(sources) == 1)
    pointer = Pointer(window)
    try:
        pointer.move(*center(state, sources[0]["slot"]))
        pointer.button(True)
        steps.wait(lambda s: s["container"]["carried"].get("item_id") == "minecraft:oak_log"
                   and s["container"]["carried"].get("count") == 1,
                   "chest_drag_pickup_received")
        pointer.move(*center(state, 0))
        pointer.button(False)
        native_client.capture(steps.client, artifact, "chest-drag-release")
        image = observe_menu(steps, "chest_drag_release", slots=63)
        steps.check("chest_drag_cursor_retained", holds(image["carried"], "minecraft:oak_log", 1))
        steps.check("chest_drag_no_slot_duplicate", all(
            stack is None or stack.get("item") != "minecraft:oak_log" for stack in image["slots"]))
        steps.call("minecraft_disconnect")
        steps.check("chest_disconnect_left_play", steps.call("minecraft_observe").get("in_play") is False)
        steps.call("minecraft_connect", {"server_addr": address})
        steps.call("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})
        rejoined = _echo(steps, "chest_rejoin_ordered")
        steps.call("minecraft_wait_for_no_visible_item", {
            **point, "item_id": "minecraft:oak_log", "timeout_seconds": steps.remaining(),
        })
        steps.check("chest_rejoin_no_visible_dropped_log", True)
        steps.check("chest_rejoin_item_exactly_once", item_count(rejoined, "minecraft:oak_log") == 1)
        reopened = _open(steps, point, "ChestMenu", 63)
        image = observe_menu(steps, "chest_rejoin", slots=63)
        steps.check("chest_rejoin_cursor_empty", image["carried"]["count"] == 0)
        chest = image["slots"][:27]
        steps.check("chest_rejoin_no_chest_duplicate", all(
            stack is None or stack.get("item") != "minecraft:oak_log" for stack in chest))
        steps.check("chest_rejoin_server_custody_exactly_one", sum(
            stack["count"] for stack in image["slots"]
            if stack is not None and stack.get("item") == "minecraft:oak_log") == 1)
        steps.check("chest_rejoin_received_menu", reopened["container"]["slot_count"] == 63)
        native_client.capture(steps.client, artifact, "chest-rejoin")
        steps.call("minecraft_close_screen")
    finally:
        pointer.close()
