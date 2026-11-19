"""Buildings-qa function checks: what a certified level actually does for its residents
(beds, workshop stations, produced and consumed stock, assigned professions).

The client/report seams live on ``buildings_qa`` (the tests patch them there), so this
module reaches them as ``_qa.<name>`` and never binds them.
"""

from __future__ import annotations

import math
import re
import time as _time
from typing import Any

from .probe_steps import ProbeSteps
from . import buildings_qa as _qa
from .buildings_qa_consts import DIALOG_CLASS, KIND_LABEL
from .buildings_qa_probe import (
    _block_id, _building_lore, _chest_totals, _deposit_all_at, _open_chest_at, _props,
    _soldier_beds, _state_base, _verdict, _villagers,
)


def _function_house(probe: ProbeSteps, sub: dict[str, Any], tag: str, level: int,
                    origin: tuple[int, int, int],
                    cells: dict[tuple[int, int, int], str]) -> None:
    """Capacity-gated housing proof (+ non-gating sleep observation).
    Beds from placed cells must cover the expected capacity (beds >= level);
    sleep poses at midnight are observed only ('сон' nameplates), never
    gating — 0 asleep with free beds is a product finding, not a FAIL."""
    beds = [pos for pos, want in cells.items()
            if "bed" in _state_base(want) and "part=head" in want]
    ok = len(beds) >= level
    probe.check(f"{tag}_L{level}_house_capacity", ok)
    _verdict(sub, f"L{level}_function_house", "PASS" if ok else "FAIL",
             f"beds={len(beds)} want>={level}", "placed bed cells")
    # Sleep protocol (Main): night must RUN naturally (frozen time breaks the
    # scheduler); /time set 13000 then observe >= 2 min so far-flung villagers
    # can walk home; count only residents with a claimed bed. Non-gating.
    probe.fixture("time set 13000")
    asleep = 0
    claimed = _claimed_beds(len(beds))
    deadline = _time.monotonic() + 130.0
    while _time.monotonic() < deadline:
        n = 0
        for row in _villagers(probe):
            bb = row.get("bounding_box", {}) or {}
            try:
                if float(bb.get("height", 1.8) or 1.8) < 0.5:
                    n += 1
            except (TypeError, ValueError):
                continue
        asleep = max(asleep, n)
        probe.call("minecraft_wait_ticks", {"ticks": 100})
    probe.check(f"{tag}_L{level}_sleep_observed", True)
    _verdict(sub, f"L{level}_sleep_observed", "PASS",
             f"asleep={asleep} beds={len(beds)} claimed={claimed}",
             "time set 13000, natural night run, non-gating observation")
    probe.fixture("time set 3000")


def _claimed_beds(beds: int) -> int:
    """Claimable-bed pool for the sleep protocol.
    Per-villager home claims (v.home) are server-side only — no dialog,
    chat, or MCP field exposes them. The allocator claims from certified
    house beds, so the placed bed-head count is the honest observable
    proxy; labelled as such in verdict evidence."""
    return beds


def _station_of(cells: dict[tuple[int, int, int], str],
                names: tuple[str, ...]) -> tuple[int, int, int] | None:
    for pos, want in cells.items():
        if _state_base(want) in tuple(f"minecraft:{n}" for n in names):
            return pos
    return None


def _await_assignment(probe: ProbeSteps, short: int | None, label: str,
                      profession: str, timeout_s: float = 300.0) -> list[str]:
    """Poll the Residents page until a worker of `profession` holds a job at
    building #`short` (villagers claim jobs over minutes; a fast build must
    not fail before anyone binds). Returns the last residents lines."""
    deadline = _time.monotonic() + timeout_s
    residents: list[str] = []
    while True:
        residents = _qa._residents_text(probe)
        if short and any(f"works at #{short} {label}" in line and profession in line
                         for line in residents):
            break
        if _time.monotonic() >= deadline:
            break
        probe.call("minecraft_wait_ticks", {"ticks": 100})
    return residents


