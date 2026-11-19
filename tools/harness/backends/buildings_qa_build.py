"""Buildings-qa settlement plumbing: listings and building records, the hand-built
shell, funding, site anchors, exterior captures and the adversarial breaks.

The client/report seams live on ``buildings_qa`` (the tests patch them there), so this
module reaches them as ``_qa.<name>`` and never binds them.
"""

from __future__ import annotations

import math
import re
import time as _time
from pathlib import Path
from typing import Any

from .. import runtime
from . import native_client
from .probe_steps import ProbeSteps
from . import buildings_qa as _qa
from .buildings_qa_blueprint import _scan_box
from .buildings_qa_consts import BROKEN_AUTHORED, PARALLEL_ORIGIN, PARALLEL_SITES, PARALLEL_STEP
from .buildings_qa_probe import (
    _block_id, _building_bounds, _chest_totals, _deposit_all_at, _match_kind, _open_chest_at,
    _project_origin, _props, _state_base, _verdict, _villagers,
)


def _walk_to(probe: ProbeSteps, detail: dict[str, Any],
             target: tuple[float, float, float], label: str) -> dict[str, Any]:
    probe.call("minecraft_navigate_to_block",
               {"x": int(target[0]), "y": int(target[1]), "z": int(target[2])})
    deadline = _time.monotonic() + 480.0
    state: dict[str, Any] = {}
    while _time.monotonic() < deadline:
        state = probe.observe()
        player = state.get("player") or {}
        try:
            dist = math.dist((float(player["x"]), float(player["z"])), (target[0], target[2]))
        except (KeyError, TypeError, ValueError):
            probe.call("minecraft_wait_ticks", {"ticks": 20})
            continue
        if dist < 6.0:
            break
        probe.call("minecraft_wait_ticks", {"ticks": 20})
    else:
        detail["walk_timeout"] = True
    return state


def _wait_building_level(probe: ProbeSteps, detail: dict[str, Any], kind_label: str,
                         level: int, budget_seconds: float) -> bool:
    deadline = _time.monotonic() + budget_seconds
    while _time.monotonic() < deadline:
        probe.call("minecraft_wait_ticks", {"ticks": 200})
        state = _qa._send(probe, "settlement building list")
        for line in _qa._chat_lines(state, 25):
            if kind_label.lower() in line.lower() and f"L{level}" in line:
                detail[f"build_poll_{kind_label}_L{level}"] = _qa._chat_lines(state, 8)
                return True
    return False


def _building_records(probe: ProbeSteps, kind: str) -> dict[int, tuple[int, tuple[int, int, int]]]:
    """Read current kind, IDs, levels and origins from a fresh listing."""
    import re
    state = _qa._send_fresh(probe, "settlement building list", ("Buildings:", "No buildings."))
    records = {}
    for line in _qa._chat_lines(state, 25):
        if not line.startswith("Buildings:"):
            continue
        for match in re.finditer(r"#(\d+)([^#]*)", line):
            segment = match.group(2)
            if not _match_kind(kind, segment):
                continue
            values = re.search(r"\blevel\s+(\d+)\s+\[(-?\d+),\s*(-?\d+),\s*(-?\d+)\]", segment)
            if values:
                records[int(match.group(1))] = (
                    int(values.group(1)),
                    (int(values.group(2)), int(values.group(3)), int(values.group(4))),
                )
    return records


def _current_certificate(probe: ProbeSteps, sub: dict[str, Any], tag: str, bp: str,
                         level: int) -> tuple[int, tuple[int, int, int]] | None:
    records = _qa._building_records(probe, bp)
    baseline = {int(building_id) for building_id in sub["baseline_building_ids"]}
    target = sub.get("building_id")
    candidates = [(building_id, current, origin) for building_id, (current, origin) in records.items()
                  if building_id not in baseline and (target is None or building_id == target)]
    if len(candidates) > 1:
        probe.check(f"{tag}_L{level}_unambiguous_id", False)
        raise RuntimeError(f"{bp} L{level}: multiple new building IDs: {candidates}")
    if not candidates:
        return None
    building_id, current, origin = candidates[0]
    if current > level:
        sub[f"L{level}_not_verified"] = f"building #{building_id} reached L{current} before L{level} verification"
        probe.check(f"{tag}_L{level}_not_overtaken", False)
        raise RuntimeError(sub[f"L{level}_not_verified"])
    if current != level:
        return None
    sub["building_id"] = building_id
    return building_id, origin


