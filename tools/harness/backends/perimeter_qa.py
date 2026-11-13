"""Real-client proof that a ten-resident settlement walls itself in (buildings-qa `perimeter`).

docs/settlements/construction.md#perimeter-gates-and-well. One founded settlement on a flat
pad, a certified storage shell stocked through the ordinary chest menu with cobblestone and four
oak doors (no logs, so no autonomous building is funded), ten bound villagers. Nothing else is
ordered: the planner and its Builders must lay the wall line 25 blocks from the bell, a gatehouse
with a door where each of the four roads crosses it, and a well on a plaza corner. The wall is
read back through the native client's block scan, never from the server:

- `walls_absent_before`: the wall line and the plaza corners are empty before the stock arrives,
  so no fixture command placed what is scanned afterwards;
- `well_built`: a water source ringed by eight cobblestone blocks on a plaza corner;
- `gates_built`: four gatehouses, seven columns of four blocks with an oak door in the centre;
- `wall_built`: every other wall column is three cobblestone blocks;
- `enclosed`: no column of the 200-column line is open at foot or head height;
- `gate_door_opens`: a client interaction opens a gate door.

A run that never builds records `FAIL` for the four building verdicts after the deadline, never
a crash: that is the control the bug ledger needs.
"""
from __future__ import annotations

import math
import time
from pathlib import Path
from typing import Any

from ..profiles import ProfileContext
from . import native_client
from .buildings_qa import (
    _block_id, _chat_lines, _chest_totals, _deposit_all_at, _echo, _hand_storage_at,
    _open_chest_at, _project_status, _props, _read, _scan_box, _summon_bind, _verdict,
)
from .probe_steps import ProbeSteps

NAME = "perimeter"
VERDICTS = frozenset({
    "settlement_founded", "walls_absent_before", "well_built", "gates_built", "wall_built",
    "enclosed", "gate_door_opens",
})
# Assertion names. The ledger reads a failed `buildings_<name>_*` assertion as a red control, so only
# the outcome check carries that prefix; a fixture step that fails (no settlement, no residents,
# no stock) is a run that never reached the question and stays unrunnable.
CHECK = f"buildings_{NAME}"
FIXTURE = f"{NAME}_fixture"
RING = 25
GATE_HALF = 3
RESIDENTS = 10
RADIUS = 64
# Flat pad half width around the bell: the wall line plus a margin for the Builders to stand on.
PAD = RING + 6
COBBLE_STACKS = 12
DOORS = 4
COBBLE, DOOR, AIR, WATER = ("minecraft:cobblestone", "minecraft:oak_door", "minecraft:air",
                            "minecraft:water")
PATH = "minecraft:dirt_path"
# Seconds the Builders get to place the first block of the fortification, then to finish it.
START_SECONDS = 600.0
BUILD_SECONDS = 1500.0
SCAN_TICKS = 250  # minecraft_wait_ticks accepts at most 255


def ring_columns(ax: int, az: int) -> list[tuple[int, int, int]]:
    """Every column of the wall line as (x, z, across): the offset along its side from the road
    through the bell. The corner columns belong to the north and south sides."""
    columns = []
    for dx in range(-RING, RING + 1):
        for dz in range(-RING, RING + 1):
            if max(abs(dx), abs(dz)) == RING:
                columns.append((ax + dx, az + dz, dx if abs(dz) == RING else dz))
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


def scan_ring(probe: ProbeSteps, ax: int, ay: int, az: int) -> dict[tuple[int, int, int], str]:
    """The four sides of the wall line, foot level up to one above a gatehouse."""
    blocks: dict[tuple[int, int, int], str] = {}
    for fixed in (-RING, RING):
        blocks.update(_scan_box(probe, (ax - RING, ay, az + fixed), (ax + RING, ay + 4, az + fixed)))
        blocks.update(_scan_box(probe, (ax + fixed, ay, az - RING + 1),
                                (ax + fixed, ay + 4, az + RING - 1)))
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


def well_standing(blocks: dict[tuple[int, int, int], str], ay: int,
                  centre: tuple[int, int]) -> bool:
    cx, cz = centre
    if blocks.get((cx, ay - 1, cz)) != WATER:
        return False
    return all(blocks.get((cx + dx, ay, cz + dz)) == COBBLE
               for dx in (-1, 0, 1) for dz in (-1, 0, 1) if (dx, dz) != (0, 0))


