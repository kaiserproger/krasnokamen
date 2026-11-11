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

STAGES = ("new_house", "births", "second_building", "iron_smelted")
BIRTHS_LIMITATION = "Birth proof requires retained server claim/birth audit events and a current clean checkpoint."


def acceptance_verdict(stages: dict[str, bool], *, stalled: bool) -> dict[str, bool]:
    verdict = {name: stages.get(name, False) for name in STAGES}
    verdict["all"] = all(verdict.values()) and not stalled
    return verdict


def population(lines: list[str], name: str) -> int:
    match = re.search(rf"(?m)^{re.escape(name)} \|[^\n]*\bmembers=(\d+)\b", "\n".join(lines))
    if match is None:
        raise RuntimeError(f"settlement info for {name!r} has no member count")
    return int(match.group(1))


def newborn(entities: dict[str, Any], baseline_villagers: set[str],
            baseline_complete: bool) -> dict[str, Any] | None:
    if not baseline_complete:
        return None
    return next((e for e in entities.get("entities", [])
                 if e.get("entity_type") == "minecraft:villager"
                 and e.get("received_baby") is True and e.get("entity_uuid")
                 and e["entity_uuid"] not in baseline_villagers), None)

def baseline_server_tick(server: Any, log_path: Path, timeout: float) -> int:
    """Query the server after the population count; client clocks interpolate."""
    offset = log_path.stat().st_size
    item_merge._console(server, "time query gametime")

    def replied() -> tuple[bool, int]:
        with log_path.open("rb") as stream:
            stream.seek(offset)
            lines = stream.read().decode(errors="replace").splitlines()
        for line in lines:
            match = re.fullmatch(r"The time is (\d+)", line.strip())
            if match:
                value = int(match.group(1))
                if value >= 2_147_483_647:
                    raise RuntimeError("server game time exceeds the unwrapped query range")
                return True, value
        return False, 0

    return village.runtime.wait_for_path_condition(log_path, timeout, replied)


def verify_birth_snapshot(snapshot: dict[str, Any], name: str, baseline: int, final: int,
                          baby_uuid: str, baseline_tick: int, log: Any) -> dict[str, Any]:
    def fail(reason: str) -> dict[str, Any]:
        return {"verified": False, "status": "unverified", "reason": reason}

    if snapshot.get("read_only") is not True or not isinstance(snapshot.get("checkpoint_tick"), int):
        return fail("clean read-only checkpoint is unavailable")
    towns = [row.get("payload", {}).get("value", {}) for key, row in snapshot.get("globals", {}).items()
             if key.startswith("settlement/") and row.get("payload", {}).get("value", {}).get("name") == name]
    if len(towns) != 1 or not isinstance(towns[0].get("id"), int):
        return fail("settlement identity is missing or ambiguous")
    settlement = towns[0]["id"]
    claims = []
    births = []
    lines = log.splitlines() if isinstance(log, str) else log
    for line in lines:
        claim = re.search(r"\bsettlement_audit:\s+settlement_claim settlement=(\d+) tick=(\d+)\b", line)
        if claim:
            claims.append((int(claim.group(1)), int(claim.group(2))))
        birth = re.search(r"\bsettlement_audit:\s+settlement_birth child=(\S+) settlement=(\d+) household=(\S+) tick=(\d+)\b", line)
        if birth:
            births.append((birth.group(1), int(birth.group(2)), birth.group(3), int(birth.group(4))))
    if len(claims) != 1 or claims[0][0] != settlement or claims[0][1] > snapshot["checkpoint_tick"]:
        return fail("claim audit event is missing or ambiguous")
    claim_tick = claims[0][1]
    if (not isinstance(baseline_tick, int) or isinstance(baseline_tick, bool)
            or not claim_tick <= baseline_tick <= snapshot["checkpoint_tick"]):
        return fail("post-baseline tick is missing or outside the checkpoint")
    rows = [row for row in snapshot.get("entities", []) if row.get("active") and
            (row.get("payload") or {}).get("id") == "minecraft:villager" and
            (row["payload"].get("solaris:VillagerRuntime") or {}).get("settlement") == settlement and
            (row["payload"].get("Health") or 0) > 0]
    ids = [row.get("uuid") for row in rows]
    if len(rows) != final or any(not isinstance(uuid, str) or not uuid for uuid in ids) or len(set(ids)) != len(ids):
        return fail("snapshot member census does not match the observed count")
    if final < baseline + 2:
        return fail("population growth is below two")
    by_id = {row["uuid"]: row["payload"]["solaris:VillagerRuntime"] for row in rows}
    proven: set[str] = set()
    for child, born_in, household, tick in births:
        info = by_id.get(child)
        if born_in != settlement or not baseline_tick < tick <= snapshot["checkpoint_tick"] or info is None:
            continue
        sequence = info.get("birth_sequence")
        parents = info.get("birth_parents")
        if (info.get("household") == household and info.get("birth_tick") == tick
                and isinstance(sequence, int) and sequence // 65536 == tick
                and sequence % 65536 != 0 and isinstance(parents, list) and len(parents) == 2
                and all(isinstance(parent, str) and parent for parent in parents) and len(set(parents)) == 2):
            proven.add(child)
    if len(proven) < final - baseline or baby_uuid not in proven:
        return fail("net growth or pictured baby lacks a matching settlement birth event")
    return {"verified": True, "status": "verified", "settlement_id": settlement,
            "claim_tick": claim_tick, "baseline_tick": baseline_tick, "births": sorted(proven)}