def _find_nearby_chest(probe: ProbeSteps, detail: dict[str, Any]) -> tuple[int, int, int] | None:
    state = probe.observe()
    player = state.get("player") or {}
    try:
        px, py, pz = int(float(player["x"])), int(float(player["y"])), int(float(player["z"]))
    except (KeyError, TypeError, ValueError):
        return None
    found: list[dict[str, Any]] = []
    try:
        for dx0, dy0, dz0 in ((-16, -8, -16), (0, -8, -16), (-16, -8, 0), (0, -8, 0),
                              (-16, 0, -16), (0, 0, -16), (-16, 0, 0), (0, 0, 0)):
            scan = probe.call("minecraft_scan_blocks", {
                "min_x": px + dx0, "min_y": py + dy0, "min_z": pz + dz0,
                "max_x": px + dx0 + 15, "max_y": py + dy0 + 7, "max_z": pz + dz0 + 15,
            })
            found.extend(b for b in scan.get("blocks", []) if isinstance(b, dict))
    except Exception as exc:
        detail["chest_scan_error"] = str(exc)
        return None
    best: tuple[int, int, int] | None = None
    best_dist = float("inf")
    for entry in found:
        name = str(entry.get("block_id", entry.get("id", "")))
        if "chest" not in name and "barrel" not in name:
            continue
        pos = (int(entry.get("x", 0)), int(entry.get("y", 0)), int(entry.get("z", 0)))
        dist = math.dist((pos[0], pos[2]), (px, pz))
        if dist < best_dist:
            best, best_dist = pos, dist
    if best is None:
        detail["chest_scan_miss"] = f"{len(found)} cells around {[px, py, pz]}, no chest/barrel"
        for cand in ((117, 70, 5), (118, 70, 5), (116, 70, 5)):
            try:
                blk = _qa._read(probe, cand)
                if "chest" in _block_id(blk) or "barrel" in _block_id(blk):
                    detail["chest_fallback"] = list(cand)
                    return cand
            except Exception:
                continue
    return best


def _site_center(bp: str) -> tuple[int, int, int]:
    gx, gz = PARALLEL_SITES[bp]
    return (PARALLEL_ORIGIN[0] + gx * PARALLEL_STEP, PARALLEL_ORIGIN[1],
            PARALLEL_ORIGIN[2] + gz * PARALLEL_STEP)


def _at_settlement_anchor(probe: ProbeSteps, sub: dict[str, Any]) -> None:
    """Select this settlement for proximity-scoped commands and menus."""
    x, y, z = sub["site"]
    probe.fixture(f"tp @s {x + 0.5} {y + 1.0} {z + 0.5}")
    probe.wait(lambda s: abs(float((s.get("player") or {}).get("x", -1e9)) - (x + 0.5)) < 2
               and abs(float((s.get("player") or {}).get("z", -1e9)) - (z + 0.5)) < 2,
               "settlement_anchor", timeout_seconds=60)


def _capture_level(probe: ProbeSteps, sub: dict[str, Any], artifact: Path, tag: str,
                   bp: str, level: int, origin: tuple[int, int, int]) -> None:
    """Capture an elevated exterior view; the exact block oracle remains authoritative."""
    turns = _qa._fit_orientation(probe, bp, level, origin)
    cells = _qa.oriented_cells(bp, level, origin, turns)
    xs, ys, zs = zip(*cells)
    cx, cy, cz = (min(xs) + max(xs) + 1) / 2, (min(ys) + max(ys) + 1) / 2, \
                 (min(zs) + max(zs) + 1) / 2
    x, y, z = min(xs) - 8, max(ys) + 8, min(zs) - 8
    support = (x, y - 1, z)
    probe.check(f"{tag}_L{level}_camera_space", _block_id(_qa._read(probe, support)) == "minecraft:air")
    _qa._close_any_screen(probe)
    probe.fixture("time set 3000")
    probe.fixture(f"setblock {support[0]} {support[1]} {support[2]} minecraft:barrier")
    try:
        probe.fixture(f"tp @s {x + 0.5} {y} {z + 0.5}")
        probe.wait(lambda s: abs(float((s.get("player") or {}).get("x", -1e9)) - (x + 0.5)) < 1
                   and abs(float((s.get("player") or {}).get("z", -1e9)) - (z + 0.5)) < 1
                   and float((s.get("player") or {}).get("y", -1e9)) >= y - 0.25,
                   f"{tag}_L{level}_camera", timeout_seconds=60)
        dx, dz = cx - (x + 0.5), cz - (z + 0.5)
        yaw = round(math.degrees(math.atan2(-dx, dz)))
        pitch = round(math.degrees(math.atan2(y + 1.62 - cy, math.hypot(dx, dz))))
        probe.call("minecraft_look", {"yaw_deg": yaw, "pitch_deg": pitch})
        # Let the HUD chat from the setup and listing commands fade before capture.
        probe.call("minecraft_wait_ticks", {"ticks": 240})
        sub[f"L{level}_visual_capture"] = native_client.capture(probe.client, artifact, f"{tag}-L{level}")
    finally:
        probe.fixture(f"setblock {support[0]} {support[1]} {support[2]} minecraft:air")
        _qa._at_settlement_anchor(probe, sub)