def tally(ring: dict[tuple[int, int, int], str], wells_blocks: dict[tuple[int, int, int], str],
          ax: int, ay: int, az: int) -> dict[str, Any]:
    """What the scans show: column states per kind, the standing wells, and the open columns."""
    gates = {"built": 0, "partial": 0, "empty": 0}
    walls = {"built": 0, "partial": 0, "empty": 0}
    doors = 0
    open_columns: list[tuple[int, int]] = []
    for x, z, across in ring_columns(ax, az):
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
    ax, ay, az = px, py, pz
    sub["bell"] = [ax, ay, az]
    # The storage shell sits off the plaza and off the wall line, inside the ring.
    probe.fixture(f"tp @s {ax + 0.5} {ay + 1.0} {az + 0.5}")
    shell = _hand_storage_at(probe, sub, FIXTURE, sname, (ax - 14, ay, az - 14))
    probe.check(f"{FIXTURE}_shell", shell is not None)
    sub["storage_chest"] = list(shell)
    probe.check(f"{FIXTURE}_bound", _summon_bind(probe, sub, FIXTURE, sname, shell, RESIDENTS)
                and sub.get("bound_members", 0) >= RESIDENTS)
    # Baseline before any stock exists: nothing stands on the line or the plaza corners.
    before = tally(scan_ring(probe, ax, ay, az), scan_wells(probe, ax, ay, az), ax, ay, az)
    sub["before"] = before
    empty = before["ring_blocks"] == 0 and before["well_blocks"] == 0
    _verdict(sub, "walls_absent_before", "PASS" if empty else "FAIL", str(before))
    probe.check(f"{FIXTURE}_empty_before", empty)
    # Stock the chest through the ordinary menu: cobblestone for the whole fortification and one
    # oak door per gatehouse. No logs, so no building is funded and the wall is the planner's
    # only affordable work.
    for _ in range(COBBLE_STACKS):
        probe.fixture(f"give @s {COBBLE} 64")
    probe.fixture(f"give @s {DOOR} {DOORS}")
    probe.call("minecraft_wait_ticks", {"ticks": 20})
    stock_before = _chest_totals(_open_chest_at(probe, tuple(shell)))
    sub["deposit"] = _deposit_all_at(probe, tuple(shell), [COBBLE, DOOR])
    stock = _chest_totals(_open_chest_at(probe, tuple(shell)))
    sub["stock"] = stock
    funded = (stock.get(COBBLE, 0) - stock_before.get(COBBLE, 0) >= 64 * COBBLE_STACKS
              and stock.get(DOOR, 0) - stock_before.get(DOOR, 0) >= DOORS)
    probe.check(f"{FIXTURE}_funded", funded)
    probe.fixture(f"tp @s {ax + 0.5} {ay + 1.0} {az + 0.5}")
    # The Builders' time: first block, then completion, each bounded.
    started = time.monotonic()
    first = None
    samples: list[dict[str, Any]] = []
    counts = before
    while True:
        probe.call("minecraft_wait_ticks", {"ticks": SCAN_TICKS})
        counts = tally(scan_ring(probe, ax, ay, az), scan_wells(probe, ax, ay, az), ax, ay, az)
        elapsed = time.monotonic() - started
        roads = _scan_box(probe, (ax - RING, ay - 1, az), (ax + RING, ay - 1, az))
        roads.update(_scan_box(probe, (ax, ay - 1, az - RING), (ax, ay - 1, az + RING)))
        samples.append({"seconds": round(elapsed), "ring_blocks": counts["ring_blocks"],
                        "well_blocks": counts["well_blocks"], "doors": counts["doors"],
                        "road_paths": sum(1 for block in roads.values() if block == PATH),
                        "project": _project_status(probe)})
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
    # A client interaction opens a gate door: the gate is a door, not a wall.
    door = (ax, ay, az - RING)
    opens = False
    if counts["doors"]:
        probe.fixture(f"tp @s {ax + 0.5} {ay + 1.0} {az - RING + 3.5}")
        probe.call("minecraft_wait_ticks", {"ticks": 40})
        probe.call("minecraft_set_hotbar_slot", {"slot": 8})
        probe.call("minecraft_use_item_on",
                   {"x": door[0], "y": door[1], "z": door[2], "face": "south", "hand": "main_hand"})
        probe.call("minecraft_wait_ticks", {"ticks": 20})
        block = _read(probe, door)
        opens = str(_props(block).get("open")).lower() == "true" and _block_id(block) == DOOR
    _verdict(sub, "gate_door_opens", "PASS" if opens else "FAIL", f"{door} open={opens}")
    sub["chat_tail"] = _chat_lines(probe.observe(), 6)
    probe.check(f"{CHECK}_walled", complete(counts))
