"""Script a windowed native or vanilla-Java client scene on an isolated v2 world."""
from __future__ import annotations

import asyncio
import ctypes
import ctypes.util
import hashlib
import json
import math
import os
import re
import secrets
import shutil
import subprocess
import threading
from collections.abc import Callable
from pathlib import Path
from typing import Any

from .. import runtime
from ..mcp import McpClient
from ..profiles import ProfileContext
from . import java_p5, native_client
from .gameplay import prepare_operator_fixture
from .probe_steps import ProbeSteps

# Both clients render the same fixed window; the vanilla client is only a
# rendering oracle, so its camera/GUI options are pinned to the native defaults.
WINDOW = (1280, 720)
NATIVE_FOV = 70
NATIVE_GUI_SCALE = 3

USERNAME = "SceneShot"

def scene_username(scenario: dict[str, Any]) -> str:
    name = scenario.get("username", USERNAME)
    if not isinstance(name, str) or re.fullmatch(r"[A-Za-z0-9_]{1,16}", name) is None:
        raise ValueError("username must be 1..16 ASCII letters, digits or underscores")
    return name


class LatencyProxy:
    """Delay traffic in each direction without changing host networking."""

    def __init__(self, target_port: int, delay_ms: int):
        self.target_port = target_port
        self.delay = delay_ms / 1000
        self.loop = asyncio.new_event_loop()
        self.ready = threading.Event()
        self.error = None
        self.port = None
        self.downstreams = []
        self.eof_writers = set()
        self.thread = threading.Thread(target=self._run, daemon=True)

    async def _forward(self, reader, writer):
        while data := await reader.read(65536):
            ready = self.loop.create_future()
            timer = self.loop.call_later(self.delay, ready.set_result, None)
            try:
                await ready
            finally:
                timer.cancel()
            if writer in self.eof_writers:
                await asyncio.Future()
            writer.write(data)
            await writer.drain()

    async def _connection(self, reader, writer):
        self.downstreams.append(writer)
        upstream = None
        try:
            upstream_reader, upstream = await asyncio.open_connection("127.0.0.1", self.target_port)
            tasks = [
                asyncio.create_task(self._forward(reader, upstream)),
                asyncio.create_task(self._forward(upstream_reader, writer)),
            ]
            _, pending = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
            for task in pending:
                task.cancel()
            await asyncio.gather(*tasks, return_exceptions=True)
        finally:
            self.downstreams.remove(writer)
            self.eof_writers.discard(writer)
            writer.close()
            if upstream is not None:
                upstream.close()

    def _run(self):
        asyncio.set_event_loop(self.loop)
        try:
            server = self.loop.run_until_complete(asyncio.start_server(self._connection, "127.0.0.1", 0))
            self.port = server.sockets[0].getsockname()[1]
        except Exception as error:
            self.error = error
            self.ready.set()
            self.loop.close()
            return
        self.ready.set()
        self.loop.run_forever()
        server.close()
        self.loop.run_until_complete(server.wait_closed())
        tasks = asyncio.all_tasks(self.loop)
        for task in tasks:
            task.cancel()
        self.loop.run_until_complete(asyncio.gather(*tasks, return_exceptions=True))
        self.loop.close()

    def start(self):
        self.thread.start()
        self.ready.wait()
        if self.error is not None:
            raise RuntimeError("starting latency proxy failed") from self.error
        return self.port

    def send_eof(self):
        async def close_write():
            if not self.downstreams:
                raise RuntimeError("EOF requires a connected client")
            for writer in tuple(self.downstreams):
                self.eof_writers.add(writer)
                writer.write_eof()
                await writer.drain()
        asyncio.run_coroutine_threadsafe(close_write(), self.loop).result(timeout=8)

    def close(self):
        if self.loop.is_running():
            self.loop.call_soon_threadsafe(self.loop.stop)
        self.thread.join()


def java_options(render_distance: int, fov: int) -> str:
    # Java stores fov normalized: 70 -> 0.0 and 110 -> 1.0.
    return (
        "version:4790\nonboardAccessibility:false\n"
        'graphicsPreset:"custom"\n'
        f"overrideWidth:{WINDOW[0]}\noverrideHeight:{WINDOW[1]}\n"
        f"fov:{(fov - 70) / 40}\nguiScale:{NATIVE_GUI_SCALE}\nrenderDistance:{render_distance}\n"
        f"simulationDistance:{min(render_distance, 12)}\nmaxFps:60\nenableVsync:false\n"
        "pauseOnLostFocus:false\ntutorialStep:none\nsoundCategory_master:0.0\n"
    )


def check_java_render_distance(options_path: Path, requested: int) -> None:
    values = [line.partition(":")[2] for line in options_path.read_text().splitlines()
              if line.startswith("renderDistance:")]
    if values != [str(requested)]:
        raise RuntimeError(f"Java renderDistance is {values!r}, expected {requested} in {options_path}")


def find_java_window(display: str) -> int | None:
    """The harness Xvfb hosts only this client: its largest top-level window is the game."""
    tree = subprocess.run(["xwininfo", "-display", display, "-root", "-children"], check=True,
                          capture_output=True, text=True).stdout
    windows = [(int(w) * int(h), int(ident, 16))
               for ident, w, h in re.findall(r"^\s+(0x[0-9a-f]+) .*?\s(\d+)x(\d+)\+-?\d+\+-?\d+", tree, re.M)]
    return max(windows)[1] if windows else None


