"""Scene-shot step machinery: the scenario tool vocabulary, its validation and the
dispatcher that turns one step into client calls and evidence.
"""

from __future__ import annotations

import math
import re
from typing import Any


SCENE_TOOLS = {
    "key": "minecraft_press_inputs",
    "look": "minecraft_look",
    "hotbar": "minecraft_set_hotbar_slot",
    "use_item_on": "minecraft_use_item_on",
    "click_screen_button": "minecraft_click_screen_button",
    "scan_blocks": "minecraft_scan_blocks",
    "read_block": "minecraft_read_block",
    "navigate_to_block": "minecraft_navigate_to_block",
    "list_entities": "minecraft_list_entities",
    "wait_terrain": "minecraft_wait_for_terrain",
    "open_chat": "minecraft_open_chat",
}


def scene_mcp_calls(steps: list[dict[str, Any]]):
    """Yield the input-derived MCP calls before launch for schema validation."""
    for step in steps:
        if set(step) == {"shot", "hold_keys"}:
            yield "minecraft_press_inputs", {"keys": step["hold_keys"], "ticks": 1, "hold": True}
            continue
        kind, value = next(iter(step.items()))
        if kind in ("key", "look", "hotbar", "use_item_on", "click_screen_button", "open_chat"):
            yield SCENE_TOOLS[kind], value
        elif kind == "assert_block":
            yield "minecraft_read_block", value["args"]
        elif kind == "assert_screen" or kind == "assert_observation":
            yield "minecraft_observe", {}
        elif kind in ("render_draw_sample", "assert_render_bounded", "assert_chat_components",
                      "hover_chat_component"):
            # `hover_chat_component` also calls `client_pointer`, whose screen class and title
            # are read from the observation at run time and cannot be validated before launch.
            yield "minecraft_observe", {}
        elif kind in ("scan_blocks", "read_block", "navigate_to_block", "list_entities", "wait_terrain"):
            yield SCENE_TOOLS[kind], value["args"]
        elif kind in ("timed_entities", "require_entities", "assert_entities"):
            yield "minecraft_list_entities", value["args"]
        elif kind == "wait_ticks":
            yield "minecraft_wait_ticks", {"ticks": min(value, 100)}
        elif kind == "pause":
            yield "minecraft_open_pause_menu", {}
        elif kind == "use_phases":
            yield "minecraft_press_inputs", {"keys": ["use"], "ticks": 1, "hold": True}
            yield "minecraft_wait_ticks", {"ticks": value["ticks"]}
        elif kind == "resize":
            yield "minecraft_wait_ticks", {"ticks": 5}
        elif kind == "reconnect":
            yield "minecraft_connect", {"server_addr": "127.0.0.1:1"}
        elif kind == "walk_capture":
            yield "minecraft_list_entities", {"radius": 32, "limit": 128}
            yield "minecraft_wait_ticks", {"ticks": 1}
        elif kind == "dropped_clock_capture":
            yield "minecraft_wait_ticks", {"ticks": 4}
        elif kind == "falling_block_capture":
            yield "minecraft_wait_for_visible_entity", {"entity_type": "minecraft:falling_block", "radius": 32, "timeout_seconds": 30}
            yield "minecraft_wait_ticks", {"ticks": 1}
        elif kind == "toggle_debug_overlay":
            yield "minecraft_toggle_debug_overlay", {"enabled": value}
        elif kind == "cmd":
            yield "minecraft_send_chat", {"message": value, "command": True}
        elif kind == "tp":
            yield "minecraft_send_chat", {"message": "tp @s " + " ".join(map(str, value)), "command": True}


