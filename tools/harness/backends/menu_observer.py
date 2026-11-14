"""Ordered, server-authoritative inventory snapshots for graphical probes."""
from __future__ import annotations

import json
import uuid
from typing import Any

from .probe_steps import ProbeSteps

_PREFIX = "KRASNOKAMEN_MENU "


def observe_menu(steps: ProbeSteps, label: str, *, slots: int = 46) -> dict[str, Any]:
    marker = "menu-fence-" + uuid.uuid4().hex
    steps.call("minecraft_send_chat", {"message": marker, "command": False})
    steps.wait(lambda state: any(isinstance(line, str) and marker in line for line in state["recent_chat"]),
               label + "_fence")
    steps.call("minecraft_send_chat", {"message": "debug menu", "command": True})

    def fresh_snapshot(state: dict[str, Any]) -> bool:
        lines = state["recent_chat"]
        boundary = next((index for index, line in enumerate(lines)
                         if isinstance(line, str) and marker in line), None)
        return boundary is not None and any(isinstance(line, str) and line.startswith(_PREFIX)
                                            for line in lines[:boundary])

    ordered = steps.wait(fresh_snapshot, label + "_new_server_snapshot")
    lines = ordered["recent_chat"]
    boundary = next(index for index, line in enumerate(lines) if isinstance(line, str) and marker in line)
    line = next(line for line in lines[:boundary] if isinstance(line, str) and line.startswith(_PREFIX))
    image = json.loads(line[len(_PREFIX):])
    steps.report.setdefault("server_menu_snapshots", {})[label] = image
    steps.check(label + ("_player_menu" if slots == 46 else "_container_menu"),
                len(image["slots"]) == slots
                and (image["container_id"] == 0 if slots == 46 else image["container_id"] != 0))
    return image


def holds(stack: dict[str, Any] | None, item: str, count: int) -> bool:
    # The probe also reports `components`; drag gates judge only item and count.
    return stack is not None and stack.get("item") == item and stack.get("count") == count


def log_custody(image: dict[str, Any]) -> int:
    # Slot 0 is the virtual crafting result, not an owned item stack.
    stacks = [image["carried"], *image["slots"][1:]]
    return sum(stack["count"] for stack in stacks
               if stack is not None and stack["item"] == "minecraft:oak_log")
