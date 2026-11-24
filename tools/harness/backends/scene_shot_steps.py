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
}


def scene_mcp_calls(steps: list[dict[str, Any]]):
    """Yield the input-derived MCP calls before launch for schema validation."""
    for step in steps:
        if set(step) == {"shot", "hold_keys"}:
            yield "minecraft_press_inputs", {"keys": step["hold_keys"], "ticks": 1, "hold": True}
            continue
        kind, value = next(iter(step.items()))
        if kind in ("key", "look", "hotbar", "use_item_on", "click_screen_button"):
            yield SCENE_TOOLS[kind], value
        elif kind == "assert_block":
            yield "minecraft_read_block", value["args"]
        elif kind == "assert_screen":
            yield "minecraft_observe", {}
        elif kind in ("scan_blocks", "read_block", "navigate_to_block", "list_entities", "wait_terrain"):
            yield SCENE_TOOLS[kind], value["args"]
        elif kind in ("timed_entities", "require_entities", "assert_entities"):
            yield "minecraft_list_entities", value["args"]
        elif kind == "attack_entity":
            yield "minecraft_list_entities", {"radius": 32, "limit": 128}
            yield "minecraft_approach_entity", {"entity_id": 1, "timeout_seconds": min(value["timeout_seconds"], 30)}
            yield "minecraft_attack_entity_once", {"entity_id": 1, "entity_uuid": "00000000-0000-0000-0000-000000000000",
                                                  "entity_type": value["entity_type"],
                                                  "timeout_seconds": min(value["timeout_seconds"], 30)}
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
        elif kind == "assert_render_bounded":
            valid = (round(value, 12) >= 0) if isinstance(value, float) else True
        elif kind == "key":
            valid = isinstance(value, dict) and set(value) == {"keys", "ticks"}
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
            valid = (isinstance(value, dict) and set(value) == {"samples", "name", "types"}
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
        elif kind == "attack_entity":
            valid = (client == "native" and isinstance(value, dict)
                     and {"name", "entity_type", "timeout_seconds"} <= set(value)
                     and set(value) <= {"name", "entity_type", "timeout_seconds", "expect_item",
                                        "attempts"}
                     and (type(value.get("attempts", 2)) is int and 1 <= value.get("attempts", 2) <= 3)
                     and isinstance(value["name"], str)
                     and re.fullmatch(r"[A-Za-z0-9_-]+", value["name"])
                     and isinstance(value["entity_type"], str)
                     and re.fullmatch(r"[a-z0-9_.-]+:[a-z0-9_./-]+", value["entity_type"])
                     and type(value["timeout_seconds"]) in (int, float)
                     and math.isfinite(value["timeout_seconds"])
                     and 0.1 <= value["timeout_seconds"] <= 120
                     and ("expect_item" not in value
                          or (isinstance(value["expect_item"], str)
                              and bool(re.fullmatch(r"[a-z0-9_.-]+:[a-z0-9_./-]+",
                                                    value["expect_item"])))))
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
    return metrics


def bounded_draw(metrics, samples, *, max_index_count, max_growth) -> bool:
    """True when the drawn index count stays bounded across samples instead of accumulating.

    `samples` are the earlier render_metrics dicts of the same run: a count that exceeds both the
    absolute bound and `max_growth` times the first sample means indices accumulate with camera
    movement (the ISSUE-1-03 symptom) rather than being re-submitted per frame."""
    if not samples:
        return metrics["index_count"] <= max_index_count
    first = samples[0]["index_count"]
    return (metrics["index_count"] <= max_index_count
            and metrics["index_count"] <= max_growth * max(first, 1))


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