def validate_steps(steps: list[dict[str, Any]], client: str) -> None:
    """Validate the same step contracts before any process is started."""
    for step in steps:
        if isinstance(step, dict) and set(step) == {"shot", "hold_keys"}:
            name, keys = step["shot"], step["hold_keys"]
            if client == "java":
                raise ValueError("hold_keys screenshots unsupported for --client java")
            if (not isinstance(name, str)
                    or not re.fullmatch(r"[A-Za-z0-9_-]+", name)
                    or not isinstance(keys, list) or not keys
                    or any(not isinstance(key, str) for key in keys)):
                raise ValueError(f"invalid held screenshot: {step!r}")
            continue
        if not isinstance(step, dict) or len(step) != 1:
            raise ValueError(f"invalid scene step: {step!r}")
        kind, value = next(iter(step.items()))
        if kind in ("shot", "observe", "goat_horns_upright", "goat_side_horn", "tool_wall_visible"):
            valid = isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9_-]+", value)
        elif kind == "assert_same_game_time":
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) == {"before", "after", "day_delta"}
                     and all(isinstance(value[key], str)
                             and re.fullmatch(r"[A-Za-z0-9_-]+", value[key])
                             for key in ("before", "after"))
                     and type(value["day_delta"]) is int and value["day_delta"] > 0)
        elif kind == "death_tilt":
            valid = (isinstance(value, dict) and set(value) == {"empty", "shots"}
                     and isinstance(value["shots"], list) and bool(value["shots"])
                     and all(isinstance(name, str) and re.fullmatch(r"[A-Za-z0-9_-]+", name)
                             for name in [value["empty"], *value["shots"]]))
        elif kind == "assert_chat":
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) == {"name", "contains", "excludes"}
                     and isinstance(value["name"], str)
                     and re.fullmatch(r"[A-Za-z0-9_-]+", value["name"])
                     and all(isinstance(value[key], list)
                             and all(isinstance(item, str) and bool(item) for item in value[key])
                             for key in ("contains", "excludes"))
                     and bool(value["contains"]))
        elif kind == "assert_observation":
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) == {"name", "path", "equals"}
                     and isinstance(value["name"], str)
                     and re.fullmatch(r"[A-Za-z0-9_-]+", value["name"])
                     and isinstance(value["path"], list) and 1 <= len(value["path"]) <= 4
                     and all(isinstance(key, str) and key for key in value["path"]))
        elif kind == "assert_player_sleeping":
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) == {"name", "sleeping"}
                     and isinstance(value["name"], str)
                     and re.fullmatch(r"[A-Za-z0-9_-]+", value["name"])
                     and type(value["sleeping"]) is bool)
        elif kind == "cmd":
            valid = isinstance(value, str) and bool(value) and not value.startswith("/")
        elif kind == "upgrade_certified":
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) == {"kind", "level"}
                     and isinstance(value["kind"], str)
                     and re.fullmatch(r"[a-z_]+", value["kind"])
                     and type(value["level"]) is int and 2 <= value["level"] <= 3)
        elif kind == "tp":
            valid = (isinstance(value, list) and len(value) == 5
                     and all(type(n) in (int, float) and math.isfinite(n) for n in value))
        elif kind == "wait_ticks":
            valid = type(value) is int and value > 0
        elif kind in ("wait_play", "wait_respawn"):
            valid = value is True
        elif kind == "pause":
            valid = client == "native" and value is True
        elif kind in ("reconnect", "server_eof"):
            valid = client == "native" and value is True
        elif kind == "wait_disconnect":
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) == {"name", "eof"}
                     and isinstance(value["name"], str)
                     and re.fullmatch(r"[A-Za-z0-9_-]+", value["name"])
                     and type(value["eof"]) is bool)
        elif kind == "wait_chat":
            valid = client == "native" and isinstance(value, str) and bool(value)
        elif kind == "wait_loader_block":
            valid = client == "native" and isinstance(value, str) and bool(re.fullmatch(r"[a-z0-9_.-]+:[a-z0-9_./-]+", value))
        elif kind == "wait_screen":
            valid = isinstance(value, str) and bool(value)
        elif kind == "toggle_debug_overlay":
            valid = type(value) is bool
        elif kind == "pixel_window":
            valid = (isinstance(value, dict)
                     and set(value) == {"name", "shot", "window", "color", "min_pixels"}
                     and isinstance(value["name"], str)
                     and bool(re.fullmatch(r"[A-Za-z0-9_-]+", value["name"]))
                     and isinstance(value["shot"], str) and bool(value["shot"])
                     and isinstance(value["window"], list) and len(value["window"]) == 4
                     and all(type(v) is int for v in value["window"])
                     and 0 <= value["window"][0] < value["window"][2] < PIXEL_FRAME[0]
                     and 0 <= value["window"][1] < value["window"][3] < PIXEL_FRAME[1]
                     and isinstance(value["color"], dict) and set(value["color"]) == {"min", "max"}
                     and all(isinstance(value["color"][k], list) and len(value["color"][k]) == 3
                             and all(type(c) is int and 0 <= c <= 255 for c in value["color"][k])
                             for k in ("min", "max"))
                     and all(value["color"]["min"][i] <= value["color"]["max"][i] for i in range(3))
                     and type(value["min_pixels"]) is int and value["min_pixels"] > 0)
        elif kind == "render_draw_sample":
            # One observation of the client's own draw counters at a named view. The yaw the client
            # reports must match `yaw_deg` (and `pitch_deg` when given, default horizon), so a
            # sample taken without the requested turn fails, and a frame counter that did not
            # advance between samples fails as a stale render.
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) in ({"name", "yaw_deg"}, {"name", "yaw_deg", "pitch_deg"})
                     and isinstance(value["name"], str)
                     and bool(re.fullmatch(r"[A-Za-z0-9_-]+", value["name"]))
                     and type(value["yaw_deg"]) in (int, float) and math.isfinite(value["yaw_deg"])
                     and -360 <= value["yaw_deg"] <= 360
                     and ("pitch_deg" not in value
                          or (type(value["pitch_deg"]) in (int, float)
                              and math.isfinite(value["pitch_deg"])
                              and -90 <= value["pitch_deg"] <= 90)))
        elif kind == "assert_render_bounded":
            # Aggregate over every preceding render_draw_sample: a strict shape so a typo cannot
            # silently widen the bound, and every sample must carry a real per-frame count.
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) == {"max_index_count", "max_growth", "min_index_count"}
                     and type(value["max_index_count"]) is int and value["max_index_count"] > 0
                     and type(value["max_growth"]) in (int, float)
                     and math.isfinite(value["max_growth"]) and value["max_growth"] > 1
                     and type(value["min_index_count"]) is int and value["min_index_count"] >= 0)
        elif kind == "assert_chat_components":
            # Reads the newest chat line whose plain text holds `line`, then its hover (and
            # optionally click) event from `recent_chat_components`. A named action and a
            # substring of its rendered value, so a flattened line cannot satisfy it.
            def event_spec(spec):
                return (isinstance(spec, dict) and set(spec) == {"action", "contains"}
                        and isinstance(spec["action"], str) and bool(spec["action"])
                        and isinstance(spec["contains"], str) and bool(spec["contains"]))
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) in ({"name", "line", "hover"}, {"name", "line", "hover", "click"})
                     and isinstance(value["name"], str)
                     and bool(re.fullmatch(r"[A-Za-z0-9_-]+", value["name"]))
                     and isinstance(value["line"], str) and bool(value["line"])
                     and event_spec(value["hover"])
                     and ("click" not in value or event_spec(value["click"])))
        elif kind == "hover_chat_component":
            # A real hover: the chat screen is open, the pointer moves over the painted line
            # through `client_pointer`, and the tooltip the client then shows must hold the text.
            # The shot is the frame with the tooltip; the pointer result and the UI node are kept.
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) == {"name", "line", "contains", "shot"}
                     and all(isinstance(value[key], str) and bool(value[key])
                             for key in ("name", "line", "contains", "shot"))
                     and bool(re.fullmatch(r"[A-Za-z0-9_-]+", value["name"]))
                     and bool(re.fullmatch(r"[A-Za-z0-9_-]+", value["shot"])))
        elif kind == "tool_hand_visible":
            # A frame away from the wall: the held tool must be visible on it (the pre-image, which
            # draws the hand in world geometry, keeps it visible there and hides it only up close).
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) == {"shot", "min_distance", "max_distance"}
                     and isinstance(value["shot"], str)
                     and bool(re.fullmatch(r"[A-Za-z0-9_-]+", value["shot"]))
                     and all(type(value[key]) in (int, float) and math.isfinite(value[key])
                             for key in ("min_distance", "max_distance"))
                     and 0 <= value["min_distance"] < value["max_distance"])
        elif kind == "key":
            valid = isinstance(value, dict) and set(value) == {"keys", "ticks"}
        elif kind == "open_chat":
            valid = (client == "native" and isinstance(value, dict) and set(value) == {"command"}
                     and type(value["command"]) is bool)
        elif kind in ("look", "hotbar", "use_item_on", "click_screen_button"):
            valid = isinstance(value, dict)
        elif kind == "use_phases":
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) == {"name", "ticks"}
                     and isinstance(value["name"], str)
                     and re.fullmatch(r"[A-Za-z0-9_-]+", value["name"])
                     and type(value["ticks"]) is int and 1 <= value["ticks"] <= 100)
        elif kind == "resize":
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) == {"name", "size"}
                     and isinstance(value["name"], str)
                     and re.fullmatch(r"[A-Za-z0-9_-]+", value["name"])
                     and isinstance(value["size"], list) and len(value["size"]) == 2
                     and all(type(n) is int for n in value["size"])
                     and 320 <= value["size"][0] <= 1280
                     and 240 <= value["size"][1] <= 720)
        elif kind == "walk_capture":
            valid = (isinstance(value, dict)
                     and set(value) == {"name", "entity_type", "phase", "tolerance", "min_speed", "max_ticks"}
                     and isinstance(value["name"], str)
                     and re.fullmatch(r"[A-Za-z0-9_-]+", value["name"])
                     and isinstance(value["entity_type"], str) and value["entity_type"].startswith("minecraft:")
                     and all(type(value[key]) in (int, float) and math.isfinite(value[key])
                             for key in ("phase", "tolerance", "min_speed"))
                     and 0 <= value["phase"] < math.tau and 0 < value["tolerance"] < math.pi
                     and 0 < value["min_speed"] <= 1
                     and type(value["max_ticks"]) is int and 1 <= value["max_ticks"] <= 300)
        elif kind == "dropped_clock_capture":
            valid = client == "native" and value in ("ctx-day", "ctx-night")
        elif kind == "falling_block_capture":
            valid = client == "native" and isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9_-]+", value)
        elif kind == "mob_motion":
            valid = (isinstance(value, dict)
                     and set(value) == {"samples", "name", "types"}
                     and isinstance(value["name"], str) and re.fullmatch(r"[A-Za-z0-9_-]+", value["name"])
                     and isinstance(value["samples"], list) and len(value["samples"]) == 3
                     and all(isinstance(item, str) and re.fullmatch(r"[A-Za-z0-9_-]+", item)
                             for item in value["samples"])
                     and isinstance(value["types"], list) and bool(value["types"])
                     and all((isinstance(item, str)
                              and re.fullmatch(r"[a-z0-9_.-]+:[a-z0-9_./-]+", item))
                             or (isinstance(item, dict)
                                 and set(item) == {"id", "min_speed", "max_speed"}
                                 and isinstance(item["id"], str)
                                 and re.fullmatch(r"[a-z0-9_.-]+:[a-z0-9_./-]+", item["id"])
                                 and all(type(item[key]) in (int, float) and math.isfinite(item[key])
                                         and item[key] >= 0 for key in ("min_speed", "max_speed"))
                                 and item["min_speed"] < item["max_speed"])
                             for item in value["types"]))
        elif kind == "mob_chase":
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) == {"samples", "name", "kind", "min_speed", "max_speed",
                                        "min_approach_blocks", "min_distance_last", "aggressive_bit"}
                     and isinstance(value["name"], str) and re.fullmatch(r"[A-Za-z0-9_-]+", value["name"])
                     and isinstance(value["kind"], str)
                     and re.fullmatch(r"[a-z0-9_.-]+:[a-z0-9_./-]+", value["kind"])
                     and isinstance(value["samples"], list) and len(value["samples"]) >= 2
                     and all(isinstance(item, str) and re.fullmatch(r"[A-Za-z0-9_-]+", item)
                             for item in value["samples"])
                     and all(type(value[key]) in (int, float) and math.isfinite(value[key]) and value[key] >= 0
                             for key in ("min_speed", "max_speed", "min_approach_blocks", "min_distance_last"))
                     and value["min_speed"] < value["max_speed"]
                     and type(value["aggressive_bit"]) is int and 0 <= value["aggressive_bit"] <= 7)
        elif kind == "assert_block":
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) == {"name", "args", "expect"}
                     and isinstance(value["name"], str)
                     and re.fullmatch(r"[A-Za-z0-9_-]+", value["name"])
                     and isinstance(value["args"], dict)
                     and set(value["args"]) == {"x", "y", "z"}
                     and all(type(number) is int for number in value["args"].values())
                     and isinstance(value["expect"], str)
                     and re.fullmatch(r"[a-z0-9_.-]+:[a-z0-9_./-]+", value["expect"]))
        elif kind == "assert_screen":
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) == {"name", "contains", "excludes"}
                     and isinstance(value["name"], str)
                     and re.fullmatch(r"[A-Za-z0-9_-]+", value["name"])
                     and all(isinstance(value[key], list)
                             and all(isinstance(item, str) and bool(item) for item in value[key])
                             for key in ("contains", "excludes"))
                     and bool(value["contains"]))
        elif kind == "assert_entities":
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) <= {"name", "args", "entity_type", "box", "min", "max",
                                        "max_height", "profession", "baby", "sleeping"}
                     and {"name", "args", "entity_type", "min"} <= set(value)
                     and isinstance(value["name"], str)
                     and re.fullmatch(r"[A-Za-z0-9_-]+", value["name"])
                     and isinstance(value["args"], dict)
                     and isinstance(value["entity_type"], str)
                     and re.fullmatch(r"[a-z0-9_.-]+:[a-z0-9_./-]+", value["entity_type"])
                     and type(value["min"]) is int and value["min"] >= 0
                     and type(value.get("max", 0)) is int
                     and type(value.get("max_height", 0)) in (int, float)
                     and value.get("profession", "farmer") in PROFESSIONS
                     and type(value.get("baby", True)) is bool
                     and type(value.get("sleeping", True)) is bool
                     and ("box" not in value
                          or (isinstance(value["box"], list) and len(value["box"]) == 6
                              and all(type(n) in (int, float) and math.isfinite(n)
                                      for n in value["box"]))))
        elif kind == "require_entities":
            valid = (isinstance(value, dict) and set(value) == {"name", "args", "types"}
                     and isinstance(value["name"], str)
                     and re.fullmatch(r"[A-Za-z0-9_-]+", value["name"])
                     and isinstance(value["args"], dict)
                     and isinstance(value["types"], list) and bool(value["types"])
                     and all(isinstance(item, str)
                             and re.fullmatch(r"[a-z0-9_.-]+:[a-z0-9_./-]+", item)
                             for item in value["types"]))
        elif kind in ("scan_blocks", "read_block", "navigate_to_block", "list_entities", "wait_terrain", "timed_entities"):
            if kind == "wait_terrain" and client != "native":
                raise ValueError("wait_terrain requires --client native")
            valid = (isinstance(value, dict) and set(value) == {"name", "args"}
                     and isinstance(value["name"], str)
                     and re.fullmatch(r"[A-Za-z0-9_-]+", value["name"])
                     and isinstance(value["args"], dict))
            if valid and kind == "wait_terrain":
                args = value["args"]
                sections = args.get("sections")
                timeout = args.get("timeout_seconds", 30)
                valid = (client == "native" and {"sections"} <= set(args) <= {"sections", "timeout_seconds"}
                         and isinstance(sections, list) and 1 <= len(sections) <= 64
                         and all(isinstance(section, list) and len(section) == 3
                                 and all(type(n) is int and -(2**31) <= n < 2**31 for n in section)
                                 for section in sections)
                         and len({tuple(section) for section in sections}) == len(sections)
                         and type(timeout) in (int, float) and math.isfinite(timeout)
                         and 0.1 <= timeout <= 120)
        else:
            valid = False
        if not valid:
            raise ValueError(f"invalid scene step: {step!r}")