def _check_house_repair(probe: ProbeSteps, sub: dict[str, Any], tag: str) -> None:
    from . import village_autonomy_qa as village
    point = sub["L1_broken_cell"]
    restored = _block_id(_qa._read(probe, tuple(point))) == "minecraft:oak_log"
    progress = village._project_progress(village._project_status(probe))
    freed = (not progress or progress["blueprint"] != "house"
             or progress["origin"] != sub["L1_broken_origin"] or progress["level"] != 1)
    probe.check(f"{tag}_L1_broken_cell_repaired", restored and freed)
    _verdict(sub, "L1_broken_cell_repaired", "PASS" if restored and freed else "FAIL",
             f"cell {point} restored={restored}, original project ended={freed}")


def _break_placed_house_cell(probe: ProbeSteps, sub: dict[str, Any], tag: str) -> None:
    """Break an observed, previously placed log while the same House L1 is active."""
    from . import village_autonomy_qa as village
    probe.fixture("give @s minecraft:diamond_axe 1")
    point = None
    origin = None
    for _ in range(80):
        progress = village._project_progress(village._project_status(probe))
        if progress and progress["blueprint"] == "house" and progress["level"] == 1:
            origin = tuple(progress["origin"])
            candidates = set()
            for turns in range(4):
                candidates.update(pos for pos, block in _qa.cells_true("house", 1, origin, turns).items()
                                  if _state_base(block) == "minecraft:oak_log" and pos[1] > origin[1])
            if candidates:
                lo = tuple(min(p[i] for p in candidates) for i in range(3))
                hi = tuple(max(p[i] for p in candidates) for i in range(3))
                world = _scan_box(probe, lo, hi)
                point = next((pos for pos in sorted(candidates)
                              if world.get(pos) == "minecraft:oak_log"
                              and world.get((pos[0] - 1, pos[1], pos[2]), "minecraft:air") == "minecraft:air"), None)
                if point:
                    break
        probe.call("minecraft_wait_ticks", {"ticks": 20})
    probe.check(f"{tag}_L1_placed_cell_found", point is not None)
    x, y, z = point
    probe.fixture(f"tp @s {x - 1.5} {y} {z + 0.5}")
    probe.fixture("gamemode survival @s")
    probe.call("minecraft_select_hotbar_item", {"item_id": "minecraft:diamond_axe", "count": 1,
                                                "timeout_seconds": probe.remaining()})
    active = village._project_progress(village._project_status(probe))
    probe.check(f"{tag}_L1_break_during_project", bool(active) and active["origin"] == list(origin)
                and active["blueprint"] == "house" and active["level"] == 1)
    probe.call("minecraft_look_at_block", {"x": x, "y": y, "z": z, "face": "west"})
    broken = probe.call("minecraft_break_block", {"x": x, "y": y, "z": z, "face": "west",
                        "expected_drop_item_id": "minecraft:oak_log", "expected_drop_count": 0,
                        "timeout_seconds": probe.remaining()})
    probe.check(f"{tag}_L1_player_broke_cell", broken.get("became_air") is True
                and _block_id(_qa._read(probe, point)) == "minecraft:air")
    sub["L1_broken_cell"] = list(point)
    sub["L1_broken_origin"] = list(origin)
    probe.fixture("gamemode creative @s")


def _corner_posts(origin: tuple[int, int, int]) -> list[tuple[tuple[int, int, int], int]]:
    """House L1's lower-layer `oak_log[axis=y]` corner posts for each rotation about the project
    origin, as (cell, quarter turns). The rotation is chosen by the layout planner and not shown
    before the build, and L1 sits at a different offset of the maximum frame for each one."""
    posts = []
    for turns in range(4):
        for pos, want in _qa.cells_true("house", 1, origin, turns).items():
            if pos[1] == origin[1] + 1 and _state_base(want) == BROKEN_AUTHORED \
                    and "axis=y" in want and (pos, turns) not in posts:
                posts.append((pos, turns))
    return posts


