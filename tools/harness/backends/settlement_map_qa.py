"""Settlement map dialog on a real client: native, or vanilla Java (`--client java`).

Reuses the scene-shot lifecycle (isolated seed-100 world, operator player, 1280x720
window at GUI scale 3) with a scripted body and judges the client's own screenshots.
Map cells are coloured `█` text glyphs (palette in
`crates/mc-core/src/settlements/ui_map.rs`), so palette pixel counts show what the
map draws on either client. Every check compares two shots of the same scene, so
world pixels behind the dialog cancel out:

1. found `mapqa` (radius 16) on a cleared stone pad and open the main page: baseline;
2. the main page's Map button opens the map of the empty town: territory green and
   the viewer's magenta appear;
3. with the map still open, build a real Storage (stone floor + chest) and certify
   it with `/settlement building certify`: the open map is a snapshot, so no storage
   brown yet (stale shot, negative control);
4. the map's Refresh button redraws it: the storage brown appears on its cells;
5. Back opens the settlement main page;
6. native only: Map again, then the real pointer path (`client_pointer`, window pixels
   from `ui_nodes`): hovering the plain cell west of the storage shows the territory
   tooltip, hovering a storage cell (a `link` run) shows `Storage #<seq> ...` (shot
   `map-hover-storage`), and clicking it sends the cell's `custom` click; the server
   answers with the Buildings page (or the building's own `#<seq> ...` page).

The Loader MCP presses only buttons that are direct screen children, and vanilla
keeps dialog grid buttons inside the body's scroll container, so on `--client java`
steps 2, 4 and 5 send the equivalent `/settlement menu <page>` command (same server
page) and the run is a rendering check; native presses the real buttons. The Loader
MCP has no pointer tool, so step 6 does not run on Java; `ui_map_tests.rs` covers the
wire payloads of hovers and clicks.
"""
from __future__ import annotations

import math
from pathlib import Path
from typing import Any, Callable

from PIL import Image

from ..profiles import ProfileContext
from . import scene_shot
from .probe_steps import ProbeSteps

DIALOG = "net.minecraft.client.gui.screens.dialog.MultiButtonDialogScreen"
NAME = "mapqa"
RADIUS = 16
PALETTE = {
    "territory": (0x4C, 0x7A, 0x34),
    "you": (0xFF, 0x55, 0xFF),
    "storage": (0x9C, 0x6B, 0x30),
}
TOLERANCE = 12
# Smallest plausible cell: one 8x8 glyph at GUI scale 1. The real cell size is
# measured from the viewer marker, which is exactly one cell (`_largest_patch`).
MIN_CELL = 8 * 8


def run(ctx: ProfileContext) -> dict[str, Any]:
    return scene_shot.run(ctx, scenario={"steps": []}, script=_script)


def _near(pixel: tuple[int, ...], color: tuple[int, int, int]) -> bool:
    return all(abs(a - b) <= TOLERANCE for a, b in zip(pixel, color))


def _pixels(path: str) -> dict[str, int]:
    counts = dict.fromkeys(PALETTE, 0)
    with Image.open(path) as image:
        for pixel in image.convert("RGB").getdata():
            for name, color in PALETTE.items():
                if _near(pixel, color):
                    counts[name] += 1
    return counts


def _largest_patch(path: str, color: tuple[int, int, int], baseline: str) -> int:
    """Pixel area of the largest 4-connected patch of `color` that the baseline shot does not
    have at the same pixels. Every map glyph is its own patch, so for the viewer colour this is
    one cell: the "You" legend swatch is a second patch, not added to it."""
    with Image.open(path) as image, Image.open(baseline) as before:
        if image.size != before.size:
            raise ValueError(f"{path} is {image.size}, baseline {baseline} is {before.size}")
        width = image.width
        pixels = image.convert("RGB").getdata()
        old = before.convert("RGB").getdata()
        left = {index for index, (pixel, prior) in enumerate(zip(pixels, old))
                if _near(pixel, color) and not _near(prior, color)}
    largest = 0
    while left:
        stack = [left.pop()]
        area = 0
        while stack:
            index = stack.pop()
            area += 1
            column = index % width
            for neighbour in (index - width, index + width,
                              index - 1 if column > 0 else -1,
                              index + 1 if column + 1 < width else -1):
                if neighbour in left:
                    left.remove(neighbour)
                    stack.append(neighbour)
        largest = max(largest, area)
    return largest


def _command(probe: ProbeSteps, command: str, expect: str) -> None:
    probe.fixture(command)
    probe.wait(lambda state: any(expect in str(line) for line in state.get("recent_chat", [])),
               f"chat {expect}", timeout_seconds=30)


def _dialog(probe: ProbeSteps, title: str) -> None:
    probe.wait(lambda state: bool((state.get("screen") or {}).get("open"))
               and state["screen"].get("class") == DIALOG
               and str(state["screen"].get("title", "")) == title,
               f"dialog {title}", timeout_seconds=30)


def _go(probe: ProbeSteps, client: str, title: str, label: str, page: str, next_title: str) -> None:
    """Press `label` on the open dialog (native), or send the command that opens
    the same server page (Java, see the module note)."""
    if client == "java":
        probe.fixture(f"settlement menu {page}")
    else:
        probe.call("minecraft_click_screen_button", {
            "expected_screen_class": DIALOG, "expected_title": title, "button_label": label,
        })
    # A click closes the dialog on the client; the server answers with the next one.
    probe.call("minecraft_wait_ticks", {"ticks": 20})
    _dialog(probe, next_title)


