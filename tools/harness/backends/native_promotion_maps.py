"""Graphical map repros with exact received pixels/markers on the viewer wire."""
from __future__ import annotations

import json

from pathlib import Path

from .. import runtime
from .native_p1 import _capture
from .native_v2 import _block, _click, _echo, _give, _insert, _open, _set, _use
from .probe_steps import ProbeSteps, item_count


SCENARIO_KINDS = ("frame-waypoint", "banner-removal", "locked-banners", "pumpkin-hiding",
                  "terrain-shading", "nether-terrain", "inventory-player-markers")


# Region of the 1280x720 retained frame that holds the drawn held map's centre (where the frame
# marker is drawn) while never holding the chat overlay (which reaches x≈560 over the map), the
# top-right message box (y<160) or the held-item name tooltip (y>560).
HELD_MAP_REGIONS = ((620, 170, 930, 560),)
# The frame-marker icon is ~12x15 drawn pixels (measured on the fixed client: see the hunks map);
# 0 on a client that receives the decoration and never draws it.
FRAME_MARKER_MIN_PIXELS = 20


def _shot_path(steps: ProbeSteps, name: str) -> Path:
    return steps.artifact_dir.parent / "screenshots" / f"{name}.png"


def _differing_pixels(first: Path, second: Path, regions=HELD_MAP_REGIONS,
                      tolerance: int = 24) -> dict[str, int]:
    """Pixels that changed between two retained frames, per region plus the whole frame.

    A per-channel tolerance, not equality: the daylight lightmap moves a few units between two
    captures of an outdoor scene, while the marker sprite changes at least one channel by far more."""
    from PIL import Image

    def changed(a: tuple[int, int, int], b: tuple[int, int, int]) -> bool:
        return max(abs(a[0] - b[0]), abs(a[1] - b[1]), abs(a[2] - b[2])) > tolerance

    counts: dict[str, int] = {}
    with Image.open(first) as one, Image.open(second) as two:
        if one.size != two.size:
            raise ValueError(f"{first} is {one.size}, {second} is {two.size}")
        before, after = one.convert("RGB"), two.convert("RGB")
        before_px, after_px = list(before.getdata()), list(after.getdata())
        width = before.width
        counts["frame"] = sum(1 for a, b in zip(before_px, after_px) if changed(a, b))
        for index, region in enumerate(regions):
            left, top, right, bottom = region
            total = 0
            for y in range(top, min(bottom, before.height)):
                row = y * width
                for x in range(left, min(right, before.width)):
                    offset = row + x
                    if changed(before_px[offset], after_px[offset]):
                        total += 1
            counts[f"region{index}"] = total
    return counts


def _snapshot(steps: ProbeSteps) -> dict:
    receipt = getattr(steps.client, "promotion_wire_receipt", None)
    if not callable(receipt):
        raise RuntimeError("map probe requires its unmodified graphical viewer wire receipt")
    return receipt()


def _map(steps: ProbeSteps, map_id: int) -> dict | None:
    return _snapshot(steps)["maps"].get(map_id)


def _updates(steps: ProbeSteps, map_id: int) -> int:
    return sum(row["packet"] == "map_item_data" and row["id"] == map_id
               for row in _snapshot(steps)["events"])


def _cycle(steps: ProbeSteps, map_id: int, label: str) -> None:
    before = _updates(steps, map_id)
    # One scale-zero traversal is 16384/128 batches. Count actual viewer
    # packets, not wall-clock sleeps or assumed TPS. Extra batches cover the
    # action's position within its current batch.
    steps.wait(lambda _state: _updates(steps, map_id) >= before + 130, label,
               timeout_seconds=75)