def resize_window(owned: dict[str, Any], size: tuple[int, int]) -> None:
    """Resize only the window owned by this harness Xvfb."""
    x = ctypes.CDLL(ctypes.util.find_library("X11"))
    x.XOpenDisplay.restype = ctypes.c_void_p
    x.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x.XResizeWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_uint, ctypes.c_uint]
    x.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
    x.XCloseDisplay.argtypes = [ctypes.c_void_p]
    connection = x.XOpenDisplay(owned["display"].encode())
    if not connection:
        raise RuntimeError("cannot open the scenario's private Xvfb")
    try:
        x.XResizeWindow(connection, owned["window_id"], *size)
        x.XSync(connection, 0)
    finally:
        x.XCloseDisplay(connection)


def png_size(path: str) -> tuple[int, int]:
    with open(path, "rb") as image:
        header = image.read(24)
    return int.from_bytes(header[16:20], "big"), int.from_bytes(header[20:24], "big")


def capture_java(probe: ProbeSteps, artifact: Path, name: str) -> str:
    path = java_p5.capture(probe, name)
    if png_size(path) != WINDOW:
        raise RuntimeError(f"Java screenshot {path} is {png_size(path)}, expected {WINDOW}")
    return path


def capture_native(probe: ProbeSteps, artifact: Path, name: str) -> str:
    return native_client.capture(probe.client, artifact, name)["retained_path"]


@runtime.in_phase("join")
def wait_play(probe: ProbeSteps) -> None:
    # Each MCP wait is capped at 120s; keep the same overall scenario deadline.
    while True:
        result = probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
        if result.get("in_play") is True:
            return
        if result.get("disconnect_reason"):
            raise RuntimeError(f"client disconnected before play: {result['disconnect_reason']}")


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
        elif kind in ("scan_blocks", "read_block", "navigate_to_block", "list_entities", "wait_terrain"):
            yield SCENE_TOOLS[kind], value["args"]
        elif kind == "timed_entities":
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
        if kind in ("shot", "observe", "goat_horns_upright", "goat_side_horn"):
            valid = isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9_-]+", value)
        elif kind == "assert_chat":
            valid = (client == "native" and isinstance(value, dict)
                     and set(value) == {"name", "contains", "excludes"}
                     and isinstance(value["name"], str)
                     and re.fullmatch(r"[A-Za-z0-9_-]+", value["name"])
                     and all(isinstance(value[key], list)
                             and all(isinstance(item, str) and bool(item) for item in value[key])
                             for key in ("contains", "excludes"))
                     and bool(value["contains"]))
        elif kind == "cmd":
            valid = isinstance(value, str) and bool(value) and not value.startswith("/")
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
        elif kind == "wait_screen":
            valid = isinstance(value, str) and bool(value)
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
        elif kind == "mob_motion":
            valid = (isinstance(value, dict) and set(value) == {"samples", "name"}
                     and isinstance(value["name"], str) and re.fullmatch(r"[A-Za-z0-9_-]+", value["name"])
                     and isinstance(value["samples"], list) and len(value["samples"]) == 3
                     and all(isinstance(item, str) and re.fullmatch(r"[A-Za-z0-9_-]+", item)
                             for item in value["samples"]))
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


def execute(probe: ProbeSteps, artifact: Path, steps: list[dict[str, Any]], client: str = "native") -> list[str]:
    validate_steps(steps, client)
    capture = capture_java if client == "java" else capture_native
    shots = []
    for step in steps:
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
            resize_window(owned, size)
            probe.call("minecraft_wait_ticks", {"ticks": 5})
            path = capture(probe, artifact, value["name"])
            probe.check(f"{value['name']}_dimensions", png_size(path) == size)
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
            for _ in range(value["max_ticks"]):
                before = probe.call("minecraft_list_entities", args)
                candidate = matching_walk(before, value)
                # The observation samples the end of the tick; rendering
                # interpolates from the previous tick's smaller walk position.
                if candidate and candidate["walk_position"] >= value["phase"]:
                    break
                probe.call("minecraft_wait_ticks", {"ticks": 1})
            else:
                raise RuntimeError(f"walk phase not reached: {value['name']}")
            image = capture(probe, artifact, value["name"])
            after = probe.call("minecraft_list_entities", args)
            frame_path = Path(image + ".gait.json") if client == "java" else Path(image).with_suffix(".json")
            frame_data = json.loads(frame_path.read_text())
            frame_entities = frame_data if client == "java" else frame_data["gait_frame"]
            frame = {"entities": frame_entities}
            first = matching_walk(before, value)
            same_entity = [entity for entity in after.get("entities", [])
                           if first is not None and first.get("entity_uuid")
                           and entity.get("entity_uuid") == first["entity_uuid"]]
            (artifact / f"{value['name']}-walk.json").write_text(
                json.dumps({"before": before, "after": after, "frame": frame,
                            "frame_path": str(frame_path), "spec": value}, indent=2) + "\n")
            probe.check(f"{value['name']}_same_entity", first is not None and len(same_entity) == 1)
            probe.check(f"{value['name']}_render_phase", matching_walk(frame, value) is not None)
            shots.append(image)
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
        elif kind == "cmd":
            wait_play(probe)
            probe.call("minecraft_send_chat", {"message": value, "command": True})
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
        elif kind == "timed_entities":
            result = probe.call("minecraft_list_entities", value["args"])
            if type(result.get("game_time")) is not int:
                raise RuntimeError("entity snapshot omitted same-thread game_time")
            receipt = {"game_time": result["game_time"], "snapshot": result}
            (artifact / f"{value['name']}.json").write_text(json.dumps(receipt, indent=2) + "\n")
        elif kind == "mob_motion":
            from . import mob_motion_oracle

            samples = [json.loads((artifact / f"{item}.json").read_text()) for item in value["samples"]]
            measurement = mob_motion_oracle.measure(samples, ["minecraft:zombie", "minecraft:spider"])
            (artifact / f"{value['name']}.json").write_text(json.dumps(measurement, indent=2) + "\n")
            for entity, row in measurement.items():
                probe.check("mob_moved_" + entity.split(":")[1], row["total_displacement_blocks"] > 0.1)
        elif kind in ("scan_blocks", "read_block", "navigate_to_block", "list_entities"):
            name, args = value.get("name"), value.get("args")
            result = probe.call(SCENE_TOOLS[kind], args)
            (artifact / f"{name}.json").write_text(json.dumps(result, indent=2) + "\n")
        else:
            raise ValueError(f"invalid scene step: {step!r}")
    return shots


