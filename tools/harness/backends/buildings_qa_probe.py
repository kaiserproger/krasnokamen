"""Buildings-qa client and report primitives: one function per observed thing (block, chat,
census, chest, listing, menu lore) plus the verdict record the matrix reads.

The client/report seams live on ``buildings_qa`` (the tests patch them there), so this
module reaches them as ``_qa.<name>`` and never binds them.
"""

from __future__ import annotations

import re
from typing import Any

from .probe_steps import ProbeSteps
from . import buildings_qa as _qa
from .buildings_qa_consts import CONTAINER_CLASS, LEVELS, PROJECT_MAP_SLOT


def _state_base(text: str) -> str:
    name = text.split("[")[0]
    return name if ":" in name else f"minecraft:{name}"


def _block_id(block: dict[str, Any]) -> str:
    inner = block.get("block") if isinstance(block.get("block"), dict) else block
    if isinstance(inner, dict):
        return str(inner.get("block_id", inner.get("id", "")))
    return str(block.get("block_id", ""))


def _props(block: dict[str, Any]) -> dict[str, Any]:
    props = block.get("properties") or {}
    return dict(props) if isinstance(props, dict) else {}


def _read(probe: ProbeSteps, pos: tuple[int, int, int]) -> dict[str, Any]:
    result = probe.call("minecraft_read_block", {"x": pos[0], "y": pos[1], "z": pos[2]})
    inner = result.get("block")
    return inner if isinstance(inner, dict) else result


def _chat_lines(state: dict[str, Any], n: int = 20) -> list[str]:
    # Client buffer is newest-first (observation .rev().take(20)): take the
    # FIRST n, not the last n.
    return [str(l) for l in (state.get("recent_chat", []) or [])][:n]


def _send(probe: ProbeSteps, command: str, settle_ticks: int = 40) -> dict[str, Any]:
    probe.call("minecraft_send_chat", {"message": command, "command": True})
    probe.call("minecraft_wait_ticks", {"ticks": settle_ticks})
    return probe.observe()


def _send_fresh(probe: ProbeSteps, command: str, prefixes: tuple[str, ...] = ()) -> dict[str, Any]:
    """Send a command and return only chat received after its unique pre-command marker."""
    import uuid
    marker = f"bqa_fresh_{uuid.uuid4().hex}"
    probe.call("minecraft_send_chat", {"message": marker, "command": False})
    probe.wait(lambda s: any(marker in line for line in _qa._chat_lines(s)),
               "fresh_chat_marker", timeout_seconds=60)
    probe.call("minecraft_send_chat", {"message": command, "command": True})

    def response(state: dict[str, Any]) -> bool:
        lines = _qa._chat_lines(state)
        boundary = next((i for i, line in enumerate(lines) if marker in line), None)
        return boundary is not None and any(
            not prefixes or line.startswith(prefixes) for line in lines[:boundary])

    state = probe.wait(response, "fresh_command_response", timeout_seconds=60)
    lines = _qa._chat_lines(state)
    boundary = next(i for i, line in enumerate(lines) if marker in line)
    return {**state, "recent_chat": lines[:boundary]}


def _echo(probe: ProbeSteps, label: str) -> dict[str, Any]:
    import uuid as _uuid
    marker = f"bqa_ping_{label}_{_uuid.uuid4().hex[:8]}"
    probe.call("minecraft_send_chat", {"message": marker, "command": False})
    return probe.wait(lambda s: any(marker in line for line in s.get("recent_chat", [])),
                      f"{label}_server_echo")


def _villagers(probe: ProbeSteps) -> list[dict[str, Any]]:
    # Typed and bounded: the unfiltered 128-block census of a generated village overflows the
    # client's 256 KiB MCP result (`result_limit_exceeded`, every buildings-qa run of 2026-11-07).
    state = probe.call("minecraft_list_entities",
                       {"radius": 128, "limit": 128, "entity_type": "minecraft:villager"})
    entities = state.get("entities", state.get("visible_entities", []))
    return [r for r in entities
            if isinstance(r, dict) and r.get("entity_type") == "minecraft:villager"]


