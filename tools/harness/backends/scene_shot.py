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
import time
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

def stage_plugins(source: Path | None, target: Path) -> None:
    """Each scene owns its plugin files; never edit the shared source pack."""
    if source is None:
        target.mkdir()
    else:
        shutil.copytree(source, target)



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
        if not self.ready.wait(timeout=8):
            raise TimeoutError("latency proxy start exceeded 8 seconds")
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
        self.thread.join(timeout=8)
        if self.thread.is_alive():
            raise TimeoutError("latency proxy teardown exceeded 8 seconds")


def _validate_java_display(gui_scale: int, language: str, screen_size: tuple[int, int] = WINDOW) -> None:
    if type(gui_scale) is not int or gui_scale not in (1, 2, 3):
        raise ValueError("gui_scale must be an integer in 1..3")
    if language not in ("en_us", "ru_ru"):
        raise ValueError("language must be en_us or ru_ru")
    width, height = runtime.validate_screen_size(screen_size)
    if width // 320 < gui_scale or height // 240 < gui_scale:
        raise ValueError("gui_scale does not fit screen_size without vanilla clamping")


def java_options(render_distance: int, fov: int, *, gui_scale: int = NATIVE_GUI_SCALE,
                 language: str = "en_us", screen_size: tuple[int, int] = WINDOW,
                 hide_chat: bool = False) -> str:
    # Java stores fov normalized: 70 -> 0.0 and 110 -> 1.0.
    _validate_java_display(gui_scale, language, screen_size)
    return (
        "version:4790\nonboardAccessibility:false\n"
        'graphicsPreset:"custom"\n'
        f"overrideWidth:{screen_size[0]}\noverrideHeight:{screen_size[1]}\n"
        f"fov:{(fov - 70) / 40}\nguiScale:{gui_scale}\nlang:{language}\nrenderDistance:{render_distance}\n"
        f"simulationDistance:{min(render_distance, 12)}\nmaxFps:60\nenableVsync:false\n"
        f"chatVisibility:{2 if hide_chat else 0}\n"
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

def tool_wall_pixels(image: Path) -> tuple[int, int]:
    """Count diamond-tool and dark wall pixels in the fixed close-wall scene."""
    from PIL import Image

    with Image.open(image) as source:
        frame = source.convert("RGB")
        if frame.size != WINDOW:
            raise ValueError("tool-wall check requires a 1280x720 frame")
        pixels = frame.crop((1000, 390, 1154, 535)).getdata()
        tool = wall = 0
        for r, g, b in pixels:
            tool += g > 30 and b > 30 and g > r * 1.5 and b > r * 1.5 and abs(g - b) < 50
            wall += max(r, g, b) < 45
        return tool, wall



def capture_java(probe: ProbeSteps, artifact: Path, name: str) -> str:
    path = java_p5.capture(probe, name)
    expected = getattr(probe.client, "scene_window_size", WINDOW)
    if png_size(path) != expected:
        raise RuntimeError(f"Java screenshot {path} is {png_size(path)}, expected {expected}")
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


def execute(probe: ProbeSteps, artifact: Path, steps: list[dict[str, Any]], client: str = "native") -> list[str]:
    validate_steps(steps, client)
    capture = capture_java if client == "java" else capture_native
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
        elif kind == "assert_render_bounded":
            samples = probe.report.setdefault("render_samples", [])
            metrics = render_metrics(probe.observe())
            samples.append({**metrics, "max_index_count": value["max_index_count"],
                            "max_growth": value["max_growth"]})
            probe.check("render_drawn_index_bounded",
                        bounded_draw(metrics, samples[:-1], **{k: value[k] for k in ("max_index_count", "max_growth")}))
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
        elif kind in ("scan_blocks", "read_block", "navigate_to_block", "list_entities"):
            name, args = value.get("name"), value.get("args")
            result = probe.call(SCENE_TOOLS[kind], args)
            (artifact / f"{name}.json").write_text(json.dumps(result, indent=2) + "\n")
        else:
            raise ValueError(f"invalid scene step: {step!r}")
    return shots


def wait_java_bootstrap(mcp: McpClient, process: Any, timeout: float) -> dict[str, Any]:
    """The agent listens in premain, before the client thread can answer MCP."""
    import time

    deadline = time.monotonic() + timeout
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("Java client bootstrap did not become responsive")
        if process.poll() is not None:
            raise RuntimeError(f"Java client exited during bootstrap: {process.returncode}")
        try:
            return runtime.wait_client_ready_for_connect(mcp, remaining)
        except RuntimeError as error:
            # Only the agent's client-thread startup timeout is transient here;
            # all other MCP, loading-screen and protocol failures stay fatal.
            if not (str(error).startswith("minecraft_observe failed:")
                    and '"message": "TimeoutException"' in str(error)):
                raise


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
    teal = lambda r, g, b: g > r + 15 and b > r + 15
    pink = lambda r, g, b: r - b > 40 and r > 120
    white = lambda r, g, b: r > 170 and g > 170 and b > 170 and max(r, g, b) - min(r, g, b) < 35
    orange = lambda r, g, b: r > 150 and 60 < g < 210 and b < 110
    rope = lambda r, g, b: 60 < r < 140 and 40 < g < 100 and b < 70 and r > g > b
    vindicator = shots["vindicator"][1]
    probe.check("vindicator_visible", _changed(shots["vindicator"][1], shots["empty"][1], (560, 280, 780, 560)) > 500)
    probe.check("vindicator_torso_not_maroon", _count(vindicator, (600, 380, 680, 450), maroon) < 50)
    arm_pixels = sum(_count(vindicator, box, teal) for box in ((590, 390, 615, 470), (665, 390, 700, 470)))
    probe.report["vindicator_teal_arm_pixels"] = arm_pixels
    probe.check("vindicator_arms_not_teal", arm_pixels < 50)
    leg_teal = _count(vindicator, (600, 460, 680, 540), teal)
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


def player_head_skin_pack(artifact: Path) -> Path:
    """An authored offline Steve skin: cyan base, opaque magenta hat with a face window."""
    from PIL import Image, ImageDraw

    pack = artifact / "head-skin-pack"
    skin = Image.new("RGBA", (64, 64), (0, 220, 220, 255))
    draw = ImageDraw.Draw(skin)
    draw.rectangle((32, 0, 63, 15), fill=(240, 0, 240, 255))
    draw.rectangle((42, 10, 45, 13), fill=(0, 0, 0, 0))
    # The pinned entity facts select slim/steve for unresolved placed heads
    # (default_skins[6]); the special-item path selects wide/steve.
    for model in ("slim", "wide"):
        texture = pack / f"assets/minecraft/textures/entity/player/{model}/steve.png"
        texture.parent.mkdir(parents=True)
        skin.save(texture)
    (pack / "pack.mcmeta").write_text(json.dumps({
        "pack": {"description": "Authored player-head hat regression skin", "pack_format": 84},
    }) + "\n")
    return pack


def check_player_head(probe: ProbeSteps, artifact: Path) -> None:
    """Placed/worn offline heads and the contrasting authored hat layer."""
    shots = _load_shots(probe, artifact, ("head-empty", "head-block", "head-worn", "head-none"))
    empty_f, block_f = shots["head-empty"][0], shots["head-block"][0]
    probe.check("head_front_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(empty_f["camera_position"], block_f["camera_position"])
    ) and abs(empty_f["yaw"] - block_f["yaw"]) < 0.01 and abs(empty_f["pitch"] - block_f["pitch"]) < 0.01)
    worn_f, none_f = shots["head-worn"][0], shots["head-none"][0]
    probe.check("head_back_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(worn_f["camera_position"], none_f["camera_position"])
    ) and abs(worn_f["yaw"] - none_f["yaw"]) < 0.01 and abs(worn_f["pitch"] - none_f["pitch"]) < 0.01)
    # player_head_scene: head at (1000.5,160.25,1004.5), camera
    # (1000.5,161.62,1001.5), yaw/pitch 0, vertical FOV 70, 1280x720.
    # The projected half-block head is x593..687/y534..641 (hat expands it).
    head_box = (580, 520, 700, 655)
    placed = _changed(shots["head-block"][1], shots["head-empty"][1], head_box)
    worn = _changed(shots["head-worn"][1], shots["head-none"][1], (560, 280, 760, 420))
    probe.report["placed_head_changed_pixels"] = placed
    probe.report["worn_head_changed_pixels"] = worn
    magenta = lambda r, g, b: r > 60 and b > 60 and r > g * 2 and b > g * 2
    worn_hat = _count(shots["head-worn"][1], (600, 280, 680, 375), magenta)
    probe.report["worn_head_hat_layer_pixels"] = worn_hat
    hat = _count(shots["head-block"][1], head_box, magenta)
    probe.report["placed_head_hat_layer_pixels"] = hat
    probe.report["placed_head_roi"] = list(head_box)
    # The authored skull's magenta hat is distinct from the avatar's brown
    # torso. Raw frame differences also count idle body/arm motion, so compare
    # hat pixels instead; the bare frame accounts for the held item's edge.
    back_box = (600, 400, 680, 510)
    back_changed = _changed(shots["head-worn"][1], shots["head-none"][1], back_box)
    back_hat = _count(shots["head-worn"][1], back_box, magenta)
    bare_back_hat = _count(shots["head-none"][1], back_box, magenta)
    added_back_hat = max(0, back_hat - bare_back_hat)
    probe.report["worn_head_back_roi"] = list(back_box)
    probe.report["worn_head_back_changed_pixels"] = back_changed
    probe.report["worn_head_back_hat_pixels"] = back_hat
    probe.report["bare_head_back_hat_pixels"] = bare_back_hat
    probe.report["worn_head_added_back_hat_pixels"] = added_back_hat
    probe.check("placed_head_renders", placed > 100)
    probe.check("placed_head_hat_layer_pixels", hat > 100)
    probe.check("worn_head_renders", worn > 500 and worn_hat > 100)
    probe.check("worn_head_back_unchanged", added_back_hat <= 100)