def check_263_entity_rigs(probe: ProbeSteps, artifact: Path) -> None:
    kinds = ("minecraft:sulfur_cube", "minecraft:cushion",
             "minecraft:poplar_boat", "minecraft:poplar_chest_boat")
    received = json.loads((artifact / "entities.json").read_text())
    for kind in kinds:
        probe.check(f"received_{kind}", sum(row.get("entity_type") == kind
                                            for row in received["entities"]) == 1)
    frame = json.loads((artifact / "screenshots/entity-rigs.json").read_text())
    before_frame = json.loads((artifact / "screenshots/before-rigs.json").read_text())
    probe.check("rig_frames_gpu_complete", before_frame["gpu_completion"] is True and frame["gpu_completion"] is True)
    probe.check("rig_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(before_frame["camera_position"], frame["camera_position"])
    ) and abs(before_frame["yaw"] - frame["yaw"]) < 0.01
      and abs(before_frame["pitch"] - frame["pitch"]) < 0.01)
    unrendered = frame["entity_extraction"]["unrendered"]
    for kind in kinds:
        probe.check(f"rendered_{kind}", kind not in unrendered)
        row = next(row for row in received["entities"] if row.get("entity_type") == kind)
        probe.check(f"resolved_{kind}", row.get("cached_rig_status") == "resolved")
    probe.check("four_entities_submitted", frame["entity_extraction"]["submitted"] >= 4)
    from PIL import Image
    with Image.open(artifact / "screenshots/before-rigs.png") as before_image, \
            Image.open(artifact / "screenshots/entity-rigs.png") as after_image:
        before, after = before_image.convert("RGB"), after_image.convert("RGB")
        probe.check("matched_rig_frames", before.size == after.size == (1280, 720))
        # Fixed camera, flat platform and separate boxes isolate each received entity.
        # The two pale foreground models must brighten the platform, not merely cast a shadow.
        regions = {
            "cushion": ((430, 520, 580, 560), True),
            "sulfur_cube": ((740, 495, 805, 545), True),
            "poplar_chest_boat": ((410, 415, 550, 480), False),
            "poplar_boat": ((720, 415, 860, 480), False),
        }
        for name, (box, bright) in regions.items():
            old = before.crop(box).getdata()
            new = after.crop(box).getdata()
            pixels = sum(
                all(y > x + 25 for x, y in zip(a, b)) if bright
                else max(abs(x - y) for x, y in zip(a, b)) > 25
                for a, b in zip(old, new)
            )
            probe.report[f"{name}_changed_pixels"] = pixels
            probe.check(f"visible_pixels_{name}", pixels >= 200)
    probe.report["visual_review_required"] = ["Inspect all four models in entity-rigs.png; pixel changes alone do not prove model parity."]

def check_shelf_clock(probe: ProbeSteps, artifact: Path) -> None:
    frames = [
        json.loads((artifact / f"screenshots/clocks-{phase}.json").read_text())
        for phase in ("day", "night")
    ]
    day, night = frames
    probe.check("clock_frames_gpu_complete", all(frame["gpu_completion"] is True for frame in frames))
    probe.check("clock_camera_at_fixture", all(
        abs(a - b) < 0.01 for a, b in zip(day["camera_position"], (1000.5, 161.62, 1000.5))
    ) and abs(day["yaw"]) < 0.01 and abs(day["pitch"]) < 0.01)
    probe.check("clock_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(day["camera_position"], night["camera_position"])
    ) and abs(day["yaw"] - night["yaw"]) < 0.01 and abs(day["pitch"] - night["pitch"]) < 0.01)
    probe.check("clock_received_time_changed",
                day["environment"]["time_of_day"] == "1000"
                and night["environment"]["time_of_day"] == "13000")
    from PIL import Image
    masks = []
    for phase in ("day", "night"):
        with Image.open(artifact / f"screenshots/clocks-{phase}.png") as image:
            probe.check(f"clock_{phase}_image_size", image.size == (1280, 720))
            pixels = image.convert("RGB").crop((435, 457, 453, 481)).getdata()
            masks.append([b > r * 1.35 and b > g * 1.15 and b > 15 for r, g, b in pixels])
    probe.report["clock_blue_pixels"] = [sum(mask) for mask in masks]
    changes = sum(a != b for a, b in zip(*masks))
    probe.report["clock_changed_blue_pixels"] = changes
    probe.check("clock_face_visible", all(sum(mask) >= 20 for mask in masks))
    probe.check("clock_face_changed", changes >= 20)

def _load_shots(probe: ProbeSteps, artifact: Path, names: tuple[str, ...]) -> dict:
    from PIL import Image
    shots = {}
    for name in names:
        frame = json.loads((artifact / f"screenshots/{name}.json").read_text())
        probe.check(f"{name}_gpu_complete", frame.get("gpu_completion") is True)
        with Image.open(artifact / f"screenshots/{name}.png") as image:
            probe.check(f"{name}_image_size", image.size == (1280, 720))
            shots[name] = (frame, image.convert("RGB"))
    return shots