def _open_chest(probe: ProbeSteps, chest: tuple[int, int, int]) -> dict[int, dict[str, Any]]:
    probe.call("minecraft_set_hotbar_slot", {"slot": 8})
    probe.call("minecraft_use_item_on",
               {"x": chest[0], "y": chest[1], "z": chest[2], "face": "up", "hand": "main_hand"})
    state = probe.wait(
        lambda s: s["container"]["container_id"] != 0
        and s["container"]["menu_class"] == "net.minecraft.world.inventory.ChestMenu",
        "chest_menu_received")
    if state["container"]["slot_count"] != 63:
        raise RuntimeError(f"buildings-qa chest slot_count is {state['container']['slot_count']}, want 63")
    slots: dict[int, dict[str, Any]] = {}
    for row in state["container"]["slots"]:
        if isinstance(row, dict) and 0 <= int(row.get("slot", -1)) < 27:
            slots[int(row["slot"])] = {"item_id": row.get("item_id"), "count": row.get("count", 0)}
    probe.call("minecraft_close_screen", {})
    probe.wait(lambda s: s["container"]["container_id"] == 0, "chest_menu_closed")
    return slots


def _chest_totals(slots: dict[int, dict[str, Any]]) -> dict[str, int]:
    totals: dict[str, int] = {}
    for entry in slots.values():
        name = str(entry.get("item_id", "minecraft:air"))
        if name != "minecraft:air" and int(entry.get("count", 0)) > 0:
            totals[name] = totals.get(name, 0) + int(entry.get("count", 0))
    return totals


def _deposit_all(probe: ProbeSteps, chest: tuple[int, int, int], items: list[str]) -> dict[str, Any]:
    """Shift-click every matching player-inventory stack into the open chest (ordinary path)."""
    report: dict[str, Any] = {"moved": []}
    probe.call("minecraft_use_item_on",
               {"x": chest[0], "y": chest[1], "z": chest[2], "face": "up", "hand": "main_hand"})
    st = probe.wait(lambda s: s["container"]["container_id"] != 0
                    and s["container"]["menu_class"] == "net.minecraft.world.inventory.ChestMenu",
                    "chest_open_for_deposit")
    moved = 0
    for row in list(st["container"]["slots"]):
        if not isinstance(row, dict):
            continue
        slot = int(row.get("slot", -1))
        if slot < 27 or str(row.get("item_id")) not in items:
            continue
        try:
            probe.call("minecraft_quick_move_container_slot", {"slot": slot})
            moved += 1
        except Exception as e:
            report.setdefault("errors", []).append(f"slot {slot}: {e}")
    report["moved_count"] = moved
    probe.call("minecraft_close_screen", {})
    probe.wait(lambda s: s["container"]["container_id"] == 0, "chest_closed_after_deposit")
    return report


def _matrix_19(detail: dict[str, Any]) -> dict[str, str]:
    """Enumerate all 19 cases (9 blueprint ids x their levels) PASS/FAIL/NOT-RUN."""
    out: dict[str, str] = {}
    blueprints = detail.get("blueprints", {})
    for bp, count in LEVELS.items():
        for level in range(1, count + 1):
            sub = blueprints.get(bp, {})
            checks = [sub.get("behaviour", {}).get(f"L{level}_blockmatch"),
                      sub.get("behaviour", {}).get(f"L{level}_entrance"),
                      sub.get("behaviour", {}).get(f"L{level}_light")]
            if all(c == "PASS" for c in checks):
                out[f"{bp} L{level}"] = "PASS"
            elif any(c == "FAIL" for c in checks):
                out[f"{bp} L{level}"] = "FAIL"
            else:
                out[f"{bp} L{level}"] = "NOT-RUN"
    return out