def _break_corner(probe: ProbeSteps, sub: dict[str, Any], tag: str,
                  bp: str) -> tuple[int, int, int] | None:
    """SETTLEMENT-BUILDER-PROJECT-STALLS-AFTER-PLACED-BLOCK-BROKEN, first half: once the Builder
    has placed one of the house's lower corner posts, a survival-mode player breaks it with the
    real client. Returns the cell, or None when the break could not be made while the project was
    still open."""
    corner = None
    turned = 0
    deadline = _time.monotonic() + 900.0
    while corner is None and _time.monotonic() < deadline:
        origin = _project_origin(probe)
        if origin is not None:
            for candidate, turns in _corner_posts(origin):
                try:
                    block = _qa._read(probe, candidate)
                except Exception:
                    continue
                if _block_id(block) == BROKEN_AUTHORED and _props(block).get("axis") == "y":
                    corner, turned = candidate, turns
                    break
        if corner is None:
            probe.call("minecraft_wait_ticks", {"ticks": 100})
    sub["broken_cell"] = {"cell": list(corner) if corner else None, "authored": BROKEN_AUTHORED}
    if corner is None or _building_bounds(probe, bp, 1) is not None:
        probe.check(f"{tag}_broken_cell_injected", False)
        return None
    x, y, z = corner
    # The listing and menus address the settlement that contains the player, and the generated
    # village here overlaps this one: go back to where the player stood once the cell is broken.
    me = probe.observe().get("player") or {}
    home = (float(me.get("x", x)), float(me.get("y", y)), float(me.get("z", z)))
    # Diagonally outside the footprint, within reach of the corner's outer face.
    footprint = [pos for pos in _qa.cells_true("house", 1, origin, turned)]
    sx = -1 if x == min(p[0] for p in footprint) else 1
    sz = -1 if z == min(p[2] for p in footprint) else 1
    probe.fixture(f"tp @s {x + sx + 0.5} {y} {z + sz + 0.5}")
    probe.fixture("gamemode survival @s")
    probe.call("minecraft_wait_ticks", {"ticks": 20})
    broken = probe.call("minecraft_break_block", {
        "x": x, "y": y, "z": z, "face": "north" if sz < 0 else "south",
        "expected_drop_item_id": BROKEN_AUTHORED, "expected_drop_count": 1,
        "timeout_seconds": probe.remaining()})
    probe.fixture("gamemode creative @s")
    probe.fixture(f"tp @s {home[0]} {home[1]} {home[2]}")
    read_back = None
    for _ in range(30):
        read_back = _block_id(_qa._read(probe, corner))
        if read_back == "minecraft:air":
            break
        probe.call("minecraft_wait_ticks", {"ticks": 10})
    sub["broken_cell"]["break_result"] = broken
    sub["broken_cell"]["read_back_after_break"] = read_back
    sub["broken_cell"]["project_open_at_break"] = _building_bounds(probe, bp, 1) is None
    ok = read_back == "minecraft:air" and sub["broken_cell"]["project_open_at_break"]
    probe.check(f"{tag}_broken_cell_injected", ok)
    return corner if ok else None


def _broken_replaced(probe: ProbeSteps, sub: dict[str, Any], tag: str,
                     corner: tuple[int, int, int]) -> None:
    """Second half, once the building list shows the certificate: the authored block is back in
    the broken cell, so the project finished instead of stalling on the hole."""
    final = _block_id(_qa._read(probe, corner))
    sub["broken_cell"]["at_certificate"] = final
    ok = final == BROKEN_AUTHORED
    probe.check(f"{tag}_broken_cell_replaced", ok)
    _verdict(sub, "broken_cell_replaced", "PASS" if ok else "FAIL",
             f"{list(corner)}: broken by a survival player mid-build, {final} at the certificate")


