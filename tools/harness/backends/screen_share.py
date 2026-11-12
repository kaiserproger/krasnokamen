"""Native graphical screen-sharing gate: F8 (LEAVE) withdraws an unanswered request.

Two real windowed native clients on a release server with the computer plugin:
the owner shares the Test pattern on a `computer:monitor` (right-click, picker,
Return), the viewer looks at it. Each step after that goes through the
clients' own sharing paths (`client_screen_share`: the viewer's right-click on
the monitor, the owner's consent dialog, F8).

* cancel: the viewer asks to watch, the owner's consent dialog opens, the viewer
  presses F8 before the owner answers, then the owner allows. The server must
  have dropped the request: the viewer receives no permission answer and no
  frame (ledger P4: a later Allow still granted the cancelled request).
* control: the viewer asks again and the owner allows; the picture reaches the
  viewer. F8 then stops it.

The staged copy of the shared pack's computer plugin lists the native loader
(the shared manifest offers the monitor bundle to Fabric/NeoForge/Forge only),
so the native clients admit the monitor and get a `solaris:screen` session.
"""
from __future__ import annotations

import hashlib
import json
import math
import os
import re
import shutil
import subprocess
from dataclasses import replace
from pathlib import Path
from typing import Any

from .. import runtime
from ..profiles import ProfileContext
from . import native_client
from .chisel import rewrite_config
from .gameplay import prepare_operator_fixture, require_tools
from .native_craft_drag import Pointer
from .probe_steps import ProbeSteps
from .scene_shot import WINDOW, wait_play

SCENARIO = "native-screen-share-leave"
SEED = 100
OWNER, VIEWER = "ScreenOwner", "ScreenViewer"
MONITOR = (1000, 161, 1004)          # computer:monitor carrier; its screen is the north face
SCREEN = (1000.5, 161.5, 1004.0)     # centre of that face
CONSENT_TITLE = "Allow screen and audio sharing?"
TOOLS = frozenset({"minecraft_observe", "minecraft_wait_for_play", "minecraft_wait_for_state_change",
                   "minecraft_send_chat", "minecraft_read_block", "minecraft_wait_ticks",
                   "client_screen_share", "client_capture"})


def validate_args(ctx: ProfileContext) -> float:
    if ctx.client != "native" or not ctx.windowed or ctx.extra_args or ctx.platform is not None:
        raise ValueError("screen-share requires --client native --windowed")
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else 900.0
    if not 0 < timeout <= 3600:
        raise ValueError("screen-share timeout must be in (0, 3600]")
    return timeout


def computer_plugin_source() -> Path:
    """The shared live pack's computer plugin (never edited: each run stages its own copy)."""
    work = Path(os.environ.get("SOLARIS_WORK_ROOT", "/run/media/kaiserroman/games/solaris-work"))
    return work / "scenarios" / "live-plugins" / "computer"


def with_native_loader(manifest: str) -> str:
    """The computer manifest with "native" first in its bundle's loaders (pure helper)."""
    match = re.search(r'^loaders = \[(.*)\]$', manifest, re.MULTILINE)
    if match is None or manifest.count("\nloaders = ") != 1:
        raise RuntimeError("expected one loaders line in the computer manifest")
    loaders = [part.strip() for part in match.group(1).split(",") if part.strip()]
    if '"native"' in loaders:
        return manifest
    return manifest[:match.start()] + "loaders = [" + ", ".join(['"native"', *loaders]) + "]" + manifest[match.end():]


def stage_computer_plugin(source: Path, plugins: Path) -> dict[str, Any]:
    target = plugins / "computer"
    shutil.copytree(source, target)
    manifest = target / "plugin.toml"
    manifest.write_text(with_native_loader(manifest.read_text()))
    return {"source": str(source), "manifest": manifest.read_text()}


def facing(feet: tuple[float, float, float], target: tuple[float, float, float]) -> tuple[float, float]:
    """Vanilla yaw/pitch that put the eye's crosshair on target."""
    dx, dy, dz = target[0] - feet[0], target[1] - (feet[1] + 1.62), target[2] - feet[2]
    return math.degrees(math.atan2(-dx, dz)), math.degrees(-math.atan2(dy, math.hypot(dx, dz)))


