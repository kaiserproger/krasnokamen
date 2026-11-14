"""BREAKER GLUE (BreakIssue1x11, not for merge): attacks on ISSUE-1-11 around sleep.

Reuses the village-qa fixture build verbatim (spliced from village_qa._scenario),
then runs one attack set chosen by VILLAGE_BREAK_SET (A, B or C). Every
observation comes from client-received state (nameplate + VillagerData + pose).
"""
from __future__ import annotations

import hashlib
import json
import math
import os
import subprocess
import time as _time
from dataclasses import replace
from pathlib import Path
from typing import Any

from .. import runtime
from ..profiles import ProfileContext
from . import native_client
from .gameplay import peak_rss_kib, prepare_operator_fixture
from .probe_steps import ProbeSteps
from .village_qa import (SETTLEMENT, USERNAME, VILLAGER_ROLES, _block_id, _chat_echo,
                         _expect_block, _game_time, _props, _read, _rect, _tick, _villagers,
                         bed_cells, bed_facing_valid, count_time_sets, house_headroom_ok,
                         nameplate, required_radius, trees_separated, villager_job)

WORD_WIRE = {"безработный": 0, "бронник": 1, "мясник": 2, "картограф": 3, "священник": 4,
             "фермер": 5, "рыбак": 6, "лучник": 7, "кожевник": 8, "библиотекарь": 9,
             "каменщик": 10, "простак": 11, "пастух": 12, "инструментальщик": 13,
             "оружейник": 14, "шахтёр": 13, "лесоруб": 7}


def summary(row: dict[str, Any]) -> dict[str, Any]:
    plate = nameplate(row)
    parts = (plate or "").split(" · ")
    word = parts[1] if len(parts) == 3 else None
    data = ((row.get("received_metadata") or {}).get("19") or {}).get("value")
    wire = data[1] if isinstance(data, list) and len(data) == 3 else None
    bounds = row.get("bounding_box", {}) or {}
    height = float(bounds.get("max_y", 0)) - float(bounds.get("min_y", 0))
    return {"pos": [round(float(row[k]), 2) for k in ("x", "y", "z")], "word": word,
            "activity": parts[2] if len(parts) == 3 else None, "wire": wire,
            "agree": word is None or word == "ребёнок" or WORD_WIRE.get(word) == wire,
            "sleep_pose": abs(height - 0.2) < 0.02, "height": round(height, 3),
            "job": villager_job(row)}


class Session:
    """Server + client lifecycle that an attack can restart cleanly."""

    def __init__(self, ctx: ProfileContext, binary: Path, config: Path, port: int,
                 timeout: float) -> None:
        self.ctx, self.binary, self.config, self.port, self.timeout = ctx, binary, config, port, timeout
        self.address = f"127.0.0.1:{port}"
        self.server = self.client_process = self.client_log = self.mcp = self.xvfb = None
        self.display = None
        self.server_log = None
        self.restarts = 0
        self.client_evidence = ctx.client_evidence
        self.env = os.environ.copy()
        self.env["SOLARIS_HARNESS_LOG_STDOUT"] = "1"

    def start_server(self, name: str) -> None:
        path = self.ctx.artifact_dir / name
        self.server_log = path.open("wb")
        self.server = runtime.start_server([str(self.binary), "--config", str(self.config), "--no-console"],
                                           cwd=runtime.REPO_ROOT, stdout=self.server_log,
                                           stderr=subprocess.STDOUT, start_new_session=True, env=self.env)
        self.ctx.logs.append(str(path))
        runtime.wait_port(self.port, self.timeout, self.server, log_path=path)

    def start_client(self, ctx: ProfileContext) -> None:
        self.client_process, self.client_log, self.mcp = native_client.start(
            ctx, native_client.resolve_binary(ctx), self.address, USERNAME, self.timeout,
            display=self.display)
        self.client_evidence = ctx.client_evidence
        self.mcp.request_timeout_seconds = min(self.timeout + 10, 130.0)

    def restart(self, probe: ProbeSteps, kill: bool = False) -> dict[str, Any]:
        """Stop client and server (SIGINT, or SIGKILL when `kill`), same world, fresh client join."""
        self.restarts += 1
        self.mcp.close()
        runtime.stop_process(self.client_process)
        self.ctx.exits[self.client_evidence["command_index"]] = self.client_process.returncode
        self.client_log.close()
        before = self.server.pid
        if kill:
            import signal
            os.killpg(self.server.pid, signal.SIGKILL)
            self.server.wait(timeout=60)
        else:
            runtime.stop_process(self.server, interrupt=True, timeout_seconds=60.0)
        code = self.server.returncode
        self.server_log.close()
        self.start_server(f"server-restart-{self.restarts}.log")
        rctx = replace(self.ctx, artifact_dir=self.ctx.artifact_dir / f"restart-client-{self.restarts}",
                       client_evidence={})
        self.start_client(rctx)
        probe.client = self.mcp
        probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
        return {"server_exit": code, "before_pid": before, "after_pid": self.server.pid}

    def relog(self, probe: ProbeSteps) -> None:
        """Client disconnect + fresh join; the server keeps running."""
        self.restarts += 1
        self.mcp.close()
        runtime.stop_process(self.client_process)
        self.ctx.exits[self.client_evidence["command_index"]] = self.client_process.returncode
        self.client_log.close()
        rctx = replace(self.ctx, artifact_dir=self.ctx.artifact_dir / f"relog-client-{self.restarts}",
                       client_evidence={})
        self.start_client(rctx)
        probe.client = self.mcp
        probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})

    def close(self) -> None:
        if self.mcp is not None:
            try:
                self.mcp.close()
            except Exception:
                pass
        runtime.stop_process(self.client_process)
        runtime.stop_process(self.server, interrupt=True)
        runtime.stop_process(self.xvfb)
        if self.server_log is not None:
            self.server_log.close()


def run(ctx: ProfileContext) -> dict[str, Any]:
    if ctx.client != "native":
        raise ValueError("village-break requires --client native")
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else 1200.0
    attack_set = os.environ.get("VILLAGE_BREAK_SET", "A")
    artifact = ctx.artifact_dir
    binary = Path(os.environ["SOLARIS_SERVER_BINARY"]).expanduser().resolve()
    port = runtime.reserve_port()
    config = artifact / "village-break.toml"
    source = (runtime.REPO_ROOT / "playable.toml").read_text()
    for old_tok, new_tok in (
        ("port = 25565", f"port = {port}"),
        ('world_dir = ".analysis/test-world-v12"',
         f'world_dir = "{(artifact / "world").relative_to(runtime.REPO_ROOT).as_posix()}"'),
        ('directory = "plugins"', f"directory = {json.dumps(str(artifact / 'plugins'))}"),
        ("seed = 0", "seed = 81"),
    ):
        if source.count(old_tok) != 1:
            raise RuntimeError(f"expected one config token: {old_tok}")
        source = source.replace(old_tok, new_tok, 1)
    config.write_text(source)
    (artifact / "plugins").mkdir(parents=True, exist_ok=True)
    prepare_operator_fixture(ctx, [USERNAME])
    with binary.open("rb") as handle:
        server_hash = hashlib.file_digest(handle, "sha256").hexdigest()
    ctx.client_evidence.update(server_binary_path=str(binary), server_binary_sha256=server_hash,
                               world_dir=str(artifact / "world"), seed=81, attack_set=attack_set)
    session = Session(ctx, binary, config, port, timeout)
    try:
        session.start_server("server.log")
        if ctx.windowed:
            session.xvfb, session.display, _ = runtime.start_xvfb(artifact)
            ctx.logs.append(str(artifact / "xvfb.log"))
        session.start_client(ctx)
        with ProbeSteps(session.mcp, artifact / "village-break", timeout, f"village-break-{attack_set}") as probe:
            probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
            detail: dict[str, Any] = {"set": attack_set, "attacks": {}, "disagreements": [], "timeline": []}
            probe.report["break"] = detail
            try:
                fx = _build(probe, detail, ctx, artifact)
                Attacks(probe, detail, ctx, artifact, session, fx).run(attack_set)
            finally:
                (artifact / "break.json").write_text(json.dumps(detail, indent=2, ensure_ascii=False) + "\n")
        return {"scenario": probe.report}
    finally:
        session.close()