def _hand_storage(probe: ProbeSteps, sub: dict[str, Any], tag: str,
                  settlement: str) -> tuple[int, int, int] | None:
    """Hand-built storage SHELL fixture: 5x5x5 certifiable box + 1 chest.

    NOT the blueprint building: a player-placed certificate box (fixture),
    certified via chat, that funds later Builder-built blueprint projects.
    Precedent: army-qa storage hut. Placed on flat ground near the player.
    """
    state = probe.observe()
    player = state.get("player") or {}
    try:
        px = int(math.floor(float(player["x"])))
        py = int(math.floor(float(player["y"])))
        pz = int(math.floor(float(player["z"])))
    except (KeyError, TypeError, ValueError):
        probe.check(f"{tag}_shell_site", False)
        _verdict(sub, "shell_site", "FAIL", "no player pos")
        return None
    # Flat pad WELL clear of generated buildings (shell must not overlap any
    # certificate): 60 blocks east of the player, outside the village core.
    sx, sy, sz = px + 60, py, pz
    ex, ey, ez = sx + 4, sy + 4, sz + 4
    probe.fixture(f"fill {sx - 2} {sy - 2} {sz - 2} {ex + 2} {sy - 2} {ez + 2} minecraft:dirt")
    probe.fixture(f"fill {sx - 2} {sy - 1} {sz - 2} {ex + 2} {sy - 1} {ez + 2} minecraft:grass_block")
    probe.fixture(f"fill {sx} {sy} {sz} {ex} {sy} {ez} minecraft:oak_planks")
    probe.fixture(f"fill {sx} {ey} {sz} {ex} {ey} {ez} minecraft:oak_planks")
    for yy in (sy + 1, sy + 2, sy + 3):
        probe.fixture(f"fill {sx} {yy} {sz} {ex} {yy} {sz} minecraft:cobblestone")
        probe.fixture(f"fill {sx} {yy} {ez} {ex} {yy} {ez} minecraft:cobblestone")
        probe.fixture(f"fill {sx} {yy} {sz} {sx} {yy} {ez} minecraft:cobblestone")
        probe.fixture(f"fill {ex} {yy} {sz} {ex} {yy} {ez} minecraft:cobblestone")
    probe.fixture(f"setblock {ex} {sy + 1} {sz + 2} minecraft:air")
    probe.fixture(f"setblock {ex} {sy + 2} {sz + 2} minecraft:air")
    probe.fixture(f"setblock {ex} {sy + 1} {sz + 2} minecraft:oak_door[facing=west,half=lower,hinge=left,open=false]")
    probe.fixture(f"setblock {ex} {sy + 2} {sz + 2} minecraft:oak_door[facing=west,half=upper,hinge=left,open=false]")
    # Step block outside the door: shell floor top (sy+1) is one above the
    # outside grass; without a step villagers drop out but can never jump
    # back in (head hits the doorway top). Product S1 geometry trap.
    probe.fixture(f"setblock {ex + 1} {sy} {sz + 2} minecraft:cobblestone")
    probe.fixture(f"fill {sx + 1} {sy + 1} {sz + 1} {ex - 1} {sy + 2} {ez - 1} minecraft:air")
    chest = (sx + 1, sy + 1, sz + 1)
    probe.call("minecraft_wait_ticks", {"ticks": 20})
    raw = _qa._read(probe, chest)
    got = _block_id(raw)
    sub["shell_chest_raw"] = {"block_id": got, "pos": list(chest)}
    if got != "minecraft:chest":
        probe.check(f"{tag}_shell_chest", False)
        _verdict(sub, "shell_chest", "FAIL", f"chest holds {got}")
        return None
    probe.call("minecraft_send_chat", {"message": f"settlement building certify storage {sx} {sy} {sz} {ex} {ey} {ez}", "command": True})
    probe.call("minecraft_wait_ticks", {"ticks": 40})
    listed = _qa._chat_lines(probe.observe(), 30)
    sub["shell_cert_chat"] = listed
    ok = any("Certified storage" in line or ("storage" in line.lower() and "level 1" in line) for line in listed)
    probe.check(f"{tag}_shell_certified", ok)
    _verdict(sub, "shell_storage_certified", "PASS" if ok else "FAIL", f"{listed[:6]}")
    sub["shell_bounds"] = [[sx, sy, sz], [ex, ey, ez]]
    return chest if ok else None