def _changed(a, b, box) -> int:
    return sum(
        max(abs(x - y) for x, y in zip(p, q)) > 25
        for p, q in zip(a.crop(box).getdata(), b.crop(box).getdata())
    )


def _count(image, box, pred) -> int:
    return sum(1 for p in image.crop(box).getdata() if pred(*p))


def check_entity_visuals(probe: ProbeSteps, artifact: Path) -> None:
    """MODEL-OPUS-12, MODEL-OPUS-16, MODEL-ENTITY-FIRE, MODEL-ENTITY-LEASH."""
    names = ("empty", "vindicator", "ghastling", "burning", "plain", "leashed")
    shots = _load_shots(probe, artifact, names)
    ref = shots["empty"][0]
    for name in names[1:]:
        frame = shots[name][0]
        probe.check(f"{name}_camera_unchanged", all(
            abs(a - b) < 0.01 for a, b in zip(ref["camera_position"], frame["camera_position"])
        ) and abs(ref["yaw"] - frame["yaw"]) < 0.01 and abs(ref["pitch"] - frame["pitch"]) < 0.01)
    maroon = lambda r, g, b: r > g + 25 and r > b + 25
    teal = lambda r, g, b: g > r + 15 and b > r + 15 and b < 210
    pink = lambda r, g, b: r - b > 40 and r > 120
    white = lambda r, g, b: r > 170 and g > 170 and b > 170 and max(r, g, b) - min(r, g, b) < 35
    orange = lambda r, g, b: r > 150 and 60 < g < 210 and b < 110
    rope = lambda r, g, b: 60 < r < 140 and 40 < g < 100 and b < 70 and r > g > b
    vindicator = shots["vindicator"][1]
    probe.check("vindicator_visible", _changed(shots["vindicator"][1], shots["empty"][1], (540, 280, 760, 580)) > 500)
    probe.check("vindicator_torso_not_maroon", _count(vindicator, (610, 400, 680, 475), maroon) < 50)
    arm_pixels = sum(_count(vindicator, box, teal) for box in ((545, 390, 600, 470), (690, 390, 735, 470)))
    probe.report["vindicator_teal_arm_pixels"] = arm_pixels
    probe.check("vindicator_arms_not_teal", arm_pixels < 50)
    leg_teal = _count(vindicator, (600, 480, 690, 560), teal)
    probe.report["vindicator_teal_leg_pixels"] = leg_teal
    probe.check("vindicator_legs_teal", leg_teal > 150)
    ghast_box = (560, 260, 780, 470)
    pink_n = _count(shots["ghastling"][1], ghast_box, pink)
    white_excess = _count(shots["ghastling"][1], ghast_box, white) - _count(shots["empty"][1], ghast_box, white)
    probe.report["ghastling_pink_pixels"] = pink_n
    probe.report["ghastling_white_excess"] = white_excess
    probe.check("ghastling_not_pink", pink_n < 500)
    probe.check("ghastling_body_white", white_excess > 3000)
    fire_box = (560, 300, 760, 520)
    burning_orange = _count(shots["burning"][1], fire_box, orange)
    plain_orange = _count(shots["plain"][1], fire_box, orange)
    probe.report["fire_orange_pixels"] = [plain_orange, burning_orange]
    probe.check("fire_overlay_only_when_burning", burning_orange - plain_orange > 4000)
    probe.check("fire_overlay_present", burning_orange > 5000)
    rope_n = _count(shots["leashed"][1], (560, 400, 760, 500), rope)
    probe.report["leash_rope_pixels"] = rope_n
    probe.check("leash_rope_drawn", rope_n > 300)
    probe.report["visual_review_required"] = [
        "Inspect vindicator/ghastling/burning/plain/leashed frames; pixel predicates do not prove full model parity."
    ]


def check_item_bars(probe: ProbeSteps, artifact: Path) -> None:
    """MODEL-ITEM-BARS: durability bar and bundle fullness bar in the GUI hotbar."""
    shots = _load_shots(probe, artifact, ("bars-damaged", "bars-clean"))
    ref = shots["bars-damaged"][0]
    frame = shots["bars-clean"][0]
    probe.check("bars_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(ref["camera_position"], frame["camera_position"])
    ) and abs(ref["yaw"] - frame["yaw"]) < 0.01 and abs(ref["pitch"] - frame["pitch"]) < 0.01)
    yellow = lambda r, g, b: r > 70 and g > 70 and b < r - 20 and b < g - 10
    blue = lambda r, g, b: b > r + 25 and b > g + 10 and b > 70
    damaged, clean = shots["bars-damaged"][1], shots["bars-clean"][1]
    slot0, slot2 = (376, 664, 420, 712), (420, 664, 464, 712)
    probe.report["pickaxe_slot_changed_pixels"] = _changed(damaged, clean, slot0)
    probe.report["bundle_slot_changed_pixels"] = _changed(damaged, clean, slot2)
    probe.check("pickaxe_slot_bar_changes", _changed(damaged, clean, slot0) > 50)
    probe.check("bundle_slot_bar_changes", _changed(damaged, clean, slot2) > 50)
    probe.report["durability_bar_pixels"] = _count(damaged, (376, 698, 420, 710), yellow)
    probe.report["fullness_bar_pixels"] = _count(damaged, (420, 698, 464, 710), blue)
    probe.check("durability_bar_drawn", _count(damaged, (376, 698, 420, 710), yellow) > 20)
    probe.check("fullness_bar_drawn", _count(damaged, (420, 698, 464, 710), blue) > 15)