def dialog_nodes(state: dict[str, Any]) -> dict[str, str]:
    return {str(node.get("id")): str(node.get("label", node.get("text", "")))
            for node in state.get("ui_nodes", []) if str(node.get("id", "")).startswith("desktop/")}


def sharing(state: dict[str, Any]) -> dict[str, Any]:
    return state.get("screen_sharing") or {}


def viewer_counters(state: dict[str, Any]) -> dict[str, int]:
    """What the server relayed to this viewer: permission answers and frames."""
    shared = sharing(state)
    answers = shared.get("permission_answers") or {}
    return {"allowed": int(answers.get("allowed", 0)), "denied": int(answers.get("denied", 0)),
            "cancelled_grants_left": int(shared.get("cancelled_grants_left", 0)),
            "received_frames": int(shared.get("received_frames", 0)),
            "decoded_frames": int(shared.get("decoded_frames", 0))}


def granted_nothing(before: dict[str, int], after: dict[str, int]) -> bool:
    """A cancelled request: no answer, no late grant to leave, no frame reached the viewer."""
    return all(after[key] == before[key] for key in ("allowed", "denied", "cancelled_grants_left",
                                                       "received_frames", "decoded_frames"))


class Clients:
    """The server, both displays and both native clients of one run; closed in reverse."""

    def __init__(self, ctx: ProfileContext, binary: Path, timeout: float):
        self.ctx, self.binary, self.timeout = ctx, binary, timeout
        self.cleanup: list[Any] = []
        self.failure: BaseException | None = None

    def close(self) -> None:
        while self.cleanup:
            try:
                self.cleanup.pop()()
            except Exception:
                pass

    def server(self) -> str:
        artifact = self.ctx.artifact_dir
        port = runtime.reserve_port()
        plugins = artifact / "plugins"
        plugins.mkdir()
        self.ctx.client_evidence["computer_plugin"] = stage_computer_plugin(computer_plugin_source(), plugins)
        vanilla = os.environ.get("SOLARIS_SERVER_VANILLA_DATA_DIR", "data/vanilla")
        source = (runtime.REPO_ROOT / "playable.toml").read_text()
        token = 'vanilla_data_dir = "data/vanilla"'
        if source.count(token) != 1:
            raise RuntimeError("expected one vanilla_data_dir config token")
        source = source.replace(token, f"vanilla_data_dir = {json.dumps(vanilla)}")
        config = artifact / "screen-share.toml"
        config.write_text(rewrite_config(source, port=port, world=str(artifact / "world"),
                                         plugins=str(plugins), seed=SEED))
        prepare_operator_fixture(self.ctx, [OWNER, VIEWER])
        log_path = artifact / "server.log"
        log = log_path.open("wb")
        self.cleanup.append(log.close)
        self.ctx.logs.append(str(log_path))
        server = runtime.start_server([str(self.binary), "--config", str(config), "--no-console"],
                                      cwd=runtime.REPO_ROOT, stdout=log, stderr=subprocess.STDOUT,
                                      start_new_session=True)
        self.cleanup.append(lambda: runtime.stop_process(server, interrupt=True))
        runtime.wait_port(port, self.timeout, server, log_path=log_path)
        return f"127.0.0.1:{port}"

    def native(self, name: str, address: str, env: dict[str, str]) -> ProbeSteps:
        from ..preflight import CLIENT_CONTENT_DIRS, client_template

        sub = replace(self.ctx, artifact_dir=self.ctx.artifact_dir / name, client_evidence={})
        sub.artifact_dir.mkdir()
        xvfb, display, _ = runtime.start_xvfb(sub.artifact_dir)
        self.cleanup.append(lambda: runtime.stop_process(xvfb))
        settings_path = sub.artifact_dir / "xdg" / "solaris-client" / "client-settings.json"
        settings_path.parent.mkdir(parents=True)
        template = client_template()
        for entry in CLIENT_CONTENT_DIRS:
            if (template / entry).exists():
                (settings_path.parent / entry).symlink_to((template / entry).resolve(),
                                                          target_is_directory=(template / entry).is_dir())
        settings_path.write_text(json.dumps({"fov": 70, "desktop_computer": True}, indent=2) + "\n")
        overrides = {"XDG_DATA_HOME": str(settings_path.parent.parent),
                     "SOLARIS_CLIENT_SETTINGS": str(settings_path), **env}
        saved = {key: os.environ.get(key) for key in overrides}
        os.environ.update(overrides)
        try:
            process, log, mcp = native_client.start(sub, native_client.resolve_binary(sub), address, name,
                                                    self.timeout, display=display)
        finally:
            for key, value in saved.items():
                if value is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = value
        self.cleanup.append(lambda: runtime.stop_process(process))
        self.cleanup.append(log.close)
        self.cleanup.append(mcp.close)
        mcp.p4_window = sub.client_evidence.get("window_focus")
        mcp.request_timeout_seconds = min(self.timeout + 10, 130.0)
        probe = ProbeSteps(mcp, sub.artifact_dir / "probe", self.timeout, f"{SCENARIO}/{name}")
        probe.__enter__()
        self.cleanup.append(lambda: probe.__exit__(type(self.failure) if self.failure else None, self.failure, None))
        require_tools(mcp.list_tools(), TOOLS, "screen-share")
        wait_play(probe)
        self.ctx.client_evidence.setdefault("clients", {})[name] = {"display": display, **sub.client_evidence}
        return probe


