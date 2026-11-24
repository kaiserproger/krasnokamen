"""Scene-shot dispatcher: one validated step becomes client calls, waits, checks and the
retained frames the picture checks read.

The client/report seams live on ``scene_shot`` (the tests patch them there), so this
module reaches them as ``_qa.<name>`` and never binds them.
"""

from __future__ import annotations

import json
import math
import re
import time
from pathlib import Path
from typing import Any

from .probe_steps import ProbeSteps
from . import scene_shot as _qa
from .scene_shot_checks import _capture_dropped_clock
from .scene_shot_client import tool_wall_pixels, wait_play
from .scene_shot_steps import (
    SCENE_TOOLS, matching_entities, matching_walk, pixel_window_count,
    received_sleeping, render_index_bounded, render_metrics, repeated_view_stable, validate_steps,
)


def execute(probe: ProbeSteps, artifact: Path, steps: list[dict[str, Any]], client: str = "native") -> list[str]:
    validate_steps(steps, client)
    capture = _qa.capture_java if client == "java" else _qa.capture_native
    shots = []
    for index, step in enumerate(steps):
        if isinstance(probe, ProbeSteps):
            (artifact / "scene-progress.json").write_text(json.dumps({
                "step_index": index, "step": next(iter(step)), "started_unix": time.time(),
                "remaining_seconds": probe.remaining(),
            }) + "\n")
        if isinstance(step, dict) and set(step) == {"shot", "hold_keys"}:
            name, keys = step["shot"], step["hold_keys"]
            args = {"keys": keys, "ticks": 1, "hold": True}
            probe.call("minecraft_press_inputs", args)
            try:
                if "key.keyboard.tab" in keys:
                    probe.wait(lambda state: any(str(node.get("id", "")).startswith("tab/player/")
                                                  for node in state.get("ui_nodes", [])),
                               "tab_list_visible")
                shots.append(capture(probe, artifact, name))
            finally:
                probe.call("minecraft_press_inputs", {**args, "hold": False})
            continue
        kind, value = next(iter(step.items()))
        if kind in ("shot", "observe"):
            if kind == "shot":
                shots.append(capture(probe, artifact, value))
            else:
                (artifact / f"{value}.json").write_text(json.dumps(probe.observe(), indent=2) + "\n")
        elif kind == "dropped_clock_capture":
            shots.append(_capture_dropped_clock(probe, artifact, value))
        elif kind == "assert_same_game_time":
            before = json.loads((artifact / f"{value['before']}.json").read_text())
            after = json.loads((artifact / f"{value['after']}.json").read_text())
            before_age = (before.get("time") or {}).get("game_time")
            after_age = (after.get("time") or {}).get("game_time")
            if type(before_age) is not int or type(after_age) is not int:
                raise RuntimeError("client observation omitted game_time")
            probe.check("frozen_game_time_holds_across_teleport", before_age == after_age)
            before_day = (before.get("time") or {}).get("clock_time")
            after_day = (after.get("time") or {}).get("clock_time")
            if type(before_day) is not int or type(after_day) is not int:
                raise RuntimeError("client observation omitted clock_time")
            probe.check("received_fresh_day_time", after_day == before_day + value["day_delta"])
        elif kind == "wait_chat":
            probe.wait(
                lambda state: any(value in line for line in state["recent_chat"]),
                f"chat_received_{value}", wait_for_play=True,
            )
        elif kind == "assert_chat":
            state = probe.observe()
            lines = state.get("recent_chat")
            if not isinstance(lines, list) or any(not isinstance(line, str) for line in lines):
                raise RuntimeError("client observation omitted recent_chat")
            (artifact / f"{value['name']}.json").write_text(json.dumps(state, indent=2) + "\n")
            for index, text in enumerate(value["contains"]):
                probe.check(f"{value['name']}_contains_{index}", any(text in line for line in lines))
            for index, text in enumerate(value["excludes"]):
                probe.check(f"{value['name']}_excludes_{index}", not any(text in line for line in lines))
        elif kind == "assert_player_sleeping":
            state = probe.observe()
            metadata = (state.get("player") or {}).get("received_metadata")
            if not isinstance(metadata, dict):
                raise RuntimeError("client observation omitted received player metadata")
            sleeping = received_sleeping(metadata)
            (artifact / f"{value['name']}.json").write_text(json.dumps(state, indent=2) + "\n")
            probe.check(value["name"], sleeping is value["sleeping"])
        elif kind == "use_phases":
            name = value["name"]
            before = probe.observe()
            probe.check(f"{name}_initially_idle", before.get("using") is None)
            args = {"keys": ["use"], "ticks": 1, "hold": True}
            probe.call("minecraft_press_inputs", args)
            try:
                probe.call("minecraft_wait_ticks", {"ticks": 1})
                started = probe.observe()
                probe.check(f"{name}_started", started.get("using") is not None)
                (artifact / f"{name}-start.json").write_text(json.dumps(started, indent=2) + "\n")
                shots.append(capture(probe, artifact, f"{name}-start"))
                probe.call("minecraft_wait_ticks", {"ticks": value["ticks"]})
                held = probe.observe()
                probe.check(f"{name}_held", held.get("using") is not None)
                (artifact / f"{name}-hold.json").write_text(json.dumps(held, indent=2) + "\n")
                shots.append(capture(probe, artifact, f"{name}-hold"))
            finally:
                probe.call("minecraft_press_inputs", {**args, "hold": False})
            probe.call("minecraft_wait_ticks", {"ticks": 1})
            released = probe.observe()
            probe.check(f"{name}_released", released.get("using") is None)
            (artifact / f"{name}-release.json").write_text(json.dumps(released, indent=2) + "\n")
            shots.append(capture(probe, artifact, f"{name}-release"))
        elif kind == "resize":
            owned = getattr(probe.client, "p4_window", None)
            if not owned:
                raise RuntimeError("resize requires a harness-owned window")
            size = tuple(value["size"])
            _qa.resize_window(owned, size)
            probe.call("minecraft_wait_ticks", {"ticks": 5})
            path = capture(probe, artifact, value["name"])
            probe.check(f"{value['name']}_dimensions", _qa.png_size(path) == size)
            shots.append(path)
        elif kind == "reconnect":
            address = getattr(probe.client, "scene_address", None)
            if not address:
                raise RuntimeError("reconnect requires the harness-owned server address")
            probe.call("minecraft_connect", {"server_addr": address})
            wait_play(probe)
        elif kind == "server_eof":
            proxy = getattr(probe.client, "scene_proxy", None)
            if proxy is None:
                raise RuntimeError("EOF requires a harness-owned proxy")
            proxy.send_eof()
        elif kind == "wait_disconnect":
            while True:
                state = probe.call("minecraft_observe")
                phase = state.get("loading_readiness", {}).get("phase")
                if phase in ("disconnected", "error"):
                    break
                probe.call("minecraft_wait_for_state_change", {
                    "observed_version": state["state_version"],
                    "timeout_seconds": probe.remaining(),
                })
            probe.check(f"{value['name']}_left_play", state.get("in_play") is False)
            probe.check(f"{value['name']}_cleared_use", state.get("using") is None)
            if value["eof"]:
                probe.check(f"{value['name']}_eof", state.get("disconnect_reason") == "server closed connection")
            (artifact / f"{value['name']}.json").write_text(json.dumps(state, indent=2) + "\n")
        elif kind == "walk_capture":
            args = {"radius": 32, "limit": 128}
            attempts = []
            before = after = frame = None
            image = frame_path = first = None
            same_entity: list[dict[str, Any]] = []
            for _ in range(value["max_ticks"]):
                if first is None:
                    before = probe.call("minecraft_list_entities", args)
                    first = matching_walk(before, value)
                    # Start as soon as the observation enters the bracket.
                    if first is None:
                        probe.call("minecraft_wait_ticks", {"ticks": 1})
                        continue
                # The image is the last presented render, interpolated from the
                # previous tick: resample it instead of waiting a whole tick,
                # which under load can skip the requested phase entirely.
                image = capture(probe, artifact, f"{value['name']}-attempt-{len(attempts) + 1}")
                after = probe.call("minecraft_list_entities", args)
                frame_path = Path(image + ".gait.json") if client == "java" else Path(image).with_suffix(".json")
                frame_data = json.loads(frame_path.read_text())
                frame_entities = frame_data if client == "java" else frame_data["gait_frame"]
                frame = {"entities": frame_entities}
                same_entity = [entity for entity in after.get("entities", [])
                               if first.get("entity_uuid")
                               and entity.get("entity_uuid") == first["entity_uuid"]]
                attempts.append({"before": before, "after": after, "frame": frame,
                                 "image": image, "frame_path": str(frame_path)})
                if matching_walk(frame, value) is not None and len(same_entity) == 1:
                    break
                rendered = [entity for entity in frame_entities
                            if entity.get("entity_type") == value["entity_type"]]
                rendered_position = rendered[0].get("walk_position") if len(rendered) == 1 else None
                if (type(rendered_position) in (int, float) and math.isfinite(rendered_position)
                        and rendered_position > value["phase"] + value["tolerance"]):
                    # The render already passed the requested phase: no resample can
                    # land inside it, so report the miss instead of looping to the deadline.
                    (artifact / f"{value['name']}-walk.json").write_text(
                        json.dumps({"attempts": attempts, "spec": value}, indent=2) + "\n")
                    raise RuntimeError(f"walk phase not reached: {value['name']}")
            else:
                (artifact / f"{value['name']}-walk.json").write_text(
                    json.dumps({"attempts": attempts, "spec": value}, indent=2) + "\n")
                raise RuntimeError(f"walk phase not reached: {value['name']}")
            (artifact / f"{value['name']}-walk.json").write_text(
                json.dumps({"before": before, "after": after, "frame": frame,
                            "frame_path": str(frame_path), "spec": value,
                            "attempts": attempts}, indent=2) + "\n")
            probe.check(f"{value['name']}_same_entity", first is not None and len(same_entity) == 1)
            probe.check(f"{value['name']}_render_phase", matching_walk(frame, value) is not None)
            shots.append(image)
        elif kind == "falling_block_capture":
            from . import falling_block_oracle

            visible = probe.call("minecraft_wait_for_visible_entity", {
                "entity_type": "minecraft:falling_block", "radius": 32,
                "timeout_seconds": min(30, probe.remaining()),
            })
            identity = visible.get("entity_uuid")
            probe.check("falling_entity_received", visible.get("matched") is True and bool(identity))
            samples = []
            for index in range(12):
                image = capture(probe, artifact, f"{value}-{index}")
                shots.append(image)
                frame = json.loads(Path(image).with_suffix(".json").read_text())
                rows = [row for row in frame.get("gait_frame", [])
                        if row.get("entity_uuid") == identity and row.get("entity_type") == "minecraft:falling_block"]
                probe.check("falling_frame_identity", len(rows) == 1)
                samples.append({"frame": rows[0], "image": image})
                if len(samples) >= 2 and falling_block_oracle.same_packet(samples[-2]["frame"], samples[-1]["frame"]):
                    break
                probe.call("minecraft_wait_ticks", {"ticks": 1})
            else:
                probe.check("falling_between_packets_sampled", False)
            before, after = samples[-2:]
            probe.check("falling_between_packets_sampled", True)
            probe.check("falling_moves_down", after["frame"]["position"][1] < before["frame"]["position"][1] - 0.01)
            boxes = [falling_block_oracle.sand_bounds(Path(sample["image"])) for sample in (before, after)]
            probe.check("falling_sand_visible", all(box is not None for box in boxes))
            probe.check("falling_pixels_move_down", boxes[1][1] > boxes[0][1])
            (artifact / f"{value}-evidence.json").write_text(json.dumps({
                "entity_uuid": identity, "samples": samples, "sand_bounds": boxes,
            }, indent=2) + "\n")
        elif kind == "toggle_debug_overlay":
            # F3 is a dedicated client tool: the named-key list has no f3, so press it through it.
            result = probe.call("minecraft_toggle_debug_overlay", {"enabled": value})
            probe.report.setdefault("debug_overlay", []).append(
                {"enabled": value, "lines": result.get("lines"), "status": result.get("status")})
            probe.check(f"debug_overlay_{'on' if value else 'off'}",
                        result.get("debug_overlay") is value)
        elif kind == "render_draw_sample":
            # One fresh observation of the client's draw counters at the requested view. The yaw the
            # client reports proves the turn landed before the sample; a stale sample (same yaw as
            # the previous view) is not a direction sample.
            state = probe.observe()
            metrics = render_metrics(state)
            yaw = (state.get("player") or {}).get("yaw")
            if type(yaw) not in (int, float) or not math.isfinite(yaw):
                raise RuntimeError("client observation omitted player.yaw for a render sample")
            probe.report.setdefault("render_samples", []).append(
                {"name": value["name"], "yaw": float(yaw), "yaw_requested": value["yaw_deg"], **metrics})
            probe.check(f"render_sample_{value['name']}_yaw_observed",
                        abs((yaw - value["yaw_deg"] + 180) % 360 - 180) <= 1)
        elif kind == "assert_render_bounded":
            # Aggregate verdict over every preceding sample: the count must be per-frame bounded at
            # every view and must not grow when a view is sampled again or returned to.
            samples = probe.report.get("render_samples") or []
            views = {round(float(sample["yaw"]) % 360, 3) for sample in samples}
            if len(samples) < 3 or len(views) < 2 or len(views) == len(samples):
                raise RuntimeError("assert_render_bounded needs at least three samples over two views "
                                   "with one repeated view")
            (artifact / "render-samples.json").write_text(json.dumps(samples, indent=2) + "\n")
            probe.check("render_drawn_index_bounded",
                        render_index_bounded(samples, **{key: value[key] for key in
                                                         ("max_index_count", "max_growth", "min_index_count")}))
            probe.check("render_repeat_view_stable", repeated_view_stable(samples, max_growth=value["max_growth"]))
        elif kind == "pixel_window":
            from PIL import Image

            shot_path = artifact / "screenshots" / f"{value['shot']}.png"
            count = pixel_window_count(Image.open(shot_path), value["window"], value["color"])
            probe.report.setdefault("pixel_windows", []).append(
                {"name": value["name"], "window": value["window"], "color": value["color"], "count": count})
            probe.check(f"{value['name']}_pixels", count >= value["min_pixels"])
        elif kind == "goat_horns_upright":
            from . import goat_horn_oracle

            edges = goat_horn_oracle.horn_left_edges(artifact / "screenshots" / f"{value}.png")
            probe.report["goat_horn_left_edges"] = edges
            probe.check("goat_horns_upright", goat_horn_oracle.upright(edges))
            body_edges = goat_horn_oracle.body_left_edges(artifact / "screenshots" / f"{value}.png")
            probe.report["goat_body_left_edges"] = body_edges
            probe.check("goat_body_aligned", goat_horn_oracle.body_aligned(body_edges))
            ear_coverage = goat_horn_oracle.ear_lower_coverage(artifact / "screenshots" / f"{value}.png")
            probe.report["goat_ear_lower_coverage"] = ear_coverage
            probe.check("goat_ear_full_height", ear_coverage >= 25)
        elif kind == "goat_side_horn":
            from . import goat_horn_oracle

            edges = goat_horn_oracle.side_horn_left_edges(artifact / "screenshots" / f"{value}.png")
            probe.report["goat_side_horn_left_edges"] = edges
            probe.check("goat_side_horn_aligned", goat_horn_oracle.side_horn_aligned(edges))
        elif kind == "tool_hand_visible":
            # Away from the wall the pre-image draws the tool too: a control that hides it here is
            # not a wall-clipping control (review REVIEW-wave2-9 finding 1).
            state = probe.observe()
            target = state.get("target") or {}
            item = state.get("selected_item") or {}
            player = state.get("player") or {}
            eye = player.get("camera_position") or []
            hit = target.get("location") or {}
            distance = math.dist(eye, [hit.get(axis, math.inf) for axis in ("x", "y", "z")]) if len(eye) == 3 else math.inf
            tool, wall = tool_wall_pixels(artifact / "screenshots" / f"{value['shot']}.png")
            probe.report["tool_far_pixels"] = {"tool": tool, "wall": wall, "distance": distance}
            probe.check("tool_far_fixture", target.get("block_id") == "minecraft:obsidian"
                        and item.get("item_id") == "minecraft:diamond_pickaxe"
                        and value["min_distance"] <= distance <= value["max_distance"])
            probe.check("tool_hand_visible", tool >= 500)
        elif kind == "tool_wall_visible":
            state = probe.observe()
            target = state.get("target") or {}
            item = state.get("selected_item") or {}
            player = state.get("player") or {}
            eye = player.get("camera_position") or []
            hit = target.get("location") or {}
            distance = math.dist(eye, [hit.get(axis, math.inf) for axis in ("x", "y", "z")]) if len(eye) == 3 else math.inf
            tool, wall = tool_wall_pixels(artifact / "screenshots" / f"{value}.png")
            probe.report["tool_wall_pixels"] = {"tool": tool, "wall": wall, "distance": distance}
            probe.check("tool_wall_fixture", target.get("block_id") == "minecraft:obsidian"
                        and item.get("item_id") == "minecraft:diamond_pickaxe" and distance <= 0.6)
            probe.check("tool_wall_close_background", wall >= 10000)
            probe.check("tool_wall_visible", tool >= 500)
        elif kind == "death_tilt":
            from . import death_tilt_oracle

            screenshots = artifact / "screenshots"
            measured = {name: death_tilt_oracle.edges(screenshots / f"{value['empty']}.png",
                                                      screenshots / f"{name}.png")
                        for name in value["shots"]}
            probe.report["death_tilt_edges"] = measured
            for name, edges in measured.items():
                probe.check(f"death_tilt_{name}", death_tilt_oracle.matches(name, edges))
        elif kind == "cmd":
            wait_play(probe)
            probe.call("minecraft_send_chat", {"message": value, "command": True})
        elif kind == "upgrade_certified":
            # Upgrade the building the `Certified <kind> #<id>` reply named, then check the reply: the
            # id is a settlement/sequence pair, so a fixed scenario cannot spell it. `recent_chat` is
            # the client's last 20 lines, newest first: take the one distinct id, never an order.
            wait_play(probe)
            certified = re.compile(rf"Certified {value['kind']} #(\d+)")

            def certified_ids(state: dict[str, Any]) -> set[str]:
                return {match.group(1) for line in state.get("recent_chat") or []
                        if isinstance(line, str) and (match := certified.search(line))}

            probe.wait(lambda state: bool(certified_ids(state)),
                       f"{value['kind']}_certification_reply", wait_for_play=True,
                       timeout_seconds=30)
            ids = certified_ids(probe.observe())
            probe.check(f"{value['kind']}_one_certified_building", len(ids) == 1)
            (building,) = ids
            probe.call("minecraft_send_chat",
                       {"message": f"settlement building upgrade {building}", "command": True})
            probe.call("minecraft_wait_ticks", {"ticks": 20})
            reply = f"Upgraded building #{building} to level {value['level']}."
            probe.check(f"{value['kind']}_upgraded_to_level_{value['level']}",
                        any(isinstance(line, str) and reply in line
                            for line in probe.observe().get("recent_chat") or []))
        elif kind == "tp":
            probe.call("minecraft_send_chat", {"message": "tp @s " + " ".join(map(str, value)), "command": True})
            probe.wait(
                lambda state: state.get("player") is not None
                and abs(state["player"]["x"] - value[0]) < 1.5
                and abs(state["player"]["z"] - value[2]) < 1.5,
                "teleport_arrived", wait_for_play=True,
            )
        elif kind == "wait_ticks":
            remaining = value
            while remaining:
                ticks = min(remaining, 100)
                probe.call("minecraft_wait_ticks", {"ticks": ticks})
                remaining -= ticks
        elif kind == "pause":
            probe.call("minecraft_open_pause_menu", {})
        elif kind == "wait_play":
            wait_play(probe)
        elif kind == "wait_respawn":
            probe.wait(
                lambda state: state.get("player") is not None
                and state["player"].get("health") is not None
                and state["player"]["health"] > 0
                and state.get("screen", {}).get("open") is False,
                "respawn_alive_screen_closed", wait_for_play=True,
            )
        elif kind == "wait_screen":
            # Server-driven screens (dialogs, menus) open asynchronously; wait for
            # the client's own screen state whose class or title names `value`.
            probe.wait(lambda state: bool((state.get("screen") or {}).get("open"))
                       and (value in str(state["screen"].get("class", ""))
                            or value in str(state["screen"].get("title", ""))),
                       "screen_open")
        elif kind == "wait_loader_block":
            state = probe.wait(
                lambda state: (state.get("loader") or {}).get("acknowledged") is True
                and value in (state.get("loader") or {}).get("registered_blocks", []),
                "loader_block_registered",
            )
            probe.report.setdefault("loader_blocks", {})[value] = state["loader"]
        elif kind in ("key", "look", "hotbar", "use_item_on", "click_screen_button"):
            probe.call(SCENE_TOOLS[kind], value)
        elif kind == "wait_terrain":
            name, args = value.get("name"), value.get("args")
            sections = args["sections"]
            timeout = args.get("timeout_seconds", 30)
            result = probe.call(SCENE_TOOLS[kind], {
                "sections": sections, "timeout_seconds": min(timeout, probe.remaining()),
            })
            if result.get("status") != "ready":
                raise RuntimeError(f"minecraft_wait_for_terrain did not report ready: {result!r}")
            rows = result.get("sections")
            if (any(not isinstance(result.get(key), str) or not result[key]
                    for key in ("connection_epoch", "dimension_epoch", "resource_epoch", "frame_id"))
                    or not isinstance(rows, list) or len(rows) != len(sections)
                    or any(not isinstance(row, dict) or row.get("section") != section
                           or not isinstance(row.get("requested_revision"), str)
                           or not row["requested_revision"].isascii()
                           or not row["requested_revision"].isdigit()
                           or row.get("mesh_revision") != row["requested_revision"]
                           or type(row.get("selected_for_draw")) is not bool
                           for row, section in zip(rows, sections))):
                raise RuntimeError(f"invalid minecraft_wait_for_terrain receipt: {result!r}")
            receipt = artifact / f"{name}.json"
            receipt.write_text(json.dumps(result, indent=2) + "\n")
            probe.report.setdefault("terrain_waits", []).append({"receipt": str(receipt), "result": result})
        elif kind == "assert_entities":
            result = probe.call("minecraft_list_entities", value["args"])
            entities = result.get("entities")
            if not isinstance(entities, list) or any(not isinstance(row, dict) for row in entities):
                raise RuntimeError("entity snapshot omitted entities")
            if result.get("truncated") is not False:
                raise RuntimeError("entity snapshot is truncated or omitted truncation status")
            if type(result.get("game_time")) is not int:
                raise RuntimeError("entity snapshot omitted same-thread game_time")
            found = matching_entities(entities, value)
            (artifact / f"{value['name']}.json").write_text(json.dumps(
                {"game_time": result["game_time"], "spec": value, "matched": found,
                 "received": entities}, indent=2) + "\n")
            probe.check(f"{value['name']}_min", len(found) >= value["min"])
            if "max" in value:
                probe.check(f"{value['name']}_max", len(found) <= value["max"])
        elif kind == "require_entities":
            result = probe.call("minecraft_list_entities", value["args"])
            (artifact / f"{value['name']}.json").write_text(json.dumps(result, indent=2) + "\n")
            entities = result.get("entities")
            if not isinstance(entities, list) or any(not isinstance(row, dict) for row in entities):
                raise RuntimeError("entity snapshot omitted entities")
            present = {row.get("entity_type") for row in entities}
            for entity_type in value["types"]:
                probe.check(f"{value['name']}_{entity_type}", entity_type in present)
        elif kind == "timed_entities":
            result = probe.call("minecraft_list_entities", value["args"])
            if type(result.get("game_time")) is not int:
                raise RuntimeError("entity snapshot omitted same-thread game_time")
            receipt = {"game_time": result["game_time"], "snapshot": result}
            (artifact / f"{value['name']}.json").write_text(json.dumps(receipt, indent=2) + "\n")
        elif kind == "mob_motion":
            from . import mob_motion_oracle

            samples = [json.loads((artifact / f"{item}.json").read_text()) for item in value["samples"]]
            # Only the kinds this window drives: asserting another kind's motion in a
            # window that leaves it standing reads its standstill as a client defect.
            kinds = [entry if isinstance(entry, str) else entry["id"] for entry in value["types"]]
            measurement = mob_motion_oracle.measure(samples, kinds)
            (artifact / f"{value['name']}.json").write_text(json.dumps(measurement, indent=2) + "\n")
            for entry in value["types"]:
                kind = entry if isinstance(entry, str) else entry["id"]
                slug = kind.split(":")[1]
                row = measurement[kind]
                probe.check("mob_moved_" + slug, row["total_displacement_blocks"] > 0.1)
                if isinstance(entry, dict):
                    # A displacement alone passes a frozen or wrongly paced gait: every
                    # segment's client-visible rate must sit inside the declared range.
                    rates = [segment["blocks_per_client_tick"] for segment in row["segments"]]
                    probe.check("mob_speed_" + slug,
                                all(entry["min_speed"] <= rate <= entry["max_speed"] for rate in rates))
        elif kind == "assert_block":
            result = probe.call("minecraft_read_block", value["args"])
            (artifact / f"{value['name']}.json").write_text(json.dumps(result, indent=2) + "\n")
            expect = value["expect"]
            matched = (result.get("is_air") is True if expect == "minecraft:air"
                       else result.get("block_id") == expect)
            probe.check(value["name"], matched)
        elif kind == "assert_screen":
            shown = probe.observe()
            screen = shown.get("screen") or {}
            texts = [str(screen.get("title") or "")]
            texts.extend(str(node["label"]) for node in shown.get("ui_nodes") or [] if node.get("label"))
            texts.extend(str(row["name"]) for row in (shown.get("container") or {}).get("slots", [])
                         if row.get("name"))
            (artifact / f"{value['name']}.json").write_text(json.dumps(
                {"screen": screen, "texts": texts}, indent=2) + "\n")
            blob = "\n".join(texts)
            for index, text in enumerate(value["contains"]):
                probe.check(f"{value['name']}_contains_{index}", text in blob)
            for index, text in enumerate(value["excludes"]):
                probe.check(f"{value['name']}_excludes_{index}", text not in blob)
        elif kind in ("scan_blocks", "read_block", "navigate_to_block", "list_entities"):
            name, args = value.get("name"), value.get("args")
            result = probe.call(SCENE_TOOLS[kind], args)
            (artifact / f"{name}.json").write_text(json.dumps(result, indent=2) + "\n")
        else:
            raise ValueError(f"invalid scene step: {step!r}")
    return shots
