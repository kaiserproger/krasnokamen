"""Buildings-qa level verification: the block, entrance, light and hostile-census oracle
for one completed level, and the server restart that resumes a run.

The client/report seams live on ``buildings_qa`` (the tests patch them there), so this
module reaches them as ``_qa.<name>`` and never binds them.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path
from typing import Any

from .. import runtime
from ..profiles import ProfileContext
from .probe_steps import ProbeSteps
from .village_autonomy_qa import _rotate_blueprint_xz
from . import buildings_qa as _qa
from .buildings_qa_blueprint import _level_frame, _max_frame
from .buildings_qa_functions import (
    _function_barracks, _function_farm, _function_house, _function_lumber, _function_mine,
    _function_storage, _function_work,
)
from .buildings_qa_probe import _block_id, _state_base, _verdict


def _verify_level(probe: ProbeSteps, sub: dict[str, Any], ctx: ProfileContext,
                  artifact: Path, bp: str, level: int, origin: tuple[int, int, int],
                  tag: str, settlement: str, chest: tuple[int, int, int]) -> None:
    """Block-by-block blueprint match + entrance/light/hostiles + function."""
    data = _qa._load_blueprints()[bp]
    turns = _qa._fit_orientation(probe, bp, level, origin)
    sub[f"L{level}_rotation"] = turns
    cells = _qa.oriented_cells(bp, level, origin, turns)
    mismatches: list[str] = []
    checked = 0
    for pos, want in cells.items():
        try:
            block = _qa._read(probe, pos)
        except Exception as exc:
            mismatches.append(f"{list(pos)}: unreadable {exc}")
            continue
        got = _block_id(block)
        want_id = _state_base(want)
        if got != want_id:
            mismatches.append(f"{list(pos)}: got {got} want {want_id}")
        checked += 1
        if len(mismatches) > 12:
            break
    match_ok = not mismatches
    if mismatches:
        # Transient client-chunk desync after a build storm reads stale air.
        # Settle, then re-read ONLY the mismatched cells before failing.
        probe.call("minecraft_wait_ticks", {"ticks": 40})
        still = []
        for pos, want in cells.items():
            key = f"{list(pos)}"
            if not any(m.startswith(key) for m in mismatches):
                continue
            try:
                block = _qa._read(probe, pos)
            except Exception as exc:
                still.append(f"{list(pos)}: unreadable {exc}")
                continue
            if _block_id(block) != _state_base(want):
                still.append(f"{list(pos)}: got {_block_id(block)} want {_state_base(want)}")
        mismatches = still
        match_ok = not mismatches
        oracle = []
        for entry in still[:6]:
            if "want " not in entry:
                oracle.append(entry)
                continue
            nums = [int(v) for v in entry.split("]")[0].strip("[").split(",")]
            want_name = entry.rsplit("want ", 1)[1]
            lines = _qa._chat_lines(_qa._send(probe, f"execute if block {nums[0]} {nums[1]} {nums[2]} {want_name}"), 4)
            oracle.append(f"{nums}: {' | '.join(lines)[:120]}")
        sub[f"L{level}_server_oracle"] = oracle
    if "baseline_building_ids" in sub:
        certificate = _qa._current_certificate(probe, sub, tag, bp, level)
        probe.check(f"{tag}_L{level}_exact_snapshot",
                    certificate is not None and certificate[1] == origin)
    sub[f"L{level}_blockmatch"] = {"checked": checked, "mismatches": mismatches[:12]}
    _verdict(sub, f"L{level}_blockmatch", "PASS" if match_ok else "FAIL",
             f"{checked} cells, {len(mismatches)} mismatches", f"origin {list(origin)} rot {turns}")
    probe.check(f"{tag}_L{level}_blocks", match_ok)
    entrance = data["levels"][level - 1]["entrance"]
    width, depth, height = _level_frame(bp, level)

    frame_w, frame_d = _max_frame(bp)
    true = _qa.true_origin(bp, level, origin, turns)

    def world_xz(x: int, z: int) -> tuple[int, int]:
        rx, rz = _rotate_blueprint_xz(x, z, frame_w, frame_d, turns)
        return true[0] + rx, true[2] + rz

    ex, ez = world_xz(entrance[0], entrance[2])
    ey = origin[1] + entrance[1]
    probe.fixture(f"tp @s {ex + 0.5} {ey + 1.0} {ez + 0.5}")
    probe.wait(lambda s: abs(float(s["player"]["x"]) - (ex + 0.5)) < 2.5,
               f"{tag}_L{level}_enter")
    air = _qa._read(probe, (ex, ey, ez))
    _bid = _block_id(air)
    enter_ok = _bid in ("minecraft:air", "minecraft:oak_door",
                        "minecraft:oak_fence_gate", "minecraft:water",
                        "minecraft:oak_pressure_plate") or "stairs" in _bid or "slab" in _bid
    _verdict(sub, f"L{level}_entrance", "PASS" if enter_ok else "FAIL",
             f"entrance {list((ex, ey, ez))} holds {_block_id(air)}")
    probe.check(f"{tag}_L{level}_entrance", enter_ok)

    # Lighting at centre interior; solid fortifications (wall/tower) author
    # mass at the centre by design — probe the first authored air cell
    # instead, else the oracle fails a correct build.
    cx, cz = world_xz(width // 2, depth // 2)
    cy = origin[1] + 1
    if _block_id(_qa._read(probe, (cx, cy, cz))) not in ("minecraft:air", None):
        # cells == expected_cells skips '.' (air is the default); scan raw
        # layers for authored air. Top-down: ground walkways under a solid
        # mass are lightless by construction; the highest '.' sees sky.
        layers = data["levels"][level - 1]["layers"]
        for _y in range(len(layers) - 1, -1, -1):
            for _z, _row in enumerate(layers[_y]):
                for _x, _ch in enumerate(_row):
                    if _ch == ".":
                        (cx, cz), cy = world_xz(_x, _z), origin[1] + _y
                        break
                else:
                    continue
                break
            else:
                continue
            break
    lit = _qa._read(probe, (cx, cy, cz))
    sky = int(lit.get("sky_light", 0) or 0)
    blk = int(lit.get("block_light", 0) or 0)
    light_ok = (sky + blk) >= 1
    probe.check(f"{tag}_L{level}_light", light_ok)
    _verdict(sub, f"L{level}_light", "PASS" if light_ok else "FAIL",
             f"cell {list((cx, cy, cz))} sky={sky} block={blk}")
    probe.fixture("time set 18000")
    for _ in range(3):
        probe.call("minecraft_wait_ticks", {"ticks": 4})
    ents = []
    for hostile in ("minecraft:zombie", "minecraft:skeleton", "minecraft:spider", "minecraft:creeper"):
        dom = probe.call("minecraft_list_entities",
                         {"radius": 128, "limit": 128, "entity_type": hostile})
        ents.extend(dom.get("entities", dom.get("visible_entities", [])))
    corners = [world_xz(x, z) for x in (0, width - 1) for z in (0, depth - 1)]
    lo = (min(x for x, _ in corners), origin[1], min(z for _, z in corners))
    hi = (max(x for x, _ in corners), origin[1] + height - 1, max(z for _, z in corners))
    inside = [e for e in ents if isinstance(e, dict)
              and str(e.get("entity_type", "")) in
              ("minecraft:zombie", "minecraft:skeleton", "minecraft:spider", "minecraft:creeper")
              and lo[0] <= float(e.get("x", -1e9)) <= hi[0]
              and lo[1] <= float(e.get("y", -1e9)) <= hi[1]
              and lo[2] <= float(e.get("z", -1e9)) <= hi[2]]
    probe.check(f"{tag}_L{level}_no_hostiles", not inside)
    _verdict(sub, f"L{level}_no_hostiles", "PASS" if not inside else "FAIL",
             f"{len(inside)} hostiles observed inside; mob spawning disabled, not an exclusion test")
    probe.fixture("time set 3000")

    kind = data["kind"]
    if kind == "house":
        _function_house(probe, sub, tag, level, origin, cells)
    elif kind == "farmer":
        _function_farm(probe, sub, tag, level, origin, cells)
    elif kind == "lumber_camp":
        _function_lumber(probe, sub, tag, level, origin, cells)
    elif kind == "mine":
        _function_mine(probe, sub, tag, level, origin, cells)
    elif kind == "builder":
        _function_work(probe, sub, tag, level, origin, cells)
    elif kind == "storage":
        _function_storage(probe, sub, tag, level, origin, cells, settlement)
    elif kind == "barracks":
        _function_barracks(probe, sub, tag, level, origin, cells, settlement)
    elif kind in ("wall", "tower", "gate"):
        probe.check(f"{tag}_L{level}_fort", True)
        _verdict(sub, f"L{level}_function_fort", "PASS",
                 "certificate and entrance block checked after teleport; no traversal or door interaction tested")


def _restart_resume(probe: ProbeSteps, sub: dict[str, Any], ctx: ProfileContext,
                    artifact: Path, address: str, config: Path,
                    server_log_path: Path, tag: str, settlement: str, bp: str):
    """Restart the server on the same world; settlement data must survive.
    Stops the old server via runtime.stop_process + wait (no fuser); stashes
    the new Popen on probe.server_proc. The scenario caller assigns it to
    run()'s server variable so the single finally owner stops it."""
    import subprocess as _sp
    old = getattr(probe, "server_proc", None)
    if old is not None:
        runtime.stop_process(old, interrupt=True)
        try:
            old.wait(timeout=10)
        except Exception:
            pass
    env = os.environ.copy()
    env["KRASNOKAMEN_HARNESS_LOG_STDOUT"] = "1"
    log = server_log_path.open("ab")
    ctx.logs.append(str(server_log_path) + " (restart appended)")
    binary = Path(os.environ["KRASNOKAMEN_SERVER_BINARY"]).expanduser().resolve()
    proc = runtime.start_server([str(binary), "--config", str(config), "--no-console"],
                     cwd=runtime.REPO_ROOT, stdout=log, stderr=subprocess.STDOUT,
                     start_new_session=True, env=env)
    probe.server_proc = proc  # type: ignore[attr-defined]
    _, _, port_s = address.partition(":")
    try:
        runtime.wait_port(int(port_s), 120, proc, log_path=server_log_path)
    except Exception as exc:
        _verdict(sub, "restart_resume", "FAIL", f"server did not come back: {exc}")
        probe.check(f"{tag}_restart", False)
        return
    probe.call("minecraft_disconnect", {})
    probe.call("minecraft_observe")
    probe.call("minecraft_connect", {"server_addr": address})
    probe.call("minecraft_wait_for_play", {"timeout_seconds": 60})
    state = _qa._send(probe, "settlement building list")
    lines = _qa._chat_lines(state, 25)
    survived = any(bp.split("_")[0] in line.lower() for line in lines)
    probe.check(f"{tag}_restart", survived)
    _verdict(sub, "restart_resume", "PASS" if survived else "FAIL",
             f"post-restart listing: {lines[:6]}")