# Villager profession order of the received VillagerData metadata (wire profession ids).
PROFESSIONS = ("none", "armorer", "butcher", "cartographer", "cleric", "farmer", "fisherman",
               "fletcher", "leatherworker", "librarian", "mason", "nitwit", "shepherd",
               "toolsmith", "weaponsmith")


def received_sleeping(metadata: dict[str, Any]) -> bool:
    """Read the wire pose, never a locally predicted pose or a missing default."""
    pose = metadata.get("6")
    if (not isinstance(pose, dict) or pose.get("kind") != "pose"
            or type(pose.get("value")) is not int):
        raise RuntimeError("received metadata omitted entity pose")
    return pose["value"] == 2


PIXEL_FRAME = (1280, 720)


def pixel_window_count(frame, window, color) -> int:
    """Pixels inside `window` whose RGB lies inside the inclusive `color` box.

    A capture at another size cannot be compared with hand-picked coordinates, so it refuses."""
    if frame.size != PIXEL_FRAME:
        raise ValueError(f"pixel_window requires the {PIXEL_FRAME[0]}x{PIXEL_FRAME[1]} frame, got {frame.size}")
    x0, y0, x1, y1 = window
    low, high = color["min"], color["max"]
    count = 0
    for y in range(y0, y1 + 1):
        for x in range(x0, x1 + 1):
            r, g, b = frame.convert("RGB").getpixel((x, y))
            if low[0] <= r <= high[0] and low[1] <= g <= high[1] and low[2] <= b <= high[2]:
                count += 1
    return count