def _scene(steps: ProbeSteps, origin: tuple[int, int, int], nether: bool) -> tuple[int, int, int]:
    x, _, z = origin
    x, z = ((value + 64) // 128 * 128 for value in (x, z))
    steps.fixture("gamemode creative @s")
    command = f"tp @s {x + .5} 201 {z + .5}"
    steps.fixture(f"execute in the_nether run {command}" if nether else command)
    steps.wait(lambda state: abs(state["player"]["x"] - x - .5) < .2
               and abs(state["player"]["z"] - z - .5) < .2
               and state["dimension"] == ("minecraft:the_nether" if nether else "minecraft:overworld"),
               "map_scene_arrival", wait_for_play=True)
    steps.fixture(f"fill {x - 8} 200 {z - 8} {x + 12} 200 {z + 12} minecraft:obsidian")
    steps.fixture(f"fill {x - 8} 201 {z - 8} {x + 12} 207 {z + 12} minecraft:air")
    _block(steps, {"x": x, "y": 200, "z": z}, "minecraft:obsidian")
    # The initial travel loads the destination, but gravity can move the
    # creative player below this new platform while terrain arrives. Re-seat
    # only after its actual block has been received, before enabling damage.
    steps.fixture(command)
    steps.wait(lambda state: abs(state["player"]["y"] - 201) < .1,
               "map_scene_seated_on_received_platform", wait_for_play=True)
    steps.fixture("gamemode survival @s")
    steps.wait(lambda state: state["player"]["on_ground"] and abs(state["player"]["y"] - 201) < .1
               and state["player"]["health"] > 0, "map_scene_grounded")
    steps.report["map_scene"] = {"center": [x, z], "floor_y": 200, "nether": nether}
    return x, 201, z


def _create(steps: ProbeSteps) -> int:
    _give(steps, "minecraft:map", 0)
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    steps.call("minecraft_press_inputs", {"keys": ["use"], "ticks": 1})
    steps.wait(lambda state: item_count(state, "minecraft:filled_map") == 1,
               "map_empty_item_used_into_filled_map")
    stack = steps.call("minecraft_read_item_components", {"source": "selected"})
    map_id = stack.get("components", {}).get("minecraft:map_id")
    steps.check("map_received_real_map_id_component", type(map_id) is int and map_id >= 0)
    steps.report.setdefault("created_maps", []).append(stack)
    steps.wait(lambda _state: _map(steps, map_id) is not None, "map_initial_packet_received")
    return map_id


def _clone(steps: ProbeSteps, map_id: int, slot: int) -> None:
    # Debug give accepts only hotbar indices. Pocket clones must get there
    # through the same ordinary inventory clicks available to a player.
    steps.fixture(f"debug give minecraft:filled_map[map_id={map_id}] 1 {slot if slot < 9 else 0}")
    _echo(steps, f"map_clone_{map_id}_{slot}_fixture_ordered")
    if slot >= 9:
        steps.call("minecraft_open_inventory")
        steps.wait(lambda state: state["screen"]["open"]
                   and state["container"]["container_id"] == 0,
                   f"map_clone_{map_id}_{slot}_inventory_open")
        _click(steps, 36)
        _click(steps, slot)
        steps.wait(lambda state: any(stack["slot"] == slot and stack["item_id"] == "minecraft:filled_map"
                                    for stack in state["inventory"])
                   and state["container"]["carried"]["count"] == 0,
                   f"map_clone_{map_id}_{slot}_ordinary_move_received")
        steps.call("minecraft_close_screen")
        _echo(steps, f"map_clone_{map_id}_{slot}_move_close_ordered")
    stack = steps.call("minecraft_read_item_components", {"source": "player", "slot": slot})
    steps.check(f"map_clone_{map_id}_{slot}_received", stack.get("item_id") == "minecraft:filled_map"
                and stack.get("components", {}).get("minecraft:map_id") == map_id)


def _marker_count(steps: ProbeSteps, map_id: int, kind: str) -> int:
    snapshot = _snapshot(steps)
    state = snapshot["maps"].get(map_id)
    marker_id = snapshot["marker_ids"][f"minecraft:{kind}"]
    return sum(marker["kind"] == marker_id for marker in state["markers"]) if state else 0


def _held_shot(steps: ProbeSteps, name: str) -> None:
    steps.call("minecraft_look", {"yaw_deg": 0, "pitch_deg": 80})
    # A use/interact immediately before the shot leaves the hand mid-swing; a sub-pixel offset would
    # change almost every map pixel, so let the pose settle before the retained frame.
    steps.call("minecraft_wait_ticks", {"ticks": 10})
    _capture(steps, name)


def _banner_shot(steps: ProbeSteps, origin: tuple[int, int, int], name: str) -> None:
    # The nearby white banner icon otherwise overlaps the white player pointer.
    x, y, z = origin
    steps.fixture(f"tp @s {x - 8.5} {y} {z + .5}")
    steps.wait(lambda state: abs(state["player"]["x"] - x + 8.5) < .2,
               name + "_viewer_separated_from_banner")
    _held_shot(steps, name)
    steps.fixture(f"tp @s {x + .5} {y} {z + .5}")
    steps.wait(lambda state: abs(state["player"]["x"] - x - .5) < .2,
               name + "_viewer_returned_to_interaction_position")


def _frame_place(steps: ProbeSteps, origin: tuple[int, int, int], slot: int = 0) -> dict:
    """Place the support block and one empty item frame, without a map: returns the frame identity."""
    x, y, z = origin
    support = {"x": x, "y": y + 1, "z": z + 3}
    _set(steps, support, "minecraft:stone")
    _give(steps, "minecraft:item_frame", slot)
    steps.call("minecraft_set_hotbar_slot", {"slot": slot})
    _use(steps, support, "north")
    frame = steps.call("minecraft_wait_for_visible_entity", {
        "entity_type": "minecraft:item_frame", "radius": 8, "timeout_seconds": steps.remaining()})
    return {key: frame[key] for key in ("entity_id", "entity_uuid", "entity_type")}


def _frame_insert(steps: ProbeSteps, identity: dict, map_id: int) -> None:
    """Insert a clone of `map_id` into the frame through ordinary hand interaction."""
    _clone(steps, map_id, 0)
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    steps.call("minecraft_interact_entity", {**identity, "hand": "main_hand"})
    steps.wait(lambda state: item_count(state, "minecraft:filled_map") == 0,
               "map_ordinary_frame_insertion_debited")
    steps.wait(lambda _state: _marker_count(steps, map_id, "frame") == 1,
               "map_near_frame_waypoint_received")


def _frame_waypoint(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    map_id = _create(steps)
    # Drawn-marker predicate on a *locked* map: only an unlocked map samples terrain
    # (crates/mc-core/src/maps/runtime.rs gates sampling on `!map.locked`; framed copies still
    # deliver pixels and markers), so the received colour array cannot change while the world does.
    # Both captures are taken with the support and the empty frame already placed and the same
    # locked map held; only the map inside the frame differs (absent, then present), so the only
    # rendered difference is the frame's marker icon: a client that receives the decoration and
    # never draws it produces the same frame twice.
    # The frame is placed from an unused hotbar slot so the created map survives for the cartography
    # lock; the locked copy then stays in slot 0 and is held in both captures.
    frame = _frame_place(steps, origin, slot=8)
    locked = _locked(steps, origin, map_id)
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    steps.check("map_unframed_map_has_no_frame_waypoint", _marker_count(steps, locked, "frame") == 0)
    steps.check("map_capture_without_frame_holds_the_map",
                steps.call("minecraft_read_item_components", {"source": "selected"}).get("item_id")
                == "minecraft:filled_map")
    colours_before = _map(steps, locked)["colors"][:]
    # The chat overlay is drawn over the left part of the held map and scrolls up with every new
    # line, so the read region starts right of the chat's longest line. Command feedback is turned
    # off for the two captures so only the ordered marker line is added between them (F1 would hide
    # the held item itself, which is why the HUD stays visible).
    steps.fixture("gamerule send_command_feedback false")
    _echo(steps, "map_captures_command_feedback_off")
    _held_shot(steps, "map-held-without-frame-waypoint")
    steps.fixture("clear @s minecraft:filled_map")
    _frame_insert(steps, frame, locked)
    steps.check("map_framed_map_has_exactly_one_frame_waypoint",
                _marker_count(steps, locked, "frame") == 1)
    _clone(steps, locked, 0)
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    steps.check("map_capture_with_frame_holds_the_map",
                steps.call("minecraft_read_item_components", {"source": "selected"}).get("item_id")
                == "minecraft:filled_map")
    _held_shot(steps, "map-held-with-frame-waypoint")
    steps.fixture("gamerule send_command_feedback true")
    colours_after = _map(steps, locked)["colors"][:]
    steps.report["map_terrain_changed_pixels"] = sum(
        1 for before, after in zip(colours_before, colours_after) if before != after)
    steps.check("map_pair_identical_terrain", colours_after == colours_before)
    changed = _differing_pixels(_shot_path(steps, "map-held-without-frame-waypoint"),
                                _shot_path(steps, "map-held-with-frame-waypoint"))
    steps.report["frame_marker_pixels"] = changed
    drawn = max(value for name, value in changed.items() if name.startswith("region"))
    steps.report["frame_marker_drawn_pixels"] = drawn
    steps.check("waypoint_marker_drawn", drawn >= FRAME_MARKER_MIN_PIXELS)
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


def _locked(steps: ProbeSteps, origin: tuple[int, int, int], map_id: int) -> int:
    x, y, z = origin
    table = {"x": x - 2, "y": y, "z": z + 2}
    _set(steps, table, "minecraft:cartography_table")
    _give(steps, "minecraft:glass_pane", 1)
    _open(steps, table, "CartographyTableMenu", 39)
    _insert(steps, "minecraft:filled_map", 0)
    _insert(steps, "minecraft:glass_pane", 1)
    steps.wait(lambda state: any(stack["slot"] == 2 and stack["item_id"] == "minecraft:filled_map"
                                for stack in state["container"]["slots"]),
               "map_cartography_locked_output_received")
    _click(steps, 2, quick=True)
    state = steps.wait(lambda state: item_count(state, "minecraft:filled_map") == 1,
                       "map_ordinary_cartography_lock_taken")
    slots = [stack["slot"] for stack in state["inventory"] if stack["item_id"] == "minecraft:filled_map"]
    steps.check("map_locked_output_unique", len(slots) == 1)
    stack = steps.call("minecraft_read_item_components", {"source": "player", "slot": slots[0]})
    locked_id = stack.get("components", {}).get("minecraft:map_id")
    steps.check("map_lock_allocates_new_id", type(locked_id) is int and locked_id != map_id)
    steps.call("minecraft_close_screen")
    steps.fixture("clear @s minecraft:filled_map")
    _clone(steps, locked_id, 0)
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    steps.wait(lambda _state: (_map(steps, locked_id) or {}).get("locked") is True,
               "map_received_locked_flag")
    return locked_id


def _banners(steps: ProbeSteps, origin: tuple[int, int, int], locked: bool, colors: dict) -> None:
    x, y, z = origin
    map_id = _create(steps)
    if locked:
        map_id = _locked(steps, origin, map_id)
    point = {"x": x, "y": y, "z": z + 2}
    _set(steps, point, "minecraft:white_banner")
    _use(steps, point)
    steps.wait(lambda _state: _marker_count(steps, map_id, "banner_white") == 1,
               "map_explicit_banner_waypoint_added", timeout_seconds=15)
    _banner_shot(steps, origin, "map-banner-waypoint-added")
    if locked:
        before = _map(steps, map_id)["colors"][:]
        _use(steps, point)
        steps.wait(lambda _state: _marker_count(steps, map_id, "banner_white") == 0,
                   "map_locked_banner_waypoint_toggled_off")
        steps.check("map_locked_banner_actions_keep_terrain_pixels", _map(steps, map_id)["colors"] == before)
    else:
        pixel = 66 * 128 + 64
        steps.wait(lambda _state: _map(steps, map_id)["colors"][pixel] // 4 == colors["white_banner"],
                   "map_banner_terrain_initially_explored", timeout_seconds=75)
        steps.fixture("gamemode creative @s")
        steps.call("minecraft_break_block", {**point, "face": "up", "expected_drop_item_id": "minecraft:white_banner",
                   "expected_drop_count": 0, "timeout_seconds": steps.remaining()})
        _block(steps, point, "minecraft:air")
        steps.fixture(f"tp @s {x + 8.5} {y} {z + .5}")
        steps.wait(lambda state: state["player"]["x"] > x + 8, "map_banner_area_departed")
        steps.fixture(f"tp @s {x + .5} {y} {z + .5}")
        steps.wait(lambda state: abs(state["player"]["x"] - x - .5) < .2, "map_destroyed_banner_revisited")
        steps.wait(lambda _state: _marker_count(steps, map_id, "banner_white") == 0
                   and _map(steps, map_id)["colors"][pixel] // 4 == colors["obsidian"],
                   "map_revisit_remaps_terrain_and_evicts_destroyed_banner", timeout_seconds=75)
    _banner_shot(steps, origin, "map-banner-waypoint-removed")


def _players(steps: ProbeSteps, origin: tuple[int, int, int], pumpkin: bool) -> None:
    peer = getattr(steps.client, "p5_armor_actor", None)
    if peer is None:
        raise RuntimeError("multiplayer map probe requires its independent headless native actor")
    first = _create(steps)
    # Two distinct ids of the same item type discriminate the old item-id
    # deduplication bug, not merely whether one held copy produces a marker.
    second = first if pumpkin else _create(steps)
    steps.check("map_distinct_ids_for_same_item_type", pumpkin or second != first)
    x, y, z = origin
    with ProbeSteps(peer, steps.artifact_dir / "map-actor", steps.remaining(), "map-actor") as actor:
        actor.fixture("clear @s")
        actor.fixture(f"tp @s {x + 8.5} {y} {z + .5}")
        actor.wait(lambda state: abs(state["player"]["x"] - x - 8.5) < .2,
                   "map_actor_at_distinct_represented_position")
        _clone(actor, first, 10)
        if second != first:
            _clone(actor, second, 11)
        if pumpkin:
            # Keep a real held clone in the offhand so the pumpkin assertion
            # is independent of the separate pocket-inventory tracking fix.
            _clone(actor, first, 0)
            actor.call("minecraft_set_hotbar_slot", {"slot": 0})
            actor.call("minecraft_press_inputs", {"keys": ["swap_offhand"], "ticks": 1})
            actor.wait(lambda state: any(stack["slot"] == 40 and stack["item_id"] == "minecraft:filled_map"
                                         for stack in state["inventory"]),
                       "map_pumpkin_actor_clone_received_in_offhand")
        _give(actor, "minecraft:stick", 8)
        actor.call("minecraft_set_hotbar_slot", {"slot": 8})
        actor_state = _echo(actor, "map_actor_pocket_clones_ordered")
        steps.report["map_actor_before"] = actor_state["inventory"]
        steps.wait(lambda _state: _marker_count(steps, second, "player") == 2,
                   "map_observer_sees_carrier_of_held_clone" if pumpkin else "map_observer_sees_carrier_of_pocket_clone",
                   timeout_seconds=15)
        _held_shot(steps, "map-two-player-markers-before-pumpkin" if pumpkin else "map-pocket-clone-player-markers")
        if pumpkin:
            _give(actor, "minecraft:carved_pumpkin", 0)
            actor.call("minecraft_open_inventory")
            actor.wait(lambda state: state["screen"]["open"]
                       and state["container"]["container_id"] == 0,
                       "map_actor_inventory_open_for_pumpkin")
            _click(actor, 36)
            _click(actor, 5)
            equipped = actor.wait(lambda state: any(stack["slot"] == 39 and stack["item_id"] == "minecraft:carved_pumpkin"
                                                   for stack in state["inventory"]),
                                  "map_actor_pumpkin_equipped_ordinary_inventory_clicks")
            steps.report["map_actor_pumpkin_inventory"] = equipped["inventory"]
            actor.call("minecraft_close_screen")
            steps.wait(lambda _state: _marker_count(steps, second, "player") == 1,
                       "map_pumpkin_carrier_hidden_from_other_player", timeout_seconds=15)
            _held_shot(steps, "map-other-player-pumpkin-marker-hidden")


def _terrain(steps: ProbeSteps, origin: tuple[int, int, int], nether: bool, colors: dict) -> None:
    x, y, z = origin
    if nether:
        steps.fixture(f"fill {x - 8} {y + 9} {z - 8} {x + 8} {y + 9} {z + 8} minecraft:gold_block")
    else:
        # Stable contained source columns: equal surface heights, one vs four
        # water cells. Adjacent solid columns have equal vs north-raised tops.
        steps.fixture(f"fill {x + 1} {y} {z - 1} {x + 6} {y + 3} {z + 1} minecraft:stone")
        steps.fixture(f"setblock {x + 2} {y + 3} {z} minecraft:water")
        steps.fixture(f"fill {x + 4} {y} {z} {x + 4} {y + 3} {z} minecraft:water")
        steps.fixture(f"fill {x + 6} {y + 2} {z - 1} {x + 6} {y + 3} {z - 1} minecraft:air")
    _echo(steps, "map_terrain_fixture_ordered")
    map_id = _create(steps)
    samples = [64 * 128 + 64 + offset for offset in (2, 4, 5, 6)]
    if nether:
        samples = [(64 + dz) * 128 + 64 + dx for dz in range(-4, 5) for dx in range(-4, 5)]
        expected = {colors["stone"], colors["dirt"]}
        steps.wait(lambda _state: all(_map(steps, map_id)["colors"][index] // 4 in expected for index in samples),
                   "map_nether_noise_not_physical_gold_roof", timeout_seconds=75)
        before = [_map(steps, map_id)["colors"][index] for index in samples]
        steps.check("map_nether_noise_has_both_vanilla_base_colors", {color // 4 for color in before} == expected)
        steps.fixture(f"fill {x - 8} {y + 9} {z - 8} {x + 8} {y + 9} {z + 8} minecraft:diamond_block")
        _echo(steps, "map_nether_roof_replaced_ordered")
        _cycle(steps, map_id, "map_nether_full_sampling_cycle_after_roof_change")
        steps.check("map_nether_noise_unchanged_by_roof_build", [_map(steps, map_id)["colors"][index] for index in samples] == before)
    else:
        def shaded(_state: dict) -> bool:
            current = [_map(steps, map_id)["colors"][index] for index in samples]
            return (current[0] // 4 == current[1] // 4 == colors["water"]
                    and current[0] != current[1]
                    and current[2] // 4 == current[3] // 4 == colors["stone"]
                    and current[2] != current[3])
        steps.wait(shaded, "map_received_distinct_water_depth_and_height_shades", timeout_seconds=75)
    steps.report["map_terrain_samples"] = [{"index": index, "color": _map(steps, map_id)["colors"][index]} for index in samples]
    _held_shot(steps, "map-nether-ceiling-noise" if nether else "map-depth-and-height-shading")


def run(steps: ProbeSteps, origin: tuple[int, int, int], kind: str) -> None:
    if kind not in SCENARIO_KINDS:
        raise ValueError(f"unknown map probe: {kind}")
    if not getattr(steps.client, "p4_window", None):
        raise RuntimeError("map probe requires a harness-owned windowed native viewer")
    reports = runtime.REPO_ROOT / "data/vanilla/reports"
    blocks = json.loads((reports / "blocks.json").read_text())
    palette = json.loads((reports / "block_map_colors.json").read_text())
    colors = {name: palette[next(state["id"] for state in blocks[f"minecraft:{name}"]["states"]
                                if state.get("default"))]
              for name in ("gold_block", "diamond_block", "water", "stone", "dirt", "white_banner", "obsidian")}
    origin = _scene(steps, origin, kind == "nether-terrain")
    if kind == "frame-waypoint":
        _frame_waypoint(steps, origin)
    elif kind in ("banner-removal", "locked-banners"):
        _banners(steps, origin, kind == "locked-banners", colors)
    elif kind in ("pumpkin-hiding", "inventory-player-markers"):
        _players(steps, origin, kind == "pumpkin-hiding")
    else:
        _terrain(steps, origin, kind == "nether-terrain", colors)
    snapshot = _snapshot(steps)
    steps.report["map_wire"] = {"receipt": str(steps.artifact_dir.parent / "promotion-wire-receipts.json"),
                                "map_ids": list(snapshot["maps"]), "event_count": len(snapshot["events"])}
    steps.report["visual_review_required"] = ["Review actual held-map screenshots against the received marker/pixel predicates."]
    steps.report["limits"] = ["scale-zero fixture maps; declared cloned-item/teleport geometry fixtures, not an earned cartography playthrough"]