def _clock_face_samples(image: Any, box: tuple[int, ...]) -> tuple[int, list, int, int]:
    """Sample the gold-enclosed face, including its pose-independent sky colours."""
    crop = image.crop(box)
    gold = lambda r, g, b: r > b * 1.25 and g > b * 1.15 and g > 20 and r > g * 0.9
    rows = []
    for y in range(crop.height):
        ring = [x for x in range(crop.width) if gold(*crop.getpixel((x, y)))]
        if len(ring) >= 2 and ring[-1] - ring[0] >= 12:
            rows.append((y, ring[0] + 3, ring[-1] - 2))
    pixels = sum(x1 - x0 for _, x0, x1 in rows)
    cells = [[] for _ in range(64)]
    blue_pixels = dark_pixels = 0
    if rows:
        top, bottom = rows[0][0], rows[-1][0] + 1
        # Normalize between rim edges so yaw/bob cannot move the background
        # into the oracle. Time changes the pattern, not its total blue area.
        for y, x0, x1 in rows:
            for x in range(x0, x1):
                r, g, b = crop.getpixel((x, y))
                index = min(7, (y - top) * 8 // (bottom - top)) * 8 + min(7, (x - x0) * 8 // (x1 - x0))
                blue = b > r * 1.35 and b > g * 1.15 and b > 15
                dark = max(r, g, b) < 85 and max(r, g, b) - min(r, g, b) <= max(r, g, b) * 0.25
                cells[index].append(blue)
                blue_pixels += blue
                dark_pixels += dark
    return pixels, [sum(cell) / len(cell) if cell else None for cell in cells], blue_pixels, dark_pixels


def _capture_dropped_clock(probe: ProbeSteps, artifact: Path, name: str) -> str:
    # Item entities spin. Select on face coverage alone, never on animation
    # success: the unchanged day/night oracle must still reject a static clock.
    attempts = []
    selected = None
    for index in range(32):
        if index:
            probe.call("minecraft_wait_ticks", {"ticks": 4})
        sample = f"{name}-attempt-{index + 1}"
        image = capture_native(probe, artifact, sample)
        _, frame = _load_shots(probe, artifact, (sample,))[sample]
        pixels, grid, _, _ = _clock_face_samples(frame, (300, 260, 460, 420))
        occupied = sum(cell is not None for cell in grid)
        attempts.append({"image": image, "face_pixels": pixels, "sampled_cells": occupied})
        if pixels >= 100 and occupied == 64:
            selected = Path(image)
            break
    (artifact / f"{name}-clock-capture.json").write_text(
        json.dumps({"attempts": attempts, "selected": str(selected) if selected else None}, indent=2) + "\n")
    probe.check(f"{name}_clock_face_captured", selected is not None)
    # Keep the selected bytes and their original same-frame sidecar; do not
    # take another screenshot after the item has advanced to a different yaw.
    target = artifact / f"screenshots/{name}.png"
    shutil.copyfile(selected, target)
    shutil.copyfile(selected.with_suffix(".json"), target.with_suffix(".json"))
    return str(target)


def check_clock_face(probe: ProbeSteps, day: Any, night: Any, name: str, box: tuple[int, ...]) -> None:
    """Require both changed face cells and a changed blue/dark sky proportion."""
    grids = []
    blue_fractions = []
    for phase, image in (("day", day), ("night", night)):
        pixels, grid, blue, dark = _clock_face_samples(image, box)
        probe.report[f"{name}_face_pixels_{phase}"] = pixels
        probe.check(f"{name}_face_{phase}", pixels >= 100)
        grids.append(grid)
        probe.report[f"{name}_face_blue_dark_pixels_{phase}"] = [blue, dark]
        probe.check(f"{name}_face_colours_{phase}", blue + dark >= 20)
        blue_fractions.append(blue / (blue + dark))
    changes = [abs(a - b) for a, b in zip(*grids) if a is not None and b is not None]
    probe.report[f"{name}_face_blue_grid"] = grids
    probe.report[f"{name}_face_changed_cells"] = sum(delta > 0.35 for delta in changes)
    probe.check(f"{name}_face_sampled", len(changes) >= 48)
    # Bob/clipping/yaw can move unchanged blue pixels between normalized cells.
    # Blue versus neutral-dark sky is a material proportion within the same
    # face: projection scales both, while noon/midnight swap those sky colours.
    # Excluding the gold rim/sun also avoids counting a clipped lower bezel.
    probe.report[f"{name}_face_blue_fractions"] = blue_fractions
    probe.check(f"{name}_face_changed", sum(delta > 0.35 for delta in changes) >= 4
                and abs(blue_fractions[0] - blue_fractions[1]) >= 0.35)


def check_dynamic_context(probe: ProbeSteps, artifact: Path) -> None:
    """MODEL-DYNAMIC-CONTEXT: shelf, dropped and held clocks plus two shelf
    compasses at different positions all receive world context."""
    import math
    shots = _load_shots(probe, artifact, ("ctx-day", "ctx-night", "compass-day", "held-day", "held-night"))
    ref = shots["ctx-day"][0]
    for name in ("ctx-night", "compass-day"):
        frame = shots[name][0]
        probe.check(f"{name}_camera_unchanged", all(
            abs(a - b) < 0.01 for a, b in zip(ref["camera_position"], frame["camera_position"])
        ) and abs(ref["yaw"] - frame["yaw"]) < 0.01 and abs(ref["pitch"] - frame["pitch"]) < 0.01)
    probe.check("ctx_received_time_changed",
                shots["ctx-day"][0]["environment"]["time_of_day"] == "6000"
                and shots["ctx-night"][0]["environment"]["time_of_day"] == "18000"
                and shots["compass-day"][0]["environment"]["time_of_day"] == "6000")
    def axis_angle(points):
        cx = sum(p[0] for p in points) / len(points)
        cy = sum(p[1] for p in points) / len(points)
        sxx = sum((x - cx) ** 2 for x, _ in points) / len(points)
        syy = sum((y - cy) ** 2 for _, y in points) / len(points)
        sxy = sum((x - cx) * (y - cy) for x, y in points) / len(points)
        return math.degrees(0.5 * math.atan2(2 * sxy, sxx - syy))

    def axis_diff(a, b):
        d = abs(a - b) % 180.0
        return min(d, 180.0 - d)

    blue = lambda r, g, b: b > r * 1.35 and b > g * 1.15 and b > 15
    day, night = shots["ctx-day"][1], shots["ctx-night"][1]
    for name, box, threshold in (
        ("shelf_clock_left", (385, 480, 410, 505), 20),
        ("shelf_clock_right", (948, 480, 982, 515), 20),
    ):
        m1 = [blue(*p) for p in day.crop(box).getdata()]
        m2 = [blue(*p) for p in night.crop(box).getdata()]
        changed = sum(a != b for a, b in zip(m1, m2))
        probe.report[f"{name}_blue_pixels"] = [sum(m1), sum(m2)]
        probe.report[f"{name}_changed"] = changed
        probe.check(f"{name}_face_changed", changed >= threshold)
    check_clock_face(probe, day, night, "dropped_clock", (300, 260, 460, 420))
    held_day, held_night = shots["held-day"], shots["held-night"]
    probe.check("held_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(held_day[0]["camera_position"], held_night[0]["camera_position"])
    ) and abs(held_day[0]["yaw"] - held_night[0]["yaw"]) < 0.01
      and abs(held_day[0]["pitch"] - held_night[0]["pitch"]) < 0.01)
    probe.check("held_received_time_changed", held_day[0]["environment"]["time_of_day"] == "6000"
                and held_night[0]["environment"]["time_of_day"] == "18000")
    check_clock_face(probe, held_day[1], held_night[1], "held_clock", (570, 300, 710, 440))
    red = lambda r, g, b: r > g * 1.25 and r > b * 1.25 and r > 50
    comp = shots["compass-day"][1]
    angles = []
    for name, box in (("compass_left", (425, 485, 450, 510)), ("compass_right", (1000, 480, 1025, 505))):
        crop = comp.crop(box)
        w, h = crop.size
        px = [(x, y) for y in range(h) for x in range(w) if red(*crop.getpixel((x, y)))]
        probe.report[f"{name}_needle_pixels"] = len(px)
        probe.check(f"{name}_needle_visible", len(px) >= 50)
        angles.append(axis_angle(px))
    probe.report["compass_needle_axis_angles"] = [round(a, 1) for a in angles]
    probe.check("compass_needle_follows_position", axis_diff(angles[0], angles[1]) > 30.0)
    probe.report["visual_review_required"] = [
        "Inspect ctx-day/ctx-night/compass-day frames; mask predicates do not prove vanilla needle parity."
    ]

