"""Real-client proof that a ten-resident settlement walls itself in (buildings-qa `perimeter`).

docs/settlements/construction.md#perimeter-gates-and-well. One founded settlement on a flat
pad, a certified storage shell stocked through the ordinary chest menu with cobblestone and four
oak doors (no logs, so no autonomous building is funded), ten bound villagers. Nothing else is
ordered: the planner and its Builders must lay the wall line 30 blocks from the bell (the core's
`settlements/layout.rs` RING, mirrored as `LAYOUT_RING`), a gatehouse with a door where each of
the four roads crosses it, and a well on a plaza corner. The wall is read back through the native
client's block scan, never from the server:

- `walls_absent_before`: the wall line and the plaza corners are empty before the stock arrives,
  so no fixture command placed what is scanned afterwards;
- `well_built`: a water source ringed by eight cobblestone blocks on a plaza corner;
- `gates_built`: four gatehouses, seven columns of four blocks with an oak door in the centre;
- `wall_built`: every other wall column is three cobblestone blocks;
- `enclosed`: no column of the 240-column line is open at foot or head height;
- `gate_door_opens`: a gate door reads closed before a client interaction and open after it;
- `project_named`: every active project the Construction page showed during the build carries a
  display name (never `Unknown blueprint`), and at least one of the built-in layout kinds
  (wall section, gate, well) was among them, so the name row had something to read.

A run that never builds records `FAIL` for the four building verdicts after the deadline, never
a crash: that is the control the bug ledger needs.
"""
from __future__ import annotations

import math
import re
import time
from pathlib import Path
from typing import Any

from ..profiles import ProfileContext
from . import buildings_qa as _qa
from . import native_client
# The layout mirror `LayoutMirrorTests` pins to `settlements/layout.rs` (`RING = PERIOD + 2`,
# `MAX_SIZE[2] = 11`): the one place the harness knows the wall line's distance from the bell.
from .settlement_map_qa import LAYOUT_RING
from .buildings_qa import (
    _block_id, _chat_lines, _chest_totals, _deposit_all_at, _echo, _hand_storage_at,
    _open_chest_at, _props, _read, _scan_box, _summon_bind, _verdict, _villagers,
)
from .probe_steps import ProbeSteps

CONTAINER = "net.minecraft.client.gui.screens.inventory.ContainerScreen"

NAME = "perimeter"
VERDICTS = frozenset({
    "settlement_founded", "walls_absent_before", "well_built", "gates_built", "wall_built",
    "enclosed", "gate_door_opens", "project_named",
})
# Assertion names. The ledger reads a failed `buildings_<name>_*` assertion as a red control, so only
# the outcome check carries that prefix; a fixture step that fails (no settlement, no residents,
# no stock) is a run that never reached the question and stays unrunnable.
CHECK = f"buildings_{NAME}"
FIXTURE = f"{NAME}_fixture"
GATE_HALF = 3
RESIDENTS = 10
RADIUS = 64
# Flat pad half width around the bell: the wall line plus a margin for the Builders to stand on.
# The line itself is the core's `settlements/layout.rs` RING (derived from the deepest blueprint,
# `MAX_SIZE[2] = 11` -> 30). It is read from the harness mirror `settlement_map_qa.LAYOUT_RING`,
# the one constant `LayoutMirrorTests` pins against that Rust source, instead of a copy here or a
# new chat convention (review wave 2, M1: a `wall ring` suffix broke
# `tools/coreperf/constantinople.py` and printed in every founder's chat). A ring that does not
# fit the pad fails the fixture; a stale mirror scans the wrong line and the wall verdicts FAIL,
# so no wrong ring can pass.
PAD = 40
COBBLE_STACKS = 12
DOORS = 4
COBBLE, DOOR, AIR, WATER = ("minecraft:cobblestone", "minecraft:oak_door", "minecraft:air",
                            "minecraft:water")
PATH = "minecraft:dirt_path"
SEEDS = "minecraft:wheat_seeds"
# Seconds the Builders get to place the first block of the fortification, then to finish it.
START_SECONDS = 600.0
BUILD_SECONDS = 1500.0
SCAN_TICKS = 250  # minecraft_wait_ticks accepts at most 255


