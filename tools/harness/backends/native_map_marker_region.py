"""The held map draws the marker the server decorated: a stability-guarded region predicate.

One map id, one terrain, two captures. A locked map is held and captured; a second copy of *some*
map is inserted by hand into an already-placed item frame, its decoration arrives on the wire, and
the held copy is captured again. A locked map samples no terrain, so the received colour array must
be identical between the frames and the only drawn change can be the frame's marker icon: a client
that receives the decoration and never draws it produces the same frame twice.

Two guards make the pixel comparison noise-proof, both measured on the first receipts
(2026-11-25, 1280x720):

* **A stable pair per side.** The held item's pose drifts a few pixels between frames (hand bob and
  frame pacing): three of the first six runs showed a 679-136500 px change with nothing drawn, one of
  them covering the whole window. So each capture is taken as the second of two frames of an
  unchanged scene whose changed-pixel count inside the window is below `DRIFT_MAX`; a scene that does
  not settle is retried and, if it never settles, reported inconclusive instead of red.
* **One compact blob.** The marker icon was 383-410 px in a 22x28 box on the fixed client, while the
  largest pre-fix blob was 19 px of one-pixel hairlines along the map edge. The blob must also be
  filled (area / bounding box >= `MARKER_MIN_FILL`), which a shifted map edge is not.

The decoration wait is on the marker itself (`map markers include one frame decoration`), because a
`map_item_data` packet carries colours and markers in separate sections: counting packets says
nothing about the decoration having arrived.
"""
from __future__ import annotations

from pathlib import Path
from typing import Any

from .native_promotion_maps import (
    _create,
    _echo,
    _give,
    _held_shot,
    _locked,
    _map,
    _marker_count,
    _scene,
    _set,
    _snapshot,
    _updates,
)
from .native_v2 import _use
from .probe_steps import ProbeSteps


# The 1280x720 frame's window that holds the drawn held map around its centre while never
# holding the chat overlay (it reaches x~=560), the top-right message box (y<160) or the
# held-item name tooltip (y>560).
HELD_MAP_REGION = (580, 170, 930, 560)
# Changed pixels allowed inside the window between two frames of an unchanged scene.
DRIFT_MAX = 40
# Measured: the marker icon covers 383-410 px in a 22x28 box; the largest pre-fix blob is 19 px.
MARKER_MIN_AREA = 120
MARKER_MIN_SPAN = 8
MARKER_MIN_FILL = 0.4
# Vanilla fades a chat line 200 ticks after it arrives: two 255-tick waits with no new line leave
# the overlay visually empty for both captures (`recent_chat` keeps the ring buffer, so waiting for
# it to empty can never fire).
CHAT_FADE_TICKS = 255
# How many settle attempts a capture may take before the run reports inconclusive.
SETTLE_ATTEMPTS = 8


def _shot_path(steps: ProbeSteps, name: str) -> Path:
    return steps.artifact_dir.parent / "screenshots" / f"{name}.png"


