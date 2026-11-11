"""Real Java MCP implementation of the shared matrix_api.Client verbs.

No world mutation is inferred from successful input dispatch. Domain adapters
must inspect snapshots or wait for their exact publication oracle. Unsupported
verbs fail explicitly, never emulate a successful no-op or elevate permissions.
"""
from __future__ import annotations

import json
import math
from pathlib import Path
import time
from typing import Any, Callable, TypeVar

from . import runtime
from .matrix_api import MissingCapability

T = TypeVar("T")


class JavaMatrixClient:
    def __init__(self, name: str, mcp: Any, address: str, artifact_dir: Path,
                 *, fixture_command: Callable[[str], Any] | None = None,
                 timeout_seconds: float = 30) -> None:
        self.name = name
        self.mcp = mcp
        self.address = address
        self.artifact_dir = artifact_dir
        self.fixture_command = fixture_command
        self.timeout_seconds = timeout_seconds
        self._last_observed: dict[str, Any] | None = None
        self.artifact_dir.mkdir(parents=True, exist_ok=True)
        self._transcript = self.artifact_dir / f"{name}-verbs.jsonl"

    def call(self, tool: str, arguments: dict[str, Any] | None = None) -> dict[str, Any]:
        entry: dict[str, Any] = {"tool": tool, "arguments": arguments or {}}
        try:
            result = self.mcp.call_tool(tool, arguments or {})
            entry["result"] = result
            return result
        except Exception as error:
            entry["error"] = f"{type(error).__name__}: {error}"
            raise
        finally:
            # MCP bearer credentials are never included in this local transcript.
            # Auth commands should use the group's dedicated redacted helper.
            if tool == "minecraft_send_chat" and str(entry["arguments"].get("message", "")).lstrip("/").split(" ", 1)[0] in {"register", "login", "changepassword"}:
                entry["arguments"] = {"message": "[AUTH REDACTED]", "command": True}
            with self._transcript.open("a") as handle:
                handle.write(json.dumps(entry) + "\n")

    @staticmethod
    def inventory_slots(stacks: list[dict[str, Any]] | None) -> list[dict[str, Any]] | None:
        """Normalize Java Inventory indices into the shared player-menu slots."""
        if stacks is None:
            return None
        result = []
        for stack in stacks:
            slot = stack["slot"]
            if 0 <= slot <= 8:
                slot += 36
            elif 36 <= slot <= 39:
                slot = 44 - slot
            elif slot == 40:
                slot = 45
            result.append(stack | {"slot": slot})
        return result

    @staticmethod
    def normalize(observed: dict[str, Any]) -> dict[str, Any]:
        player = observed.get("player") or {}
        position = [player.get(axis) for axis in ("x", "y", "z")]
        if any(value is None for value in position):
            position = None
        container = observed.get("container")
        menu = None
        cursor = None
        if isinstance(container, dict):
            menu = {"id": container.get("container_id"),
                    "slots": container.get("slots"),
                    "slot_count": container.get("slot_count"),
                    "state_id": container.get("container_state_id")}
            cursor = container.get("carried")
        events = None
        if "received_system_chat" in observed:
            messages = observed["received_system_chat"]
            dropped = observed.get("dropped_system_chat")
            if not isinstance(messages, list) or not all(isinstance(message, str) for message in messages):
                raise MissingCapability("snapshot:system_chat:invalid_receipts")
            if type(dropped) is not int or dropped < 0:
                raise MissingCapability("snapshot:system_chat:missing_loss_counter")
            if dropped:
                raise MissingCapability(f"snapshot:system_chat:lost_receipts:{dropped}")
            events = messages
        xp = {key: player[native] for key, native in (
            ("total", "total_experience"), ("level", "experience_level"),
            ("progress", "experience_progress"),
        ) if player.get(native) is not None}
        return {"position": position, "inventory": JavaMatrixClient.inventory_slots(observed.get("inventory")),
                "cursor": cursor, "menu": menu, "health": player.get("health"),
                # The applied-packet journal, not bounded GUI history, fences inputs.
                "food": player.get("food"), "xp": xp, "events": events,
                "in_play": observed.get("in_play"), "raw": observed,
                "connected": observed.get("connected"),
                "uuid": player.get("entity_uuid"), "dimension": observed.get("dimension"),
                "selected_hotbar": player.get("selected_hotbar_slot"),
                "time": observed.get("time"), "screen": observed.get("screen"),
                "connection": {"phase": "play" if observed.get("in_play") is True else
                               "disconnected" if observed.get("disconnect_reason") is not None else None,
                               "disconnect_reason": observed.get("disconnect_reason")}}

    def snapshot(self) -> dict[str, Any]:
        self._last_observed = self.call("minecraft_observe")
        result = self.normalize(self._last_observed)
        if self._last_observed.get("in_play") is True:
            visible = self.call("minecraft_list_entities", {"radius": 128, "limit": 512})
            if visible.get("truncated") is True:
                raise MissingCapability("snapshot:entities:visible_set_truncated")
            entities = visible.get("entities")
            if isinstance(entities, list):
                result["entities"] = [entity | {"id": entity["entity_id"],
                                                "type": entity["entity_type"],
                                                "uuid": entity["entity_uuid"],
                                                "position": [entity["x"], entity["y"], entity["z"]]}
                                      for entity in entities]
            if result["inventory"] is not None:
                for original, stack in zip(self._last_observed["inventory"], result["inventory"], strict=True):
                    if stack.get("count", 0) <= 0:
                        continue
                    native = self.call("minecraft_read_item_components",
                                       {"source": "player", "slot": original["slot"]})
                    if (native.get("source"), native.get("slot"), native.get("item_id"), native.get("count")) != (
                            "player", original["slot"], stack["item_id"], stack["count"]):
                        raise MissingCapability("snapshot:stack:components:identity_changed")
                    components = native.get("components")
                    if not isinstance(components, dict):
                        raise MissingCapability("snapshot:stack:components:missing_native_data")
                    stack["components"] = components
            screen = self._last_observed.get("screen") or {}
            menu = result["menu"]
            if menu and screen.get("open") and ".screens.inventory." in screen.get("class", ""):
                for stack in menu.get("slots") or []:
                    if not stack or stack.get("count", 0) <= 0:
                        continue
                    native = self.call("minecraft_read_item_components",
                                       {"source": "container", "slot": stack["slot"]})
                    if (native.get("source"), native.get("slot"), native.get("container_id"),
                            native.get("item_id"), native.get("count")) != (
                            "container", stack["slot"], menu["id"], stack["item_id"], stack["count"]):
                        raise MissingCapability("snapshot:stack:components:identity_changed")
                    components = native.get("components")
                    if not isinstance(components, dict):
                        raise MissingCapability("snapshot:stack:components:missing_native_data")
                    stack["components"] = components
        return result

    def command(self, text: str) -> None:
        self.call("minecraft_send_chat", {"message": text.removeprefix("/"), "command": True})

    def set_auto_auth(self, enabled: bool) -> None:
        if type(enabled) is not bool:
            raise ValueError("auto authentication flag must be bool")
        self._auto_auth = enabled

    def chat(self, text: str) -> None:
        self.call("minecraft_send_chat", {"message": text, "command": False})

    def teleport_to(self, x: float, y: float, z: float) -> None:
        if self.fixture_command is None:
            raise MissingCapability("teleport_fixture:server_command_authority")
        self.fixture_command(f"teleport {self.name} {x} {y} {z}")
        self.wait_until(lambda: (state if (state := self.snapshot())["position"] is not None
                                and math.dist(state["position"], [x, y, z]) < 0.25 else None),
                        description="received fixture teleport")
        self.wait_for_chunk((math.floor(x), math.floor(y), math.floor(z)))

    def look_at(self, x: float, y: float, z: float) -> None:
        state = self.snapshot()
        position = state["position"]
        if position is None:
            raise RuntimeError("look_at requires an observed player position")
        # Read actual eye height rather than fabricate a standing pose: the
        # Minecraft observation exposes eye_y with the local player's entity.
        eye_y = state["raw"].get("player", {}).get("eye_y")
        if eye_y is None:
            raise MissingCapability("look_at:player.eye_y")
        dx, dy, dz = x - position[0], y - eye_y, z - position[2]
        self.call("minecraft_look", {"yaw_deg": round(math.degrees(math.atan2(-dx, dz))),
                                     "pitch_deg": round(-math.degrees(math.atan2(dy, math.hypot(dx, dz))))})

    def use_item_on(self, position: tuple[int, int, int], face: str = "up",
                    hand: str = "main", cursor: tuple[float, float, float] = (.5, .5, .5)) -> None:
        if cursor != (.5, .5, .5):
            raise MissingCapability("client:use_item_on:custom_cursor")
        if hand not in {"main", "off"}:
            raise ValueError("hand must be main or off")
        self.call("minecraft_use_item_on", dict(zip(("x", "y", "z"), position)) |
                  {"face": face, "hand": "main_hand" if hand == "main" else "off_hand"})

    def attack(self, entity_id: int) -> None:
        result = self.call("minecraft_list_entities", {"radius": 128, "limit": 512})
        entities = [entity for entity in result.get("entities", []) if entity.get("entity_id") == entity_id]
        if len(entities) != 1:
            raise RuntimeError(f"attack target {entity_id} is not uniquely client-visible")
        entity = entities[0]
        if not entity.get("entity_uuid") or not entity.get("entity_type"):
            raise MissingCapability(f"attack:entity_identity:{entity_id}")
        self.call("minecraft_attack_entity_once", {"entity_id": entity_id, "entity_uuid": entity["entity_uuid"],
                                                   "entity_type": entity["entity_type"],
                                                   "timeout_seconds": min(self.timeout_seconds, 120)})

    def click_slot(self, slot: int, kind: str = "pickup", button: int = 0) -> None:
        if kind not in {"pickup", "quick_move"}:
            raise MissingCapability(f"click_slot:{kind}")
        if button not in (0, 1) or (kind == "quick_move" and button != 0):
            raise MissingCapability(f"click_slot:{kind}:button_{button}")
        args: dict[str, Any] = {"slot": slot, "timeout_seconds": min(self.timeout_seconds, 120)}
        if kind == "quick_move":
            self.call("minecraft_quick_move_container_slot", args)
        else:
            args["button"] = "primary" if button == 0 else "secondary"
            self.call("minecraft_click_container_slot", args)

    def submit_input_batch(self, actions: list[dict[str, Any]]) -> dict[str, Any]:
        return self.call("minecraft_submit_input_batch", {"actions": actions})

    def open_container(self, position: tuple[int, int, int]) -> None:
        self.call("minecraft_look_at_block", dict(zip(("x", "y", "z"), position)) | {"face": "up"})
        before = self.snapshot()["menu"]
        self.use_item_on(position)
        self.wait_until(lambda: (menu if (menu := self.snapshot()["menu"]) is not None
                                and menu["id"] != (before or {}).get("id") else None),
                        description="received opened container")

    def drop(self, all: bool = False) -> None:
        observed = self.call("minecraft_observe")
        item = observed.get("selected_item")
        if not item or not item.get("count"):
            raise RuntimeError("drop requires an actual nonempty selected item")
        self.call("minecraft_drop_selected_item", {"item_id": item["item_id"],
                                                    "count": item["count"] if all else 1,
                                                    "timeout_seconds": min(self.timeout_seconds, 120)})

    def select_hotbar(self, slot: int) -> None:
        self.call("minecraft_set_hotbar_slot", {"slot": slot})

    def respawn(self) -> None:
        self.call("minecraft_respawn", {"timeout_seconds": min(self.timeout_seconds, 120)})

    def swap_hands(self) -> None:
        self.call("minecraft_press_inputs", {"keys": ["swap_offhand"], "ticks": 1})

    def disconnect(self) -> None:
        self.call("minecraft_disconnect")
        self.wait_until(lambda: not self.snapshot()["in_play"], description="client disconnected")

    def reconnect(self) -> None:
        runtime.wait_client_ready_for_connect(self.mcp, self.timeout_seconds)
        self.call("minecraft_connect", {"server_addr": self.address})
        if getattr(self.mcp, "_matrix_platform", None) is not None:
            from .java_clients import _confirm_exact_permission
            _confirm_exact_permission(self.mcp, self.address, self.timeout_seconds)
        play = self.call("minecraft_wait_for_play", {"timeout_seconds": self.timeout_seconds})
        if play.get("in_play") is not True:
            raise RuntimeError(f"reconnect did not enter Play: {play}")
        password = getattr(self.mcp, "_matrix_auth_password", None)
        if isinstance(password, str) and getattr(self, "_auto_auth", True):
            from .java_clients import authenticate_client
            authenticate_client(self.mcp, password, timeout_seconds=self.timeout_seconds, register=False)

    def press_inputs(self, *, forward: bool = False, backward: bool = False,
                     left: bool = False, right: bool = False, jump: bool = False,
                     sneak: bool = False, sprint: bool = False, ticks: int = 1) -> None:
        keys = [key for key, active in (("forward", forward), ("back", backward),
                                       ("left", left), ("right", right), ("jump", jump),
                                       ("sneak", sneak), ("sprint", sprint)) if active]
        self.call("minecraft_press_inputs", {"keys": keys, "ticks": ticks})

    def move(self, forward: float = 0, strafe: float = 0, ticks: int = 20,
             sprint: bool = False) -> None:
        if forward not in (-1, 0, 1) or strafe not in (-1, 0, 1):
            raise MissingCapability("client:move:analog_input")
        self.press_inputs(forward=forward > 0, backward=forward < 0,
                          left=strafe > 0, right=strafe < 0, ticks=ticks, sprint=sprint)

    def jump(self, ticks: int = 1) -> None:
        self.press_inputs(jump=True, ticks=ticks)

    def wait_ticks(self, ticks: int) -> None:
        if isinstance(ticks, bool) or not isinstance(ticks, int) or ticks <= 0:
            raise ValueError("ticks must be a positive integer")
        while ticks > 0:
            amount = min(ticks, 255)
            self.call("minecraft_wait_ticks", {"ticks": amount})
            ticks -= amount

    def use_item(self, hand: str = "main", ticks: int = 1) -> None:
        if hand != "main":
            raise MissingCapability("client:use_item:off_hand")
        self.call("minecraft_press_inputs", {"keys": ["use"], "ticks": ticks})

    def release_use_item(self) -> None:
        # One real client tick with all ordinary keys released, not a sleep.
        self.call("minecraft_press_inputs", {"keys": [], "ticks": 1})

    def interact_entity(self, entity_id: int, hand: str = "main", secondary: bool = False,
                        location: tuple[float, float, float] | None = None) -> None:
        if secondary or location is not None:
            raise MissingCapability("client:interact_entity:secondary_or_location")
        if hand not in {"main", "off"}:
            raise ValueError("hand must be main or off")
        result = self.call("minecraft_list_entities", {"radius": 128, "limit": 512})
        matches = [entity for entity in result.get("entities", []) if entity.get("entity_id") == entity_id]
        if len(matches) != 1:
            raise RuntimeError(f"interaction target {entity_id} is not uniquely client-visible")
        entity = matches[0]
        self.call("minecraft_interact_entity", {
            "entity_id": entity_id, "entity_uuid": entity["entity_uuid"],
            "entity_type": entity["entity_type"], "hand": "main_hand" if hand == "main" else "off_hand"})

    def break_block(self, position: tuple[int, int, int], face: str = "up",
                    phase: str = "complete") -> None:
        if phase != "complete":
            raise MissingCapability("client:break_block:" + phase)
        self.call("minecraft_break_block", dict(zip(("x", "y", "z"), position)) |
                  {"face": face, "expected_drop_item_id": "minecraft:air",
                   "expected_drop_count": 0, "timeout_seconds": min(self.timeout_seconds, 120)})

    def container_button(self, button: int) -> None:
        self.call("minecraft_click_container_button", {"button_id": button,
                                                       "timeout_seconds": min(self.timeout_seconds, 120)})

    def close_container(self) -> None:
        self.call("minecraft_close_screen")

    def open_inventory(self) -> None:
        self.call("minecraft_open_inventory")

    def wait_for_chunk(self, position: tuple[int, int, int]) -> None:
        self.call("minecraft_wait_for_loaded_block", dict(zip(("x", "y", "z"), position)) |
                  {"timeout_seconds": min(self.timeout_seconds, 120)})

    def ensure_loaded(self, area: tuple[tuple[int, int, int], tuple[int, int, int]]) -> None:
        low, high = area
        for chunk_x in range(low[0] // 16, high[0] // 16 + 1):
            for chunk_z in range(low[2] // 16, high[2] // 16 + 1):
                self.wait_for_chunk((chunk_x * 16, low[1], chunk_z * 16))

    def read_block_state(self, position: tuple[int, int, int]) -> dict[str, Any]:
        try:
            block = self.call("minecraft_read_block", dict(zip(("x", "y", "z"), position)))
        except RuntimeError as error:
            if "client chunk is not loaded" in str(error):
                raise MissingCapability("client:read_block_state:chunk_not_loaded") from error
            raise
        if not isinstance(block.get("block_id"), str) or not isinstance(block.get("properties"), dict):
            raise MissingCapability("client:read_block_state:missing_observation")
        return {"block_id": block["block_id"], "properties": block["properties"]}

    def wait_until(self, predicate: Callable[[], T], *, description: str,
                   timeout_seconds: float = 30) -> T:
        deadline = time.monotonic() + timeout_seconds
        while True:
            # Snapshot before predicate closes the event-race even when the
            # predicate reads another client's state or a server receipt.
            observed = self.call("minecraft_observe")
            value = predicate()
            if value:
                return value
            remaining = deadline - time.monotonic()
            if remaining < 0.1:
                raise TimeoutError(f"{description} not observed within {timeout_seconds}s")
            version = observed.get("state_version")
            if not isinstance(version, int):
                raise RuntimeError("client observation omitted state_version")
            self.call("minecraft_wait_for_state_change", {"observed_version": version,
                                                         "timeout_seconds": min(remaining, 120)})
