"""Buildings-qa blueprint geometry: the authored levels, the frame the server rotates
them in, the cells they occupy and the quarter-turn fit read from the world.

The client/report seams live on ``buildings_qa`` (the tests patch them there), so this
module reaches them as ``_qa.<name>`` and never binds them.
"""

from __future__ import annotations

import json
from typing import Any

from .. import runtime
from .probe_steps import ProbeSteps
from .village_autonomy_qa import _rotate_blueprint_xz
from . import buildings_qa as _qa
from .buildings_qa_probe import _state_base


BLUEPRINT_DATA: dict[str, dict[str, Any]] = {}


def _load_blueprints() -> dict[str, dict[str, Any]]:
    if BLUEPRINT_DATA:
        return BLUEPRINT_DATA
    root = runtime.REPO_ROOT / "crates" / "mc-core" / "assets" / "blueprints"
    for path in sorted(root.glob("*.json")):
        raw = json.loads(path.read_text())
        BLUEPRINT_DATA[raw["id"]] = {
            "kind": raw["kind"],
            "palette": raw["palette"],
            "levels": [
                {"cert": lvl["cert"], "entrance": lvl["entrance"],
                 "layers": lvl["layers"]}
                for lvl in raw["levels"]
            ],
        }
    return BLUEPRINT_DATA


def _level_frame(bp: str, level: int) -> tuple[int, int, int]:
    """(width x, depth z, height y) of one blueprint level's layers."""
    layers = _qa._load_blueprints()[bp]["levels"][level - 1]["layers"]
    return len(layers[0][0]), len(layers[0]), len(layers)


def _max_frame(bp: str) -> tuple[int, int]:
    """The (x, z) extent of the largest level: the frame the server rotates every level in."""
    levels = _qa._load_blueprints()[bp]["levels"]
    return (max(len(l["layers"][0][0]) for l in levels), max(len(l["layers"][0]) for l in levels))


def _cert_offset(bp: str, level: int, turns: int) -> tuple[int, int]:
    """Where the level's certificate minimum sits from the placement origin after `turns`."""
    width, depth = _max_frame(bp)
    cx, _, cz = _qa._load_blueprints()[bp]["levels"][level - 1]["cert"]
    corners = [_rotate_blueprint_xz(x, z, width, depth, turns)
               for x in (0, cx - 1) for z in (0, cz - 1)]
    return min(c[0] for c in corners), min(c[1] for c in corners)


def cells_true(bp: str, level: int, true: tuple[int, int, int],
               turns: int) -> dict[tuple[int, int, int], str]:
    """The level's authored cells as the server places them: every level rotates in the
    maximum frame about the placement origin `true`."""
    width, depth = _max_frame(bp)
    data = _qa._load_blueprints()[bp]
    out: dict[tuple[int, int, int], str] = {}
    for y, layer in enumerate(data["levels"][level - 1]["layers"]):
        for z, row in enumerate(layer):
            for x, ch in enumerate(row):
                if ch != ".":
                    rx, rz = _rotate_blueprint_xz(x, z, width, depth, turns)
                    out[(true[0] + rx, true[1] + y, true[2] + rz)] = data["palette"][ch]
    return out


def true_origin(bp: str, level: int, origin: tuple[int, int, int],
                turns: int) -> tuple[int, int, int]:
    """Placement origin from a level's certificate minimum (the listed bounds)."""
    dx, dz = _cert_offset(bp, level, turns)
    return origin[0] - dx, origin[1], origin[2] - dz


def oriented_cells(bp: str, level: int, origin: tuple[int, int, int],
                   turns: int) -> dict[tuple[int, int, int], str]:
    return _qa.cells_true(bp, level, _qa.true_origin(bp, level, origin, turns), turns)


def best_turns(cells_by_turns: dict[int, dict[tuple[int, int, int], str]],
               world: dict[tuple[int, int, int], str]) -> int:
    """The quarter-turn count whose authored cells agree with the observed block ids the most
    (the lowest count wins a tie). The house is rotated by the layout planner's door side, so the
    gate reads the orientation from the world instead of assuming rot 0."""
    def mismatches(turns: int) -> int:
        return sum(1 for pos, want in cells_by_turns[turns].items()
                   if world.get(pos) != _state_base(want))
    return min(range(4), key=lambda turns: (mismatches(turns), turns))


def _scan_box(probe: ProbeSteps, lo: tuple[int, int, int],
              hi: tuple[int, int, int]) -> dict[tuple[int, int, int], str]:
    """Every block id in the inclusive box, split spatially when a response is truncated."""
    blocks: dict[tuple[int, int, int], str] = {}

    def scan(low: list[int], high: list[int]) -> None:
        arguments = {f"{edge}_{axis}": position[index]
                     for edge, position in (("min", low), ("max", high))
                     for index, axis in enumerate(("x", "y", "z"))}
        page = probe.call("minecraft_scan_blocks", {**arguments, "max_blocks": 4096})
        if page.get("next_offset") is None:
            blocks.update(((row["x"], row["y"], row["z"]), row["block_id"]) for row in page["blocks"])
            return
        axis = max(range(3), key=lambda a: high[a] - low[a])
        if low[axis] == high[axis]:
            raise RuntimeError("buildings-qa scan truncated a single cell")
        middle = (low[axis] + high[axis]) // 2
        lower, upper = high.copy(), low.copy()
        lower[axis], upper[axis] = middle, middle + 1
        scan(low, lower)
        scan(upper, high)

    scan(list(lo), list(hi))
    return blocks


def _fit_orientation(probe: ProbeSteps, bp: str, level: int,
                     origin: tuple[int, int, int]) -> int:
    candidates = {turns: _qa.oriented_cells(bp, level, origin, turns) for turns in range(4)}
    positions = [pos for cells in candidates.values() for pos in cells]
    lo = tuple(min(p[i] for p in positions) for i in range(3))
    hi = tuple(max(p[i] for p in positions) for i in range(3))
    world = _scan_box(probe, lo, hi)
    return best_turns(candidates, world)