def _opened_building(state: dict[str, Any], seq: str) -> bool:
    """The server answered a building cell click: the Buildings page, or the per-building
    page (title `#<seq> <kind> L<level>`) when that page handles `b`."""
    screen = state.get("screen") or {}
    title = str(screen.get("title", ""))
    return bool(screen.get("open")) and (title == f"Buildings - {NAME}" or title.startswith(f"#{seq} "))


def _cells(probe: ProbeSteps, map_title: str, shot: Callable[[str], Any]) -> None:
    """Native step 6: hover a territory cell and a storage cell, then click the storage."""
    nodes = probe.observe().get("ui_nodes", [])
    links = [node for node in nodes if node.get("role") == "link"
             and str(node.get("id", "")).startswith("dialog/body/0/action/")]
    probe.report["map_links"] = len(links)
    probe.check("building_cells_clickable", bool(links))
    left, top, width, height = links[0]["bounds"]
    storage = (left + width / 2, top + height / 2)

    def pointer(action: str, at: tuple[float, float]) -> str:
        result = probe.call("client_pointer", {
            "expected_screen_class": DIALOG, "expected_title": map_title,
            "action": action, "x": at[0], "y": at[1],
        })
        return str(result.get("tooltip") or "")

    # Cells are square, so a run's height is one cell: step one cell west of the storage.
    territory = pointer("move", (left - height / 2, top + height / 2))
    probe.report["territory_tooltip"] = territory
    probe.check("territory_cell_tooltip", territory == f"Territory, radius {RADIUS}")
    tooltip = pointer("move", storage)
    probe.report["storage_tooltip"] = tooltip
    probe.check("storage_cell_tooltip", tooltip.startswith("Storage #"))
    shot("map-hover-storage")
    seq = tooltip.split("#", 1)[1].split()[0]
    pointer("click", storage)
    probe.call("minecraft_wait_ticks", {"ticks": 20})
    probe.wait(lambda state: _opened_building(state, seq), "storage_cell_click_opens_building",
               timeout_seconds=30)
    shot("map-cell-click")


def _script(probe: ProbeSteps, artifact: Path, _steps: list[dict[str, Any]], client: str) -> list[str]:
    capture = scene_shot.capture_java if client == "java" else scene_shot.capture_native
    shots: list[str] = []
    counts: dict[str, dict[str, int]] = probe.report.setdefault("map_pixels", {})

    def shot(name: str) -> dict[str, int]:
        probe.call("minecraft_wait_ticks", {"ticks": 10})
        path = capture(probe, artifact, name)
        shots.append(path)
        counts[name] = _pixels(path)
        return counts[name]

    for command in ("gamerule advance_time false", "gamerule spawn_mobs false",
                    "time set 6000", "weather clear"):
        probe.fixture(command)
    me = probe.observe(wait_for_play=True)["player"]
    x, y, z = (math.floor(float(me[axis])) for axis in ("x", "y", "z"))
    probe.report["anchor"] = [x, y, z]
    probe.fixture(f"fill {x - 4} {y - 1} {z - 4} {x + 8} {y - 1} {z + 8} minecraft:stone")
    probe.fixture(f"fill {x - 4} {y} {z - 4} {x + 8} {y + 5} {z + 8} minecraft:air")
    _command(probe, f"settlement create {NAME} {RADIUS}", f"Founded {NAME}")
    main_title, map_title = f"Settlement {NAME}", f"Map - {NAME}"

    probe.fixture("settlement menu main")
    _dialog(probe, main_title)
    baseline = shot("main-baseline")
    baseline_shot = shots[-1]
    _go(probe, client, main_title, "Map", "map", map_title)
    empty = shot("map-empty")
    # One glyph's area calibrates the rest: the largest new magenta patch is the viewer's cell.
    # The whole-screen magenta delta would also count the "You" legend swatch (two glyphs).
    cell = _largest_patch(shots[-1], PALETTE["you"], baseline_shot)
    probe.report["cell_pixels"] = cell
    probe.report["you_pixels_delta"] = empty["you"] - baseline["you"]
    probe.check("viewer_drawn", cell >= MIN_CELL)
    # Radius 16 at 2 blocks per cell: ~200 territory cells.
    probe.check("territory_drawn", empty["territory"] - baseline["territory"] >= 50 * cell)

    # Storage L1: solid 3x3 floor (the pad), 4 blocks high, one chest.
    probe.fixture(f"setblock {x + 5} {y} {z + 5} minecraft:chest")
    _command(probe, f"settlement building certify storage {x + 4} {y - 1} {z + 4} {x + 6} {y + 2} {z + 6}",
             "Certified storage #")
    stale = shot("map-stale")
    probe.check("open_map_is_a_snapshot", stale["storage"] - empty["storage"] < cell // 4)
    _go(probe, client, map_title, "Refresh", "map", map_title)
    refreshed = shot("map-refreshed")
    # The 3x3 storage spans 2x2 cells of 2x2 blocks.
    probe.check("storage_drawn_after_refresh", refreshed["storage"] - stale["storage"] >= 2 * cell)
    probe.check("territory_kept_after_refresh", refreshed["territory"] - baseline["territory"] >= 50 * cell)

    _go(probe, client, map_title, "Back", "main", main_title)
    if client != "java":
        _go(probe, client, main_title, "Map", "map", map_title)
        _cells(probe, map_title, shot)
    return shots