def render_metrics(observation) -> dict[str, int]:
    """The client's own draw counters (data.render): present whenever a renderer exists, F3 or not."""
    render = observation.get("render") if isinstance(observation, dict) else None
    if not isinstance(render, dict):
        raise RuntimeError("client observation omitted render metrics (data.render)")
    metrics = {}
    for name in ("draw_calls", "index_count", "entity_submitted_indices"):
        value = render.get(name)
        if type(value) is not int or value < 0:
            raise RuntimeError(f"client observation render metrics lack a non-negative {name}")
        metrics[name] = value
    # The renderer publishes its submitted frame counter as a decimal string; a sample whose
    # counter did not advance between two views read a stale render, not the requested view.
    frame = render.get("submitted_frame_id")
    if isinstance(frame, str) and frame.isascii() and frame.isdigit():
        metrics["submitted_frame_id"] = int(frame)
    elif type(frame) is int and frame >= 0:
        metrics["submitted_frame_id"] = frame
    else:
        raise RuntimeError("client observation render metrics lack a submitted_frame_id")
    return metrics


def render_index_bounded(samples, *, max_index_count, max_growth, min_index_count) -> bool:
    """True when every sampled draw count is a per-frame submission, not an accumulation.

    Each sample must be a real non-zero count under the absolute bound and within `max_growth` of
    the first sample: with the per-frame FrameStats reset dropped the count accumulates with camera
    movement (the ISSUE-1-03 symptom) instead of being re-submitted every frame."""
    if not samples:
        return False
    first = samples[0]["index_count"]
    return all(min_index_count <= sample["index_count"] <= max_index_count
               and sample["index_count"] <= max_growth * max(first, 1)
               for sample in samples)