def _changed(first: Path, second: Path, region: tuple[int, int, int, int],
             tolerance: int = 24) -> set[tuple[int, int]]:
    from PIL import Image

    with Image.open(first) as one, Image.open(second) as other:
        before = one.convert("RGB").crop(region)
        after = other.convert("RGB").crop(region)
    return {(index % before.width, index // before.width)
            for index, (a, b) in enumerate(zip(before.getdata(), after.getdata()))
            if max(abs(u - v) for u, v in zip(a, b)) > tolerance}


def _largest_blob(changed: set[tuple[int, int]]) -> dict[str, int]:
    seen: set[tuple[int, int]] = set()
    best: list[tuple[int, int]] = []
    for start in changed:
        if start in seen:
            continue
        stack, blob = [start], []
        seen.add(start)
        while stack:
            x, y = stack.pop()
            blob.append((x, y))
            for dx in (-1, 0, 1):
                for dy in (-1, 0, 1):
                    point = (x + dx, y + dy)
                    if point in changed and point not in seen:
                        seen.add(point)
                        stack.append(point)
        if len(blob) > len(best):
            best = blob
    if not best:
        return {"area": 0, "x0": 0, "y0": 0, "width": 0, "height": 0, "fill": 0.0}
    xs = [point[0] for point in best]
    ys = [point[1] for point in best]
    width, height = max(xs) - min(xs) + 1, max(ys) - min(ys) + 1
    return {"area": len(best), "x0": min(xs), "y0": min(ys), "width": width, "height": height,
            "fill": round(len(best) / (width * height), 3)}


def _player_markers(steps: ProbeSteps, map_id: int) -> list[tuple[int, int, int]]:
    """The player decoration as the server sent it: one read per capture, so no frame vouches for itself."""
    snapshot = _snapshot(steps)
    state = snapshot["maps"].get(map_id) or {}
    kind = snapshot["marker_ids"]["minecraft:player"]
    return sorted((marker["x"], marker["y"], marker["rotation"])
                  for marker in state.get("markers", []) if marker["kind"] == kind)


def _newest_chat(steps: ProbeSteps) -> str:
    lines = [line for line in steps.observe().get("recent_chat", []) if isinstance(line, str)]
    return lines[0] if lines else ""


def _stable_capture(steps: ProbeSteps, label: str) -> Path:
    """Two frames of an unchanged scene; returns the later one when their drift is in budget."""
    drift: list[int] = []
    for attempt in range(SETTLE_ATTEMPTS):
        _held_shot(steps, f"{label}-{attempt}a")
        _held_shot(steps, f"{label}-{attempt}b")
        first = _shot_path(steps, f"{label}-{attempt}a")
        second = _shot_path(steps, f"{label}-{attempt}b")
        changed = len(_changed(first, second, HELD_MAP_REGION))
        drift.append(changed)
        if changed <= DRIFT_MAX:
            steps.report.setdefault("held_map_drift", {})[label] = drift
            _held_shot(steps, label)
            return _shot_path(steps, label)
        steps.call("minecraft_wait_ticks", {"ticks": 20})
    steps.report.setdefault("held_map_drift", {})[label] = drift
    raise RuntimeError(f"inconclusive: the held map did not settle in {SETTLE_ATTEMPTS} attempts "
                       f"(drift {drift}, budget {DRIFT_MAX} px)")


def _frame_place(steps: ProbeSteps, origin: tuple[int, int, int], slot: int) -> dict[str, Any]:
    """A placed, empty item frame; nothing is inserted yet, so both captures share it."""
    x, y, z = origin
    support = {"x": x, "y": y + 1, "z": z + 3}
    _set(steps, support, "minecraft:stone")
    _give(steps, "minecraft:item_frame", slot)
    steps.call("minecraft_set_hotbar_slot", {"slot": slot})
    _use(steps, support, "north")
    frame = steps.call("minecraft_wait_for_visible_entity", {
        "entity_type": "minecraft:item_frame", "radius": 8,
        "timeout_seconds": steps.remaining()})
    return {key: frame[key] for key in ("entity_id", "entity_uuid", "entity_type")}


def _clone_quiet(steps: ProbeSteps, map_id: int, slot: int) -> None:
    """`_clone` without its ordered chat line: a give plus a state wait, so no line scrolls the chat
    text drawn over the map between the two captures."""
    steps.fixture(f"debug give minecraft:filled_map[map_id={map_id}] 1 {slot}")
    steps.wait(lambda state: any(stack["slot"] == slot and stack.get("item_id") == "minecraft:filled_map"
                                for stack in state["inventory"]), "map_clone_quiet_received")
    for _attempt in range(20):
        stack = steps.call("minecraft_read_item_components", {"source": "player", "slot": slot})
        if (stack.get("item_id") == "minecraft:filled_map"
                and stack.get("components", {}).get("minecraft:map_id") == map_id):
            steps.check("map_clone_quiet_same_id", True)
            return
        steps.call("minecraft_wait_ticks", {"ticks": 2})
    steps.check("map_clone_quiet_same_id", False)


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    if not getattr(steps.client, "p4_window", None):
        raise RuntimeError("map marker probe requires a harness-owned windowed native viewer")
    origin = _scene(steps, origin, False)
    map_id = _create(steps)
    frame = _frame_place(steps, origin, slot=8)
    locked = _locked(steps, origin, map_id)
    _clone_quiet(steps, locked, slot=1)
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    steps.check("map_unframed_map_has_no_frame_waypoint", _marker_count(steps, locked, "frame") == 0)
    steps.check("map_capture_without_frame_holds_the_map",
                steps.call("minecraft_read_item_components", {"source": "selected"}).get("item_id")
                == "minecraft:filled_map")
    colours_before = _map(steps, locked)["colors"][:]
    for _fade in range(2):
        steps.call("minecraft_wait_ticks", {"ticks": CHAT_FADE_TICKS})
    chat_before = _newest_chat(steps)
    player_before = _player_markers(steps, locked)
    updates_before = _updates(steps, locked)
    without = _stable_capture(steps, "map-held-without-frame-waypoint")
    # Insert the second copy by hand: no command runs between the captures, so the chat cannot
    # scroll and no ordered line is added.
    steps.call("minecraft_set_hotbar_slot", {"slot": 1})
    steps.call("minecraft_interact_entity", {**frame, "hand": "main_hand"})
    steps.wait(lambda state: not any(stack["slot"] == 1 and stack.get("item_id") == "minecraft:filled_map"
                                     for stack in state["inventory"]),
               "map_ordinary_frame_insertion_debited")
    # Decoration-driven: the marker exists only once a decoration carrying map packet arrived, so the
    # wait is on the marker itself, not on a packet counter (one packet carries colours and markers
    # in separate sections).
    steps.wait(lambda _state: _marker_count(steps, locked, "frame") == 1,
               "map_frame_decoration_revision_arrived", timeout_seconds=20)
    steps.check("map_framed_map_has_exactly_one_frame_waypoint",
                _marker_count(steps, locked, "frame") == 1)
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    steps.check("map_capture_with_frame_holds_the_map",
                steps.call("minecraft_read_item_components", {"source": "selected"}).get("item_id")
                == "minecraft:filled_map")
    with_frame = _stable_capture(steps, "map-held-with-frame-waypoint")
    chat_after = _newest_chat(steps)
    player_after = _player_markers(steps, locked)
    colours_after = _map(steps, locked)["colors"][:]
    steps.report["map_marker_guard"] = {
        "newest_chat_before": chat_before, "newest_chat_after": chat_after,
        "player_marker_before": player_before, "player_marker_after": player_after,
        "decoration_packets_before": updates_before,
        "decoration_packets_after": _updates(steps, locked),
        "terrain_changed_pixels": sum(1 for before, after in zip(colours_before, colours_after)
                                      if before != after),
    }
    # A line expiring between the captures rewrites the text drawn over the map; the ring buffer
    # keeps old lines, so the *newest* line is the observable that a full buffer cannot fool.
    steps.check("map_captures_same_newest_chat", chat_after == chat_before)
    steps.check("map_pair_identical_terrain", colours_after == colours_before)
    # The player's own decoration is read once per capture: it must be what the server sent both
    # times, or a moved player marker could stand in for the frame marker.
    steps.check("map_player_marker_unchanged", player_after == player_before)
    changed = _changed(without, with_frame, HELD_MAP_REGION)
    blob = _largest_blob(changed)
    snapshot = _snapshot(steps)
    kind = snapshot["marker_ids"]["minecraft:frame"]
    steps.report["frame_marker_map_pixel"] = [
        [marker["x"], marker["y"], marker["rotation"]]
        for marker in (snapshot["maps"].get(locked) or {}).get("markers", []) if marker["kind"] == kind]
    steps.report["frame_marker_region"] = {
        "region": list(HELD_MAP_REGION), "drift_max": DRIFT_MAX, "min_area": MARKER_MIN_AREA,
        "min_span": MARKER_MIN_SPAN, "min_fill": MARKER_MIN_FILL,
        "changed_pixels": len(changed), "largest_blob": blob}
    drawn = (blob["area"] >= MARKER_MIN_AREA
             and blob["width"] >= MARKER_MIN_SPAN and blob["height"] >= MARKER_MIN_SPAN
             and blob["fill"] >= MARKER_MIN_FILL
             and blob["x0"] > 0 and blob["y0"] > 0
             and blob["x0"] + blob["width"] < HELD_MAP_REGION[2] - HELD_MAP_REGION[0]
             and blob["y0"] + blob["height"] < HELD_MAP_REGION[3] - HELD_MAP_REGION[1])
    steps.check("waypoint_marker_drawn", drawn)
    x, y, z = origin
    steps.fixture("gamemode creative @s")
    steps.fixture(f"tp @s {x + 80.5} {y} {z + .5}")
    state = steps.wait(lambda state: state["player"]["x"] > x + 79,
                       "map_only_player_beyond_frame_view_radius")
    steps.check("map_frame_beyond_configured_64_block_view", state["player"]["x"] - x > 64)
    before = _updates(steps, locked)
    steps.wait(lambda _state: _updates(steps, locked) > before + 2,
               "map_distant_viewer_fresh_packets")
    _held_shot(steps, "map-distant-held-clone")
    steps.check("map_distant_clone_keeps_exactly_one_frame_waypoint",
                _marker_count(steps, locked, "frame") == 1)
    steps.report["visual_review_required"] = [
        "Review map-held-without-frame-waypoint.png and map-held-with-frame-waypoint.png against the "
        "recorded blob box."]
    steps.report["limits"] = [
        "scale-zero fixture map; declared cloned-item/teleport geometry fixtures",
        "the pixel predicate is guarded by a stable-pair drift budget; a scene that never settles "
        "reports inconclusive"]
    _echo(steps, "map_marker_region_ordered_chat")
