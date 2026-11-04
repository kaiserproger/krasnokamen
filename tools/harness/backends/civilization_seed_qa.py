"""Observe a generated-village claim with a documented player-deposited founding fixture."""
from __future__ import annotations

import re
import math
import time
from pathlib import Path
from typing import Any, Callable

from . import item_merge, native_client, village_autonomy_qa as village
from .probe_steps import ProbeSteps


SPRINT_TICKS = 3000
STALL_TICKS = 24000  # One complete day/night cycle, not merely a work shift.
MAX_TICKS = 96000
FOUNDING_STOCK = {"minecraft:oak_log": 32, "minecraft:bread": 16,
                  "minecraft:raw_iron": 8, "minecraft:coal": 8,
                  "minecraft:cobblestone": 32}


def inventory_totals(state: dict[str, Any]) -> dict[str, int]:
    result: dict[str, int] = {}
    for row in state.get("inventory", []):
        item = row.get("item_id")
        if item in FOUNDING_STOCK:
            result[item] = result.get(item, 0) + int(row.get("count") or 0)
    return result


def storage_bounds(chest: tuple[int, int, int], blocks: list[dict[str, Any]],
                   buildings: list[dict[str, Any]]) -> tuple[list[int], list[int]] | None:
    x, y, z = chest
    cells = {(block["x"], block["y"], block["z"]): block for block in blocks}
    for floor in range(y - 1, y - 5, -1):
        for x0 in range(x - 2, x + 1):
            for z0 in range(z - 2, z + 1):
                low, high = [x0, floor, z0], [x0 + 2, max(floor + 3, y), z0 + 2]
                if any(all(low[i] <= b["max"][i] and b["min"][i] <= high[i]
                           for i in range(3)) for b in buildings):
                    continue
                if all((cell := cells.get((bx, floor, bz))) is not None
                       and not cell.get("collision_empty", True)
                       for bx in range(x0, x0 + 3) for bz in range(z0, z0 + 3)):
                    return low, high
    return None


def fund(probe: ProbeSteps, detail: dict[str, Any], server: Any,
         chest: tuple[int, int, int]) -> None:
    x, y, z = chest
    lines = village._recent_chat(village._send(probe, "settlement building list"), 20)
    buildings = village._listed_buildings(lines)
    if not any(b["kind"] == "storage" and all(b["min"][i] <= chest[i] <= b["max"][i]
                                                for i in range(3)) for b in buildings):
        scan = probe.call("minecraft_scan_blocks", {
            "min_x": x - 2, "max_x": x + 2, "min_y": y - 4, "max_y": y - 1,
            "min_z": z - 2, "max_z": z + 2,
        })
        bounds = storage_bounds(chest, scan.get("blocks", []), buildings)
        if bounds is None:
            raise RuntimeError("no non-overlapping solid 3x3 storage floor under the founding chest")
        low, high = bounds
        detail["storage_certification_bounds"] = [low, high]
        command = "settlement building certify storage " + " ".join(str(v) for v in (*low, *high))
        detail["certify_chat"] = village._recent_chat(village._send(probe, command), 20)
        lines = village._recent_chat(village._send(probe, "settlement building list"), 20)
        buildings = village._listed_buildings(lines)
    probe.check("founding_chest_certified", any(
        b["kind"] == "storage" and all(b["min"][i] <= chest[i] <= b["max"][i]
                                       for i in range(3)) for b in buildings))
    detail["certified_buildings_before_stock"] = buildings
    detail["founding_grants"] = dict(FOUNDING_STOCK)
    for item, count in FOUNDING_STOCK.items():
        command = f"give {village.USERNAME} {item} {count}"
        probe.report["fixture_commands"].append("console: " + command)
        item_merge._console(server, command)
    before = probe.wait(lambda state: all(inventory_totals(state).get(item, 0) >= count
                                          for item, count in FOUNDING_STOCK.items()),
                        "founding_grants_received")
    detail["player_inventory_before_transfer"] = inventory_totals(before)
    village._open_chest(probe, chest)
    opened = probe.observe()
    container = opened["container"]
    storage_slots = int(container["slot_count"]) - 36
    detail["chest_before_transfer"] = village._chest_totals({
        int(row["slot"]): row for row in container["slots"]
        if int(row["slot"]) < storage_slots
    })
    detail["transfer_attempts"] = []
    for row in list(container["slots"]):
        if int(row["slot"]) >= storage_slots and row.get("item_id") in FOUNDING_STOCK:
            probe.call("minecraft_quick_move_container_slot", {"slot": int(row["slot"])})
            detail["transfer_attempts"].append({"slot": row["slot"], "item_id": row["item_id"], "count": row["count"]})
            probe.wait(lambda state: all(int(s.get("slot", -1)) != int(row["slot"])
                                         or s.get("item_id") != row["item_id"]
                                         for s in (state.get("container") or {}).get("slots", [])),
                       f"founding_transfer_slot_{row['slot']}")
    after = probe.observe()
    detail["player_inventory_after_transfer"] = inventory_totals(after)
    detail["chest_after_transfer"] = village._chest_totals({
        int(row["slot"]): row for row in after["container"]["slots"]
        if int(row["slot"]) < storage_slots
    })
    probe.call("minecraft_close_screen", {})