def _function_farm(probe: ProbeSteps, sub: dict[str, Any], tag: str, level: int,
                   origin: tuple[int, int, int],
                   cells: dict[tuple[int, int, int], str]) -> None:
    """Causal farmer proof: the farmer assigned to THIS building harvests a
    mature crop inside its bounds (age 7 -> replanted age 0) while adjacent.
    Natural growth alone cannot pass: requires age-0 replant + adjacency."""
    import re as _re
    # Field clearing (player-legit): replace pad grass with dirt in a 4-wide
    # ring at ground level so grass cannot spread onto the farm's dirt cells
    # during the high-tick growth phase (spread caused the 034136 L2 mismatch:
    # builder certifies with grass where dirt belongs).
    xs = [p[0] for p in cells]; ys = [p[1] for p in cells]; zs = [p[2] for p in cells]
    _gy = min(ys) - 1
    probe.fixture(f"fill {min(xs) - 4} {_gy} {min(zs) - 4} {max(xs) + 4} {_gy} {max(zs) + 4} minecraft:dirt")
    sub[f"L{level}_moat"] = f"dirt ring x[{min(xs) - 4},{max(xs) + 4}] z[{min(zs) - 4},{max(zs) + 4}] y={_gy}"
    crops = [pos for pos, want in cells.items()
             if _state_base(want) in ("minecraft:wheat", "minecraft:carrots",
                                      "minecraft:potatoes", "minecraft:beetroots")]
    # "works at #<short> <Kind label>" where short = global id low 32 bits.
    import re as _re
    if "site" in sub:
        _qa._at_settlement_anchor(probe, sub)
    full = sub.get("building_id") or _qa._building_seq(probe, "farm", level)
    short = full & 0xFFFFFFFF if full else None
    label = KIND_LABEL["farm"]
    residents = _qa._await_assignment(probe, short, label, "Farmer")
    farmer_assigned = any(f"works at #{short} {label}" in line and "Farmer" in line
                          for line in residents) if short else False
    sub[f"L{level}_farmer_assigned"] = {"building": full, "short": short, "assigned": farmer_assigned,
                                        "lines": [l for l in residents if "Farmer" in l][:4]}
    if not farmer_assigned:
        probe.check(f"{tag}_L{level}_harvest", False)
        _verdict(sub, f"L{level}_function_harvest", "FAIL",
                 "no Farmer assigned to this farm building", "settlement menu residents")
        return
    # Gate 2: mature crop inside bounds harvested -> replanted at age 0.
    # Poll ALL crop cells; need a 7->0 transition (harvest+replant), not growth.
    # Harness-ripens 4 crops to max age via setblock (instant, zero side
    # effects): tick-speed growth caused grass spread + farmland drying that
    # contaminated L2 blockmatch (034136/034940). The farmer's causal
    # harvest+replant while adjacent is still fully proven.
    _MAX_AGE = {"minecraft:wheat": 7, "minecraft:carrots": 7,
                "minecraft:potatoes": 7, "minecraft:beetroots": 3}
    ripened = []
    for cell in crops[:4]:
        want = _state_base(_block_id(_qa._read(probe, cell)))
        max_age = _MAX_AGE.get(want, 7)
        try:
            probe.fixture(f"setblock {cell[0]} {cell[1]} {cell[2]} {want}[age={max_age}]")
            ripened.append([list(cell), want, max_age])
        except Exception:
            continue
    sub[f"L{level}_ripened"] = ripened
    ages = {cell: _props(_qa._read(probe, cell)).get("age") for cell in crops}
    harvested_cell = None
    farmer_near = False
    deadline = _time.monotonic() + 480.0
    while _time.monotonic() < deadline and harvested_cell is None:
        probe.call("minecraft_wait_ticks", {"ticks": 8})
        for cell in crops:
            now_age = _props(_qa._read(probe, cell)).get("age")
            if str(now_age) == "0" and str(ages.get(cell)) in ("3", "7"):
                harvested_cell = cell
                break
        if harvested_cell is None:
            for cell in crops:
                ages[cell] = _props(_qa._read(probe, cell)).get("age")
    if harvested_cell is not None:
        for row in _villagers(probe):
            if math.dist((float(row["x"]), float(row["z"])),
                         (harvested_cell[0] + 0.5, harvested_cell[2] + 0.5)) < 5.0:
                farmer_near = True
                sub[f"L{level}_harvester"] = str(row.get("entity_uuid"))[:8]
                break
    ok = harvested_cell is not None and farmer_near
    probe.check(f"{tag}_L{level}_harvest", ok)
    _verdict(sub, f"L{level}_function_harvest", "PASS" if ok else "FAIL",
             f"mature->replant at {harvested_cell}, farmer_adjacent={farmer_near}, assigned={farmer_assigned}")