def _hand_storage_at(probe: ProbeSteps, sub: dict[str, Any], tag: str,
                     settlement: str, center: tuple[int, int, int]) -> tuple[int, int, int] | None:
    """Shell variant placed at an explicit site center (parallel flow)."""
    cx, cy, cz = center
    sx, sy, sz = cx + 20, cy, cz + 20
    ex, ey, ez = sx + 4, sy + 4, sz + 4
    probe.fixture(f"fill {sx - 2} {sy - 2} {sz - 2} {ex + 2} {sy - 2} {ez + 2} minecraft:dirt")
    probe.fixture(f"fill {sx - 2} {sy - 1} {sz - 2} {ex + 2} {sy - 1} {ez + 2} minecraft:grass_block")
    probe.fixture(f"fill {sx} {sy} {sz} {ex} {sy} {ez} minecraft:oak_planks")
    probe.fixture(f"fill {sx} {ey} {sz} {ex} {ey} {ez} minecraft:oak_planks")
    for yy in (sy + 1, sy + 2, sy + 3):
        probe.fixture(f"fill {sx} {yy} {sz} {ex} {yy} {sz} minecraft:cobblestone")
        probe.fixture(f"fill {sx} {yy} {ez} {ex} {yy} {ez} minecraft:cobblestone")
        probe.fixture(f"fill {sx} {yy} {sz} {sx} {yy} {ez} minecraft:cobblestone")
        probe.fixture(f"fill {ex} {yy} {sz} {ex} {yy} {ez} minecraft:cobblestone")
    probe.fixture(f"setblock {ex} {sy + 1} {sz + 2} minecraft:air")
    probe.fixture(f"setblock {ex} {sy + 2} {sz + 2} minecraft:air")
    probe.fixture(f"setblock {ex} {sy + 1} {sz + 2} minecraft:oak_door[facing=west,half=lower,hinge=left,open=false]")
    probe.fixture(f"setblock {ex} {sy + 2} {sz + 2} minecraft:oak_door[facing=west,half=upper,hinge=left,open=false]")
    probe.fixture(f"setblock {ex + 1} {sy} {sz + 2} minecraft:cobblestone")
    probe.fixture(f"fill {sx + 1} {sy + 1} {sz + 1} {ex - 1} {sy + 2} {ez - 1} minecraft:air")
    chest = (sx + 1, sy + 1, sz + 1)
    probe.fixture(f"setblock {chest[0]} {chest[1]} {chest[2]} minecraft:chest[facing=north,type=single,waterlogged=false]")
    try:
        probe.call("minecraft_wait_for_block_state", {"x": chest[0], "y": chest[1], "z": chest[2], "block_id": "minecraft:chest", "timeout_seconds": 30})
    except Exception as exc:
        sub["shell_chest_wait_error"] = str(exc)[:200]
    raw = _qa._read(probe, chest)
    got = _block_id(raw)
    sub["shell_chest_raw"] = {"block_id": got, "pos": list(chest)}
    if got != "minecraft:chest":
        probe.fixture(f"setblock {chest[0]} {chest[1]} {chest[2]} minecraft:chest[facing=north,type=single,waterlogged=false]")
        probe.call("minecraft_wait_ticks", {"ticks": 40})
        got = _block_id(_qa._read(probe, chest))
        sub["shell_chest_retry"] = got
    if got != "minecraft:chest":
        probe.check(f"{tag}_shell_chest", False)
        _verdict(sub, "shell_chest", "FAIL", f"chest holds {got}")
        return None
    probe.call("minecraft_send_chat", {"message": f"settlement building certify storage {sx} {sy} {sz} {ex} {ey} {ez}", "command": True})
    listed: list[str] = []
    try:
        end = probe.wait(lambda s: any("Certified storage" in str(l) or ("storage" in str(l).lower() and "level 1" in str(l))
                                       for l in _qa._chat_lines(s, 80)), f"{tag}_shell_cert_echo")
        listed = _qa._chat_lines(end, 80)
    except Exception:
        probe.call("minecraft_wait_ticks", {"ticks": 120})
        listed = _qa._chat_lines(probe.observe(), 80)
    sub["shell_cert_chat"] = listed
    ok = any("Certified storage" in line or ("storage" in line.lower() and "level 1" in line) for line in listed)
    probe.check(f"{tag}_shell_certified", ok)
    _verdict(sub, "shell_storage_certified", "PASS" if ok else "FAIL", f"{listed[:6]}")
    sub["shell_bounds"] = [[sx, sy, sz], [ex, ey, ez]]
    return chest if ok else None


def _building_bounds_for(probe: ProbeSteps, settlement: str, bp: str, level: int,
                         exclude: list[list[int]] | None = None) -> tuple[int, int, int] | None:
    """Bounds for OUR kind+level: tp to OUR site first (listing is positional)."""
    import re as _re
    cx, cy, cz = _site_center(bp)
    probe.fixture(f"tp @s {cx + 0.5} {cy + 1.0} {cz + 0.5}")
    probe.call("minecraft_wait_ticks", {"ticks": 20})
    probe.call("minecraft_send_chat", {"message": "settlement building list", "command": True})
    probe.call("minecraft_wait_ticks", {"ticks": 20})
    for line in _qa._chat_lines(probe.observe(), 30):
        if not _match_kind(bp, line) or f"level {level}" not in line:
            continue
        for seg in _re.split(r"#\d+", line):
            if not _match_kind(bp, seg) or f"level {level}" not in seg:
                continue
            m = _re.search(r"\[(-?\d+),\s*(-?\d+),\s*(-?\d+)\]", seg)
            if m:
                origin = (int(m.group(1)), int(m.group(2)), int(m.group(3)))
                if exclude and list(origin) in exclude:
                    continue
                return origin
    return None