def building_blocks(probe: ProbeSteps, lines: list[str]) -> list[dict[str, Any]]:
    evidence = []
    for building in village._listed_buildings(lines)[:30]:
        if building["natural"]:
            continue
        lo, hi = building["min"], building["max"]
        volume = (hi[0] - lo[0] + 1) * (hi[1] - lo[1] + 1) * (hi[2] - lo[2] + 1)
        if volume > 4096:
            evidence.append({"building": building, "skipped": "scan volume exceeds 4096"})
            continue
        scan = probe.call("minecraft_scan_blocks", {
            "min_x": lo[0], "min_y": lo[1], "min_z": lo[2],
            "max_x": hi[0], "max_y": hi[1], "max_z": hi[2],
        })
        relevant = [block for block in scan.get("blocks", [])
                    if str(block.get("block_id", "")).endswith(("_bed", ":furnace", ":blast_furnace"))]
        evidence.append({"building": building, "blocks": relevant, "truncated": scan.get("truncated", False)})
    return evidence



def workstation_blocks(probe: ProbeSteps, chest: tuple[int, int, int]) -> list[dict[str, Any]]:
    """Observe the bounded outdoor placement columns, without inferring ownership."""
    found = []
    x, y, z = chest
    for distance in range(2, 6):
        for dx, dz in ((distance, 0), (-distance, 0), (0, distance), (0, -distance)):
            scan = probe.call("minecraft_scan_blocks", {
                "min_x": x + dx, "max_x": x + dx, "min_z": z + dz, "max_z": z + dz,
                "min_y": y - 8, "max_y": y + 12,
            })
            found.extend(block for block in scan.get("blocks", [])
                         if block.get("block_id") == "minecraft:furnace")
    return found


def capture_iron_menu(probe: ProbeSteps, artifact: Path, pos: tuple[int, int, int],
                      advanced: int) -> dict[str, Any]:
    """Open the actual furnace as the player and capture its observed output slot."""
    x, y, z = pos
    probe.call("minecraft_close_screen", {})
    probe.call("minecraft_set_hotbar_slot", {"slot": 8})
    probe.call("minecraft_use_item_on", {"x": x, "y": y, "z": z, "face": "up", "hand": "main_hand"})
    state = probe.wait(
        lambda s: (s.get("container") or {}).get("container_id", 0) != 0
        and (s.get("container") or {}).get("menu_class") == "net.minecraft.world.inventory.FurnaceMenu"
        and any(slot.get("slot") == 2 and slot.get("item_id") == "minecraft:iron_ingot"
                and slot.get("count", 0) > 0 for slot in (s.get("container") or {}).get("slots", [])),
        "furnace_iron_output_visible", timeout_seconds=15.0,
    )
    shot = native_client.capture(probe.client, artifact, f"civilization-iron-{advanced:06d}")
    probe.call("minecraft_close_screen", {})
    return {"screenshot": shot, "container": state["container"]}


def standable_height(blocks: list[dict[str, Any]], player_y: float) -> int | None:
    cells = {int(block["y"]): block for block in blocks}
    base = math.floor(player_y)
    heights = [y for y in range(base - 11, base + 13)
               if y - 1 in cells and y in cells and y + 1 in cells
               and not cells[y - 1].get("collision_empty", True)
               and cells[y].get("collision_empty", False)
               and cells[y + 1].get("collision_empty", False)
               and (cells[y].get("fluid") or {}).get("empty", False)]
    return min(heights, key=lambda h: abs(h - player_y)) if heights else None