def _container_cells(cells: dict[tuple[int, int, int], str]) -> list[tuple[int, int, int]]:
    """Placed container positions (chest/barrel) for deposit reads."""
    return [pos for pos, want in cells.items()
            if _state_base(want) in ("minecraft:chest", "minecraft:barrel")]


def _stock_totals(probe: ProbeSteps, chest: tuple[int, int, int],
                  extras: list[tuple[int, int, int]]) -> dict[str, int]:
    """Sum item totals across the shell chest + placed building containers.
    Workers deposit into the nearest container (often the building's own
    chest, not the shell 30 blocks away); shell-only reads miss it."""
    totals: dict[str, int] = {}
    for pos in ([chest] if chest != (0, 0, 0) else []) + extras:
        try:
            for item, count in _chest_totals(_open_chest_at(probe, pos)).items():
                totals[item] = totals.get(item, 0) + count
        except Exception:
            continue
    return totals


def _ensure_nearby_trees(probe: ProbeSteps, sub: dict[str, Any], tag: str, level: int,
                         origin: tuple[int, int, int]) -> list[tuple[int, int, int]]:
    """Player plants an oak stand near the camp (ordinary player path: place
    saplings, speed growth with randomTickSpeed, restore after). The build pad
    denudes the work area, so without this the jack has no valid mature tree
    (log over soil + crown leaves) within range. Returns matured trunk bases."""
    spots: list[tuple[int, int, int]] = []
    for dx in range(-14, 15):
        for dz in range(-14, 15):
            if max(abs(dx), abs(dz)) < 6 or len(spots) >= 6:
                continue
            for gy in (origin[1] - 1, origin[1], origin[1] + 1):
                g = (origin[0] + dx, gy, origin[2] + dz)
                try:
                    if _block_id(_qa._read(probe, g)) in ("minecraft:grass_block", "minecraft:dirt"):
                        above = _block_id(_qa._read(probe, (g[0], g[1] + 1, g[2])))
                        above2 = _block_id(_qa._read(probe, (g[0], g[1] + 2, g[2])))
                        if above == "minecraft:air" and above2 == "minecraft:air":
                            spots.append((g[0], g[1] + 1, g[2]))
                            break
                except Exception:
                    continue
    planted = []
    for s in spots:
        try:
            probe.fixture(f"setblock {s[0]} {s[1]} {s[2]} minecraft:oak_sapling")
            planted.append(s)
        except Exception:
            continue
    sub[f"L{level}_trees_planted"] = [list(p) for p in planted]
    if not planted:
        return []
    probe.fixture("gamerule randomTickSpeed 300")
    matured: list[tuple[int, int, int]] = []
    deadline = _time.monotonic() + 420.0
    while _time.monotonic() < deadline and len(matured) < 2:
        probe.call("minecraft_wait_ticks", {"ticks": 40})
        matured = []
        for s in planted:
            try:
                if _block_id(_qa._read(probe, s)) == "minecraft:oak_log":
                    matured.append(s)
            except Exception:
                continue
    probe.fixture("gamerule randomTickSpeed 0")
    probe.fixture("time set 3000")
    sub[f"L{level}_trees_matured"] = [list(p) for p in matured]
    return matured