def repeated_view_stable(samples, *, max_growth) -> bool:
    """True when every repeat at one view (yaw and pitch), and every return to an earlier view,
    stays within `max_growth` of that view's earlier sample: a stationary camera must not grow
    the count. Two looks with the same yaw but a different pitch are different views."""
    seen: dict[tuple[float, float], int] = {}
    for sample in samples:
        view = (round(float(sample["yaw"]) % 360, 3), round(float(sample["pitch"]), 3))
        if view in seen and sample["index_count"] > max_growth * max(seen[view], 1):
            return False
        seen[view] = sample["index_count"]
    return True


def matching_entities(entities: list[dict[str, Any]], spec: dict[str, Any]) -> list[dict[str, Any]]:
    """Received entities of `spec["entity_type"]` that satisfy every optional filter of an
    assert_entities step: feet inside `box` (x0,y0,z0,x1,y1,z1), standing no taller than
    `max_height` (a sleeping pose), showing the villager `profession`, and whose received
    DATA_BABY_ID equals `baby` (a row that never received it matches neither value)."""
    found = []
    for row in entities:
        if row.get("entity_type") != spec["entity_type"]:
            continue
        x, y, z = float(row["x"]), float(row["y"]), float(row["z"])
        box = spec.get("box")
        if box and not (box[0] <= x <= box[3] and box[1] <= y <= box[4] and box[2] <= z <= box[5]):
            continue
        if "max_height" in spec:
            bounds = row.get("bounding_box") or {}
            if float(bounds.get("max_y", 0)) - float(bounds.get("min_y", 0)) > spec["max_height"]:
                continue
        if "baby" in spec and row.get("received_baby") is not spec["baby"]:
            continue
        if "sleeping" in spec and received_sleeping(row.get("received_metadata") or {}) is not spec["sleeping"]:
            continue
        if "profession" in spec:
            data = ((row.get("received_metadata") or {}).get("19") or {}).get("value")
            if not (isinstance(data, list) and len(data) == 3 and isinstance(data[1], int)
                    and 0 <= data[1] < len(PROFESSIONS)
                    and PROFESSIONS[data[1]] == spec["profession"]):
                continue
        found.append(row)
    return found


def matching_walk(entities: dict[str, Any], spec: dict[str, Any]) -> dict[str, Any] | None:
    matches = [entity for entity in entities.get("entities", [])
               if entity.get("entity_type") == spec["entity_type"]]
    if len(matches) != 1:
        return None
    entity = matches[0]
    position, speed = entity.get("walk_position"), entity.get("walk_speed")
    if (type(position) not in (int, float) or type(speed) not in (int, float)
            or not math.isfinite(position) or not math.isfinite(speed)
            or speed < spec["min_speed"]):
        return None
    return entity if abs(position - spec["phase"]) <= spec["tolerance"] else None