def _verdict(detail: dict[str, Any], name: str, verdict: str, evidence: str = "",
             repro: str = "") -> None:
    detail.setdefault("behaviour", {})[name] = verdict
    if evidence:
        detail.setdefault("evidence", {})[name] = evidence
    if repro:
        detail.setdefault("repro", {})[name] = repro


def _match_kind(bp: str, line: str) -> bool:
    """Listing line matches the blueprint kind (not the hand shell)."""
    low = line.lower()
    if bp == "storage":
        return "storage" in low and "storage shell" not in low
    if bp == "farm":
        return "farmer" in low
    if bp == "mason":
        return "builder" in low
    if bp == "lumber_camp":
        return "lumber" in low
    if bp == "watch_tower":
        return "tower" in low
    return bp.split("_")[0] in low


def _building_bounds(probe: ProbeSteps, kind_label: str, level: int,
                     exclude: list[list[int]] | None = None) -> tuple[int, int, int] | None:
    """Parse `#<id> <kind> level <n> [min]-...` listing -> bounds min corner (origin).
    The listing packs several buildings per line; the bracket must be the one
    NEAR our kind token, not the first bracket on the line (often the shell)."""
    import re as _re
    state = _qa._send(probe, "settlement building list")
    for line in _qa._chat_lines(state, 25):
        low = line.lower()
        if not _match_kind(kind_label, line) or f"level {level}" not in line:
            continue
        # Split the line into per-building segments at each '#<id>' marker and
        # only accept a segment that names our kind.
        for seg in _re.split(r"#\d+", line):
            if not _match_kind(kind_label, seg) or f"level {level}" not in seg:
                continue
            m = _re.search(r"\[(-?\d+),\s*(-?\d+),\s*(-?\d+)\]", seg)
            if m:
                origin = (int(m.group(1)), int(m.group(2)), int(m.group(3)))
                if exclude and list(origin) in exclude:
                    continue
                return origin
    return None


def _project_origin(probe: ProbeSteps) -> tuple[int, int, int] | None:
    """Origin of the active project, or None: the Construction page's map icon (slot 14) carries
    it as the lore line `<x> <y> <z>` (ui_menus.rs project_screen, ui.rs corner_text)."""
    page = _qa._project_page(probe)
    try:
        if not any(row.get("slot") == PROJECT_MAP_SLOT for row in page["container"]["slots"]):
            return None
        lore = probe.call("minecraft_read_item_components",
                          {"source": "container", "slot": PROJECT_MAP_SLOT})["components"].get("minecraft:lore", [])
    finally:
        probe.call("minecraft_close_screen", {})
    for line in lore:
        match = re.fullmatch(r"\s*(-?\d+) (-?\d+) (-?\d+)\s*", _text_plain(line))
        if match:
            return (int(match.group(1)), int(match.group(2)), int(match.group(3)))
    return None


def _container_titled(state: dict[str, Any], prefix: str) -> bool:
    screen = state.get("screen") or {}
    return screen.get("class") == CONTAINER_CLASS and str(screen.get("title") or "").startswith(prefix)


def _close_any_screen(probe: ProbeSteps) -> None:
    """Close dialog/container screens so use_item_on hits the world."""
    for _ in range(3):
        state = probe.observe()
        screen = (state.get("screen") or {}).get("class") or "none"
        cid = (state.get("container") or {}).get("container_id", 0)
        if screen in (None, "none", "null") and cid == 0:
            return
        try:
            probe.call("minecraft_close_screen", {})
        except Exception:
            pass
        probe.call("minecraft_wait_ticks", {"ticks": 8})