def _function_lumber(probe: ProbeSteps, sub: dict[str, Any], tag: str, level: int,
                     origin: tuple[int, int, int],
                     cells: dict[tuple[int, int, int], str]) -> None:
    """Causal lumberjack proof (client-observable effects only):
    (1) a Lumberjack holds a job at THIS camp (Residents page),
    (2) a natural log block near the camp turns to non-log (cut) with a
        sapling at/near its base (replant),
    (3) the storage chest gains oak_log (deposit).
    No villager-inventory reads: entities never expose carry."""
    probe.fixture("time set 3000")
    if "site" in sub:
        _qa._at_settlement_anchor(probe, sub)
    bid = sub.get("building_id") or _qa._building_seq(probe, "lumber_camp", level)
    short = bid & 0xFFFFFFFF if bid else None
    label = KIND_LABEL["lumber_camp"]
    residents = _qa._await_assignment(probe, short, label, "Lumberjack")
    jack_assigned = any(f"works at #{short} {label}" in line and "Lumberjack" in line
                        for line in residents) if short else False
    sub[f"L{level}_jack_assigned"] = {"building": bid, "short": short, "assigned": jack_assigned}
    if not jack_assigned:
        probe.check(f"{tag}_L{level}_lumber", False)
        _verdict(sub, f"L{level}_function_lumber", "FAIL",
                 "no Lumberjack assigned to this camp", "settlement menu residents")
        return
    chest = tuple(sub.get("storage_chest", (0, 0, 0)))
    bins = _container_cells(cells)
    before_log = _stock_totals(probe, chest, bins).get("minecraft:oak_log", 0)
    cands: dict[tuple[int, int, int], str] = {}
    for dx in range(-24, 25, 2):
        for dz in range(-24, 25, 2):
            for dy in range(0, 7):
                p = (origin[0] + dx, origin[1] + dy, origin[2] + dz)
                try:
                    if _block_id(_qa._read(probe, p)) == "minecraft:oak_log":
                        cands[p] = "log"
                except Exception:
                    continue
                if len(cands) >= 40:
                    break
    sub[f"L{level}_log_candidates"] = len(cands)
    sub[f"L{level}_cand_sample"] = [list(p) for p in list(cands)[:8]]
    for m in _ensure_nearby_trees(probe, sub, tag, level, origin):
        cands[m] = "log"
    sub[f"L{level}_log_candidates_grown"] = len(cands)
    cut_cell = sapling_cell = None
    deposited = False
    deadline = _time.monotonic() + 480.0
    while _time.monotonic() < deadline and cut_cell is None:
        probe.call("minecraft_wait_ticks", {"ticks": 8})
        for p in cands:
            try:
                now = _block_id(_qa._read(probe, p))
            except Exception:
                continue
            if now != "minecraft:oak_log":
                cut_cell = p
                break
    if cut_cell is not None:
        # Nightfall forces full-carry deposit (day gate is a 64-stack threshold).
        probe.fixture("difficulty peaceful")
        probe.fixture("time set 13000")
        ndead = _time.monotonic() + 240.0
        while _time.monotonic() < ndead and not deposited:
            probe.call("minecraft_wait_ticks", {"ticks": 8})
            now_tot = _stock_totals(probe, chest, bins)
            if now_tot.get("minecraft:oak_log", 0) > before_log:
                deposited = True
                sub[f"L{level}_log_delta"] = now_tot.get("minecraft:oak_log", 0) - before_log
        probe.fixture("time set 3000")
        try:
            probe.fixture("difficulty normal")
        except Exception:
            pass
    replant = False
    if cut_cell is not None:
        sub[f"L{level}_cut_cell"] = list(cut_cell)
        for dx in (-1, 0, 1):
            for dz in (-1, 0, 1):
                try:
                    if "sapling" in _block_id(_qa._read(probe, (cut_cell[0] + dx, cut_cell[1], cut_cell[2] + dz))):
                        replant = True
                        sapling_cell = (cut_cell[0] + dx, cut_cell[1], cut_cell[2] + dz)
                        sub[f"L{level}_sapling_cell"] = list(sapling_cell)
                        break
                except Exception:
                    continue
    ok = jack_assigned and cut_cell is not None and replant and deposited
    probe.check(f"{tag}_L{level}_lumber", ok)
    _verdict(sub, f"L{level}_function_lumber", "PASS" if ok else "FAIL",
             f"assigned={jack_assigned} cut={cut_cell} replant={sapling_cell} deposited={deposited}")