def check_shade_override(probe: ProbeSteps, artifact: Path) -> None:
    """MODEL-SHADE-OVERRIDE: cross plants get no directional face shading in 26.3."""
    shots = _load_shots(probe, artifact, ("shade-north", "shade-east"))
    north_f, east_f = shots["shade-north"][0], shots["shade-east"][0]
    probe.check("shade_north_camera", all(
        abs(a - b) < 0.01 for a, b in zip(north_f["camera_position"], (1000.5, 161.62, 1000.5))
    ) and abs(north_f["yaw"]) < 0.01)
    probe.check("shade_east_camera", all(
        abs(a - b) < 0.01 for a, b in zip(east_f["camera_position"], (1004.5, 161.62, 1004.5))
    ) and abs(east_f["yaw"] - 90.0) < 0.01)
    plant = lambda r, g, b: (r > 120 and g < 100 and b < 100) or (g > 90 and r < 120 and b < 120)
    box = (560, 440, 720, 580)
    lums = []
    counts = []
    for name in ("shade-north", "shade-east"):
        image = shots[name][1]
        pixels = [p for p in image.crop(box).getdata() if plant(*p)]
        counts.append(len(pixels))
        lums.append(sum(sum(p) for p in pixels) / len(pixels) / 3 if pixels else 0.0)
    probe.report["poppy_plant_pixels"] = counts
    probe.report["poppy_mean_luminance"] = [round(v, 2) for v in lums]
    probe.check("poppy_visible_both_views", all(c > 3000 for c in counts))
    probe.check("poppy_no_directional_shading", abs(lums[0] - lums[1]) < 8.0)


def check_player_head(probe: ProbeSteps, artifact: Path) -> None:
    """MODEL-OPUS-N3: placed and worn player heads render (hat layer needs an online skin)."""
    shots = _load_shots(probe, artifact, ("head-empty", "head-block", "head-worn", "head-none"))
    empty_f, block_f = shots["head-empty"][0], shots["head-block"][0]
    probe.check("head_front_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(empty_f["camera_position"], block_f["camera_position"])
    ) and abs(empty_f["yaw"] - block_f["yaw"]) < 0.01 and abs(empty_f["pitch"] - block_f["pitch"]) < 0.01)
    worn_f, none_f = shots["head-worn"][0], shots["head-none"][0]
    probe.check("head_back_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(worn_f["camera_position"], none_f["camera_position"])
    ) and abs(worn_f["yaw"] - none_f["yaw"]) < 0.01 and abs(worn_f["pitch"] - none_f["pitch"]) < 0.01)
    placed = _changed(shots["head-block"][1], shots["head-empty"][1], (880, 520, 1080, 680))
    worn = _changed(shots["head-worn"][1], shots["head-none"][1], (560, 280, 760, 420))
    probe.report["placed_head_changed_pixels"] = placed
    probe.report["worn_head_changed_pixels"] = worn
    probe.check("placed_head_renders", placed > 100)
    probe.check("worn_head_renders", worn > 500)

def check_dynamic_context(probe: ProbeSteps, artifact: Path) -> None:
    """MODEL-DYNAMIC-CONTEXT: shelf, dropped and held clocks plus two shelf
    compasses at different positions all receive world context."""
    import math
    shots = _load_shots(probe, artifact, ("ctx-day", "ctx-night", "compass-day"))
    ref = shots["ctx-day"][0]
    for name in ("ctx-night", "compass-day"):
        frame = shots[name][0]
        probe.check(f"{name}_camera_unchanged", all(
            abs(a - b) < 0.01 for a, b in zip(ref["camera_position"], frame["camera_position"])
        ) and abs(ref["yaw"] - frame["yaw"]) < 0.01 and abs(ref["pitch"] - frame["pitch"]) < 0.01)
    probe.check("ctx_received_time_changed",
                shots["ctx-day"][0]["environment"]["time_of_day"] == "1000"
                and shots["ctx-night"][0]["environment"]["time_of_day"] == "13000"
                and shots["compass-day"][0]["environment"]["time_of_day"] == "1000")
    def needle_angle(points, w, h):
        cx = sum(p[0] for p in points) / len(points)
        cy = sum(p[1] for p in points) / len(points)
        return math.degrees(math.atan2(-(cy - h / 2), cx - w / 2))

    def angle_diff(a, b):
        d = abs(a - b) % 360.0
        return min(d, 360.0 - d)


    blue = lambda r, g, b: b > r * 1.35 and b > g * 1.15 and b > 15
    day, night = shots["ctx-day"][1], shots["ctx-night"][1]
    for name, box, threshold in (
        ("shelf_clock_left", (385, 480, 410, 505), 20),
        ("shelf_clock_right", (975, 470, 1005, 500), 20),
        ("held_clock", (660, 390, 705, 435), 50),
    ):
        m1 = [blue(*p) for p in day.crop(box).getdata()]
        m2 = [blue(*p) for p in night.crop(box).getdata()]
        changed = sum(a != b for a, b in zip(m1, m2))
        probe.report[f"{name}_blue_pixels"] = [sum(m1), sum(m2)]
        probe.report[f"{name}_changed"] = changed
        probe.check(f"{name}_face_changed", changed >= threshold)
    # Dropped clock: the dark needle angle must change between received day and night.
    dark = lambda r, g, b: max(r, g, b) < 95 and max(r, g, b) - min(r, g, b) < 25
    drop_box = (335, 640, 395, 700)
    drop_angles = []
    for name, image in (("day", day), ("night", night)):
        crop = image.crop(drop_box)
        w, h = crop.size
        px = [(x, y) for y in range(h) for x in range(w) if dark(*crop.getpixel((x, y)))]
        probe.report[f"dropped_clock_dark_pixels_{name}"] = len(px)
        probe.check(f"dropped_clock_face_{name}", len(px) >= 100)
        drop_angles.append(needle_angle(px, w, h))
    probe.report["dropped_clock_angles"] = [round(a, 1) for a in drop_angles]
    probe.check("dropped_clock_face_changed", angle_diff(drop_angles[0], drop_angles[1]) > 30.0)
    red = lambda r, g, b: r > g * 1.25 and r > b * 1.25 and r > 50
    comp = shots["compass-day"][1]
    angles = []
    for name, box in (("compass_left", (425, 485, 450, 510)), ("compass_right", (1000, 480, 1025, 505))):
        crop = comp.crop(box)
        w, h = crop.size
        px = [(x, y) for y in range(h) for x in range(w) if red(*crop.getpixel((x, y)))]
        probe.report[f"{name}_needle_pixels"] = len(px)
        probe.check(f"{name}_needle_visible", len(px) >= 50)
        angles.append(needle_angle(px, w, h))
    probe.report["compass_needle_angles"] = [round(a, 1) for a in angles]
    probe.check("compass_needle_follows_position", angle_diff(angles[0], angles[1]) > 30.0)
    probe.report["visual_review_required"] = [
        "Inspect ctx-day/ctx-night/compass-day frames; mask predicates do not prove vanilla needle parity."
    ]