def ring_columns(ax: int, az: int, ring: int) -> list[tuple[int, int, int]]:
    """Every column of the wall line as (x, z, across): the offset along its side from the road
    through the bell. The corner columns belong to the north and south sides."""
    columns = []
    for dx in range(-ring, ring + 1):
        for dz in range(-ring, ring + 1):
            if max(abs(dx), abs(dz)) == ring:
                columns.append((ax + dx, az + dz, dx if abs(dz) == ring else dz))
    return columns


def wanted(across: int) -> list[str]:
    """The blocks one column holds above the ground, bottom up."""
    if abs(across) > GATE_HALF:
        return [COBBLE] * 3
    if across == 0:
        return [DOOR, DOOR, COBBLE, COBBLE]
    return [COBBLE] * 4


def wells(ax: int, az: int) -> list[tuple[int, int]]:
    """Centres of the three plaza corners a well may stand on."""
    return [(ax - 3, az - 3), (ax + 3, az - 3), (ax - 3, az + 3)]


def scan_ring(probe: ProbeSteps, ax: int, ay: int, az: int,
              ring: int) -> dict[tuple[int, int, int], str]:
    """The four sides of the wall line, foot level up to one above a gatehouse."""
    blocks: dict[tuple[int, int, int], str] = {}
    for fixed in (-ring, ring):
        blocks.update(_scan_box(probe, (ax - ring, ay, az + fixed), (ax + ring, ay + 4, az + fixed)))
        blocks.update(_scan_box(probe, (ax + fixed, ay, az - ring + 1),
                                (ax + fixed, ay + 4, az + ring - 1)))
    return blocks


def scan_wells(probe: ProbeSteps, ax: int, ay: int, az: int) -> dict[tuple[int, int, int], str]:
    blocks: dict[tuple[int, int, int], str] = {}
    for cx, cz in wells(ax, az):
        blocks.update(_scan_box(probe, (cx - 1, ay - 1, cz - 1), (cx + 1, ay, cz + 1)))
    return blocks


def column_state(blocks: dict[tuple[int, int, int], str], x: int, ay: int, z: int,
                 across: int) -> str:
    """`built`, `empty` or `partial` for one wall column."""
    want = wanted(across)
    have = [blocks.get((x, ay + step, z), AIR) for step in range(len(want))]
    if have == want:
        return "built"
    return "empty" if all(block == AIR for block in have) else "partial"


def door_property(block: dict[str, Any]) -> str | None:
    """The gate door's `open` property, or None when the block is not a door."""
    if _block_id(block) != DOOR:
        return None
    return str(_props(block).get("open")).lower()


def door_opens(before: dict[str, Any], after: dict[str, Any]) -> bool:
    """The gate door was closed before the client interaction and open after it."""
    return door_property(before) == "false" and door_property(after) == "true"


def well_standing(blocks: dict[tuple[int, int, int], str], ay: int,
                  centre: tuple[int, int]) -> bool:
    cx, cz = centre
    if blocks.get((cx, ay - 1, cz)) != WATER:
        return False
    return all(blocks.get((cx + dx, ay, cz + dz)) == COBBLE
               for dx in (-1, 0, 1) for dz in (-1, 0, 1) if (dx, dz) != (0, 0))


def tally(ring: dict[tuple[int, int, int], str], wells_blocks: dict[tuple[int, int, int], str],
          ax: int, ay: int, az: int, wall_ring: int) -> dict[str, Any]:
    """What the scans show: column states per kind, the standing wells, and the open columns."""
    gates = {"built": 0, "partial": 0, "empty": 0}
    walls = {"built": 0, "partial": 0, "empty": 0}
    doors = 0
    open_columns: list[tuple[int, int]] = []
    for x, z, across in ring_columns(ax, az, wall_ring):
        state = column_state(ring, x, ay, z, across)
        (gates if abs(across) <= GATE_HALF else walls)[state] += 1
        if across == 0 and ring.get((x, ay, z)) == DOOR:
            doors += 1
        # Open at foot or head height: neither a solid block nor a closed door stands there.
        if any(ring.get((x, ay + step, z), AIR) not in (COBBLE, DOOR) for step in (0, 1)):
            open_columns.append((x, z))
    standing = [centre for centre in wells(ax, az) if well_standing(wells_blocks, ay, centre)]
    return {"gate_columns": gates, "wall_columns": walls, "doors": doors,
            "open_columns": len(open_columns), "open_sample": open_columns[:6],
            "wells": standing, "ring_blocks": sum(1 for block in ring.values() if block == COBBLE),
            "well_blocks": sum(1 for block in wells_blocks.values() if block in (COBBLE, WATER))}