def _building_seq_in(probe: ProbeSteps, settlement: str, bp: str, level: int,
                     exclude: list[list[int]] | None = None) -> int | None:
    import re as _re
    cx, cy, cz = _site_center(bp)
    probe.fixture(f"tp @s {cx + 0.5} {cy + 1.0} {cz + 0.5}")
    probe.call("minecraft_wait_ticks", {"ticks": 20})
    probe.call("minecraft_send_chat", {"message": "settlement building list", "command": True})
    probe.call("minecraft_wait_ticks", {"ticks": 20})
    for line in _qa._chat_lines(probe.observe(), 30):
        if not _match_kind(bp, line) or f"level {level}" not in line:
            continue
        for m in _re.finditer(r"#(\d+)([^#]*)", line):
            seg = m.group(2)
            if not _match_kind(bp, seg) or f"level {level}" not in seg:
                continue
            if exclude:
                b = _re.search(r"\[(-?\d+),\s*(-?\d+),\s*(-?\d+)\]", seg)
                if b and [int(b.group(1)), int(b.group(2)), int(b.group(3))] in exclude:
                    continue
            return int(m.group(1))
    return None


def _bound_count(probe: ProbeSteps, settlement: str) -> int:
    """Current bound members from settlement info (chain-resume guard)."""
    import re as _re
    state = _qa._send(probe, f"settlement info {settlement}")
    members = 0
    for line in _qa._chat_lines(state):
        m = _re.search(r"members=(\d+)", line)
        if m:
            members = max(members, int(m.group(1)))
    return members


def _free_summon_cells(probe: ProbeSteps, near: tuple[int, int, int], count: int) -> list[tuple[int, int, int]]:
    """Free cells outside the shell: air at foot+head, solid below (per-cell reads)."""
    out: list[tuple[int, int, int]] = []
    ring = 6
    for dx in range(-ring, ring + 1):
        for dz in range(-ring, ring + 1):
            if len(out) >= count:
                return out
            x, z = near[0] + dx, near[2] + dz
            for dy in (1, 0, 2):
                y = near[1] + dy
                try:
                    foot = _block_id(_qa._read(probe, (x, y, z)))
                    head = _block_id(_qa._read(probe, (x, y + 1, z)))
                    below = _block_id(_qa._read(probe, (x, y - 1, z)))
                except Exception:
                    continue
                if foot == "minecraft:air" and head == "minecraft:air" and below != "minecraft:air":
                    out.append((x, y, z))
                    break
    return out


def _summon_bind(probe: ProbeSteps, sub: dict[str, Any], tag: str,
                 settlement: str, near: tuple[int, int, int], count: int) -> bool:
    """Summon plain villagers at the shell and bind each to the settlement."""
    cells = _free_summon_cells(probe, near, count) or [(near[0], near[1] + 1, near[2])] * count
    taken: set[str] = set()
    for i in range(count):
        cx, cy, cz = cells[min(i, len(cells) - 1)]
        probe.fixture(f"summon minecraft:villager {cx + 0.5} {cy} {cz + 0.5}")
        probe.call("minecraft_wait_ticks", {"ticks": 20})
        cands = [r for r in _villagers(probe) if str(r.get("entity_uuid")) not in taken]
        if not cands:
            continue
        pick = min(cands, key=lambda r: abs(float(r["x"]) - near[0]) + abs(float(r["z"]) - near[2]))
        uuid = str(pick["entity_uuid"])
        probe.fixture(f"settlement bind {settlement} {uuid}")
        taken.add(uuid)
    probe.call("minecraft_wait_ticks", {"ticks": 40})
    state = _qa._send(probe, f"settlement info {settlement}")
    members = 0
    import re as _re
    lines = _qa._chat_lines(state)
    for line in lines:
        m = _re.search(r"members=(\d+)", line)
        if m:
            members = max(members, int(m.group(1)))
    sub["bound_members"] = members
    sub["bound_info_lines"] = lines[:6]
    ok = len(taken) >= 2 and members >= 2
    probe.check(f"{tag}_bound", ok)
    _verdict(sub, "bound_residents", "PASS" if ok else "FAIL",
             f"summoned={len(taken)} members={members}")
    return ok