def run(ctx: ProfileContext) -> dict[str, Any]:
    timeout = validate_args(ctx)
    override = os.environ.get("SOLARIS_SERVER_BINARY")
    if not override or not os.environ.get("SOLARIS_CLIENT_BINARY"):
        raise RuntimeError("screen-share requires prebuilt SOLARIS_SERVER_BINARY and SOLARIS_CLIENT_BINARY")
    binary = Path(override).expanduser().resolve()
    if not binary.is_file():
        raise RuntimeError(f"missing server binary: {binary}")
    with binary.open("rb") as handle:
        ctx.client_evidence.update(server_binary_path=str(binary), seed=SEED,
                                   server_binary_sha256=hashlib.file_digest(handle, "sha256").hexdigest())
    clients = Clients(ctx, binary, timeout)
    reports: dict[str, Any] = {}
    try:
        address = clients.server()
        owner = clients.native(OWNER, address, {"SOLARIS_SCREEN_TEST_SOURCE": "video:900"})
        viewer = clients.native(VIEWER, address, {})
        reports.update({OWNER: owner.report, VIEWER: viewer.report})
        scenario(owner, viewer)
        return {"scenario": reports}
    except BaseException as error:
        clients.failure = error
        raise
    finally:
        clients.close()


def place(op: ProbeSteps, client: ProbeSteps, name: str, feet: tuple[float, float, float]) -> None:
    yaw, pitch = facing(feet, SCREEN)
    op.fixture(f"tp {name} {feet[0]} {feet[1]} {feet[2]} {yaw:.2f} {pitch:.2f}")
    # A teleport is a travel boundary: the client may rebuild its world before it is in play again.
    client.wait(lambda state: abs((state.get("player") or {}).get("x", 1e9) - feet[0]) < 0.3
                and abs((state.get("player") or {}).get("z", 1e9) - feet[2]) < 0.3, f"{name}_placed",
                wait_for_play=True)
    client.call("minecraft_wait_ticks", {"ticks": 20})


def require_block(probe: ProbeSteps, position: tuple[int, int, int], block_id: str) -> None:
    """Wait until the client's own world shows a fixture block (commands are fire-and-forget)."""
    probe.wait(lambda _state: probe.call("minecraft_read_block", dict(zip("xyz", position))).get("block_id") == block_id,
               f"block_{block_id}_at_{'_'.join(map(str, position))}", timeout_seconds=120.0)


def shot(probe: ProbeSteps, name: str) -> str:
    return native_client.capture(probe.client, probe.artifact_dir.parent, name)["retained_path"]


def consent(owner: ProbeSteps, label: str) -> dict[str, str]:
    state = owner.wait(lambda s: dialog_nodes(s).get("desktop/title") == CONSENT_TITLE, f"{label}_consent_dialog")
    return dialog_nodes(state)