def complete(counts: dict[str, Any]) -> bool:
    return (counts["gate_columns"]["built"] == 4 * (2 * GATE_HALF + 1)
            and counts["wall_columns"]["empty"] == 0 and counts["wall_columns"]["partial"] == 0
            and counts["open_columns"] == 0 and counts["doors"] == 4 and len(counts["wells"]) == 1)


# The display names the built-in layout kinds carry (settlements/ui_text.rs `blueprint`). The
# perimeter gate is the only real-client run that makes the planner raise these projects.
LAYOUT_INFRASTRUCTURE = ("Wall section", "Gate", "Well")


def project_named(labels: set[str]) -> bool:
    """The active project labels seen during the wall build never read `Unknown blueprint`, and at
    least one of the layout kinds this gate builds was observed (otherwise nothing was judged)."""
    return bool(labels & set(LAYOUT_INFRASTRUCTURE)) and "Unknown blueprint" not in labels


def _check_project_names(probe: ProbeSteps, sub: dict[str, Any], labels: set[str]) -> None:
    named = project_named(labels)
    sub["project_labels"] = sorted(labels)
    # The probe raises on a failed check. Persist a negative verdict only when
    # the client actually displayed the fallback label; missing evidence is not red.
    if named or "Unknown blueprint" in labels:
        _verdict(sub, "project_named", "PASS" if named else "FAIL",
                 f"active project labels {sub['project_labels']}")
    probe.check(f"{NAME}_project_named", named)


def project_labels_from(names: list[str]) -> set[str]:
    """Active project labels on the Construction status page, whose slots read `<label> L<n> …`."""
    labels = set()
    for name in names:
        match = re.search(r"(.+?) L\d+", str(name))
        if match:
            labels.add(match.group(1).strip())
    return labels


def _page_names(probe: ProbeSteps, marker: str, check: str,
                timeout_seconds: float = 30.0) -> list[str]:
    state = probe.wait(lambda s: (s.get("screen") or {}).get("class") == CONTAINER
                       and (s.get("container") or {}).get("slot_count") == 90
                       and any(marker in str(row.get("name"))
                               for row in (s.get("container") or {}).get("slots", [])
                               if row.get("name")),
                       check, timeout_seconds=timeout_seconds)
    return [str(row.get("name")) for row in (state.get("container") or {}).get("slots", [])
            if row.get("name")]


def _status_names(probe: ProbeSteps) -> list[str]:
    """The Construction status page's slot names, opened by the product's own menu route.

    `settlement menu projects` opens the blueprint catalogue; the Status action slot opens the
    project page in the same window. Both are 90-slot chests and the clicks go through MCP, so no
    window handle is needed (buildings-qa's probe is not a scene-shot client and has no
    `p4_window`; the earlier Gui-based read failed with exactly that, 20261124T125551). The page
    driver is the shared one the per-kind checks use.
    """
    page = _qa._project_page(probe)
    names = [str(row.get("name")) for row in (page.get("container") or {}).get("slots", [])
             if row.get("name")]
    probe.call("minecraft_close_screen", {})
    return names