def walk_to_village(probe: ProbeSteps, detail: dict[str, Any]) -> dict[str, Any]:
    """Walk bounded, terrain-sampled local waypoints; retain every refused route."""
    target = village.VILLAGE_CENTER
    deadline = time.monotonic() + 600
    detail["approach_legs"] = []
    while time.monotonic() < deadline:
        state = probe.observe()
        player = state["player"]
        px, py, pz = float(player["x"]), float(player["y"]), float(player["z"])
        dx, dz = target[0] - px, target[2] - pz
        distance = math.hypot(dx, dz)
        if distance < 6:
            return state
        length = min(12.0, distance)
        wx, wz = px + dx / distance * length, pz + dz / distance * length
        candidates = [(wx, wz), (wx - dz / distance * 6, wz + dx / distance * 6),
                      (wx + dz / distance * 6, wz - dx / distance * 6)]
        advanced = False
        for cx, cz in candidates:
            x, z = math.floor(cx), math.floor(cz)
            base = math.floor(py)
            scan = probe.call("minecraft_scan_blocks", {
                "min_x": x, "max_x": x, "min_y": base - 12, "max_y": base + 14,
                "min_z": z, "max_z": z,
            })
            y = standable_height(scan.get("blocks", []), py)
            if y is None:
                detail["approach_legs"].append({"column": [x, z], "error": "no loaded dry standable cell"})
                continue
            leg = {"target": [x, y, z]}
            detail["approach_legs"].append(leg)
            try:
                leg["result"] = probe.call("minecraft_navigate_to_block",
                                           {"x": x, "y": y, "z": z, "timeout_seconds": 90.0})
                advanced = True
                break
            except RuntimeError as exc:
                leg["error"] = str(exc)
        if not advanced:
            raise RuntimeError(f"on-foot approach blocked near {[px, py, pz]}; all three sampled routes refused")
    raise TimeoutError("on-foot approach exceeded 600 seconds")