def start_java(ctx: ProfileContext, artifact: Path, address: str | None, username: str, timeout: float,
               display: str, render_distance: int, fov: int, *, plain: bool = False) -> tuple[Any, Any, McpClient]:
    """Launch the requested Java client, optionally connecting to a server."""
    game_dir = artifact / "game"
    game_dir.mkdir()
    (game_dir / "options.txt").write_text(java_options(render_distance, fov))
    token, mcp_port = secrets.token_urlsafe(32), runtime.reserve_port()
    log_path = artifact / "client.log"
    ctx.client_evidence["command_index"] = len(ctx.commands)
    ctx.commands.append([str(runtime.AGENT_ROOT / "gradlew"), ":fabric-agent:runClientMcp"])
    ctx.cwds.append(str(runtime.REPO_ROOT))
    ctx.exits.append(None)
    ctx.logs.append(str(log_path))
    process, log = runtime.start_client(game_dir=game_dir, log_path=log_path, token=token,
                                        mcp_port=mcp_port, username=username, display=display,
                                        client_profile="vanilla" if plain else None)
    if plain:
        ctx.commands[ctx.client_evidence["command_index"]] = list(process.args)
    mcp = None
    try:
        mcp = McpClient(f"http://127.0.0.1:{mcp_port}/mcp", token, timeout + 10)
        mcp.initialize()
        runtime.wait_client_ready_for_connect(mcp, timeout)
        if "video driver: wayland" in log_path.read_text(errors="replace").lower():
            raise RuntimeError("Java client opened its window on Wayland (the owner's desktop), not the harness Xvfb")
        if address is not None:
            mcp.call_tool("minecraft_connect", {"server_addr": address})
    except BaseException:
        if mcp is not None:
            mcp.close()
        runtime.stop_process(process)
        log.close()
        raise
    return process, log, mcp