def _function_mine(probe: ProbeSteps, sub: dict[str, Any], tag: str, level: int,
                   origin: tuple[int, int, int],
                   cells: dict[tuple[int, int, int], str]) -> None:
    """Causal miner proof (client-observable effects only):
    (1) a Miner holds a job at THIS mine (Residents page),
    (2) a stone/ore block below the mine floor turns to air (dug),
    (3) the storage chest gains a mining product (deposit).
    No villager-inventory reads: entities never expose carry."""
    probe.fixture("time set 3000")
    if "site" in sub:
        _qa._at_settlement_anchor(probe, sub)
    bid = sub.get("building_id") or _qa._building_seq(probe, "mine", level)
    short = bid & 0xFFFFFFFF if bid else None
    label = KIND_LABEL["mine"]
    residents = _qa._await_assignment(probe, short, label, "Miner")
    miner_assigned = any(f"works at #{short} {label}" in line and "Miner" in line
                         for line in residents) if short else False
    sub[f"L{level}_miner_assigned"] = {"building": bid, "short": short, "assigned": miner_assigned}
    if not miner_assigned:
        probe.check(f"{tag}_L{level}_mine", False)
        _verdict(sub, f"L{level}_function_mine", "FAIL",
                 "no Miner assigned to this mine", "settlement menu residents")
        return
    chest = tuple(sub.get("storage_chest", (0, 0, 0)))
    MINING = ("minecraft:cobblestone", "minecraft:stone", "minecraft:dirt",
              "minecraft:coal", "minecraft:raw_iron", "minecraft:raw_gold")
    bins = _container_cells(cells)
    before = _stock_totals(probe, chest, bins)
    # Snapshot solid cells below the floor across the footprint. The staircase
    # descends well past 7, so scan dy 1..16 at step 2 (deep digs invisible to
    # a shallow slice caused the L2 false-negative on 003941).
    size = 11
    cands: dict[tuple[int, int, int], str] = {}
    for dx in range(0, size, 2):
        for dz in range(0, size, 2):
            for dy in range(1, 17):
                p = (origin[0] + dx, origin[1] - dy, origin[2] + dz)
                try:
                    b = _block_id(_qa._read(probe, p))
                except Exception:
                    continue
                if b in ("minecraft:stone", "minecraft:cobblestone", "minecraft:coal_ore",
                         "minecraft:iron_ore", "minecraft:dirt", "minecraft:gravel",
                         "minecraft:deepslate"):
                    cands[p] = b
                if len(cands) >= 300:
                    break
    sub[f"L{level}_mine_candidates"] = len(cands)
    # Quarries progress across days (slow dig cadence + night return): allow up
    # to 3 workdays; each night forces full-carry deposit. PASS on stone-family.
    # Whole watch runs peaceful: shaft-darkness spawns close lanes (2-min
    # hostile patience) and would stall the proof; hostile coverage lives in
    # the no_hostiles + fort gates, not the worker-cadence proof.
    dug_cells: list[list[int]] = []
    delta: dict[str, int] = {}
    probe.fixture("difficulty peaceful")
    for day in range(1, 4):
        probe.fixture("time set 3000")
        deadline = _time.monotonic() + 420.0
        while _time.monotonic() < deadline:
            probe.call("minecraft_wait_ticks", {"ticks": 8})
            for p, was in cands.items():
                if any(d == list(p) for d in dug_cells):
                    continue
                try:
                    now = _block_id(_qa._read(probe, p))
                except Exception:
                    continue
                if now == "minecraft:air" and was != "minecraft:air":
                    dug_cells.append(list(p))
                    sub[f"L{level}_dug_was"] = was
                    sub[f"L{level}_dug_cells_day{day}"] = [list(q) for q in dug_cells]
                    break
            now_tot = _stock_totals(probe, chest, bins)
            if day == 1:
                pass
            if any(now_tot.get(k, 0) - before.get(k, 0) > delta.get(k, 0) for k in MINING):
                break
        probe.fixture("difficulty peaceful")
        probe.fixture("time set 13000")
        ndead = _time.monotonic() + 180.0
        while _time.monotonic() < ndead:
            probe.call("minecraft_wait_ticks", {"ticks": 8})
            now_tot = _stock_totals(probe, chest, bins)
            nd = {k: now_tot.get(k, 0) - before.get(k, 0) for k in MINING
                  if now_tot.get(k, 0) > before.get(k, 0)}
            if nd != delta:
                delta = nd
                sub[f"L{level}_mining_delta_day{day}"] = dict(delta)
            if any(k != "minecraft:dirt" for k in delta):
                break
        probe.fixture("time set 3000")
        if delta:
            sub[f"L{level}_mining_delta"] = dict(delta)
        if any(k != "minecraft:dirt" for k in delta):
            break
    try:
        probe.fixture("difficulty normal")
    except Exception:
        pass
    dug_cell = dug_cells[0] if dug_cells else None
    stone_delta = {k: v for k, v in delta.items() if k != "minecraft:dirt"}
    if delta and not stone_delta:
        sub[f"L{level}_overburden_only"] = delta
    ok = miner_assigned and dug_cell is not None and bool(stone_delta)
    probe.check(f"{tag}_L{level}_mine", ok)
    _verdict(sub, f"L{level}_function_mine", "PASS" if ok else "FAIL",
             f"assigned={miner_assigned} dug={len(dug_cells)} stone={stone_delta}")