def scenario(probe: ProbeSteps, detail: dict[str, Any], artifact: Path, server: Any,
             server_log: Path, read_container: Callable[[tuple[int, int, int]], dict[str, int]]) -> None:
    spawn = probe.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float), "player_present")
    detail["player_spawn"] = {key: spawn["player"].get(key) for key in ("x", "y", "z")}
    detail["spawn_entities"] = probe.call("minecraft_list_entities", {"radius": 128, "limit": 48})
    detail["census_coverage"] = "Nearest at most 48 client-visible entities within 128 blocks of player; visible_count can exceed returned count, so this is not an exhaustive census."
    arrived = walk_to_village(probe, detail)
    detail["player_after_walk"] = {key: arrived["player"].get(key) for key in ("x", "y", "z")}
    detail["spawn_to_target_horizontal_blocks"] = math.dist(
        (float(spawn["player"]["x"]), float(spawn["player"]["z"])),
        (float(village.VILLAGE_CENTER[0]), float(village.VILLAGE_CENTER[2])))
    probe.check("walked_to_village_target", not detail.get("walk_timeout", False))
    native_client.capture(probe.client, artifact, "civilization-approach")
    line = None
    deadline = time.monotonic() + 180
    while not line and time.monotonic() < deadline:
        names = village._recent_chat(village._send(probe, "settlement list"), 20)
        line = next((line for line in names if line.startswith("Settlements:") and "No settlements" not in line), None)
        if not line:
            probe.call("minecraft_wait_ticks", {"ticks": 100})
    probe.check("generated_settlement_listed", bool(line))
    name = line.split(":", 1)[1].split(",")[0].strip().rstrip(".")
    info = village._recent_chat(village._send(probe, f"settlement info {name}"), 20)
    probe.check("generated_settlement_ownerless", any("owner=autonomous" in line for line in info))
    match = next((m for line in info if (m := re.search(r"anchor=\[(-?\d+),\s*(-?\d+),\s*(-?\d+)\]", line))), None)
    probe.check("settlement_anchor", match is not None)
    anchor = tuple(int(match.group(i)) for i in range(1, 4))
    detail.update(settlement_name=name, settlement_anchor=list(anchor), settlement_info=info)
    probe.call("minecraft_navigate_to_block", {"x": anchor[0], "y": anchor[1], "z": anchor[2]})
    probe.call("minecraft_wait_ticks", {"ticks": 100})
    chest = village._founding_storage(probe, anchor)
    detail["founding_chest"] = list(chest)
    detail["founding_stock_observed"] = village._chest_totals(village._open_chest(probe, chest))
    detail["stock_provenance"] = "Generated village container as first observed; no item grants or container writes. Workers may have delivered before observation."
    probe.call("minecraft_close_screen", {})
    claim = village._recent_chat(village._send(probe, "settlement claim"), 20)
    detail["claim_chat"] = claim
    probe.check("ordinary_player_claimed", any(line.startswith("Claimed ") for line in claim))
    detail["claim_screenshot"] = native_client.capture(probe.client, artifact, "civilization-claim")
    fund(probe, detail, server, chest)
    entities = probe.call("minecraft_list_entities", {"radius": 128, "limit": 48})
    detail["initial_entities"] = entities
    detail["initial_sheep_count"] = sum(
        1 for entity in entities.get("entities", [])
        if isinstance(entity, dict) and entity.get("entity_type") == "minecraft:sheep"
    )
    detail["interpretation"] = "Initial buildings, beds and villagers are generated baseline, not later construction or births."
    started = time.monotonic()
    detail["snapshots"] = []
    previous = None
    unchanged_since = 0
    seen_furnaces: set[tuple[int, int, int]] = set()
    iron_captured: set[tuple[int, int, int]] = set()
    advanced = 0
    while True:
        project_shot: dict[str, Any] = {}
        def capture_project() -> None:
            project_shot.update(native_client.capture(probe.client, artifact, f"civilization-project-{advanced:06d}"))

        status = village._planner_status(probe, detail, chest, read_container, capture_project)
        if "projects_error" in status or "storage_error" in status:
            raise RuntimeError(f"cannot establish planner or storage state: {status}")
        if not project_shot:
            raise RuntimeError("project status dialog was not observed and captured")
        entities = probe.call("minecraft_list_entities", {"radius": 128, "limit": 48})
        ecology = village._probe_ecology(probe, detail, status["buildings_now"])
        blocks = building_blocks(probe, status["buildings_now"])
        workstations = workstation_blocks(probe, chest)
        workstation_shots = []
        for furnace in workstations:
            pos = tuple(int(furnace[axis]) for axis in ("x", "y", "z"))
            try:
                furnace["container_totals"] = read_container(pos)
                furnace["container_observation"] = "read-only data get block; totals across slots"
            except Exception as exc:
                furnace["container_error"] = str(exc)
            if pos not in seen_furnaces:
                probe.call("minecraft_close_screen", {})
                player = probe.observe()["player"]
                dx, dz = pos[0] + 0.5 - player["x"], pos[2] + 0.5 - player["z"]
                dy = pos[1] + 0.5 - (player["y"] + 1.62)
                yaw = int(round(math.degrees(math.atan2(-dx, dz))))
                pitch = int(round(-math.degrees(math.atan2(dy, math.hypot(dx, dz)))))
                probe.call("minecraft_look", {"yaw_deg": yaw, "pitch_deg": pitch})
                workstation_shots.append(native_client.capture(
                    probe.client, artifact, f"civilization-workstation-{advanced:06d}"))
                seen_furnaces.add(pos)
            if (furnace.get("container_totals") or {}).get("minecraft:iron_ingot", 0) > 0 and pos not in iron_captured:
                furnace["iron_menu"] = capture_iron_menu(probe, artifact, pos, advanced)
                iron_captured.add(pos)
        shot = native_client.capture(probe.client, artifact, f"civilization-{advanced:06d}")
        row = {"sprint_ticks": advanced, "elapsed_seconds": time.monotonic() - started,
               "client_game_time": village._gametime(probe),
               "planner": status, "entities": entities, "ecology": ecology, "building_blocks": blocks,
               "workstation_blocks": workstations, "workstation_screenshots": workstation_shots,
               "screenshot": shot, "project_screenshot": project_shot}
        detail["snapshots"].append(row)
        # Unrelated resource growth must not conceal a stalled project.
        # Project UI retains its actual material blocker and progress.
        signature = (status.get("project"), status.get("project_ui"),
                     village._listed_buildings(status["buildings_now"]))
        if signature != previous:
            previous = signature
            unchanged_since = advanced
        elif advanced - unchanged_since >= STALL_TICKS:
            detail["stop"] = {"reason": "unchanged project status and buildings over a full day", "since_sprint_tick": unchanged_since, "at_sprint_tick": advanced}
            raise RuntimeError("sustained unchanged project status and buildings for one full game day")
        if advanced >= MAX_TICKS:
            detail["stop"] = {"reason": "bounded observation limit; no sustained stall established", "at_sprint_tick": advanced}
            raise RuntimeError("bounded observation ended without establishing a sustained stall")
        row["sprint_completions"] = []
        interval = 300 if advanced < 3000 else SPRINT_TICKS
        for _ in range(interval // 300):
            row["sprint_completions"].append(item_merge._sprint(server, server_log, 300, probe.remaining()))
            advanced += 300