def _open_chest_at(probe: ProbeSteps, chest: tuple[int, int, int]) -> dict[int, dict[str, Any]]:
    """Close screens, tp within reach, open the chest, assert ChestMenu."""
    _qa._close_any_screen(probe)
    probe.fixture(f"tp @s {chest[0] + 0.5} {chest[1] + 1.0} {chest[2] + 1.5}")
    probe.wait(lambda s: abs(float((s.get("player") or {}).get("x", 0)) - (chest[0] + 0.5)) < 4.0,
               "chest_reach")
    return _open_chest(probe, chest)


def _deposit_all_at(probe: ProbeSteps, chest: tuple[int, int, int], items: list[str]) -> dict[str, Any]:
    _qa._close_any_screen(probe)
    probe.fixture(f"tp @s {chest[0] + 0.5} {chest[1] + 1.0} {chest[2] + 1.5}")
    probe.wait(lambda s: abs(float((s.get("player") or {}).get("x", 0)) - (chest[0] + 0.5)) < 4.0,
               "deposit_reach")
    return _deposit_all(probe, chest, items)


def _residents_text(probe: ProbeSteps) -> list[str]:
    """Read each resident's profession and workplace from the census icon lore."""
    _qa._send(probe, "settlement menu residents")
    opened = probe.wait(lambda s: _container_titled(s, "Residents"), "residents_page_open",
                        timeout_seconds=60)
    residents = []
    while True:
        for row in opened["container"]["slots"]:
            slot = int(row.get("slot", -1))
            if 9 <= slot < 45 and row.get("item_id"):
                components = probe.call("minecraft_read_item_components",
                                        {"source": "container", "slot": slot})["components"]
                lore = [_text_plain(line) for line in components.get("minecraft:lore", [])]
                residents.append(" ".join(lore).replace("Works at", "works at"))
        if not any(row.get("slot") == 50 and row.get("name") == "Next page"
                   for row in opened["container"]["slots"]):
            break
        previous = opened["container"]["slots"]
        probe.call("minecraft_click_container_slot", {"slot": 50, "button": "primary",
                                                      "timeout_seconds": probe.remaining()})
        opened = probe.wait(lambda s: _container_titled(s, "Residents")
                            and s["container"]["slots"] != previous,
                            "residents_next_page", timeout_seconds=60)
    probe.call("minecraft_close_screen", {})
    return residents


def _building_lore(probe: ProbeSteps, name: re.Pattern[str]) -> tuple[str, list[str]] | None:
    """(icon name, lore lines) of the first Buildings-menu icon matching `name`, or None.

    `settlement menu buildings` opens the container menu `Buildings · <settlement>` (ui_menus.rs
    filtered_buildings_screen); an icon's lore (category, `Soldiers: f/n` or `Workers: f/n`,
    bounds) is read with `minecraft_read_item_components`."""
    _qa._close_any_screen(probe)
    probe.call("minecraft_send_chat", {"message": "settlement menu buildings", "command": True})
    state = probe.wait(lambda s: _container_titled(s, "Buildings"), "buildings_menu_open",
                       timeout_seconds=60)
    found = None
    for row in state["container"]["slots"]:
        if name.fullmatch(str(row.get("name") or "")):
            components = probe.call("minecraft_read_item_components",
                                    {"source": "container", "slot": int(row["slot"])})["components"]
            found = (str(row["name"]), [_text_plain(line) for line in components.get("minecraft:lore", [])])
            break
    probe.call("minecraft_close_screen", {})
    return found


def _text_plain(value: Any) -> str:
    """Plain text of a JSON text component (string, list, or object with extra)."""
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return "".join(_text_plain(part) for part in value)
    if isinstance(value, dict):
        return _text_plain(value.get("text", "")) + _text_plain(value.get("extra", []))
    return ""


def _soldier_beds(lore: list[str]) -> tuple[int, int] | None:
    """(filled, beds) from a barracks icon's `Soldiers: f/n` lore line."""
    for line in lore:
        match = re.search(r"Soldiers:\s*(\d+)\s*/\s*(\d+)", line)
        if match:
            return int(match.group(1)), int(match.group(2))
    return None