def scenario(probe: ProbeSteps, detail: dict[str, Any], ctx: ProfileContext, artifact: Path) -> None:
    sub = detail.setdefault("blueprints", {}).setdefault(NAME, {})
    sname = f"bqa_{NAME}"
    probe.fixture("gamemode creative @s")
    probe.fixture("gamerule advance_time false")
    probe.fixture("gamerule spawn_mobs false")
    probe.fixture("gamerule random_tick_speed 0")
    probe.fixture("time set 3000")
    me = probe.observe().get("player") or {}
    px, py, pz = (int(math.floor(float(me[axis]))) for axis in ("x", "y", "z"))
    # A flat grass pad over the whole wall line; the anchor is the player's feet, so the ground
    # the planner reads is the block below the anchor (settlements/layout.rs `RING`).
    for x0 in range(px - PAD, px + PAD + 1, 32):
        for z0 in range(pz - PAD, pz + PAD + 1, 32):
            x1, z1 = min(x0 + 31, px + PAD), min(z0 + 31, pz + PAD)
            probe.fixture(f"fill {x0} {py - 2} {z0} {x1} {py - 2} {z1} minecraft:dirt")
            probe.fixture(f"fill {x0} {py - 1} {z0} {x1} {py - 1} {z1} minecraft:grass_block")
            probe.fixture(f"fill {x0} {py} {z0} {x1} {py + 24} {z1} minecraft:air")
    probe.call("minecraft_wait_ticks", {"ticks": 200})
    probe.fixture(f"settlement create {sname} {RADIUS}")
    probe.call("minecraft_wait_ticks", {"ticks": 40})
    echo = _echo(probe, f"{FIXTURE}_founded")
    founded = any(f"Founded {sname}" in line for line in echo.get("recent_chat", []))
    _verdict(sub, "settlement_founded", "PASS" if founded else "FAIL", f"{sname} r={RADIUS}")
    probe.check(f"{FIXTURE}_founded", founded)
    # The wall line is the core's RING, taken from the harness mirror `LAYOUT_RING` (the constant
    # `LayoutMirrorTests` pins to `layout.rs`). It must fit the flat pad, or the fixture never
    # reached the question: fail it instead of scanning a line the Builders cannot reach. A
    # stale mirror scans the wrong line and the wall verdicts fail, never pass.
    wall_ring = LAYOUT_RING
    fits = wall_ring > 0 and wall_ring + 6 <= PAD
    sub["wall_ring"] = {"source": "settlement_map_qa.LAYOUT_RING", "ring": wall_ring, "pad": PAD}
    probe.check(f"{FIXTURE}_ring", fits)
    _verdict(sub, "wall_ring", "PASS" if fits else "FAIL", f"ring {wall_ring} under pad {PAD}")
    if not fits:
        raise RuntimeError(f"buildings-qa fixture: layout mirror ring {wall_ring} does not fit "
                           f"pad {PAD}")
    ax, ay, az = px, py, pz
    sub["bell"] = [ax, ay, az]
    # The buyer's own way into the hub: a bell on the anchor, as the civil fixture has. Without it
    # the D63 client offers no route to the Construction status page, where project_named reads.
    probe.fixture(f"setblock {ax} {ay} {az} minecraft:bell")
    # The storage shell sits off the plaza and off the wall line, inside the ring.
    probe.fixture(f"tp @s {ax + 0.5} {ay + 1.0} {az + 0.5}")
    shell = _hand_storage_at(probe, sub, FIXTURE, sname, (ax - 14, ay, az - 14))
    probe.check(f"{FIXTURE}_shell", shell is not None)
    sub["storage_chest"] = list(shell)
    # Baseline before any stock exists: nothing stands on the line or the plaza corners.
    before = tally(scan_ring(probe, ax, ay, az, wall_ring), scan_wells(probe, ax, ay, az),
                   ax, ay, az, wall_ring)
    sub["before"] = before
    empty = before["ring_blocks"] == 0 and before["well_blocks"] == 0
    _verdict(sub, "walls_absent_before", "PASS" if empty else "FAIL", str(before))
    probe.check(f"{FIXTURE}_empty_before", empty)
    # Stock the chest through the ordinary menu (before any resident exists): cobblestone for the whole fortification and one
    # oak door per gatehouse. No logs, so no building is funded and the wall is the planner's
    # only affordable work.
    for _ in range(COBBLE_STACKS):
        probe.fixture(f"give @s {COBBLE} 64")
    probe.fixture(f"give @s {DOOR} {DOORS}")
    # The first Farm's seeds are gathered before any fortification work: stock them, so the Builders
    # owe no meadow clearing and the wall is the first work the plan can start.
    probe.fixture(f"give @s {SEEDS} 64")
    probe.call("minecraft_wait_ticks", {"ticks": 20})
    stock_before = _chest_totals(_open_chest_at(probe, tuple(shell)))
    sub["deposit"] = _deposit_all_at(probe, tuple(shell), [COBBLE, DOOR, SEEDS])
    stock = _chest_totals(_open_chest_at(probe, tuple(shell)))
    sub["stock"] = stock
    funded = (stock.get(COBBLE, 0) - stock_before.get(COBBLE, 0) >= 64 * COBBLE_STACKS
              and stock.get(DOOR, 0) - stock_before.get(DOOR, 0) >= DOORS)
    probe.check(f"{FIXTURE}_funded", funded)
    # Residents come last: the planner runs from the first resident, and with the stock already
    # in storage the first project it starts is funded fortification work, not a meadow clearing.
    # Summon on the open pad beside the anchor, never at the shell chest: a summon inside the
    # chest block took Suffocation damage and the fixture reported no bound residents
    # (20261124T103654). The pad is cleared to ay + 24, so these cells are air over grass.
    probe.check(f"{FIXTURE}_bound",
                _summon_bind(probe, sub, FIXTURE, sname, (ax + 3, ay, az + 3), RESIDENTS)
                and sub.get("bound_members", 0) >= RESIDENTS)
    probe.fixture(f"tp @s {ax + 0.5} {ay + 1.0} {az + 0.5}")
    # The Builders' time: first block, then completion, each bounded.
    started = time.monotonic()
    first = None
    samples: list[dict[str, Any]] = []
    # SETTLEMENT-LAYOUT-PROJECT-SHOWS-UNKNOWN-BLUEPRINT needs an ACTIVE layout project: the
    # status page shows `Active projects: 0/8` and no card once the bounded fortification list is
    # complete (20261124T163621: 212 wall columns, 28 gate columns and the well stood, then the
    # page had nothing to name and the verdict read false). Read it while the wall is being built,
    # once for the well and again once a wall/gate column stands.
    labels: set[str] = set()
    status_reads: list[list[str]] = []
    counts = before
    while True:
        probe.call("minecraft_wait_ticks", {"ticks": SCAN_TICKS})
        counts = tally(scan_ring(probe, ax, ay, az, wall_ring), scan_wells(probe, ax, ay, az),
                       ax, ay, az, wall_ring)
        elapsed = time.monotonic() - started
        roads = _scan_box(probe, (ax - wall_ring, ay - 1, az), (ax + wall_ring, ay - 1, az))
        roads.update(_scan_box(probe, (ax, ay - 1, az - wall_ring),
                               (ax, ay - 1, az + wall_ring)))
        samples.append({"seconds": round(elapsed), "ring_blocks": counts["ring_blocks"],
                        "well_blocks": counts["well_blocks"], "doors": counts["doors"],
                        "road_paths": sum(1 for block in roads.values() if block == PATH)})
        if len(samples) in (1, 4, 12, 30):
            sub.setdefault("villagers", {})[str(len(samples))] = [
                [round(float(v["x"]), 1), round(float(v["y"]), 1), round(float(v["z"]), 1)]
                for v in _villagers(probe)]
        missing = not labels & set(LAYOUT_INFRASTRUCTURE) or (
            counts["ring_blocks"] > 0 and not labels & {"Wall section", "Gate"})
        if (missing and len(status_reads) < 4
                and counts["ring_blocks"] + counts["well_blocks"] > 0
                and probe.deadline - time.monotonic() > 300.0):
            try:
                names = _status_names(probe)
                status_reads.append(names[:16])
                labels |= project_labels_from(names)
            except Exception as error:  # a page the run could not open is recorded, never a pass
                sub["status_error"] = str(error)[:300]
                probe.call("minecraft_close_screen", {})
        if first is None and counts["ring_blocks"] + counts["well_blocks"] > 0:
            first = elapsed
        if complete(counts):
            break
        if first is None and elapsed > START_SECONDS:
            break
        if first is not None and elapsed - first > BUILD_SECONDS:
            break
        if probe.deadline - time.monotonic() < 240.0:
            break
    sub["progress"] = samples[-40:]
    sub["after"] = counts
    # SETTLEMENT-LAYOUT-PROJECT-SHOWS-UNKNOWN-BLUEPRINT: the built-in layout kinds (wall section,
    # gate, well) are exactly the projects this gate makes the settlement plan, so the live
    # Construction page is where their display names are readable. Every active project observed
    # must carry a name — `Unknown blueprint` is the symptom — and at least one of the fortification
    # kinds must have been observed, or this run never reached the question. The page is opened the
    # way the D63 client offers it (hub -> Management -> Construction -> Status), never by the
    # retired `settlement menu projects` command route.
    if not labels & set(LAYOUT_INFRASTRUCTURE):
        try:
            names = _status_names(probe)
            status_reads.append(names[:16])
            labels |= project_labels_from(names)
        except Exception as error:  # a page the run could not open is recorded, never a false pass
            sub["status_error"] = str(error)[:300]
    sub["status_reads"] = status_reads
    sub["status_slots"] = status_reads[-1] if status_reads else []
    _check_project_names(probe, sub, labels)
    wall, gate = counts["wall_columns"], counts["gate_columns"]
    _verdict(sub, "well_built", "PASS" if len(counts["wells"]) == 1 else "FAIL",
             f"standing wells {counts['wells']} (water source ringed by eight cobblestone)")
    _verdict(sub, "gates_built",
             "PASS" if gate["built"] == 4 * (2 * GATE_HALF + 1) and counts["doors"] == 4 else "FAIL",
             f"gate columns {gate}, doors {counts['doors']}")
    _verdict(sub, "wall_built", "PASS" if wall["empty"] == 0 and wall["partial"] == 0 else "FAIL",
             f"wall columns {wall}")
    _verdict(sub, "enclosed", "PASS" if counts["open_columns"] == 0 else "FAIL",
             f"open columns {counts['open_columns']} e.g. {counts['open_sample']}")
    if ctx.windowed and complete(counts):
        # Gate, wall and well as the client renders them.
        probe.fixture(f"tp @s {ax + 0.5} {ay + 1.0} {az - 15.5} 180 5")
        probe.call("minecraft_wait_ticks", {"ticks": 60})
        native_client.capture(probe.client, artifact, f"{NAME}-north-gate")
        probe.fixture(f"tp @s {ax + 1.5} {ay + 1.0} {az - 3.5} 90 25")
        probe.call("minecraft_wait_ticks", {"ticks": 60})
        native_client.capture(probe.client, artifact, f"{NAME}-well")
    # A client interaction opens a gate door: the gate is a door, not a wall. The state is read on
    # both sides of the click — closed before (the product places every door closed, `layout.rs`
    # `open=false`) and open after — so a resident that opened it earlier cannot make the check
    # vacuous, and a gate that cannot be opened fails it.
    door = (ax, ay, az - wall_ring)
    closed_before = False
    opens = False
    if counts["doors"]:
        probe.fixture(f"tp @s {ax + 0.5} {ay + 1.0} {az - wall_ring + 3.5}")
        probe.call("minecraft_wait_ticks", {"ticks": 40})
        probe.call("minecraft_set_hotbar_slot", {"slot": 8})
        before_block = _read(probe, door)
        closed_before = door_property(before_block) == "false"
        probe.check(f"{FIXTURE}_door_closed_before", closed_before)
        sub["door_before"] = {"state": door_property(before_block)}
        probe.call("minecraft_use_item_on",
                   {"x": door[0], "y": door[1], "z": door[2], "face": "south", "hand": "main_hand"})
        probe.call("minecraft_wait_ticks", {"ticks": 20})
        block = _read(probe, door)
        sub["door_after"] = {"state": door_property(block)}
        opens = door_opens(before_block, block)
    _verdict(sub, "gate_door_opens", "PASS" if opens else "FAIL",
             f"{door} closed before={closed_before} open after={opens}")
    sub["chat_tail"] = _chat_lines(probe.observe(), 6)
    probe.check(f"{CHECK}_walled", complete(counts))
    # The outcome assertion (M2): a run with the door closed before and still closed after fails
    # here, and an already-open door failed the fixture above, so neither read alone can pass.
    probe.check(f"{CHECK}_door_opens", opens)