def start_java(ctx: ProfileContext, artifact: Path, address: str | None, username: str, timeout: float,
               display: str, render_distance: int, fov: int, *, plain: bool = False,
               gui_scale: int = NATIVE_GUI_SCALE, language: str = "en_us",
               screen_size: tuple[int, int] = WINDOW, hide_chat: bool = False) -> tuple[Any, Any, McpClient]:
    """Launch the requested Java client, optionally connecting to a server."""
    game_dir = artifact / "game"
    game_dir.mkdir()
    (game_dir / "options.txt").write_text(java_options(render_distance, fov, gui_scale=gui_scale,
                                                       language=language, screen_size=screen_size,
                                                       hide_chat=hide_chat))
    token, mcp_port = secrets.token_urlsafe(32), runtime.reserve_port()
    log_path = artifact / "client.log"
    ctx.client_evidence["command_index"] = len(ctx.commands)
    ctx.commands.append([str(runtime.sibling_root("loader") / "gradlew"), ":fabric-agent:runClientMcp"])
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
        wait_java_bootstrap(mcp, process, timeout)
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
    named_fixtures = {
        "native-263-entity-rigs": "entity_263_rigs_scene.json",
        "native-shelf-clock": "shelf_clock_scene.json",
        "native-entity-visuals": "entity_visuals_scene.json",
        "native-item-bars": "item_bars_scene.json",
        "native-shade-override": "shade_override_scene.json",
        "native-player-head": "player_head_scene.json",
        "native-dynamic-context": "dynamic_context_scene.json",
        "native-death-tilt": "death_tilt_scene.json",
        "native-command-feedback": "command_feedback_scene.json",
        "native-low-roof-house": "low_roof_house_scene.json",
        "native-falling-block-prediction": "falling_block_scene.json",
        "native-frozen-game-age": "frozen_game_age_scene.json",
        "native-unloaded-eye": "unloaded_eye_scene.json",
        "native-village-idle-stroll": "village_idle_stroll_scene.json",
        "native-village-night-door": "village_night_door_scene.json",
        "native-village-late-job": "village_late_job_scene.json",
        "native-village-distant-job": "village_distant_job_scene.json",
        "native-village-deep-water": "village_deep_water_scene.json",
        "native-village-mob-doors": "village_mob_doors_scene.json",
        "native-village-after-hours-job": "village_after_hours_job_scene.json",
        "native-village-breeding": "village_breeding_scene.json",
        "native-village-moved-workstation": "village_moved_workstation_scene.json",
        "native-village-occupied-bed": "village_occupied_bed_scene.json",
        "native-village-open-air-home": "village_open_air_home_scene.json",
        "native-settlement-radius-growth": "settlement_radius_scene.json",
        "native-settlement-buildings-upgrade": "settlement_buildings_scene.json",
        "issue1-01-villager-leg": "issue1_01_villager_leg_scene.json",
        "issue1-18-sheep-goat": "issue1_18_sheep_goat_scene.json",
        "issue1-19-hostile-motion": "issue1_19_hostile_motion_scene.json",
    }
    if ctx.scenario_id == "java-matrix-system-chat":
        if ctx.client != "java" or scenario is not None or ctx.extra_args or os.environ.get("SCENE_SHOT_SCENARIO"):
            raise ValueError("java-matrix-system-chat requires its registered Java fixture without overrides")
        scenario = {"steps": [], "operator": True, "server_view_distance": 2, "java_plain": True}
    elif ctx.scenario_id in named_fixtures:
        if ctx.client != "native" or scenario is not None or ctx.extra_args or os.environ.get("SCENE_SHOT_SCENARIO"):
            raise ValueError(f"{ctx.scenario_id} requires its registered native fixture without overrides")
        scenario = json.loads((runtime.REPO_ROOT / "tools/harness" / named_fixtures[ctx.scenario_id]).read_text())
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
    if type(scenario.get("java_hide_chat", False)) is not bool or (scenario.get("java_hide_chat") and ctx.client != "java"):
        raise ValueError("java_hide_chat requires --client java and a boolean value")
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
    if "lod_enabled" in scenario and type(scenario["lod_enabled"]) is not bool:
        raise ValueError("lod_enabled must be true or false")
    if "lod_share_terrain_keys" in scenario:
        if type(scenario["lod_share_terrain_keys"]) is not bool or scenario.get("lod_enabled") is not True:
            raise ValueError("lod_share_terrain_keys requires lod_enabled=true and a boolean value")
    username = scene_username(scenario)
    client_latency_ms = scenario.get("client_latency_ms", 0)
    if type(client_latency_ms) is not int or not 0 <= client_latency_ms <= 5000:
        raise ValueError("client_latency_ms must be an integer in 0..5000")
    settings = scenario.get("client_settings", {})
    if not isinstance(settings, dict) or set(settings) - {"graphics_preset", "render_distance", "fov", "gui_scale", "language", "screen_size"}:
        raise ValueError("unsupported client settings")
    if {"gui_scale", "screen_size"} & settings.keys() and ctx.client != "java":
        raise ValueError("gui_scale and screen_size are Java-only scene settings")
    if {"gui_scale", "language", "screen_size"} & settings.keys():
        # The language reaches the native client as KRASNOKAMEN_CLIENT_LANGUAGE, the Java client
        # through options.txt; both accept exactly the pinned locales.
        _validate_java_display(settings.get("gui_scale", NATIVE_GUI_SCALE), settings.get("language", "en_us"),
                               settings.get("screen_size", WINDOW))
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
    binary_text = os.environ.get("KRASNOKAMEN_SERVER_BINARY")
    if not binary_text:
        raise RuntimeError("scene-shot requires KRASNOKAMEN_SERVER_BINARY")
    binary = Path(binary_text).expanduser().resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise RuntimeError(f"missing executable server binary: {binary}")
    artifact = ctx.artifact_dir
    port = runtime.reserve_port()
    config = artifact / "scene-shot.toml"
    source = (runtime.REPO_ROOT / "playable.toml").read_text()
    # BREAK_CMD_BLOCK breaker glue (not for merge): vanilla enable-command-block for command-block feedback.
    command_block = "\nenable_command_block = true" if os.environ.get("BREAK_CMD_BLOCK") else ""
    replacements = (("[server]", f"[server]\nspawn_protection = {spawn_protection}{command_block}"),
                    ("view_distance = 4", f"view_distance = {server_view}"),
                    ("port = 25565", f"port = {port}"),
                    ('world_dir = ".analysis/test-world-v12"', f'world_dir = {json.dumps(str(artifact / "world"))}'),
                    ('directory = "plugins"', f'directory = {json.dumps(str(artifact / "plugins"))}'),
                    ("seed = 0", f"seed = {seed}"))
    for old, new in replacements:
        if source.count(old) != 1:
            raise RuntimeError(f"expected one config token: {old}")
        source = source.replace(old, new, 1)
    if "lod_enabled" in scenario:
        if "[lod]" in source:
            raise RuntimeError("playable.toml already contains a lod section")
        source += f'\n[lod]\nenabled = {str(scenario["lod_enabled"]).lower()}\n'
        if "lod_share_terrain_keys" in scenario:
            source += f'share_terrain_keys = {str(scenario["lod_share_terrain_keys"]).lower()}\n'
    config.write_text(source)
    if scenario.get("world"):
        shutil.copytree(Path(scenario["world"]).expanduser().resolve(), artifact / "world")
    stage_plugins(ctx.plugins_dir, artifact / "plugins")
    if operator:
        prepare_operator_fixture(ctx, [username])
    xdg = artifact / "xdg"
    settings_path = xdg / "krasnokamen-client" / "client-settings.json"
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
    original = os.environ.get("KRASNOKAMEN_CLIENT_SETTINGS")
    if original and Path(original).is_file():
        base = json.loads(Path(original).read_text())
    else:
        base_path = Path(os.environ.get("XDG_DATA_HOME", str(Path.home() / ".local/share"))) / "krasnokamen-client" / "client-settings.json"
        base = json.loads(base_path.read_text()) if base_path.is_file() else {}
    # The owner's personal fov must not leak into a reference comparison.
    # The native client takes its language from KRASNOKAMEN_CLIENT_LANGUAGE (below), not the file.
    base.update({"fov": NATIVE_FOV, **{key: value for key, value in settings.items()
                                       if not (key == "language" and ctx.client != "java")}})
    if ctx.client == "java" and "graphics_preset" in settings:
        raise ValueError("graphics_preset is a native-client setting; the Java client keeps its default")
    settings_path.write_text(json.dumps(base, indent=2) + "\n")
    server = client = xvfb = mcp = log = client_log = proxy = probe = None
    server_index = len(ctx.commands)
    server_command = [str(binary), "--config", str(config), "--no-console"]
    ctx.commands.append(server_command)
    ctx.cwds.append(str(runtime.REPO_ROOT))
    ctx.exits.append(None)
    try:
        (artifact / "lifecycle-progress.json").write_text(json.dumps({"phase": "server_start", "timeout_seconds": timeout, "started_unix": time.time()}) + "\n")
        log = (artifact / "server.log").open("wb")
        ctx.logs.append(str(artifact / "server.log"))
        with binary.open("rb") as server_executable:
            server_hash = hashlib.file_digest(server_executable, "sha256").hexdigest()
        ctx.client_evidence.update(server_binary_path=str(binary), server_binary_sha256=server_hash)
        server = runtime.start_server(server_command, cwd=runtime.REPO_ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        runtime.wait_port(port, timeout, server, log_path=artifact / "server.log")
        client_port = port
        (artifact / "lifecycle-progress.json").write_text(json.dumps({"phase": "proxy_start", "timeout_seconds": 8, "started_unix": time.time()}) + "\n")
        if ctx.scenario_id == "native-unloaded-eye":
            from .unloaded_eye import UnloadedEyeProxy, execute as unloaded_eye_script

            proxy = UnloadedEyeProxy(port, artifact)
            client_port = proxy.start()
            script = unloaded_eye_script
        elif client_latency_ms or any(step.get("server_eof") is True for step in scenario["steps"]):
            proxy = LatencyProxy(port, client_latency_ms)
            client_port = proxy.start()
        xvfb, display, _ = runtime.start_xvfb(artifact, tuple(settings.get("screen_size", WINDOW)))
        ctx.logs.append(str(artifact / "xvfb.log"))
        old_xdg = os.environ.get("XDG_DATA_HOME")
        old_settings = os.environ.get("KRASNOKAMEN_CLIENT_SETTINGS")
        old_packs = os.environ.get("KRASNOKAMEN_CLIENT_RESOURCE_PACKS")
        try:
            os.environ["XDG_DATA_HOME"] = str(xdg)
            os.environ["KRASNOKAMEN_CLIENT_SETTINGS"] = str(settings_path)
            if ctx.scenario_id == "native-player-head":
                pack = player_head_skin_pack(artifact)
                os.environ["KRASNOKAMEN_CLIENT_RESOURCE_PACKS"] = os.pathsep.join(
                    [old_packs, str(pack)] if old_packs else [str(pack)])
                ctx.client_evidence["head_skin_pack"] = str(pack)
            if ctx.client == "java":
                client, client_log, mcp = start_java(
                    ctx, artifact, f"127.0.0.1:{client_port}", username, timeout, display,
                    base.get("render_distance", 12), base["fov"], plain=scenario.get("java_plain", False),
                    gui_scale=settings.get("gui_scale", NATIVE_GUI_SCALE), language=settings.get("language", "en_us"),
                    screen_size=tuple(settings.get("screen_size", WINDOW)),
                    hide_chat=scenario.get("java_hide_chat", False))
            else:
                client, client_log, mcp = native_client.start(ctx, native_client.resolve_binary(ctx), f"127.0.0.1:{client_port}", username, timeout, display=display)
        finally:
            for key, old in (("XDG_DATA_HOME", old_xdg), ("KRASNOKAMEN_CLIENT_SETTINGS", old_settings),
                             ("KRASNOKAMEN_CLIENT_RESOURCE_PACKS", old_packs)):
                if old is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = old
        mcp.request_timeout_seconds = min(timeout + 10, 130.0)
        mcp.scene_address = f"127.0.0.1:{client_port}"
        mcp.scene_window_size = tuple(settings.get("screen_size", WINDOW))
        mcp.scene_proxy = proxy
        (artifact / "lifecycle-progress.json").write_text(json.dumps({"phase": "scenario", "timeout_seconds": timeout, "started_unix": time.time()}) + "\n")
        probe_id = "native-unloaded-eye" if ctx.scenario_id == "native-unloaded-eye" else "scene-shot"
        with ProbeSteps(mcp, artifact / "scene-shot", timeout, probe_id) as probe:
            wait_play(probe)
            if ctx.client == "java":
                window_id = find_java_window(display)
                if window_id is None:
                    raise RuntimeError(f"Java client opened no window on the harness Xvfb {display}; refusing to run")
                ctx.client_evidence["window_focus"] = mcp.p4_window = {"display": display, "window_id": window_id}
                resize_window(mcp.p4_window, mcp.scene_window_size)
                probe.call("minecraft_wait_ticks", {"ticks": 10})
            else:
                from . import native_focus
                ctx.client_evidence["window_focus"] = mcp.p4_window = native_focus.acquire(display, client.pid, probe.remaining())
            before = probe.observe()
            probe.call("minecraft_wait_for_state_change", {
                "observed_version": before["state_version"], "timeout_seconds": probe.remaining(),
            })
            shots = script(probe, artifact, scenario["steps"], ctx.client)
            if ctx.scenario_id == "java-matrix-system-chat":
                from .java_system_chat import check
                check(probe, artifact)
            elif ctx.scenario_id == "native-263-entity-rigs":
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
    except Exception as error:
        # Keep the scenario's own report for the failure receipt: `assertions` says whether an
        # assertion failed, which a promotion control must tell apart from a timeout or a crash.
        if probe is not None:
            ctx.failure_scenario = {"scenario": dict(probe.report), "error": str(error)}
        raise
    finally:
        (artifact / "lifecycle-progress.json").write_text(json.dumps({"phase": "teardown", "started_unix": time.time()}) + "\n")
        (artifact / "teardown-progress.json").write_text(json.dumps({"component": "mcp", "timeout_seconds": getattr(mcp, "request_timeout_seconds", 0), "started_unix": time.time()}) + "\n")
        if mcp is not None:
            mcp.close()
        (artifact / "teardown-progress.json").write_text(json.dumps({"component": "client", "timeout_seconds": 13, "started_unix": time.time()}) + "\n")
        runtime.stop_process(client)
        if client is not None:
            ctx.exits[ctx.client_evidence["command_index"]] = client.returncode
        if client_log is not None:
            client_log.close()
        (artifact / "teardown-progress.json").write_text(json.dumps({"component": "proxy", "timeout_seconds": 8, "started_unix": time.time()}) + "\n")
        if proxy is not None:
            proxy.close()
        (artifact / "teardown-progress.json").write_text(json.dumps({"component": "server", "timeout_seconds": 13, "started_unix": time.time()}) + "\n")
        runtime.stop_process(server, interrupt=True)
        if server is not None:
            ctx.exits[server_index] = server.returncode
        (artifact / "teardown-progress.json").write_text(json.dumps({"component": "display", "timeout_seconds": 13, "started_unix": time.time()}) + "\n")
        runtime.stop_process(xvfb)
        if log is not None:
            log.close()
    if ctx.client == "java":
        check_java_render_distance(artifact / "game" / "options.txt", base.get("render_distance", 12))
    return result