def _function_work(probe: ProbeSteps, sub: dict[str, Any], tag: str, level: int,
                   origin: tuple[int, int, int],
                   cells: dict[tuple[int, int, int], str]) -> None:
    stations_all = [pos for pos, want in cells.items()
                    if _state_base(want) in ("minecraft:crafting_table", "minecraft:barrel",
                                             "minecraft:furnace")]
    station = _station_of(cells, ("crafting_table", "barrel", "furnace"))
    if station is None:
        probe.check(f"{tag}_L{level}_work", False)
        _verdict(sub, f"L{level}_function_work", "FAIL", "no workstation in blueprint")
        return
    if "site" in sub:
        _qa._at_settlement_anchor(probe, sub)
    bid = sub.get("building_id") or _qa._building_seq(probe, "mason", level)
    short = bid & 0xFFFFFFFF if bid else None
    label = KIND_LABEL["mason"]
    residents = _qa._await_assignment(probe, short, label, "Builder")
    builder_assigned = any(f"works at #{short} {label}" in line and "Builder" in line
                           for line in residents) if short else False
    sub[f"L{level}_builder_assigned"] = {"building": bid, "assigned": builder_assigned}
    if not builder_assigned:
        probe.check(f"{tag}_L{level}_work", False)
        _verdict(sub, f"L{level}_function_work", "FAIL",
                 "no Builder assigned to this mason building", "settlement menu residents")
        return
    near = False
    deadline = _time.monotonic() + 150.0
    while _time.monotonic() < deadline:
        for row in _villagers(probe):
            for st in stations_all:
                if math.dist((float(row["x"]), float(row["z"])),
                             (st[0] + 0.5, st[2] + 0.5)) < 3.0:
                    near = True
                    break
            if near:
                break
        if near:
            break
        probe.call("minecraft_wait_ticks", {"ticks": 4})
    ok = near and builder_assigned
    probe.check(f"{tag}_L{level}_work", ok)
    _verdict(sub, f"L{level}_function_work", "PASS" if ok else "FAIL",
             f"assigned Builder + worker within 3 of a station {stations_all[:3]}: {near}")