def validate_scenario(ctx: ProfileContext, scenario: dict[str, Any] | None = None):
    """Load and validate exactly the inputs used by the scene runner."""
    if ctx.client not in ("native", "java") or not ctx.windowed or ctx.platform is not None:
        raise ValueError("scene-shot requires --client native|java --windowed")
    if ctx.scenario_id == "native-263-entity-rigs":
        if ctx.client != "native" or scenario is not None or ctx.extra_args or os.environ.get("SCENE_SHOT_SCENARIO"):
            raise ValueError("native-263-entity-rigs requires its registered native fixture without overrides")
        scenario = json.loads((runtime.REPO_ROOT / "tools/harness/entity_263_rigs_scene.json").read_text())
    elif ctx.scenario_id == "native-shelf-clock":
        if ctx.client != "native" or scenario is not None or ctx.extra_args or os.environ.get("SCENE_SHOT_SCENARIO"):
            raise ValueError("native-shelf-clock requires its registered native fixture without overrides")
        scenario = json.loads((runtime.REPO_ROOT / "tools/harness/shelf_clock_scene.json").read_text())
    elif ctx.scenario_id in ("native-entity-visuals", "native-item-bars", "native-shade-override",
                            "native-player-head", "native-dynamic-context"):
        fixtures = {
            "native-entity-visuals": "entity_visuals_scene.json",
            "native-item-bars": "item_bars_scene.json",
            "native-shade-override": "shade_override_scene.json",
            "native-player-head": "player_head_scene.json",
            "native-dynamic-context": "dynamic_context_scene.json",
        }
        if ctx.client != "native" or scenario is not None or ctx.extra_args or os.environ.get("SCENE_SHOT_SCENARIO"):
            raise ValueError(f"{ctx.scenario_id} requires its registered native fixture without overrides")
        scenario = json.loads((runtime.REPO_ROOT / "tools/harness" / fixtures[ctx.scenario_id]).read_text())
    elif ctx.scenario_id == "native-command-feedback":
        if ctx.client != "native" or scenario is not None or ctx.extra_args or os.environ.get("SCENE_SHOT_SCENARIO"):
            raise ValueError("native-command-feedback requires its registered native fixture without overrides")
        scenario = json.loads((runtime.REPO_ROOT / "tools/harness/command_feedback_scene.json").read_text())
    elif scenario is None:
        args = ctx.extra_args
        if args and args[0] == "--":
            args = args[1:]
        if len(args) > 1:
            raise ValueError("scene-shot accepts one scenario JSON path")
        scenario_path = args[0] if args else os.environ.get("SCENE_SHOT_SCENARIO")
        if not scenario_path:
            raise ValueError("set SCENE_SHOT_SCENARIO or pass a scenario JSON path")
        scenario = json.loads(Path(scenario_path).read_text())
    if not isinstance(scenario, dict) or not isinstance(scenario.get("steps"), list):
        raise ValueError("scenario must contain a steps array")
    if type(scenario.get("java_plain", False)) is not bool or (scenario.get("java_plain") and ctx.client != "java"):
        raise ValueError("java_plain requires --client java and a boolean value")
    if scenario.get("vanilla_singleplayer") is True:
        if ctx.client != "java":
            raise ValueError("vanilla singleplayer scenes require --client java")
        from . import integrated_scene

        return integrated_scene.run(ctx, scenario)
    seed = scenario.get("seed", 100)
    if type(seed) is not int:
        raise ValueError("seed must be an integer")
    server_view = scenario.get("server_view_distance", 4)
    if type(server_view) is not int or not 2 <= server_view <= 32:
        raise ValueError("server_view_distance must be in 2..32")
    spawn_protection = scenario.get("spawn_protection", 0)
    if type(spawn_protection) is not int or spawn_protection < 0:
        raise ValueError("spawn_protection must be a non-negative integer")
    operator = scenario.get("operator", True)
    if type(operator) is not bool:
        raise ValueError("operator must be true or false")
    username = scene_username(scenario)
    client_latency_ms = scenario.get("client_latency_ms", 0)
    if type(client_latency_ms) is not int or not 0 <= client_latency_ms <= 5000:
        raise ValueError("client_latency_ms must be an integer in 0..5000")
    settings = scenario.get("client_settings", {})
    if not isinstance(settings, dict) or set(settings) - {"graphics_preset", "render_distance", "fov"}:
        raise ValueError("unsupported client settings")
    if "fov" in settings and (type(settings["fov"]) is not int or not 30 <= settings["fov"] <= 110):
        raise ValueError("fov must be an integer in 30..110")
    if "graphics_preset" in settings:
        preset = settings["graphics_preset"]
        if not isinstance(preset, str) or preset.lower() not in ("low", "medium", "high"):
            raise ValueError("graphics_preset must be Low, Medium or High")
        settings["graphics_preset"] = preset.lower()
    if "render_distance" in settings and (type(settings["render_distance"]) is not int or not 2 <= settings["render_distance"] <= 32):
        raise ValueError("render_distance must be in 2..32")
    validate_steps(scenario["steps"], ctx.client)
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else 600.0
    if not 0 < timeout <= 3600:
        raise ValueError("timeout must be in (0, 3600]")
    if ctx.client == "java" and "graphics_preset" in settings:
        raise ValueError("graphics_preset is a native-client setting; the Java client keeps its default")
    return scenario, seed, server_view, spawn_protection, operator, username, client_latency_ms, settings, timeout