def _build(probe: ProbeSteps, detail: dict[str, Any], ctx: ProfileContext, artifact: Path) -> dict[str, Any]:
    rooted = probe.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float),
                        "player_present")
    px, py, pz = (math.floor(rooted["player"][axis]) for axis in ("x", "y", "z"))
    origin = (int(px), int(py), int(pz))
    detail["origin_block"] = list(origin)

    def fixture(command: str) -> None:
        probe.fixture(command)

    def settled(label: str) -> dict[str, Any]:
        return _chat_echo(probe, label)

    def teleport(x: float, y: float, z: float) -> None:
        fixture(f"tp @s {x:.1f} {y:.1f} {z:.1f}")
        probe.wait(lambda s: abs(float(s["player"]["x"]) - x) < 3.0
                   and abs(float(s["player"]["z"]) - z) < 3.0, "player_teleported")

    # --- level a pad through ordinary chat commands (one daylight freeze, no time spam) ---
    x0, y0, z0 = origin
    # 61*61*17 exceeds vanilla's 32768-command block limit; each slab fits.
    for low, high in ((-8, -1), (0, 7), (8, 8)):
        fixture(f"fill {x0 - 30} {y0 + low} {z0 - 30} {x0 + 30} {y0 + high} {z0 + 30} minecraft:air")
    fixture(f"fill {x0 - 30} {y0 - 2} {z0 - 30} {x0 + 30} {y0 - 2} {z0 + 30} minecraft:dirt")
    fixture(f"fill {x0 - 30} {y0 - 1} {z0 - 30} {x0 + 30} {y0 - 1} {z0 + 30} minecraft:grass_block")
    fixture(f"fill {x0 - 2} {y0 - 1} {z0 - 2} {x0 + 2} {y0 - 1} {z0 + 2} minecraft:stone")
    fixture("gamemode creative @s")
    fixture("effect give @s minecraft:regeneration infinite 255 true")
    fixture("gamerule advance_time false")
    fixture("gamerule spawn_mobs false")
    fixture("gamerule random_tick_speed 0")
    fixture("time set 18000")
    _expect_block(probe, (x0, y0 - 1, z0), "minecraft:stone", "spawn_pad_leveled")

    # --- house 5x5x5: floor y0, roof y0+4, walls y0+1..y0+3, east door, valid east bed ---
    house_lo, house_hi = _rect(origin, (0, 0, 0), (4, 4, 4))
    hx0, hy0, hz0 = house_lo
    hx1, hy1, hz1 = house_hi
    fixture(f"fill {hx0} {hy0} {hz0} {hx1} {hy0} {hz1} minecraft:oak_planks")
    fixture(f"fill {hx0} {hy1} {hz0} {hx1} {hy1} {hz1} minecraft:oak_planks")
    fixture(f"fill {hx0} {hy0 + 1} {hz0} {hx1} {hy0 + 3} {hz0} minecraft:cobblestone")
    fixture(f"fill {hx0} {hy0 + 1} {hz1} {hx1} {hy0 + 3} {hz1} minecraft:cobblestone")
    fixture(f"fill {hx0} {hy0 + 1} {hz0} {hx0} {hy0 + 3} {hz1} minecraft:cobblestone")
    fixture(f"fill {hx1} {hy0 + 1} {hz0} {hx1} {hy0 + 3} {hz1} minecraft:cobblestone")
    door = (hx1, hy0 + 1, hz0 + 2)
    door_top = (hx1, hy0 + 2, hz0 + 2)
    fixture(f"setblock {door[0]} {door[1]} {door[2]} minecraft:air")
    fixture(f"setblock {door_top[0]} {door_top[1]} {door_top[2]} minecraft:air")
    fixture(f"setblock {door[0]} {door[1]} {door[2]} minecraft:oak_door[facing=west,half=lower,hinge=left,open=false]")
    fixture(f"setblock {door_top[0]} {door_top[1]} {door_top[2]} minecraft:oak_door[facing=west,half=upper,hinge=left,open=false]")
    bed_foot, bed_head = bed_cells(hx0, hy0, hz0)
    assert bed_facing_valid(bed_foot, bed_head, "east"), "bed fixture must be east foot->head"
    assert house_headroom_ok(bed_head[1], hy1), "bed needs two air blocks under the roof"
    fixture(f"setblock {bed_foot[0]} {bed_foot[1]} {bed_foot[2]} minecraft:red_bed[facing=east,part=foot,occupied=false]")
    fixture(f"setblock {bed_head[0]} {bed_head[1]} {bed_head[2]} minecraft:red_bed[facing=east,part=head,occupied=false]")
    _expect_block(probe, door, "minecraft:oak_door", "house_door_placed")
    _expect_block(probe, bed_head, "minecraft:red_bed", "house_bed_placed")
    house_stage = (hx0 + 3, hy0 + 1, hz0 + 3)
    fixture(f"setblock {house_stage[0]} {house_stage[1]} {house_stage[2]} minecraft:air")
    if ctx.windowed:
        native_client.capture(probe.client, artifact, "village-qa-house")

    # --- workshop 5x5x5 with west door and open interior; farm inside reach east ---
    work_lo, work_hi = _rect(origin, (8, 0, 0), (12, 4, 4))
    wx0, wy0, wz0 = work_lo
    wx1, wy1, wz1 = work_hi
    fixture(f"fill {wx0} {wy0} {wz0} {wx1} {wy0} {wz1} minecraft:oak_planks")
    fixture(f"fill {wx0} {wy1} {wz0} {wx1} {wy1} {wz1} minecraft:oak_planks")
    fixture(f"fill {wx0} {wy0 + 1} {wz0} {wx1} {wy0 + 3} {wz0} minecraft:cobblestone")
    fixture(f"fill {wx0} {wy0 + 1} {wz1} {wx1} {wy0 + 3} {wz1} minecraft:cobblestone")
    fixture(f"fill {wx0} {wy0 + 1} {wz0} {wx0} {wy0 + 3} {wz1} minecraft:cobblestone")
    fixture(f"fill {wx1} {wy0 + 1} {wz0} {wx1} {wy0 + 3} {wz1} minecraft:cobblestone")
    work_door = (wx0, wy0 + 1, wz0 + 2)
    work_door_top = (wx0, wy0 + 2, wz0 + 2)
    fixture(f"setblock {work_door[0]} {work_door[1]} {work_door[2]} minecraft:air")
    fixture(f"setblock {work_door_top[0]} {work_door_top[1]} {work_door_top[2]} minecraft:air")
    fixture(f"setblock {work_door[0]} {work_door[1]} {work_door[2]} minecraft:oak_door[facing=west,half=lower,hinge=left,open=false]")
    fixture(f"setblock {work_door_top[0]} {work_door_top[1]} {work_door_top[2]} minecraft:oak_door[facing=west,half=upper,hinge=left,open=false]")
    composter = (wx0 + 1, wy0 + 1, wz0 + 1)
    fixture(f"setblock {composter[0]} {composter[1]} {composter[2]} minecraft:composter")
    farmer_stage = (wx0 + 2, wy0 + 1, wz0 + 2)
    fixture(f"setblock {farmer_stage[0]} {farmer_stage[1]} {farmer_stage[2]} minecraft:air")
    fixture(f"setblock {farmer_stage[0]} {farmer_stage[1] + 1} {farmer_stage[2]} minecraft:air")
    # Farmland row inside the east wall line: within farmer REACH 4 of the station.
    farm = [(wx1, wy0, wz0 + 1 + i) for i in range(3)]
    for fx, fy, fz in farm:
        fixture(f"setblock {fx} {fy} {fz} minecraft:farmland[moisture=7]")
        fixture(f"setblock {fx} {fy + 1} {fz} minecraft:wheat[age=7]")
    fixture(f"setblock {wx1 + 1} {wy0} {wz0 + 2} minecraft:water")
    _expect_block(probe, composter, "minecraft:composter", "workshop_composter_placed")
    _expect_block(probe, (farm[0][0], farm[0][1] + 1, farm[0][2]), "minecraft:wheat", "wheat_mature_placed")
    wheat_props = _props(_read(probe, (farm[0][0], farm[0][1] + 1, farm[0][2])))
    if wheat_props.get("age") != "7":
        raise RuntimeError(f"village-qa fixture failed: wheat age is {wheat_props}, want age=7")
    if ctx.windowed:
        native_client.capture(probe.client, artifact, "village-qa-workshop")

    # --- storage 5x5x5 with west door, open interior, one chest ---
    store_lo, store_hi = _rect(origin, (8, 0, 6), (12, 4, 10))
    sx0, sy0, sz0 = store_lo
    sx1, sy1, sz1 = store_hi
    fixture(f"fill {sx0} {sy0} {sz0} {sx1} {sy0} {sz1} minecraft:oak_planks")
    fixture(f"fill {sx0} {sy1} {sz0} {sx1} {sy1} {sz1} minecraft:oak_planks")
    fixture(f"fill {sx0} {sy0 + 1} {sz0} {sx1} {sy0 + 3} {sz0} minecraft:cobblestone")
    fixture(f"fill {sx0} {sy0 + 1} {sz1} {sx1} {sy0 + 3} {sz1} minecraft:cobblestone")
    fixture(f"fill {sx0} {sy0 + 1} {sz0} {sx0} {sy0 + 3} {sz1} minecraft:cobblestone")
    fixture(f"fill {sx1} {sy0 + 1} {sz0} {sx1} {sy0 + 3} {sz1} minecraft:cobblestone")
    store_door = (sx0, sy0 + 1, sz0 + 2)
    store_door_top = (sx0, sy0 + 2, sz0 + 2)
    fixture(f"setblock {store_door[0]} {store_door[1]} {store_door[2]} minecraft:air")
    fixture(f"setblock {store_door_top[0]} {store_door_top[1]} {store_door_top[2]} minecraft:air")
    fixture(f"setblock {store_door[0]} {store_door[1]} {store_door[2]} minecraft:oak_door[facing=west,half=lower,hinge=left,open=false]")
    fixture(f"setblock {store_door_top[0]} {store_door_top[1]} {store_door_top[2]} minecraft:oak_door[facing=west,half=upper,hinge=left,open=false]")
    chest = (sx0 + 1, sy0 + 1, sz0 + 1)
    fixture(f"setblock {chest[0]} {chest[1]} {chest[2]} minecraft:chest[facing=north,type=single,waterlogged=false]")
    store_stage = (sx0 + 2, sy0 + 1, sz0 + 2)
    fixture(f"setblock {store_stage[0]} {store_stage[1]} {store_stage[2]} minecraft:air")
    fixture(f"setblock {store_stage[0]} {store_stage[1] + 1} {store_stage[2]} minecraft:air")
    _expect_block(probe, chest, "minecraft:chest", "storage_chest_placed")
    if ctx.windowed:
        native_client.capture(probe.client, artifact, "village-qa-storage")

    # --- mine zone at grade: stone to y0, air above, smithing table at top plane ---
    mine_lo, mine_hi = _rect(origin, (14, -6, -2), (24, 1, 8))
    mx1, my1, _mz1 = mine_hi
    fixture(f"fill {mine_lo[0]} {mine_lo[1]} {mine_lo[2]} {mx1} {my1 - 1} {mine_hi[2]} minecraft:stone")
    fixture(f"fill {mine_lo[0]} {my1} {mine_lo[2]} {mx1} {my1 + 3} {mine_hi[2]} minecraft:air")
    entrance = (mine_lo[0] + 1, my1, mine_lo[2] + 1)
    fixture(f"setblock {entrance[0]} {entrance[1]} {entrance[2]} minecraft:smithing_table")
    miner_stage = (entrance[0] + 1, my1, entrance[2])
    fixture(f"setblock {miner_stage[0]} {miner_stage[1]} {miner_stage[2]} minecraft:air")
    _expect_block(probe, entrance, "minecraft:smithing_table", "mine_station_placed")
    # Permanent hazards can select any surface lane. Observe the first two
    # descending floor cells of every lane, never just the original entrance.
    dig_cells = [(entrance[0] + dx, entrance[1] - dx, z)
                 for dx in (1, 2) for z in range(mine_lo[2] + 1, mine_hi[2])]
    dig_before = {cell: _block_id(_read(probe, cell)) for cell in dig_cells}
    detail["mine_probe_cells"] = len(dig_cells)

    # --- lumber camp 5x5x5 with west door; two separated mature persistent=false oaks ---
    camp_lo, camp_hi = _rect(origin, (14, 0, 12), (18, 4, 16))
    cx0, cy0, cz0 = camp_lo
    cx1, cy1, cz1 = camp_hi
    fixture(f"fill {cx0} {cy0} {cz0} {cx1} {cy0} {cz1} minecraft:oak_planks")
    fixture(f"fill {cx0} {cy1} {cz0} {cx1} {cy1} {cz1} minecraft:oak_planks")
    fixture(f"fill {cx0} {cy0 + 1} {cz0} {cx1} {cy0 + 3} {cz0} minecraft:cobblestone")
    fixture(f"fill {cx0} {cy0 + 1} {cz1} {cx1} {cy0 + 3} {cz1} minecraft:cobblestone")
    fixture(f"fill {cx0} {cy0 + 1} {cz0} {cx0} {cy0 + 3} {cz1} minecraft:cobblestone")
    fixture(f"fill {cx1} {cy0 + 1} {cz0} {cx1} {cy0 + 3} {cz1} minecraft:cobblestone")
    camp_door = (cx0, cy0 + 1, cz0 + 2)
    camp_door_top = (cx0, cy0 + 2, cz0 + 2)
    fixture(f"setblock {camp_door[0]} {camp_door[1]} {camp_door[2]} minecraft:air")
    fixture(f"setblock {camp_door_top[0]} {camp_door_top[1]} {camp_door_top[2]} minecraft:air")
    fixture(f"setblock {camp_door[0]} {camp_door[1]} {camp_door[2]} minecraft:oak_door[facing=west,half=lower,hinge=left,open=false]")
    fixture(f"setblock {camp_door_top[0]} {camp_door_top[1]} {camp_door_top[2]} minecraft:oak_door[facing=west,half=upper,hinge=left,open=false]")
    fletcher = (cx0 + 1, cy0 + 1, cz0 + 1)
    fixture(f"setblock {fletcher[0]} {fletcher[1]} {fletcher[2]} minecraft:fletching_table")
    lumber_stage = (cx0 + 2, cy0 + 1, cz0 + 2)
    fixture(f"setblock {lumber_stage[0]} {lumber_stage[1]} {lumber_stage[2]} minecraft:air")
    fixture(f"setblock {lumber_stage[0]} {lumber_stage[1] + 1} {lumber_stage[2]} minecraft:air")
    _expect_block(probe, fletcher, "minecraft:fletching_table", "camp_station_placed")
    # Chebyshev >= 5 apart so neither 5x5 crown overwrites the other's trunk.
    tree_soils = [(cx0 + 1, cy0, cz1 + 2), (cx0 + 4, cy0, cz1 + 7)]
    assert trees_separated(tree_soils), "tree crowns must not overlap trunks"
    for tx, ty, tz in tree_soils:
        fixture(f"setblock {tx} {ty} {tz} minecraft:dirt")
        fixture(f"fill {tx - 2} {ty + 3} {tz - 2} {tx + 2} {ty + 5} {tz + 2} minecraft:oak_leaves[persistent=false,waterlogged=false]")
        fixture(f"fill {tx} {ty + 1} {tz} {tx} {ty + 4} {tz} minecraft:oak_log")
    settled("trees_planted")
    tree_trunks = {soil: [(soil[0], soil[1] + h, soil[2]) for h in (1, 2, 3, 4)]
                   for soil in tree_soils}
    for soil, cells in tree_trunks.items():
        for cell in cells:
            _expect_block(probe, cell, "minecraft:oak_log", f"tree_trunk_{soil}")
    if ctx.windowed:
        native_client.capture(probe.client, artifact, "village-qa-camp")

    # Raised floors have ordinary half-step approaches: a 1.95-high villager
    # cannot jump a full block through a two-block-high lintel.
    for cell, dx, facing in ((door, 1, "west"), (work_door, -1, "east"),
                             (store_door, -1, "east"), (camp_door, -1, "east")):
        fixture(f"setblock {cell[0] + dx} {cell[1] - 1} {cell[2]} "
                f"minecraft:oak_stairs[facing={facing},half=bottom,shape=straight]")

    # --- settlement radius covers every fixture, then certification ---
    cover = [house_lo, house_hi, work_lo, work_hi, store_lo, store_hi,
             mine_lo, mine_hi, camp_lo, camp_hi, *tree_soils,
             farmer_stage, miner_stage, lumber_stage, house_stage]
    radius = required_radius(origin, cover)
    detail["settlement_radius"] = radius
    fixture(f"settlement create {SETTLEMENT} {radius}")
    settled("settlement_created")
    fixture(f"settlement building certify house {house_lo[0]} {house_lo[1]} {house_lo[2]} "
            f"{house_hi[0]} {house_hi[1]} {house_hi[2]}")
    house_echo = settled("house_certified")
    if not any("Certified house" in line for line in house_echo.get("recent_chat", [])):
        raise RuntimeError("village-qa fixture failed: house not certified: "
                           f"{house_echo.get('recent_chat', [])[-4:]}")
    fixture(f"settlement building certify farmer {work_lo[0]} {work_lo[1]} {work_lo[2]} "
            f"{work_hi[0]} {work_hi[1]} {work_hi[2]}")
    farmer_echo = settled("farmer_certified")
    probe.check("farmer_certificate_accepted",
                any("Certified farmer" in line for line in farmer_echo.get("recent_chat", [])))
    fixture(f"settlement building certify storage {store_lo[0]} {store_lo[1]} {store_lo[2]} "
            f"{store_hi[0]} {store_hi[1]} {store_hi[2]}")
    storage_echo = settled("storage_certified")
    probe.check("storage_certificate_accepted",
                any("Certified storage" in line for line in storage_echo.get("recent_chat", [])))
    fixture(f"settlement building certify mine {mine_lo[0]} {mine_lo[1]} {mine_lo[2]} "
            f"{mine_hi[0]} {mine_hi[1]} {mine_hi[2]}")
    mine_echo = settled("mine_certified")
    probe.check("mine_certificate_accepted",
                any("Certified mine" in line for line in mine_echo.get("recent_chat", [])))
    fixture(f"settlement building certify lumber_camp {camp_lo[0]} {camp_lo[1]} {camp_lo[2]} "
            f"{camp_hi[0]} {camp_hi[1]} {camp_hi[2]}")
    camp_echo = settled("camp_certified")
    probe.check("camp_certificate_accepted",
                any("Certified lumber_camp" in line for line in camp_echo.get("recent_chat", [])))

    # --- summon + bind sequentially, nearest fresh villager per role ---
    detail["villager_uuids"] = []
    role_plan = tuple(zip(VILLAGER_ROLES, (house_stage, miner_stage, lumber_stage), strict=True))
    role_uuid: dict[str, str] = {}
    for role, stage in role_plan:
        fixture(f"summon minecraft:villager {stage[0]} {stage[1]} {stage[2]}")
        settled(f"{role}_summoned")
        candidates = _villagers(probe)
        if not candidates:
            raise RuntimeError(f"village-qa fixture failed: no villagers listed after {role} summon")
        taken = set(role_uuid.values())
        fresh = [row for row in candidates if row.get("entity_uuid") not in taken]
        probe.check(f"{role}_has_fresh_entity", bool(fresh))
        pick = min(fresh,
                   key=lambda row: abs(float(row["x"]) - stage[0]) + abs(float(row["z"]) - stage[2]))
        uuid = str(pick["entity_uuid"])
        teleport(float(pick["x"]), float(pick["y"]) + 1.0, float(pick["z"]))
        fixture(f"settlement bind {SETTLEMENT} {uuid}")
        role_uuid[role] = uuid
        detail["villager_uuids"].append({"role": role, "uuid": uuid, "stage": list(stage)})
    bound_echo = settled("villagers_bound")
    detail["bind_chat"] = bound_echo.get("recent_chat", [])[:12]
    detail["bind_ok_count"] = sum(1 for line in probe.observe().get("recent_chat", [])
                                  if "Bound " in line and f" to {SETTLEMENT}." in line)
    probe.check("three_villagers_bound", len(set(role_uuid.values())) == len(VILLAGER_ROLES)
                and detail["bind_ok_count"] >= len(VILLAGER_ROLES))
    detail["villagers_visible"] = len(_villagers(probe))
    detail["villager_uuids_observed"] = list(role_uuid.values())
    if count_time_sets(probe.report["fixture_commands"]) != 1:
        raise RuntimeError("village-qa fixture failed: expected one initial night phase")
    if ctx.windowed:
        native_client.capture(probe.client, artifact, "village-qa-bound")

    stations = {"farmer": composter, "miner": entrance, "lumberjack": fletcher}
    worker_uuids = {role_uuid[role] for role in stations}
    doors = [door, work_door, store_door, camp_door]
    teleport(x0 + 6.5, y0 + 1.0, z0 + 7.5)
    probe.call("minecraft_look", {"yaw_deg": 180, "pitch_deg": 0})

    stations = {"farmer": composter, "miner": entrance, "lumberjack": fletcher}
    return dict(origin=origin, bed_foot=bed_foot, bed_head=bed_head, stations=stations,
                role_uuid=role_uuid, house_lo=house_lo, house_hi=house_hi, mine=(mine_lo, mine_hi),
                watch=(x0 + 6.5, y0 + 1.0, z0 + 7.5), house_stage=house_stage, teleport=teleport,
                station_blocks={"farmer": "minecraft:composter", "miner": "minecraft:smithing_table",
                                "lumberjack": "minecraft:fletching_table"})