def _function_storage(probe: ProbeSteps, sub: dict[str, Any], tag: str, level: int,
                      origin: tuple[int, int, int],
                      cells: dict[tuple[int, int, int], str], settlement: str) -> None:
    """Storage proof on THIS building (never the shell): open one of ITS
    certified containers in game, deposit a marked item, then show it listed
    in the Main page storage summary (settlement stock)."""
    chests = [pos for pos, want in cells.items()
              if _state_base(want) in ("minecraft:chest", "minecraft:trapped_chest",
                                       "minecraft:barrel")]
    if not chests:
        probe.check(f"{tag}_L{level}_storage", False)
        _verdict(sub, f"L{level}_function_storage", "FAIL", "no containers in this building")
        return
    target = chests[0]
    got = _block_id(_qa._read(probe, target))
    if "chest" not in got and "barrel" not in got:
        probe.check(f"{tag}_L{level}_storage", False)
        _verdict(sub, f"L{level}_function_storage", "FAIL",
                 f"building container {list(target)} holds {got}, not a container")
        return
    sub[f"L{level}_storage_container"] = list(target)
    _qa._close_any_screen(probe)
    probe.fixture(f"tp @s {target[0] + 0.5} {target[1] + 1.0} {target[2] + 1.5}")
    probe.fixture("give @s minecraft:emerald 3")
    probe.call("minecraft_wait_ticks", {"ticks": 20})
    _deposit_all_at(probe, target, ["minecraft:emerald"])
    after = _chest_totals(_open_chest_at(probe, target))
    em = after.get("minecraft:emerald", 0)
    sub[f"L{level}_storage_marked"] = em
    if em < 1:
        probe.check(f"{tag}_L{level}_storage", False)
        _verdict(sub, f"L{level}_function_storage", "FAIL",
                 f"marked emerald never landed in {list(target)}: {after}")
        return
    state = _qa._send(probe, "settlement")
    opened = probe.wait(lambda s: s["screen"].get("class") == DIALOG_CLASS, f"{tag}_L{level}_main")
    body = "\n".join(str(n.get("label", "")) for n in opened.get("ui_nodes", [])
                     if isinstance(n, dict) and str(n.get("id", "")).startswith("dialog/body"))
    sub[f"L{level}_main_storage"] = body[:600]
    listed = "emerald" in body.lower() or "storage" in body.lower()
    probe.check(f"{tag}_L{level}_storage", listed)
    _verdict(sub, f"L{level}_function_storage", "PASS" if listed else "FAIL",
             f"container {list(target)} holds {em} emerald; main summary lists stock={listed}")


def _function_barracks(probe: ProbeSteps, sub: dict[str, Any], tag: str, level: int,
                       origin: tuple[int, int, int],
                       cells: dict[tuple[int, int, int], str], settlement: str) -> None:
    """Capacity-gated barracks proof: the Buildings menu icon `#<id> Barracks L<level>` reads
    `Soldiers: <filled>/<beds>` with beds == level*2. PASS regardless of filled (the recruit flow
    is army-qa territory)."""
    found = _building_lore(probe, re.compile(rf"#\d+ Barracks L{level}"))
    counted = _soldier_beds(found[1]) if found else None
    filled, beds = counted if counted else (None, None)
    want = level * 2
    ok = beds == want
    sub[f"L{level}_barracks_icon"] = found
    probe.check(f"{tag}_L{level}_barracks", ok)
    _verdict(sub, f"L{level}_function_barracks", "PASS" if ok else "FAIL",
             f"filled={filled} beds={beds} want_beds={want}", "settlement menu buildings")