def run(
    ctx: ProfileContext,
    scenario: dict[str, Any] | None = None,
    script: Callable[[ProbeSteps, Path, list[dict[str, Any]], str], list[str]] = execute,
) -> dict[str, Any]:
    """Run one scene; other profiles can supply a scenario and Python script."""
    (scenario, seed, server_view, spawn_protection, operator, username,
     client_latency_ms, settings, timeout) = validate_scenario(ctx, scenario)
    binary_text = os.environ.get("SOLARIS_SERVER_BINARY")
    if not binary_text:
        raise RuntimeError("scene-shot requires SOLARIS_SERVER_BINARY")
    binary = Path(binary_text).expanduser().resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise RuntimeError(f"missing executable server binary: {binary}")
    artifact = ctx.artifact_dir
    port = runtime.reserve_port()
    config = artifact / "scene-shot.toml"
    source = (runtime.REPO_ROOT / "playable.toml").read_text()
    replacements = (("[server]", f"[server]\nspawn_protection = {spawn_protection}"),
                    ("view_distance = 4", f"view_distance = {server_view}"),
                    ("port = 25565", f"port = {port}"),
                    ('world_dir = ".analysis/test-world-v12"', f'world_dir = {json.dumps(str(artifact / "world"))}'),
                    ('directory = "plugins"', f'directory = {json.dumps(str(artifact / "plugins"))}'),
                    ("seed = 0", f"seed = {seed}"))
    for old, new in replacements:
        if source.count(old) != 1:
            raise RuntimeError(f"expected one config token: {old}")
        source = source.replace(old, new, 1)
    config.write_text(source)
    if scenario.get("world"):
        shutil.copytree(Path(scenario["world"]).expanduser().resolve(), artifact / "world")
    (artifact / "plugins").mkdir()
    if operator:
        prepare_operator_fixture(ctx, [username])
    xdg = artifact / "xdg"
    settings_path = xdg / "solaris-client" / "client-settings.json"
    settings_path.parent.mkdir(parents=True)
    if ctx.client == "native":
        from ..preflight import CLIENT_CONTENT_DIRS, client_template

        # Link the read-only content of the data root the env names (the one
        # preflight checked). Settings are written below; caches (far-terrain),
        # saved runs and window state must not leak between runs.
        source_root = client_template()
        for name in CLIENT_CONTENT_DIRS:
            entry = source_root / name
            if entry.exists():
                (settings_path.parent / name).symlink_to(entry.resolve(), target_is_directory=entry.is_dir())
    original = os.environ.get("SOLARIS_CLIENT_SETTINGS")
    if original and Path(original).is_file():
        base = json.loads(Path(original).read_text())
    else:
        base_path = Path(os.environ.get("XDG_DATA_HOME", str(Path.home() / ".local/share"))) / "solaris-client" / "client-settings.json"
        base = json.loads(base_path.read_text()) if base_path.is_file() else {}
    # The owner's personal fov must not leak into a reference comparison.
    base.update({"fov": NATIVE_FOV, **settings})
    if ctx.client == "java" and "graphics_preset" in settings:
        raise ValueError("graphics_preset is a native-client setting; the Java client keeps its default")
    settings_path.write_text(json.dumps(base, indent=2) + "\n")
    server = client = xvfb = mcp = log = client_log = proxy = None
    server_index = len(ctx.commands)
    server_command = [str(binary), "--config", str(config), "--no-console"]
    ctx.commands.append(server_command)
    ctx.cwds.append(str(runtime.REPO_ROOT))
    ctx.exits.append(None)
    try:
        log = (artifact / "server.log").open("wb")
        ctx.logs.append(str(artifact / "server.log"))
        with binary.open("rb") as server_executable:
            server_hash = hashlib.file_digest(server_executable, "sha256").hexdigest()
        ctx.client_evidence.update(server_binary_path=str(binary), server_binary_sha256=server_hash)
        server = runtime.start_server(server_command, cwd=runtime.REPO_ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        runtime.wait_port(port, timeout, server, log_path=artifact / "server.log")
        client_port = port
        if client_latency_ms or any(step.get("server_eof") is True for step in scenario["steps"]):
            proxy = LatencyProxy(port, client_latency_ms)
            client_port = proxy.start()
        xvfb, display, _ = runtime.start_xvfb(artifact)
        ctx.logs.append(str(artifact / "xvfb.log"))
        old_xdg = os.environ.get("XDG_DATA_HOME")
        old_settings = os.environ.get("SOLARIS_CLIENT_SETTINGS")
        try:
            os.environ["XDG_DATA_HOME"] = str(xdg)
            os.environ["SOLARIS_CLIENT_SETTINGS"] = str(settings_path)
            if ctx.client == "java":
                client, client_log, mcp = start_java(
                    ctx, artifact, f"127.0.0.1:{client_port}", username, timeout, display,
                    base.get("render_distance", 12), base["fov"], plain=scenario.get("java_plain", False))
            else:
                client, client_log, mcp = native_client.start(ctx, native_client.resolve_binary(ctx), f"127.0.0.1:{client_port}", username, timeout, display=display)
        finally:
            for key, old in (("XDG_DATA_HOME", old_xdg), ("SOLARIS_CLIENT_SETTINGS", old_settings)):
                if old is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = old
        mcp.request_timeout_seconds = min(timeout + 10, 130.0)
        mcp.scene_address = f"127.0.0.1:{client_port}"
        mcp.scene_proxy = proxy
        with ProbeSteps(mcp, artifact / "scene-shot", timeout, "scene-shot") as probe:
            wait_play(probe)
            if ctx.client == "java":
                window_id = find_java_window(display)
                if window_id is None:
                    raise RuntimeError(f"Java client opened no window on the harness Xvfb {display}; refusing to run")
                ctx.client_evidence["window_focus"] = mcp.p4_window = {"display": display, "window_id": window_id}
                resize_window(mcp.p4_window, WINDOW)
                probe.call("minecraft_wait_ticks", {"ticks": 10})
            else:
                from . import native_focus
                ctx.client_evidence["window_focus"] = mcp.p4_window = native_focus.acquire(display, client.pid, probe.remaining())
            before = probe.observe()
            probe.call("minecraft_wait_for_state_change", {
                "observed_version": before["state_version"], "timeout_seconds": probe.remaining(),
            })
            shots = script(probe, artifact, scenario["steps"], ctx.client)
            if ctx.scenario_id == "native-263-entity-rigs":
                check_263_entity_rigs(probe, artifact)
            elif ctx.scenario_id == "native-shelf-clock":
                check_shelf_clock(probe, artifact)
            elif ctx.scenario_id == "native-entity-visuals":
                check_entity_visuals(probe, artifact)
            elif ctx.scenario_id == "native-item-bars":
                check_item_bars(probe, artifact)
            elif ctx.scenario_id == "native-shade-override":
                check_shade_override(probe, artifact)
            elif ctx.scenario_id == "native-player-head":
                check_player_head(probe, artifact)
            elif ctx.scenario_id == "native-dynamic-context":
                check_dynamic_context(probe, artifact)
        result = {"screenshots": shots, "scenario": probe.report, "seed": seed, "settings": settings, "client_latency_ms": client_latency_ms}
    finally:
        if mcp is not None:
            mcp.close()
        runtime.stop_process(client)
        if client is not None:
            ctx.exits[ctx.client_evidence["command_index"]] = client.returncode
        if client_log is not None:
            client_log.close()
        if proxy is not None:
            proxy.close()
        runtime.stop_process(server, interrupt=True)
        if server is not None:
            ctx.exits[server_index] = server.returncode
        runtime.stop_process(xvfb)
        if log is not None:
            log.close()
    if ctx.client == "java":
        check_java_render_distance(artifact / "game" / "options.txt", base.get("render_distance", 12))
    return result