def _near(pos: list[float], cell: tuple[int, int, int], radius: float) -> bool:
    return math.dist((pos[0], pos[2]), (cell[0] + 0.5, cell[2] + 0.5)) < radius


class Attacks:
    def __init__(self, probe: ProbeSteps, detail: dict[str, Any], ctx: ProfileContext,
                 artifact: Path, session: Session, fx: dict[str, Any]) -> None:
        self.probe, self.detail, self.ctx, self.artifact, self.session, self.fx = (
            probe, detail, ctx, artifact, session, fx)
        self.workers = set(fx["role_uuid"].values())
        self.vanilla: str | None = None

    # ---- plumbing -------------------------------------------------------
    def flush(self) -> None:
        (self.artifact / "break.json").write_text(
            json.dumps(self.detail, indent=2, ensure_ascii=False) + "\n")

    def cmd(self, command: str) -> None:
        self.probe.fixture(command)

    def settle(self, label: str) -> None:
        _chat_echo(self.probe, label)

    def shot(self, name: str) -> None:
        if self.ctx.windowed:
            native_client.capture(self.probe.client, self.artifact, f"break-{name}")

    def rows(self) -> dict[str, dict[str, Any]]:
        out: dict[str, dict[str, Any]] = {}
        for row in _villagers(self.probe):
            uuid = str(row.get("entity_uuid"))
            out[uuid] = item = summary(row)
            if not item["agree"]:
                self.detail["disagreements"].append({"uuid": uuid[:8], **item})
        return out

    def wait(self, pred: Any, ticks: int) -> tuple[dict[str, dict[str, Any]], int, bool]:
        start = self.gt()
        elapsed = 0
        while True:
            now = self.rows()
            if pred(now):
                return now, elapsed, True
            if elapsed >= ticks:
                return now, elapsed, False
            _tick(self.probe)
            elapsed = self.gt() - start

    def gt(self) -> int:
        return int((self.probe.observe(wait_for_play=True).get("time") or {}).get("game_time") or 0)

    def note(self, label: str, rows: dict[str, dict[str, Any]] | None = None, **data: Any) -> None:
        entry: dict[str, Any] = {"label": label, **data}
        if rows is not None:
            entry["rows"] = {uuid[:8]: row for uuid, row in rows.items()}
        try:
            entry["time"] = self.probe.observe(wait_for_play=True).get("time")
        except Exception as error:
            entry["time_error"] = str(error)
        self.detail["timeline"].append(entry)
        self.flush()

    def verdict(self, name: str, held: bool | None, expected: str, observed: Any, **extra: Any) -> None:
        self.detail["attacks"][name] = {
            "result": "held" if held else ("broke" if held is False else "inconclusive"),
            "expected": expected, "observed": observed, **extra}
        self.flush()

    def bed_props(self) -> dict[str, Any]:
        head = self.fx["bed_head"]
        block = _read(self.probe, head)
        return {"block": _block_id(block), **_props(block)}

    def sleeper(self, rows: dict[str, dict[str, Any]]) -> str | None:
        for uuid, row in rows.items():
            if row["sleep_pose"] and row["job"] and _near(row["pos"], self.fx["bed_head"], 0.75):
                return uuid
        return None

    def all_jobs(self, rows: dict[str, dict[str, Any]]) -> bool:
        return {row["job"] for uuid, row in rows.items() if uuid in self.workers} >= set(self.fx["stations"])

    def jobs(self, rows: dict[str, dict[str, Any]]) -> dict[str, Any]:
        return {uuid[:8]: (row["word"], row["wire"]) for uuid, row in rows.items()}

    def back_to(self, uuid: str) -> None:
        wx, wy, wz = self.fx["watch"]
        self.cmd(f"tp @s {wx:.1f} {wy:.1f} {wz:.1f}")
        self.probe.wait(lambda s: abs(float(s["player"]["x"]) - wx) < 3.0
                        and abs(float(s["player"]["z"]) - wz) < 3.0, f"player_back_{uuid}",
                        wait_for_play=True)

    # ---- phases ------------------------------------------------------------
    def day(self, label: str, extra: Any = None, ticks: int = 1500) -> dict[str, dict[str, Any]]:
        self.cmd("time set 3000")
        self.settle(label)
        rows, took, ok = self.wait(lambda r: self.all_jobs(r) and (extra is None or extra(r)), ticks)
        self.note(label, rows, ok=ok, ticks=took)
        return rows

    def night(self, label: str, extra: Any = None, ticks: int = 1200) -> tuple[dict[str, dict[str, Any]], str | None]:
        self.cmd("time set 18000")
        self.settle(label)
        rows, took, ok = self.wait(lambda r: self.sleeper(r) is not None and (extra is None or extra(r)), ticks)
        sleeper = self.sleeper(rows)
        self.note(label, rows, ok=ok, ticks=took, sleeper=sleeper and sleeper[:8], bed=self.bed_props())
        return rows, sleeper

    def guarded(self, name: str, fn: Any) -> None:
        try:
            fn()
        except TimeoutError:
            raise
        except Exception as error:  # record and continue with the next attack
            self.detail["attacks"].setdefault(name, {})["error"] = f"{type(error).__name__}: {error}"
            self.flush()

    def run(self, which: str) -> None:
        getattr(self, f"set_{which.lower()}")()
        self.note("final", self.rows(), bed=self.bed_props())
        self.shot("final")

    # ---- set A: vanilla librarian, reload while asleep, restart at night, time flips
    def set_a(self) -> None:
        ox, oy, oz = self.fx["origin"]
        lectern = (ox - 8, oy, oz + 2)
        lfoot, lhead = (ox - 10, oy, oz + 5), (ox - 9, oy, oz + 5)
        self.cmd(f"setblock {lectern[0]} {lectern[1]} {lectern[2]} minecraft:lectern")
        self.cmd(f"setblock {lfoot[0]} {lfoot[1]} {lfoot[2]} minecraft:white_bed[facing=east,part=foot,occupied=false]")
        self.cmd(f"setblock {lhead[0]} {lhead[1]} {lhead[2]} minecraft:white_bed[facing=east,part=head,occupied=false]")
        self.cmd(f"summon minecraft:villager {ox - 8} {oy} {oz + 4}")
        self.settle("vanilla_summoned")
        fresh = [(uuid, row) for uuid, row in self.rows().items() if uuid not in self.workers]
        if fresh:
            self.vanilla = min(fresh, key=lambda item: abs(item[1]["pos"][0] - (ox - 8)) + abs(item[1]["pos"][2] - (oz + 4)))[0]
        van = self.vanilla
        rows = self.day("a_day", lambda r: van is not None and r.get(van, {}).get("word") == "библиотекарь")
        day_jobs = {uuid: row["job"] for uuid, row in rows.items() if uuid in self.workers}
        self.verdict("A0_vanilla_librarian_by_day", van is not None and rows.get(van, {}).get("word") == "библиотекарь",
                     "unbound vanilla villager beside a lectern becomes a librarian (nameplate+VillagerData agree)",
                     rows.get(van or "", {}), vanilla=van and van[:8])
        rows, sleeper = self.night("a_night", lambda r: van is None or r.get(van, {}).get("sleep_pose"))
        before = self.jobs(rows)
        self.verdict("A0_vanilla_librarian_sleeps", bool(van and rows.get(van, {}).get("sleep_pose")),
                     "librarian lies down in its own bed at night", rows.get(van or "", {}))

        def reload() -> None:
            self.cmd(f"tp @s {ox + 1500.5:.1f} {oy + 40:.1f} {oz + 0.5:.1f}")
            gone, took, ok = self.wait(lambda r: not (set(r) & (self.workers | {van} - {None})), 600)
            self.note("a_far", gone, ok=ok, ticks=took)
            _, _, _ = self.wait(lambda r: False, 100)
            self.back_to("reload")
            back, took2, ok2 = self.wait(lambda r: (self.workers | ({van} if van else set())) <= set(r), 600)
            self.note("a_back", back, ok=ok2, ticks=took2)
            after = self.jobs(back)
            same = all(after.get(u) == before.get(u) for u in before)
            still = sleeper is not None and back.get(sleeper, {}).get("sleep_pose")
            agree = all(row["agree"] for row in back.values())
            self.verdict("A1_reload_while_asleep", bool(ok and ok2 and same and still and agree),
                         "after the village leaves and re-enters view at night: same jobs re-sent, sleeper still in sleep pose, nameplate=VillagerData",
                         {"before": before, "after": after, "sleeper": sleeper and sleeper[:8],
                          "sleeper_row": back.get(sleeper or ""), "unloaded_seen": ok, "back_seen": ok2})
        self.guarded("A1_reload_while_asleep", reload)

        def restart_night() -> None:
            info = self.session.restart(self.probe)
            self.back_to("restart")
            back, took, ok = self.wait(lambda r: (self.workers | ({van} if van else set())) <= set(r)
                                       and (sleeper is None or r[sleeper]["sleep_pose"]), 400)
            after = self.jobs(back)
            self.note("a_restart", back, ok=ok, ticks=took, restart=info, bed=self.bed_props())
            same = all(after.get(u) == before.get(u) for u in before)
            agree = all(row["agree"] for row in back.values())
            still = sleeper is not None and back.get(sleeper, {}).get("sleep_pose")
            self.verdict("A2_clean_restart_at_night", bool(ok and same and agree and still and info["server_exit"] == 0),
                         "clean stop at night keeps every job; the sleeper is still asleep in its bed after rejoin (vanilla persists SleepingX/Y/Z)",
                         {"before": before, "after": after, "sleeper_row": back.get(sleeper or ""), **info})
        self.guarded("A2_clean_restart_at_night", restart_night)

        def flips() -> None:
            self.cmd("time set 3000")
            _tick(self.probe)
            self.cmd("time set 18000")
            _tick(self.probe)
            self.cmd("time set 3000")
            self.settle("a_flips")
            rows, took, ok = self.wait(lambda r: not any(row["sleep_pose"] for row in r.values()), 300)
            bed = self.bed_props()
            stuck = [u[:8] for u, row in rows.items() if row["sleep_pose"]]
            after = self.jobs(rows)
            same = all(after.get(u, (None,))[0] == before.get(u, (None,))[0] for u in before)
            self.note("a_flips_settled", rows, ok=ok, ticks=took, bed=bed)
            self.verdict("A3_time_flips_day_night_day", bool(ok and bed.get("occupied") == "false" and same
                                                            and all(row["agree"] for row in rows.values())),
                         "day->night->day within a few ticks: everyone awake by day, bed occupied=false, jobs unchanged",
                         {"stuck_sleeping": stuck, "bed": bed, "before": before, "after": after, "ticks": took})
            target = {u: job for u, job in day_jobs.items() if job}
            def returned(r: dict[str, dict[str, Any]]) -> bool:
                return all(u in r and r[u]["job"] == job and (_near(r[u]["pos"], self.fx["stations"][job], 2.5)
                                                               or job != "farmer") for u, job in target.items())
            rows, took, ok = self.wait(returned, 1200)
            self.note("a_morning", rows, ok=ok, ticks=took)
            held = all(rows.get(u, {}).get("job") == job for u, job in target.items())
            vanilla_ok = van is None or rows.get(van, {}).get("word") == "библиотекарь"
            self.verdict("A4_all_jobs_after_morning", bool(held and vanilla_ok),
                         "every worker (sleeper and the two bedless shelterers) and the vanilla librarian keep their jobs on the next day",
                         {"day_jobs": {u[:8]: j for u, j in target.items()}, "now": self.jobs(rows),
                          "farmer_back": ok})
        self.guarded("A3_time_flips_day_night_day", flips)

    # ---- set B: bed broken / job site broken / workstation replaced at night
    def set_b(self) -> None:
        foot, head = self.fx["bed_foot"], self.fx["bed_head"]
        stations = self.fx["stations"]
        blocks = self.fx["station_blocks"]
        rows = self.day("b_day")
        day_jobs = {uuid: row["job"] for uuid, row in rows.items() if uuid in self.workers}
        rows, sleeper = self.night("b_night")
        self.shot("b-asleep")

        def bed_broken() -> None:
            if sleeper is None:
                self.verdict("B1_bed_broken_while_asleep", None, "-", "no sleeper")
                return
            job = rows[sleeper]["job"]
            self.cmd(f"fill {foot[0]} {foot[1]} {foot[2]} {head[0]} {head[1]} {head[2]} minecraft:air")
            woke, took, ok = self.wait(lambda r: sleeper in r and not r[sleeper]["sleep_pose"], 100)
            row = woke.get(sleeper, {})
            self.note("b_bed_broken", woke, ok=ok, ticks=took)
            self.shot("b-bed-broken")
            floor_ok = bool(row) and abs(row["pos"][1] - head[1]) < 0.6
            self.verdict("B1_bed_broken_while_asleep", bool(ok and row.get("job") == job and floor_ok),
                         "vanilla LivingEntity.checkBedExists: the sleeper wakes at once, stands at floor level, keeps its job",
                         {"sleeper": sleeper[:8], "row": row, "ticks": took, "bed_y": head[1]})
            self.cmd(f"setblock {foot[0]} {foot[1]} {foot[2]} minecraft:red_bed[facing=east,part=foot,occupied=false]")
            self.cmd(f"setblock {head[0]} {head[1]} {head[2]} minecraft:red_bed[facing=east,part=head,occupied=false]")
            again, took, ok = self.wait(lambda r: self.sleeper(r) is not None, 800)
            self.note("b_bed_restored", again, ok=ok, ticks=took, bed=self.bed_props())
            self.verdict("B1b_bed_restored_sleeps_again", ok,
                         "a worker sleeps in the restored bed the same night",
                         {"sleeper": (self.sleeper(again) or "")[:8], "ticks": took})
        self.guarded("B1_bed_broken_while_asleep", bed_broken)

        def station_broken() -> None:
            now, _, _ = self.wait(lambda r: self.sleeper(r) is not None, 400)
            who = self.sleeper(now)
            if who is None:
                self.verdict("B2_job_site_broken_while_asleep", None, "-", "no sleeper")
                return
            job = now[who]["job"]
            station = stations[job]
            self.cmd(f"setblock {station[0]} {station[1]} {station[2]} minecraft:air")
            lost, took, ok = self.wait(lambda r: who in r and r[who]["job"] != job, 300)
            row = lost.get(who, {})
            self.note("b_station_broken", lost, ok=ok, ticks=took)
            self.shot("b-station-broken")
            self.verdict("B2_job_site_broken_while_asleep", bool(ok and row.get("agree") and row.get("word") == "безработный"),
                         "vanilla: a novice with no trades loses its profession when its job site is broken; the client shows it at once (nameplate and outfit agree)",
                         {"sleeper": who[:8], "job": job, "row": row, "ticks": took})
            self.cmd(f"setblock {station[0]} {station[1]} {station[2]} {blocks[job]}")
            rows2 = self.day("b_day_after_station_restored")
            holder = [u[:8] for u, r in rows2.items() if r["job"] == job]
            self.verdict("B2b_job_back_after_station_restored", bool(holder) and self.all_jobs(rows2),
                         "the re-placed station is taken again the next work period; all three jobs held",
                         {"job": job, "holders": holder, "jobs": self.jobs(rows2)})
        self.guarded("B2_job_site_broken_while_asleep", station_broken)

        def station_replaced() -> None:
            rows3, who_sleeps = self.night("b_night2")
            others = [(u, r["job"]) for u, r in rows3.items()
                      if u in self.workers and u != who_sleeps and r["job"]]
            if not others:
                self.verdict("B3_workstation_swapped_at_night", None, "-", "no awake job holder")
                return
            who, job = others[0]
            station = stations[job]
            self.cmd(f"setblock {station[0]} {station[1]} {station[2]} minecraft:lectern")
            changed, took, ok = self.wait(lambda r: who in r and r[who]["job"] != job, 300)
            row = changed.get(who, {})
            self.note("b_station_swapped", changed, ok=ok, ticks=took)
            rows4 = self.day("b_day_after_swap", lambda r: who in r and r[who]["word"] not in (None,), 600)
            self.verdict("B3_workstation_swapped_at_night", bool(ok and row.get("agree")),
                         "the old job ends when its station becomes a lectern; nameplate and outfit agree at night and next day",
                         {"who": who[:8], "job": job, "night_row": row, "day_row": rows4.get(who), "ticks": took})
            self.cmd(f"setblock {station[0]} {station[1]} {station[2]} {blocks[job]}")
            rows5, took5, ok5 = self.wait(lambda r: self.all_jobs(r), 1500)
            self.note("b_station_unswapped", rows5, ok=ok5, ticks=took5)
            self.verdict("B3b_job_back_after_unswap", ok5, "restoring the station gives the job back within the work period",
                         {"jobs": self.jobs(rows5), "ticks": took5})
        self.guarded("B3_workstation_swapped_at_night", station_replaced)
        self.detail["day_jobs"] = {u[:8]: j for u, j in day_jobs.items()}

    # ---- set C: night while walking to the job site, player uses the occupied bed, restart on morning
    def set_c(self) -> None:
        self.cmd("time set 3000")
        self.settle("c_day_start")
        walking, _, _ = self.wait(lambda r: False, 60)
        self.note("c_walking", walking)
        rows, sleeper = self.night("c_night_while_walking", ticks=800)
        rows2 = self.day("c_next_day", ticks=2400)
        self.verdict("C1_night_while_walking_to_job", self.all_jobs(rows2) and all(r["agree"] for r in rows2.values()),
                     "night interrupting the first walk to the job site: every job taken the next work period",
                     {"at_walk": self.jobs(walking), "night_sleeper": sleeper and sleeper[:8], "next_day": self.jobs(rows2)})
        day_jobs = {uuid: row["job"] for uuid, row in rows2.items() if uuid in self.workers}

        def player_bed() -> None:
            rows3, who = self.night("c_night")
            if who is None:
                self.verdict("C2_player_uses_occupied_bed", None, "-", "no sleeper")
                return
            stage = self.fx["house_stage"]
            head = self.fx["bed_head"]
            self.cmd(f"tp @s {stage[0] + 0.5:.1f} {stage[1]:.1f} {stage[2] + 0.5:.1f}")
            self.probe.call("minecraft_set_hotbar_slot", {"slot": 8})
            self.probe.call("minecraft_use_item_on", {"x": head[0], "y": head[1], "z": head[2],
                                                       "face": "up", "hand": "main_hand"})
            def in_bed() -> bool:
                p = self.probe.observe(wait_for_play=True).get("player") or {}
                return (p.get("pose") == "sleeping"
                        or (abs(float(p.get("y", 0)) - (head[1] + 0.5625)) < 0.05
                            and abs(float(p.get("x", 0)) - (head[0] + 0.5)) < 0.1
                            and abs(float(p.get("z", 0)) - (head[2] + 0.5)) < 0.1))
            woke, took, woke_ok = self.wait(lambda r: who in r and not r[who]["sleep_pose"], 40)
            bed_after_click = in_bed()
            after, took2, _ = self.wait(lambda r: False, 20)
            bed_later = in_bed()
            state = self.probe.observe()
            row = woke.get(who, {})
            self.note("c_player_bed", after, chat=state.get("recent_chat", [])[:6], player=state.get("player"),
                      woke_ticks=took, player_in_bed=[bed_after_click, bed_later])
            self.shot("c-player-bed")
            self.verdict("C2_player_uses_occupied_bed",
                         bool(woke_ok and not bed_after_click and not bed_later
                              and row.get("job") == rows3[who]["job"]),
                         "vanilla BedBlock.useWithoutItem on an OCCUPIED bed: kickVillagerOutOfBed wakes the villager "
                         "(job kept) and the player is not put to bed on that click",
                         {"sleeper": who[:8], "row": row, "woke_ticks": took,
                          "player_in_bed": [bed_after_click, bed_later], "player": state.get("player", {}).get("pose"),
                          "chat": state.get("recent_chat", [])[:6]})
            self.back_to("bed")
        self.guarded("C2_player_uses_occupied_bed", player_bed)

        def restart_morning() -> None:
            rows4, _ = self.wait(lambda r: self.sleeper(r) is not None, 400)[0], None
            who = self.sleeper(rows4)
            before = self.jobs(rows4)
            self.cmd("time set 3000")
            self.settle("c_morning")
            info = self.session.restart(self.probe)
            self.back_to("restart")
            back, took, ok = self.wait(lambda r: self.workers <= set(r) and not any(x["sleep_pose"] for x in r.values()), 400)
            bed = self.bed_props()
            self.note("c_restart_morning", back, ok=ok, ticks=took, restart=info, bed=bed)
            after = self.jobs(back)
            same = all(after.get(u, (None,))[0] == before.get(u, (None,))[0] for u in before)
            self.verdict("C3_clean_restart_on_morning", bool(ok and same and bed.get("occupied") == "false"
                                                           and all(r["agree"] for r in back.values())),
                         "restart right after morning: nobody stuck asleep, bed free, same jobs",
                         {"sleeper": who and who[:8], "before": before, "after": after, "bed": bed, **info})
            target = day_jobs.get(who or "")
            if who and target:
                def returned(r: dict[str, dict[str, Any]]) -> bool:
                    row = r.get(who)
                    if not row or row["job"] != target:
                        return False
                    if _near(row["pos"], self.fx["stations"][target], 2.5):
                        return True
                    (x0, _, z0), (x1, y1, z1) = self.fx["mine"]
                    return target == "miner" and x0 <= row["pos"][0] < x1 + 1 and z0 <= row["pos"][2] < z1 + 1
                ret, took2, ok2 = self.wait(returned, 2400)
                self.note("c_after_restart_work", ret, ok=ok2, ticks=took2)
                self.verdict("C3b_back_to_work_after_restart", ok2 or None if target == "lumberjack" else ok2,
                             "the woken sleeper goes back to its station (lumberjack: inconclusive without a cut)",
                             {"who": who[:8], "job": target, "row": ret.get(who), "ticks": took2})
        self.guarded("C3_clean_restart_on_morning", restart_morning)

    # ---- set D: sleeper moved off its bed, server killed (SIGKILL) at night
    def set_d(self) -> None:
        rows = self.day("d_day")
        day_jobs = {uuid: row["job"] for uuid, row in rows.items() if uuid in self.workers}
        rows, sleeper = self.night("d_night")

        def moved() -> None:
            if sleeper is None:
                self.verdict("D1_sleeper_teleported_off_bed", None, "-", "no sleeper")
                return
            stage = self.fx["house_stage"]
            self.cmd(f"tp {sleeper} {stage[0] + 0.5:.1f} {stage[1]:.1f} {stage[2] + 0.5:.1f}")
            after, took, ok = self.wait(lambda r: sleeper in r and _near(r[sleeper]["pos"], stage, 1.0), 100)
            row = after.get(sleeper, {})
            self.note("d_teleported", after, ok=ok, ticks=took)
            self.shot("d-teleported")
            self.verdict("D1_sleeper_teleported_off_bed",
                         bool(ok and not (row.get("sleep_pose") and not _near(row["pos"], self.fx["bed_head"], 0.75))
                              and row.get("job") == rows[sleeper]["job"]),
                         "a villager moved off its bed does not keep lying in the air; it stands, keeps its job (and may walk back to bed)",
                         {"sleeper": sleeper[:8], "row": row, "ticks": took})
            back, took2, ok2 = self.wait(lambda r: self.sleeper(r) is not None, 600)
            self.note("d_back_in_bed", back, ok=ok2, ticks=took2)
            self.verdict("D1b_back_in_bed", ok2, "a job holder is back asleep in the bed the same night",
                         {"sleeper": (self.sleeper(back) or "")[:8], "ticks": took2})
        self.guarded("D1_sleeper_teleported_off_bed", moved)

        def killed() -> None:
            now, _, _ = self.wait(lambda r: self.sleeper(r) is not None, 400)
            who = self.sleeper(now)
            before = self.jobs(now)
            self.cmd("save-all flush")
            self.settle("d_saved")
            # one more autosave-free interval so the kill lands on WAL-only state
            _, _, _ = self.wait(lambda r: False, 100)
            info = self.session.restart(self.probe, kill=True)
            self.back_to("kill")
            back, took, ok = self.wait(lambda r: self.workers <= set(r), 400)
            after = self.jobs(back)
            self.note("d_after_kill", back, ok=ok, ticks=took, restart=info, bed=self.bed_props())
            same = all(after.get(u, (None,))[0] == before.get(u, (None,))[0] for u in before)
            self.verdict("D2_sigkill_at_night", bool(ok and same and all(r["agree"] for r in back.values())),
                         "after kill -9 at night and recovery every job is kept and displayed consistently",
                         {"sleeper": who and who[:8], "before": before, "after": after,
                          "sleeper_row": back.get(who or ""), **info})
            rows2 = self.day("d_morning_after_kill")
            stuck = [u[:8] for u, r in rows2.items() if r["sleep_pose"]]
            self.verdict("D2b_morning_after_kill", self.all_jobs(rows2) and not stuck,
                         "next morning nobody is stuck asleep and all jobs are held",
                         {"stuck": stuck, "jobs": self.jobs(rows2),
                          "day_jobs": {u[:8]: j for u, j in day_jobs.items()}})
        self.guarded("D2_sigkill_at_night", killed)

    # ---- set E: bed broken while asleep, NOT restored: where does the sleeper end up?
    def set_e(self) -> None:
        foot, head = self.fx["bed_foot"], self.fx["bed_head"]
        rows = self.day("e_day")
        day_jobs = {uuid: row["job"] for uuid, row in rows.items() if uuid in self.workers}
        rows, sleeper = self.night("e_night")
        if sleeper is None:
            self.verdict("E1_bed_broken_track", None, "-", "no sleeper")
            return
        self.cmd(f"fill {foot[0]} {foot[1]} {foot[2]} {head[0]} {head[1]} {head[2]} minecraft:air")
        samples: list[dict[str, Any]] = []
        start = _game_time(self.probe)
        while True:
            now = self.rows()
            gt = _game_time(self.probe)
            if sleeper in now:
                samples.append({"t": gt - start, **now[sleeper]})
            if gt - start >= 200:
                break
            _tick(self.probe)
        self.shot("e-bed-broken")
        floor = head[1]
        settled = [s for s in samples if not s["sleep_pose"] and abs(s["pos"][1] - floor) < 0.1]
        floating = [s for s in samples if not s["sleep_pose"] and s["pos"][1] - floor > 0.3]
        self.detail["e_samples"] = samples
        self.verdict("E1_bed_broken_track", bool(settled) and samples and not samples[-1]["sleep_pose"]
                     and abs(samples[-1]["pos"][1] - floor) < 0.1 and samples[-1]["job"] == rows[sleeper]["job"],
                     "vanilla: woken at once, placed standing at floor level beside the bed, keeps its job",
                     {"sleeper": sleeper[:8], "first": samples[:3], "last": samples[-1:],
                      "first_settled_t": settled[0]["t"] if settled else None,
                      "floating_samples": len(floating), "floating_last_t": floating[-1]["t"] if floating else None})
        rows2 = self.day("e_morning")
        self.verdict("E2_bedless_next_day", self.all_jobs(rows2) and not any(r["sleep_pose"] for r in rows2.values()),
                     "the now bedless worker keeps its job and works the next day",
                     {"jobs": self.jobs(rows2), "day_jobs": {u[:8]: j for u, j in day_jobs.items()}})

    # ---- set F: extra jobless resident, relog at night, vanilla librarian's lectern broken while asleep
    def set_f(self) -> None:
        ox, oy, oz = self.fx["origin"]
        stage = self.fx["house_stage"]
        before_ids = set(self.rows())
        self.cmd(f"summon minecraft:villager {stage[0]} {stage[1]} {stage[2]}")
        self.settle("f_extra_summoned")
        fresh = [u for u in self.rows() if u not in before_ids]
        extra = fresh[0] if fresh else None
        if extra:
            self.cmd(f"settlement bind {SETTLEMENT} {extra}")
            self.settle("f_extra_bound")
        lectern = (ox - 8, oy, oz + 2)
        lfoot, lhead = (ox - 10, oy, oz + 5), (ox - 9, oy, oz + 5)
        self.cmd(f"setblock {lectern[0]} {lectern[1]} {lectern[2]} minecraft:lectern")
        self.cmd(f"setblock {lfoot[0]} {lfoot[1]} {lfoot[2]} minecraft:white_bed[facing=east,part=foot,occupied=false]")
        self.cmd(f"setblock {lhead[0]} {lhead[1]} {lhead[2]} minecraft:white_bed[facing=east,part=head,occupied=false]")
        known = set(self.rows())
        self.cmd(f"summon minecraft:villager {ox - 8} {oy} {oz + 4}")
        self.settle("f_vanilla_summoned")
        fresh = [u for u in self.rows() if u not in known]
        van = fresh[0] if fresh else None
        rows = self.day("f_day", lambda r: van is not None and r.get(van, {}).get("word") == "библиотекарь", 1500)
        self.verdict("F0_extra_resident_by_day", bool(extra) and self.all_jobs(rows) and all(r["agree"] for r in rows.values()),
                     "a fourth bound villager does not take a job from the three workers; every nameplate agrees with VillagerData",
                     {"extra": extra and extra[:8], "extra_row": rows.get(extra or ""), "jobs": self.jobs(rows)})
        rows, sleeper = self.night("f_night", lambda r: van is None or r.get(van, {}).get("sleep_pose"))
        sleepers = {u[:8]: r["word"] for u, r in rows.items() if r["sleep_pose"]}
        self.note("f_sleepers", None, sleepers=sleepers)
        before = self.jobs(rows)

        def relog() -> None:
            self.session.relog(self.probe)
            self.back_to("relog")
            back, took, ok = self.wait(lambda r: set(rows) <= set(r), 400)
            after = self.jobs(back)
            asleep = {u[:8]: r["word"] for u, r in back.items() if r["sleep_pose"]}
            self.note("f_relog", back, ok=ok, ticks=took)
            self.verdict("F1_relog_at_night", bool(ok and after == before and asleep == sleepers
                                                   and all(r["agree"] for r in back.values())),
                         "a player rejoining at night sees the same jobs and the same villagers asleep",
                         {"before": before, "after": after, "sleepers_before": sleepers, "sleepers_after": asleep})
        self.guarded("F1_relog_at_night", relog)

        def lectern_broken() -> None:
            if van is None:
                self.verdict("F2_vanilla_job_site_broken_while_asleep", None, "-", "no vanilla villager")
                return
            self.cmd(f"setblock {lectern[0]} {lectern[1]} {lectern[2]} minecraft:air")
            lost, took, ok = self.wait(lambda r: van in r and r[van]["word"] != "библиотекарь", 300)
            row = lost.get(van, {})
            self.note("f_lectern_broken", lost, ok=ok, ticks=took)
            self.verdict("F2_vanilla_job_site_broken_while_asleep", bool(ok and row.get("agree") and row.get("word") == "безработный"),
                         "vanilla: a novice librarian with no trades loses its profession when the lectern breaks (outfit and nameplate agree)",
                         {"row": row, "ticks": took})
            self.cmd(f"setblock {lectern[0]} {lectern[1]} {lectern[2]} minecraft:lectern")
            back = self.day("f_day2", lambda r: r.get(van, {}).get("word") == "библиотекарь", 1500)
            self.verdict("F2b_vanilla_job_back", back.get(van, {}).get("word") == "библиотекарь" and self.all_jobs(back),
                         "re-placed lectern is taken again next day; workers keep their jobs",
                         {"van": back.get(van), "jobs": self.jobs(back)})
        self.guarded("F2_vanilla_job_site_broken_while_asleep", lectern_broken)

    # ---- set G: a workstation moved inside its certified building at night
    def set_g(self) -> None:
        stations = self.fx["stations"]
        rows = self.day("g_day")
        rows, sleeper = self.night("g_night")
        old = stations["farmer"]
        new = (old[0], old[1], old[2] + 2)
        holder = next((u for u, r in rows.items() if r["job"] == "farmer"), None)
        self.cmd(f"setblock {old[0]} {old[1]} {old[2]} minecraft:air")
        self.cmd(f"setblock {new[0]} {new[1]} {new[2]} minecraft:composter")
        lost, took, ok = self.wait(lambda r: holder in r and r[holder]["job"] != "farmer", 300)
        self.note("g_moved", lost, ok=ok, ticks=took, holder=holder and holder[:8])
        self.probe.call("minecraft_send_chat", {"message": "settlement building list", "command": True})
        listing = _chat_echo(self.probe, "g_building_list")
        self.detail["g_building_list"] = listing.get("recent_chat", [])[:12]
        back = self.day("g_morning", lambda r: holder in r and r[holder]["job"] == "farmer"
                        and _near(r[holder]["pos"], new, 2.5), 2400)
        row = back.get(holder or "", {})
        self.verdict("G1_station_moved_in_building_at_night",
                     bool(row.get("job") == "farmer" and self.all_jobs(back)),
                     "vanilla: moving a job block loses the old POI; the jobless villager takes the re-placed workstation in its work period",
                     {"holder": holder and holder[:8], "night_row": lost.get(holder or ""), "morning_row": row,
                      "jobs": self.jobs(back), "near_new": bool(row) and _near(row["pos"], new, 2.5),
                      "building_list": self.detail["g_building_list"]})
        # workaround probe: re-certify the same workshop box; does the moved composter become the job?
        ox, oy, oz = self.fx["origin"]
        self.cmd(f"settlement building certify farmer {ox + 8} {oy} {oz} {ox + 12} {oy + 4} {oz + 4}")
        echo = _chat_echo(self.probe, "g_recertified")
        self.detail["g_recertify_chat"] = echo.get("recent_chat", [])[:6]
        again, took2, ok2 = self.wait(lambda r: any(x["job"] == "farmer" for x in r.values()), 1500)
        self.note("g_recertified", again, ok=ok2, ticks=took2)
        self.verdict("G2_recertify_restores_job", ok2,
                     "re-certifying the workshop records the moved composter and a villager takes the farmer job",
                     {"ticks": took2, "jobs": self.jobs(again), "chat": self.detail["g_recertify_chat"]})

    # ---- set I: the player hits the sleeping villager (vanilla hurtServer -> stopSleeping)
    def set_i(self) -> None:
        rows = self.day("i_day")
        rows, sleeper = self.night("i_night")
        if sleeper is None:
            self.verdict("I1_sleeper_hit", None, "-", "no sleeper")
            return
        job = rows[sleeper]["job"]
        stage = self.fx["house_stage"]
        head = self.fx["bed_head"]
        self.cmd("gamemode survival @s")
        hit: Any = None
        tries = []
        for spot in ((head[0] + 1.6, head[1], head[2] + 0.5), (head[0] + 0.5, head[1], head[2] + 1.6),
                     (stage[0] + 0.5, stage[1], stage[2] + 0.5)):
            self.cmd(f"tp @s {spot[0]:.1f} {spot[1]:.1f} {spot[2]:.1f}")
            _tick(self.probe)
            raw = next(row for row in _villagers(self.probe) if str(row.get("entity_uuid")) == sleeper)
            identity = {key: raw[key] for key in ("entity_id", "entity_uuid", "entity_type")}
            try:
                hit = self.probe.call("minecraft_attack_entity_once", {**identity, "timeout_seconds": self.probe.remaining()})
                break
            except Exception as error:
                tries.append(f"{spot}: {error}"[:300])
        self.detail["i_tries"] = tries
        woke, took, ok = self.wait(lambda r: sleeper in r and not r[sleeper]["sleep_pose"], 100)
        row = woke.get(sleeper, {})
        self.note("i_hit", woke, ok=ok, ticks=took, hit=hit)
        self.shot("i-hit")
        self.verdict("I1_sleeper_hit", bool(ok and row.get("job") == job and abs(row["pos"][1] - self.fx["bed_head"][1]) < 0.6),
                     "vanilla LivingEntity.hurtServer -> stopSleeping: woken at once, standing at floor level, job kept",
                     {"sleeper": sleeper[:8], "row": row, "ticks": took, "hit": hit})
        self.cmd("gamemode creative @s")
        self.back_to("hit")
        again, took2, ok2 = self.wait(lambda r: self.sleeper(r) is not None, 900)
        self.note("i_back_to_bed", again, ok=ok2, ticks=took2)
        self.verdict("I1b_back_to_bed_after_panic", ok2, "after the panic ends a job holder sleeps in the bed again the same night",
                     {"ticks": took2, "sleeper": (self.sleeper(again) or "")[:8]})
        rows2 = self.day("i_morning", lambda r: not any(x["sleep_pose"] for x in r.values()), 400)
        self.verdict("I2_jobs_next_morning", self.all_jobs(rows2) and not any(r["sleep_pose"] for r in rows2.values()),
                     "next morning (within 400 game ticks) every job held, nobody asleep", {"jobs": self.jobs(rows2),
                     "asleep": [u[:8] for u, r in rows2.items() if r["sleep_pose"]]})

    # ---- set H: repeated sleep/wake cycles with a real sleep each time, then a working day
    def set_h(self) -> None:
        rows = self.day("h_day")
        day_jobs = {uuid: row["job"] for uuid, row in rows.items() if uuid in self.workers}
        cycles = []
        for n in range(4):
            night, sleeper = self.night(f"h_night_{n}")
            self.cmd("time set 3000")
            _tick(self.probe)
            woke = self.rows()
            cycles.append({"n": n, "sleeper": sleeper and sleeper[:8], "jobs_night": self.jobs(night),
                           "after_one_wake_tick": {u[:8]: (r["word"], r["sleep_pose"]) for u, r in woke.items()}})
        self.detail["h_cycles"] = cycles
        self.settle("h_final_day")
        settled, took, ok = self.wait(lambda r: not any(x["sleep_pose"] for x in r.values()) and self.all_jobs(r), 300)
        self.note("h_settled", settled, ok=ok, ticks=took, bed=self.bed_props())
        same = all(settled.get(u, {}).get("job") == j for u, j in day_jobs.items())
        self.verdict("H1_four_sleep_wake_cycles", bool(ok and same and all(r["agree"] for r in settled.values())),
                     "after four quick sleep/wake cycles nobody is stuck asleep by day and everyone keeps its day job",
                     {"cycles": [c["sleeper"] for c in cycles], "day_jobs": {u[:8]: j for u, j in day_jobs.items()},
                      "now": self.jobs(settled), "ticks": took})
        sleepers = [s for s in {c["sleeper"] for c in cycles} if s]
        def working(r: dict[str, dict[str, Any]]) -> bool:
            for u, job in day_jobs.items():
                if u[:8] in sleepers and job == "farmer" and not _near(r.get(u, {"pos": [0, 0, 0]})["pos"], self.fx["stations"]["farmer"], 2.5):
                    return False
                if u[:8] in sleepers and job == "miner":
                    (x0, _, z0), (x1, y1, z1) = self.fx["mine"]
                    p = r.get(u, {"pos": [0, 0, 0]})["pos"]
                    if not (_near(p, self.fx["stations"]["miner"], 2.5) or (x0 <= p[0] < x1 + 1 and z0 <= p[2] < z1 + 1)):
                        return False
            return True
        work, took2, ok2 = self.wait(working, 2400)
        self.note("h_working", work, ok=ok2, ticks=took2)
        self.verdict("H2_sleepers_back_to_work", ok2,
                     "every farmer/miner that slept in a cycle is back at its station (miner: or in the mine); lumberjack not judged",
                     {"sleepers": sleepers, "ticks": took2,
                      "rows": {u[:8]: work.get(u) for u in day_jobs}})

    # ---- set J: same move as G, but by day while the farmer works (is it sleep-specific?)
    def set_j(self) -> None:
        stations = self.fx["stations"]
        rows = self.day("j_day")
        old = stations["farmer"]
        new = (old[0], old[1], old[2] + 2)
        holder = next((u for u, r in rows.items() if r["job"] == "farmer"), None)
        self.cmd(f"setblock {old[0]} {old[1]} {old[2]} minecraft:air")
        self.cmd(f"setblock {new[0]} {new[1]} {new[2]} minecraft:composter")
        lost, took, ok = self.wait(lambda r: holder in r and r[holder]["job"] != "farmer", 300)
        self.note("j_moved", lost, ok=ok, ticks=took, holder=holder and holder[:8])
        back, took2, ok2 = self.wait(lambda r: any(x["job"] == "farmer" for x in r.values()), 2000)
        self.note("j_after", back, ok=ok2, ticks=took2)
        self.verdict("J1_station_moved_by_day", ok2,
                     "vanilla: the jobless villager takes the moved composter within the work period",
                     {"holder": holder and holder[:8], "lost_after": took, "row": back.get(holder or ""),
                      "jobs": self.jobs(back), "ticks": took2})

    # ---- set K: who holds the outdoor bed? extra bound resident vs unbound librarian (F follow-up)
    def set_k(self) -> None:
        ox, oy, oz = self.fx["origin"]
        stage = self.fx["house_stage"]
        before_ids = set(self.rows())
        self.cmd(f"summon minecraft:villager {stage[0]} {stage[1]} {stage[2]}")
        self.settle("k_extra_summoned")
        extra = next((u for u in self.rows() if u not in before_ids), None)
        if extra:
            self.cmd(f"settlement bind {SETTLEMENT} {extra}")
            self.settle("k_extra_bound")
        lectern = (ox - 8, oy, oz + 2)
        lfoot, lhead = (ox - 10, oy, oz + 5), (ox - 9, oy, oz + 5)
        self.cmd(f"setblock {lectern[0]} {lectern[1]} {lectern[2]} minecraft:lectern")
        self.cmd(f"setblock {lfoot[0]} {lfoot[1]} {lfoot[2]} minecraft:white_bed[facing=east,part=foot,occupied=false]")
        self.cmd(f"setblock {lhead[0]} {lhead[1]} {lhead[2]} minecraft:white_bed[facing=east,part=head,occupied=false]")
        known = set(self.rows())
        self.cmd(f"summon minecraft:villager {ox - 8} {oy} {oz + 4}")
        self.settle("k_vanilla_summoned")
        van = next((u for u in self.rows() if u not in known), None)
        self.cmd("time set 3000")
        self.settle("k_day")
        self.wait(lambda r: van is not None and r.get(van, {}).get("word") == "библиотекарь", 1500)
        rows, _ = self.night("k_night", lambda r: van is None or r.get(van, {}).get("sleep_pose"), 600)
        from collections import Counter
        brains = {}
        labels = [("extra", extra), ("vanilla", van)] + [(f"other_{u[:8]}", u) for u in rows
                                                         if u not in (extra, van)]
        replies = lambda chat: Counter(line for line in chat if "entity data" in line)  # noqa: E731
        seen = replies(self.probe.observe().get("recent_chat", []))
        for label, uuid in labels:
            if uuid is None:
                continue
            self.probe.call("minecraft_send_chat", {"message": f"data get entity {uuid} Brain", "command": True})
            echo = _chat_echo(self.probe, f"k_brain_{label}")
            now = replies(echo.get("recent_chat", []))
            new = list((now - seen).elements())
            seen = now
            brains[label] = new[0] if len(new) == 1 else new
        self.detail["k_brains"] = brains
        bed_key = f"[I; {lhead[0]}, {lhead[1]}, {lhead[2]}]"
        holders = [label for label, brain in brains.items()
                   if isinstance(brain, str) and f'"minecraft:home": {{value: {{dimension: "minecraft:overworld", pos: {bed_key}' in brain]
        self.verdict("K1_outdoor_bed_owner", bool(van and rows.get(van, {}).get("sleep_pose")),
                     "the unbound librarian sleeps in the free outdoor bed beside its lectern",
                     {"vanilla": rows.get(van or ""), "extra": rows.get(extra or ""), "brains": brains,
                      "outdoor_bed_holders": holders, "bed": [list(lfoot), list(lhead)]})
        in_outdoor = [u[:8] for u, r in rows.items() if r["sleep_pose"] and _near(r["pos"], lhead, 0.75)]
        self.verdict("K1v_outdoor_bed_used_by_its_holder", bool(holders) and bool(in_outdoor),
                     "vanilla: whoever claims the outdoor bed sleeps in it at night (no claimed-but-empty bed)",
                     {"holders": holders, "asleep_in_outdoor_bed": in_outdoor})