def completed_project(building: dict[str, Any], baseline_buildings: set[int],
                      projects: list[dict[str, Any]],
                      blueprints: dict[str, dict[str, Any]]) -> dict[str, Any] | None:
    if building["id"] in baseline_buildings or building["natural"]:
        return None
    return next((p for p in reversed(projects)
                 if blueprints[p["blueprint"]]["kind"] == building["kind"]
                 and p["level"] == building["level"]
                 and village._blueprint_placements(building, blueprints[p["blueprint"]], p["origin"])), None)


def entity_census(probe: ProbeSteps, entity_type: str | None = None) -> dict[str, Any]:
    """Read every page, without shrinking the 128-block received-entity census."""
    arguments: dict[str, Any] = {"radius": 128, "limit": 16, "cursor": ""}
    if entity_type is not None:
        arguments["entity_type"] = entity_type
    rows: list[dict[str, Any]] = []
    seen: set[int] = set()
    total: int | None = None
    while True:
        page = probe.call("minecraft_list_entities", dict(arguments))
        if total is None:
            total = page["visible_count"]
        elif page["visible_count"] != total:
            raise RuntimeError("entity census membership changed during pagination")
        for entity in page["entities"]:
            identity = entity["entity_id"]
            if identity in seen or (entity_type is not None and entity["entity_type"] != entity_type):
                raise RuntimeError("entity census returned a duplicate identity or wrong type")
            seen.add(identity)
            rows.append(entity)
        cursor = page.get("next_cursor")
        if cursor is None:
            if page["truncated"] or len(rows) != page["visible_count"]:
                raise RuntimeError("entity census is incomplete or changed during pagination")
            return {**page, "entities": rows, "census_pages_complete": True}
        if (not page["truncated"] or not page["entities"] or
                (arguments["cursor"] and int(cursor) <= int(arguments["cursor"]))):
            raise RuntimeError("entity census cursor did not advance")
        arguments["cursor"] = cursor


def capture_house(probe: ProbeSteps, artifact: Path, label: str,
                  target: tuple[float, float, float],
                  blocks: list[dict[str, Any]]) -> dict[str, Any]:
    """Open the received entrance, prove the exact bed, then return outside."""
    door = next((block for block in blocks
                 if str(block.get("block_id", "")).endswith("_door")
                 and block.get("properties", {}).get("half") == "lower"), None)
    if door is None:
        raise RuntimeError("house capture has no received entrance door")
    probe.call("minecraft_close_screen", {})
    initial = probe.observe()["player"]
    position = {axis: door[axis] for axis in ("x", "y", "z")}
    probe.call("minecraft_navigate_to_block", {**position, "timeout_seconds": 120.0})
    current = probe.call("minecraft_read_block", position)
    if current.get("block_id") != door["block_id"]:
        raise RuntimeError("house entrance changed before ordinary use")
    if current.get("properties", {}).get("open") != "true":
        face = {"north": "south", "south": "north",
                "east": "west", "west": "east"}[current["properties"]["facing"]]
        probe.call("minecraft_look_at_block", {**position, "face": face})
        probe.call("minecraft_use_item_on", {**position, "face": face, "hand": "main_hand"})
    for offset, half in ((0, "lower"), (1, "upper")):
        probe.call("minecraft_wait_for_block_state", {
            **position, "y": position["y"] + offset, "block_id": door["block_id"],
            "properties": {"half": half, "open": "true"}, "timeout_seconds": 10.0,
        })
    shot = capture_target(probe, artifact, label, target)
    probe.call("minecraft_navigate_to_block", {
        "x": math.floor(initial["x"]), "y": math.floor(initial["y"]),
        "z": math.floor(initial["z"]), "timeout_seconds": 120.0,
    })
    return shot


