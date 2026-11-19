"""Scene-shot client plumbing: the launch options, the window/display helpers and the
Java and native frame captures.

The client/report seams live on ``scene_shot`` (the tests patch them there), so this
module reaches them as ``_qa.<name>`` and never binds them.
"""

from __future__ import annotations

import asyncio
import ctypes
import ctypes.util
import re
import secrets
import shutil
import subprocess
import threading
import time
from pathlib import Path
from typing import Any

from .. import runtime
from ..mcp import McpClient
from ..profiles import ProfileContext
from . import java_p5, native_client
from .probe_steps import ProbeSteps
from . import scene_shot as _qa


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
    if _qa.png_size(path) != expected:
        raise RuntimeError(f"Java screenshot {path} is {_qa.png_size(path)}, expected {expected}")
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