def scenario(owner: ProbeSteps, viewer: ProbeSteps) -> None:
    report = viewer.report
    for probe in (owner, viewer):
        state = probe.wait(lambda s: (s.get("loader") or {}).get("acknowledged") is True
                           and "computer:monitor" in (s.get("loader") or {}).get("registered_blocks", []),
                           "native_loader_registered_computer_monitor")
        probe.report["loader"] = state.get("loader")
    x, z = MONITOR[0], MONITOR[2] - 4
    for command in ("difficulty peaceful", "time set day",
                    "gamemode creative @a"):
        owner.fixture(command)
    place(owner, owner, OWNER, (x + 0.5, 176.0, z + 0.5))
    owner.fixture(f"fill {x - 15} 159 {z - 15} {x + 15} 159 {z + 15} minecraft:stone")
    owner.fixture(f"fill {x - 15} 160 {z - 15} {x + 15} 175 {z + 15} minecraft:air")
    require_block(owner, (x, 159, z), "minecraft:stone")
    owner.fixture("setblock {} {} {} minecraft:gray_glazed_terracotta[facing=north]".format(*MONITOR))
    require_block(owner, MONITOR, "minecraft:gray_glazed_terracotta")
    place(owner, owner, OWNER, (999.5, 160.0, 1002.0))
    place(owner, viewer, VIEWER, (1001.5, 160.0, 1002.0))

    # The owner right-clicks the monitor and picks the Test pattern, with real X input.
    pointer = Pointer(owner.client.p4_window)
    try:
        pointer.move(WINDOW[0] // 2, WINDOW[1] // 2)
        pointer.button(True, 3)
        pointer.button(False, 3)
    finally:
        pointer.close()
    picker = owner.wait(lambda s: dialog_nodes(s).get("desktop/choice/0") == "Test pattern", "owner_picker_open")
    owner.report["picker"] = dialog_nodes(picker)
    subprocess.run(["xdotool", "key", "--delay", "80", "Return"], check=True, timeout=30,
                   env={**os.environ, "DISPLAY": owner.client.p4_window["display"]})
    viewer.wait(lambda s: len(sharing(s).get("screens", [])) == 1, "viewer_sees_active_screen")
    shot(owner, "owner-sharing")

    # Cancel: F8 before the owner answers; the owner's later Allow must grant nothing.
    report["cancel_request"] = viewer.call("client_screen_share", {"action": "request"})
    report["cancel_consent"] = consent(owner, "cancel")
    shot(owner, "owner-consent-before-cancel")
    before = viewer_counters(viewer.observe())
    report["cancel_leave"] = viewer.call("client_screen_share", {"action": "leave"})
    # The viewer's LEAVE reaches the server before the owner's answer is sent.
    viewer.call("minecraft_wait_ticks", {"ticks": 10})
    report["cancel_answer"] = owner.call("client_screen_share", {"action": "answer", "allow": True})
    owner.wait(lambda s: "desktop/title" not in dialog_nodes(s), "cancel_consent_answered")
    # Several relay ticks and owner frame slots: a granted viewer would have its answer and frames.
    viewer.call("minecraft_wait_ticks", {"ticks": 100})
    after = viewer_counters(viewer.observe())
    report["cancel_counters"] = {"before": before, "after": after}
    report["cancel_screenshot"] = shot(viewer, "viewer-after-cancelled-request")
    viewer.check("cancelled_request_grants_nothing", granted_nothing(before, after))

    # Control: an answered request streams the picture, and F8 stops it.
    report["allow_request"] = viewer.call("client_screen_share", {"action": "request"})
    consent(owner, "allow")
    report["allow_answer"] = owner.call("client_screen_share", {"action": "answer", "allow": True})
    allowed = viewer.wait(lambda s: viewer_counters(s)["allowed"] == after["allowed"] + 1
                          and viewer_counters(s)["decoded_frames"] > after["decoded_frames"],
                          "allowed_request_streams_frames", timeout_seconds=90.0)
    report["allow_counters"] = viewer_counters(allowed)
    report["allow_screenshot"] = shot(viewer, "viewer-allowed")
    report["control_leave"] = viewer.call("client_screen_share", {"action": "leave"})
    viewer.call("minecraft_wait_ticks", {"ticks": 40})  # frames already in flight
    left = viewer_counters(viewer.observe())
    viewer.call("minecraft_wait_ticks", {"ticks": 100})
    stopped = viewer_counters(viewer.observe())
    report["leave_counters"] = {"after_leave": left, "later": stopped}
    viewer.check("leave_stops_frames", stopped["received_frames"] == left["received_frames"])