def capture_target(probe: ProbeSteps, artifact: Path, label: str,
                   target: tuple[float, float, float], *,
                   entity: dict[str, Any] | None = None,
                   building: dict[str, Any] | None = None) -> dict[str, Any]:
    probe.call("minecraft_close_screen", {})
    initial = probe.observe()["player"]
    if entity:
        probe.call("minecraft_approach_entity", {"entity_id": entity["entity_id"], "timeout_seconds": 120.0})
        seen = entity_census(probe, "minecraft:villager")
        current = next((e for e in seen.get("entities", [])
                        if e.get("entity_uuid") == entity["entity_uuid"]), None)
        if current is None or current.get("received_baby") is not True:
            raise RuntimeError("baby no longer visible for milestone capture")
        target = (current["x"], current["y"] + 0.5, current["z"])
    else:
        destination = [math.floor(value) for value in target]
        if building:
            low, high = building["min"], building["max"]
            faces = ((low[0] - 1, low[0] - 1, low[2], high[2]),
                     (high[0] + 1, high[0] + 1, low[2], high[2]),
                     (low[0], high[0], low[2] - 1, low[2] - 1),
                     (low[0], high[0], high[2] + 1, high[2] + 1))
            blocks = []
            for min_x, max_x, min_z, max_z in faces:
                bounds = {"min_x": min_x, "max_x": max_x,
                          "min_y": low[1] - 2, "max_y": high[1] + 2,
                          "min_z": min_z, "max_z": max_z, "max_blocks": 4096}
                offset = 0
                while True:
                    scan = probe.call("minecraft_scan_blocks", {**bounds, "offset": offset})
                    blocks.extend(scan.get("blocks", []))
                    if not scan.get("truncated"):
                        break
                    following = scan.get("next_offset")
                    if not isinstance(following, int) or following <= offset:
                        raise RuntimeError("observer perimeter scan pagination did not advance")
                    offset = following
            destination = building_observer_stand(blocks, building, target[1], initial)
        probe.call("minecraft_navigate_to_block", {
            "x": destination[0], "y": destination[1], "z": destination[2],
            "timeout_seconds": 120.0,
        })
    player = probe.observe()["player"]
    dx, dy, dz = target[0] - player["x"], target[1] - (player["y"] + 1.62), target[2] - player["z"]
    probe.call("minecraft_look", {
        "yaw_deg": int(round(math.degrees(math.atan2(-dx, dz)))),
        "pitch_deg": int(round(-math.degrees(math.atan2(dy, math.hypot(dx, dz))))),
    })
    observed = probe.observe().get("target") or {}
    if entity:
        visible = observed.get("type") == "entity" and observed.get("entity_uuid") == entity["entity_uuid"]
    elif building:
        visible = observed.get("type") == "block" and all(
            building["min"][i] <= observed.get(axis, building["min"][i] - 1) <= building["max"][i]
            for i, axis in enumerate(("x", "y", "z")))
    else:
        visible = observed.get("type") == "block" and all(
            observed.get(axis) == math.floor(target[i]) for i, axis in enumerate(("x", "y", "z")))
    if not visible:
        raise RuntimeError(f"milestone target is occluded or out of reach: {label}: {observed}")
    shot = native_client.capture(probe.client, artifact, label)
    shot["observed_target"] = observed
    probe.call("minecraft_navigate_to_block", {
        "x": math.floor(initial["x"]), "y": math.floor(initial["y"]), "z": math.floor(initial["z"]),
        "timeout_seconds": 120.0,
    })
    return shot


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
                    if str(block.get("block_id", "")).endswith(("_bed", "_door", ":furnace", ":blast_furnace"))]
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