def _building_seq(probe: ProbeSteps, bp: str, level: int,
                  exclude: list[list[int]] | None = None) -> int | None:
    """Global building id (#<n>) of our kind+level from the listing.
    Same per-segment rule as _building_bounds: the id must come from the
    segment that names our kind, not the first #id on a packed line."""
    import re as _re
    state = _qa._send(probe, "settlement building list")
    for line in _qa._chat_lines(state, 25):
        if not _match_kind(bp, line) or f"level {level}" not in line:
            continue
        for m in _re.finditer(r"#(\d+)([^#]*)", line):
            seg = m.group(2)
            if not _match_kind(bp, seg) or f"level {level}" not in seg:
                continue
            if exclude:
                b = _re.search(r"\[(-?\d+),\s*(-?\d+),\s*(-?\d+)\]", seg)
                if b and [int(b.group(1)), int(b.group(2)), int(b.group(3))] in exclude:
                    continue
            return int(m.group(1))
    return None


def _adversarial_break(probe: ProbeSteps, sub: dict[str, Any], tag: str,
                       bp: str, level: int, bid: int) -> bool:
    """Break one placed block mid-upgrade-build (bounded: torch/fence class only)."""
    import re as _re
    if not _qa._project_active_quiet(probe, sub, tag, bp, level, upgrade_bid=bid):
        return False
    state = _qa._send_fresh(probe, "settlement building list", ("Buildings:", "No buildings."))
    bounds = None
    for line in _qa._chat_lines(state, 25):
        match = _re.search(rf"#{bid}(?!\d)([^#]*)", line)
        if match and _match_kind(bp, match.group(1)):
            points = _re.findall(r"\[(-?\d+),\s*(-?\d+),\s*(-?\d+)\]", match.group(1))
            if len(points) >= 2:
                bounds = [(int(a), int(b), int(c)) for a, b, c in points[:2]]
                break
    if not bounds:
        return False
    (x0, y0, z0), (x1, y1, z1) = bounds
    for x in range(x0, x1 + 1):
        for y in range(y0, y1 + 1):
            for z in range(z0, z1 + 1):
                got = _block_id(_qa._read(probe, (x, y, z)))
                if got in ("minecraft:torch", "minecraft:wall_torch", "minecraft:oak_fence"):
                    if not _qa._project_active_quiet(probe, sub, tag, bp, level, upgrade_bid=bid):
                        return False
                    probe.fixture(f"setblock {x} {y} {z} minecraft:air")
                    sub["adversarial_break"] = [x, y, z, got]
                    return True
    return False


def _base_materials(listing: str, level: int) -> dict[str, int]:
    """The `base {...}` of level `level` in `tools/blueprints.py materials` output; a station
    kind appends `; stations [...]` after the dict."""
    import ast
    for line in listing.splitlines():
        if f"L{level}:" in line and "base" in line:
            try:
                return dict(ast.literal_eval(line.split("base", 1)[1].split("; stations", 1)[0].strip()))
            except (ValueError, SyntaxError):
                return {}
    return {}


def _fund_level(probe: ProbeSteps, sub: dict[str, Any], bp: str, level: int,
                chest: tuple[int, int, int]) -> None:
    """Deposit 1.5x base materials into real storage and assert the stock delta.

    A deliberately unfundable pending order fences autonomous upgrades until
    each completed level has been verified."""
    import subprocess as _sp
    out = _sp.run(["python3", "tools/blueprints.py", "materials", bp],
                  capture_output=True, text=True, cwd=str(runtime.REPO_ROOT)).stdout
    base = _base_materials(out, level)
    item_of = {"cobblestone": "minecraft:cobblestone", "oak_log": "minecraft:oak_log",
               "white_wool": "minecraft:white_wool", "dirt": "minecraft:dirt",
               "wheat_seeds": "minecraft:wheat_seeds", "coal": "minecraft:coal"}
    want_items = [item_of.get(k, f"minecraft:{k}") for k in base]
    before = _chest_totals(_open_chest_at(probe, chest))
    for base_name, count in base.items():
        item = item_of.get(base_name, f"minecraft:{base_name}")
        need = int(count * 1.5) + 1
        while need > 0:
            give = min(64, need)
            probe.fixture(f"give @s {item} {give}")
            need -= give
    probe.call("minecraft_wait_ticks", {"ticks": 20})
    deposit = _deposit_all_at(probe, chest, want_items)
    sub[f"L{level}_deposit"] = deposit
    totals = _chest_totals(_open_chest_at(probe, chest))
    sub[f"L{level}_storage_totals"] = totals
    gained = {k: totals.get(k, 0) - before.get(k, 0) for k in want_items
              if totals.get(k, 0) > before.get(k, 0)}
    sub[f"L{level}_stock_gained"] = gained
    ok = bool(gained)
    probe.check(f"buildings_{bp}_L{level}_funded", ok)
    _verdict(sub, f"L{level}_funded", "PASS" if ok else "FAIL",
             f"stock delta={gained} want={want_items}")