def building_observer_stand(blocks: list[dict[str, Any]], building: dict[str, Any],
                            target_y: float, player: dict[str, Any]) -> tuple[int, int, int]:
    """Choose one received exterior standpoint, not a route through a sealed room."""
    low, high = building["min"], building["max"]
    columns: dict[tuple[int, int], list[dict[str, Any]]] = {}
    for block in blocks:
        columns.setdefault((block["x"], block["z"]), []).append(block)
    best = None
    for (x, z), column in columns.items():
        on_face = ((x in (low[0] - 1, high[0] + 1) and low[2] <= z <= high[2])
                   or (z in (low[2] - 1, high[2] + 1) and low[0] <= x <= high[0]))
        if not on_face:
            continue
        height = standable_height(column, target_y)
        if height is None or not low[1] <= height + 1.62 <= high[1] + 1:
            continue
        distance = ((x + 0.5 - player["x"]) ** 2 + (height - player["y"]) ** 2
                    + (z + 0.5 - player["z"]) ** 2)
        rank = (distance, x, height, z)
        if best is None or rank < best:
            best = rank
    if best is None:
        raise RuntimeError("building capture has no received supported standing cell")
    return best[1:]


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
    detail["spawn_entities"] = entity_census(probe)
    detail["census_coverage"] = "Every received client-visible entity within 128 blocks, paged in groups of 16 without an aggregate limit; membership counts also come from settlement info."
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
    detail["baseline_info"] = village._recent_chat(village._send(probe, f"settlement info {name}"), 20)
    detail["baseline_population"] = population(detail["baseline_info"], name)
    detail["baseline_server_tick"] = baseline_server_tick(server, server_log, min(30.0, probe.remaining()))
    detail["birth_verdict"] = {"status": "unverified", "reason": BIRTHS_LIMITATION}
    detail["baseline_furnaces"] = workstation_blocks(probe, chest)
    baseline_furnaces = {tuple(int(block[axis]) for axis in ("x", "y", "z"))
                         for block in detail["baseline_furnaces"]}
    detail["claim_screenshot"] = native_client.capture(probe.client, artifact, "civilization-claim")
    fund(probe, detail, server, chest)
    # Claim and the conserved stock transfer above use ordinary survival play.
    # The remaining gate observes residents, not player hunger/combat survival.
    # Creative preserves container interaction and normal world/mob simulation.
    command = f"gamemode creative {village.USERNAME}"
    probe.report["fixture_commands"].append("console: " + command)
    item_merge._console(server, command)
    detail["observer_mode"] = "creative after claim and founding transfer; player survival is not tested"
    entities = entity_census(probe)
    detail["initial_entities"] = entities
    detail["initial_sheep_count"] = sum(
        1 for entity in entities.get("entities", [])
        if isinstance(entity, dict) and entity.get("entity_type") == "minecraft:sheep"
    )
    baseline_buildings = {b["id"] for b in detail["certified_buildings_before_stock"]}
    baseline_villagers = {e["entity_uuid"] for e in entities.get("entities", [])
                          if e.get("entity_type") == "minecraft:villager" and e.get("entity_uuid")}
    detail["baseline_client_complete"] = (not entities.get("truncated", True)
                                          and len(baseline_villagers) >= detail["baseline_population"])
    blueprints = {b["id"]: b for b in village._authored_blueprints()}
    projects: list[dict[str, Any]] = []
    detail["milestones"] = {}
    verdicts = detail.setdefault("verdicts", {**dict.fromkeys(STAGES, False), "all": False})
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
        project = status.get("project")
        if project and project["blueprint"] in blueprints and project not in projects:
            projects.append(project)
        entities = entity_census(probe)
        ecology = village._probe_ecology(probe, detail, status["buildings_now"], entities=entities)
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
            if ((furnace.get("container_totals") or {}).get("minecraft:iron_ingot", 0) > 0
                    and pos not in baseline_furnaces and pos not in iron_captured):
                furnace["iron_menu"] = capture_iron_menu(probe, artifact, pos, advanced)
                iron_captured.add(pos)
                detail["milestones"].setdefault("iron_smelted", {
                    "sprint_ticks": advanced, "screenshot": furnace["iron_menu"]["screenshot"],
                    "furnace": pos, "menu": furnace["iron_menu"]["container"],
                })
        shot = native_client.capture(probe.client, artifact, f"civilization-{advanced:06d}")
        row = {"sprint_ticks": advanced, "elapsed_seconds": time.monotonic() - started,
               "client_game_time": village._gametime(probe),
               "planner": status, "entities": entities, "ecology": ecology, "building_blocks": blocks,
               "workstation_blocks": workstations, "workstation_screenshots": workstation_shots,
               "screenshot": shot, "project_screenshot": project_shot}
        detail["snapshots"].append(row)
        probe.call("minecraft_close_screen", {})
        info_now = village._recent_chat(village._send(probe, f"settlement info {name}"), 20)
        current_population = population(info_now, name)
        row["population"] = current_population
        row["population_info"] = info_now
        if current_population >= detail["baseline_population"] + 2 and "growth_observation" not in detail:
            detail["growth_observation"] = {
                "sprint_ticks": advanced, "population": current_population, "info": info_now,
                "screenshot": native_client.capture(probe.client, artifact, f"civilization-census-{advanced:06d}"),
            }
        baby = newborn(entities, baseline_villagers, detail["baseline_client_complete"])
        if baby and "baby_observation" not in detail:
            target = (baby["x"], baby["y"] + 0.5, baby["z"])
            detail["baby_observation"] = {
                "sprint_ticks": advanced, "entity": baby,
                "screenshot": capture_target(probe, artifact, f"civilization-births-{advanced:06d}", target, entity=baby),
            }
        if ("growth_observation" in detail and "baby_observation" in detail
                and "births" not in detail["milestones"]):
            detail["milestones"]["births"] = {
                "sprint_ticks": advanced, "growth": detail["growth_observation"],
                "baby": detail["baby_observation"], "limitation": BIRTHS_LIMITATION,
                "screenshot": detail["baby_observation"]["screenshot"],
            }
        for evidence in blocks:
            building = evidence["building"]
            project = completed_project(building, baseline_buildings, projects, blueprints)
            if project is None:
                continue
            if building["kind"] == "house":
                bed = next((b for b in evidence.get("blocks", [])
                            if str(b.get("block_id", "")).endswith("_bed")), None)
                if bed and "new_house" not in detail["milestones"]:
                    target = tuple(bed[axis] + 0.5 for axis in ("x", "y", "z"))
                    detail["milestones"]["new_house"] = {
                        "sprint_ticks": advanced, "building": building, "project": project, "bed": bed,
                        "screenshot": capture_house(
                            probe, artifact, f"civilization-house-{advanced:06d}",
                            target, evidence.get("blocks", [])),
                    }
            elif "second_building" not in detail["milestones"]:
                target = tuple((building["min"][i] + building["max"][i] + 1) / 2 for i in range(3))
                detail["milestones"]["second_building"] = {
                    "sprint_ticks": advanced, "building": building, "project": project,
                    "screenshot": capture_target(probe, artifact, f"civilization-building-{advanced:06d}", target, building=building),
                }
        candidate = acceptance_verdict({name: name in detail["milestones"] for name in STAGES}, stalled=False)
        verdicts.update(candidate)
        verdicts["births"] = verdicts["all"] = False
        # Unrelated resource growth must not conceal a stalled project.
        # Project UI retains its actual material blocker and progress.
        signature = (status.get("project"), status.get("project_ui"),
                     village._listed_buildings(status["buildings_now"]))
        if signature != previous:
            previous = signature
            unchanged_since = advanced
        elif advanced - unchanged_since >= STALL_TICKS:
            verdicts["all"] = False
            detail["stop"] = {"reason": "unchanged project status and buildings over a full day", "since_sprint_tick": unchanged_since, "at_sprint_tick": advanced}
            raise RuntimeError("sustained unchanged project status and buildings for one full game day")
        if candidate["all"] and advanced < MAX_TICKS:
            detail["stop"] = {"reason": "client milestones reached; checkpoint verification pending", "at_sprint_tick": advanced}
            return
        if advanced >= MAX_TICKS:
            verdicts["all"] = False
            missing = [name for name in STAGES if not verdicts[name]]
            detail["stop"] = {"reason": "bounded observation limit", "at_sprint_tick": advanced, "missing": missing}
            raise RuntimeError(f"bounded observation limit reached; missing milestones: {', '.join(missing) or 'none (reached too late)'}")
        row["sprint_completions"] = []
        interval = 300 if advanced < 3000 else SPRINT_TICKS
        for _ in range(interval // 300):
            # Simulation throughput is not an MCP request latency. Await this
            # sprint's exact completion, bounded by the whole scenario deadline.
            row["sprint_completions"].append(item_merge._sprint(
                server, server_log, 300, probe.remaining(limit=None)))
            advanced += 300
