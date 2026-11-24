"""Screen sharing and voice chat through Krasnokamen with real native and Fabric clients.

Both legs run by default; SCREENS_VOICE_QA_LEG=screens|voice runs one while
developing (the ledger forbids that override).

screens  A native owner shares the Test pattern, carrying a 440 Hz tone, on a
         computer monitor. A native viewer asks to watch: the owner's consent
         dialog must name the app audio; Deny must start no audio capture;
         Allow must stream picture and audio. A Fabric Krasnokamen Loader viewer
         asks too and, once allowed, its real OpenAL output must carry the
         tone (the Loader plays screen audio through Fabric's sound API). Last,
         the native viewer dies and respawns in the same dimension and must
         keep the monitor catalog and its frames.
voice    Two Fabric Simple Voice Chat clients (440 and 660 Hz microphones) and
         a native client (880 Hz). Proximity: every speaker is heard by the
         other two. Group: the Fabric players form a NORMAL group through SVC's
         own screens and stand 200 blocks apart; they still hear each other,
         while the native beside the speaker hears nothing. Last, the native
         reconnects to a second server with voice disabled and must drop its
         voice state and microphone.

Every client renders under its own Xvfb and plays and records only on the
private hardware-free audio graph (tools/harness/screen_audio_graph.sh); the
profile re-enters itself inside that graph when started outside it.
"""
from __future__ import annotations

import array
import contextlib
import hashlib
import json
import math
import os
import re
import secrets
import subprocess
import sys
import wave
from dataclasses import replace
from pathlib import Path
from typing import Any

from .. import runtime
from ..audio import SoundCapture, tone_amplitudes
from ..mcp import McpClient
from ..profiles import ProfileContext, SplitPart, run_split, split_part_label
from . import native_client, native_focus
from .gameplay import prepare_operator_fixture
from .native_craft_drag import Pointer
from .probe_steps import ProbeSteps
from .scene_shot import WINDOW, find_java_window, java_options, resize_window, stage_plugins, wait_play

LEG_ENV = "SCREENS_VOICE_QA_LEG"
LEGS = ("screens", "voice")
RATE = 48000
HEARD = 0.01      # Goertzel amplitude of an audible relayed tone (full scale 1.0)
SILENT = 0.002    # nothing but dither
TONE_SECONDS = 3
DRAIN_SECONDS = 0.8
MONITOR = (1000, 161, 1004)          # computer:monitor carrier, screen on its north face
SCREEN = (1000.5, 161.5, 1004.0)     # centre of that face
SVC_GROUP_SCREEN = "de.maxhenkel.voicechat.gui.group.GroupScreen"


def validate_args(ctx: ProfileContext) -> float:
    if ctx.client != "native" or not ctx.windowed:
        raise ValueError("screens-voice-qa requires --client native --windowed")
    if ctx.extra_args or ctx.platform is not None:
        raise ValueError("screens-voice-qa takes no extra arguments or platform")
    leg = os.environ.get(LEG_ENV)
    if leg is not None and leg not in LEGS:
        raise ValueError(f"{LEG_ENV} must be one of {LEGS}")
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else 1500.0
    if not 0 < timeout <= 3600:
        raise ValueError("screens-voice-qa --timeout-seconds must be in (0, 3600]")
    if not (runtime.sibling_root("loader") / "gradlew").is_file():
        raise ValueError("screens-voice-qa needs the Krasnokamen Loader checkout "
                             f"(KRASNOKAMEN_LOADER_ROOT): {runtime.sibling_root('loader')}")
    return timeout


def plugins_source(ctx: ProfileContext) -> Path:
    if ctx.plugins_dir is not None:
        return ctx.plugins_dir.resolve()
    work = Path(os.environ.get("KRASNOKAMEN_WORK_ROOT", "/run/media/kaiserroman/games/solaris-work"))
    return work / "scenarios" / "live-plugins"


def split_legs() -> tuple[str, ...]:
    """The parts of a split: the legs this run covers."""
    leg = os.environ.get(LEG_ENV)
    return (leg,) if leg else LEGS


def merge_leg_parts(blocks: list[dict[str, Any]], legs: tuple[str, ...]) -> dict[str, Any]:
    """One gate verdict from per-leg parts; a leg with no verdicts is not a pass."""
    verdicts: dict[str, bool] = {}
    owners: dict[str, str] = {}
    parts = []
    for leg, block in zip(legs, blocks, strict=True):
        part_verdicts = dict((block.get("details") or {}).get("verdicts") or {})
        if not part_verdicts:
            raise RuntimeError(f"{leg} part published no verdicts")
        for name, value in part_verdicts.items():
            if name == "all":
                continue
            if name in owners:
                raise RuntimeError(f"verdict {name} reported by parts {owners[name]} and {leg}")
            owners[name] = leg
            verdicts[name] = value
        parts.append({"leg": leg, "label": block.get("label"), "status": block.get("status"),
                      "artifact": block.get("artifact"), "log": block.get("log")})
    failed = [name for name, value in verdicts.items() if value is not True]
    if failed:
        raise RuntimeError(f"screens-voice-qa verdicts failed: {failed}")
    verdicts["all"] = True
    return {"verdicts": verdicts, "parts": parts}


def _split_run(ctx: ProfileContext, timeout: float) -> dict[str, Any]:
    """Run each leg as its own child, each with its own Xvfb and audio graph."""
    if os.environ.get("KRASNOKAMEN_PW_ROOT"):
        raise ValueError(
            "screens-voice-qa cannot split inside a private audio graph: every part needs its own "
            "graph and sink, and the children would inherit this one")
    legs = split_legs()
    if len(legs) != ctx.parts:
        raise ValueError(f"--parts {ctx.parts} must equal the number of legs to run "
                         f"({len(legs)}: {', '.join(legs)}); a leg is one client session")
    # The two legs are complete server+client+Xvfb lifetimes; overlapping them exceeded one
    # 12 GiB mem-run scope (HARNESS-SCREENS-VOICE-PARTS-OOM), so this gate runs them in sequence.
    blocks = run_split(ctx, "screens-voice-qa", [SplitPart(leg, {LEG_ENV: leg}) for leg in legs],
                       sequential=True)
    return merge_leg_parts(blocks, legs)


@contextlib.contextmanager
def java_endpoint(leg: str):
    """Use the pinned SVC game's endpoint only for voice; screens stays canonical."""
    version = "26.3"
    if leg == "voice":
        pin = json.loads(runtime._mod_artifact_path("svc").read_text())
        version = pin["baseline"]["minecraft"]
        entry = runtime._select_mod_entry("svc", "fabric")
        if version not in entry["game_versions"]:
            raise ValueError(f"SVC pin does not cover its baseline Minecraft {version}")
    selectors = {"KRASNOKAMEN_JAVA_VERSION": version, "KRASNOKAMEN_JAVA_PLATFORM": "fabric"}
    previous = {key: os.environ.get(key) for key in selectors}
    for key, value in selectors.items():
        if previous[key] is not None and previous[key] != value:
            raise ValueError(f"screens-voice-qa {leg} requires {key}={value}, got {previous[key]}")
    os.environ.update(selectors)
    try:
        yield
    finally:
        for key, value in previous.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value


def run(ctx: ProfileContext) -> dict[str, Any]:
    timeout = validate_args(ctx)
    if ctx.parts > 1 and split_part_label() is None:
        # Before any graph re-entry: each child must build its own graph.
        return _split_run(ctx, timeout)
    if not os.environ.get("KRASNOKAMEN_PW_ROOT"):
        return _run_in_private_graph(ctx, timeout)
    legs = (os.environ[LEG_ENV],) if os.environ.get(LEG_ENV) else LEGS
    binary = Path(os.environ["KRASNOKAMEN_SERVER_BINARY"]).expanduser().resolve()
    with binary.open("rb") as handle:
        ctx.client_evidence.update(server_binary_path=str(binary),
                                   server_binary_sha256=hashlib.file_digest(handle, "sha256").hexdigest())
    native_client.resolve_binary(ctx)
    verdicts: dict[str, bool] = {}
    evidence: dict[str, Any] = {"legs": list(legs), "plugins_source": str(plugins_source(ctx)),
                                "loader_root": str(runtime.sibling_root("loader"))}
    ctx.client_evidence["screens_voice"] = evidence
    try:
        for leg in legs:
            with java_endpoint(leg):
                {"screens": screens_leg, "voice": voice_leg}[leg](ctx, binary, timeout, verdicts, evidence)
    finally:
        (ctx.artifact_dir / "verdicts.json").write_text(json.dumps(verdicts, indent=2) + "\n")
    failed = [name for name, value in verdicts.items() if value is not True]
    if failed:
        raise RuntimeError(f"screens-voice-qa verdicts failed: {failed}")
    if legs == LEGS:
        verdicts["all"] = True
    return {"verdicts": verdicts}


def _run_in_private_graph(ctx: ProfileContext, timeout: float) -> dict[str, Any]:
    """Re-enter this profile inside the private audio graph and adopt its receipt."""
    inner = ctx.artifact_dir / "private-graph"
    inner.mkdir()
    env = os.environ.copy()
    env["KRASNOKAMEN_AUDIO_RECEIPT_DIR"] = str(inner / "audio")
    (inner / "audio").mkdir()
    command = ["sh", str(runtime.REPO_ROOT / "tools/harness/screen_audio_graph.sh"), sys.executable, "-m",
               "tools.harness.backends.screens_voice_qa", str(inner), str(timeout),
               str(plugins_source(ctx))]
    log_path = ctx.artifact_dir / "private-graph.log"
    index = len(ctx.commands)
    ctx.commands.append(command)
    ctx.cwds.append(str(runtime.REPO_ROOT))
    ctx.exits.append(None)
    ctx.logs.append(str(log_path))
    with log_path.open("wb") as log:
        process = subprocess.Popen(command, cwd=runtime.REPO_ROOT, env=env, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            ctx.exits[index] = process.wait(timeout=timeout + 300)
        except BaseException:
            runtime.stop_process(process)
            ctx.exits[index] = process.returncode
            raise
    result_path = inner / "result.json"
    if not result_path.is_file():
        raise RuntimeError(f"private-graph run wrote no result; see {log_path}")
    result = json.loads(result_path.read_text())
    for key in ("commands", "cwds", "exits", "logs"):
        getattr(ctx, key).extend(result[key])
    ctx.client_evidence.update(result["client_evidence"])
    if isinstance(result.get("details"), dict):
        # The child records its per-verdict result (a failed leg writes verdicts.json and returns
        # an error), and the ledger reads a client row's red from those verdicts: adopt them
        # before raising so a failed control still carries the observation.
        ctx.client_evidence.update(result["details"])
    if result.get("error"):
        raise RuntimeError(f"private-graph run failed: {result['error']}")
    return result["details"]


def _main(argv: list[str]) -> int:
    """Child entry inside screen_audio_graph.sh: run the legs, write result.json."""
    artifact, timeout, plugins = Path(argv[0]), float(argv[1]), Path(argv[2])
    ctx = ProfileContext(artifact_dir=artifact, client="native", windowed=True, timeout_seconds=timeout,
                         plugins_dir=plugins)
    result: dict[str, Any] = {"details": {}, "error": None}
    try:
        result["details"] = run(ctx)
    except BaseException as error:  # the parent adopts the failure with every receipt
        result["error"] = f"{type(error).__name__}: {error}"
        # `run` writes the per-verdict result before it refuses a failed leg; keep it in the
        # child's details so the parent's failure receipt reports which verdict was false.
        verdicts = artifact / "verdicts.json"
        if verdicts.is_file():
            result["details"] = {"verdicts": json.loads(verdicts.read_text())}
    result.update(commands=ctx.commands, cwds=ctx.cwds, exits=ctx.exits, logs=ctx.logs,
                  client_evidence=ctx.client_evidence)
    (artifact / "result.json").write_text(json.dumps(result, indent=2, default=str) + "\n")
    return 0 if result["error"] is None else 1


# ---------------------------------------------------------------- audio fixtures

def write_tone(path: Path, frequency: int, seconds: float, amplitude: float = 0.3) -> Path:
    """A 48 kHz mono 16-bit sine, the format every capture hook accepts."""
    samples = array.array("h", (int(amplitude * 32767 * math.sin(2 * math.pi * frequency * i / RATE))
                                for i in range(int(seconds * RATE))))
    with wave.open(str(path), "wb") as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(RATE)
        output.writeframes(samples.tobytes())
    return path


class Microphone:
    """Tones play into a null sink; a source remapped from its monitor is one client's microphone.

    Sink and source descriptions equal their names, which is how OpenAL (SVC) names capture devices."""

    def __init__(self, name: str):
        self.sink = name
        self.source = f"{name}_src"
        self.modules = [self._load("module-null-sink", f"sink_name={name}", "rate=48000", "channels=1",
                                   f"sink_properties=device.description={name}")]
        self.modules.append(self._load("module-remap-source", f"master={name}.monitor",
                                       f"source_name={self.source}",
                                       f"source_properties=device.description={self.source}"))

    @staticmethod
    def _load(*arguments: str) -> str:
        return subprocess.check_output(["pactl", "load-module", *arguments], text=True).strip()

    def speak(self, tone: Path) -> None:
        """Play the whole tone; returns when the player process has drained it."""
        subprocess.run(["paplay", f"--device={self.sink}", str(tone)], check=True, timeout=TONE_SECONDS + 30)

    def close(self) -> None:
        for module in reversed(self.modules):
            subprocess.run(["pactl", "unload-module", module], check=True)


class Recording:
    """The native client's KRASNOKAMEN_VOICE_RECORD file: every decoded remote voice frame."""

    def __init__(self, path: Path):
        self.path = path

    def size(self) -> int:
        return self.path.stat().st_size if self.path.exists() else 0

    def segment(self, start: int, end: int, frequencies: tuple[int, ...]) -> dict[str, Any]:
        start = max(start, 44)  # the WAV header is not audio
        with self.path.open("rb") as source:
            source.seek(start)
            raw = source.read(max(0, end - start) // 2 * 2)
        mono = [value / 32768 for value in array.array("h", raw)]
        return {"byte_offset": start, "byte_length": len(raw), "frames": len(mono),
                "amplitude": tone_amplitudes(mono, RATE, frequencies)}


def heard(window: dict[str, Any], frequency: int) -> bool:
    return window["amplitude"][str(frequency)] >= HEARD


def silent(window: dict[str, Any], frequency: int) -> bool:
    return window["amplitude"][str(frequency)] <= SILENT


# ---------------------------------------------------------------- process helpers

def facing(feet: tuple[float, float, float], target: tuple[float, float, float]) -> tuple[float, float]:
    """Vanilla yaw/pitch that put the eye's crosshair on target."""
    dx, dy, dz = target[0] - feet[0], target[1] - (feet[1] + 1.62), target[2] - feet[2]
    return math.degrees(math.atan2(-dx, dz)), math.degrees(-math.atan2(dy, math.hypot(dx, dz)))


def keys(display: str, *names: str) -> None:
    """Real XTest key presses on one harness-owned display (its focused window)."""
    subprocess.run(["xdotool", "key", "--delay", "80", *names], check=True, timeout=30,
                   env={**os.environ, "DISPLAY": display})


def click(owned: dict[str, Any], x: int, y: int, button: int = 1) -> None:
    pointer = Pointer(owned)
    try:
        pointer.move(x, y)
        pointer.button(True, button)
        pointer.button(False, button)
    finally:
        pointer.close()


def type_text(display: str, text: str) -> None:
    subprocess.run(["xdotool", "type", "--delay", "80", text], check=True, timeout=30,
                   env={**os.environ, "DISPLAY": display})


def _processes_with(marker: str) -> list[dict[str, str]]:
    """Every process whose environment carries `marker`: its pid, DISPLAY and command head."""
    found = []
    for proc in Path("/proc").iterdir():
        if not proc.name.isdigit():
            continue
        try:
            environ = (proc / "environ").read_bytes().split(b"\0")
            if marker.encode() in environ:
                display = next((item[8:].decode() for item in environ if item.startswith(b"DISPLAY=")), "")
                command = (proc / "cmdline").read_bytes().replace(b"\0", b" ")[:160].decode(errors="replace")
                found.append({"pid": proc.name, "DISPLAY": display, "command": command})
        except OSError:
            continue
    return found


class Run:
    """Owns every server, display and client of one leg; closes them in reverse."""

    def __init__(self, ctx: ProfileContext, binary: Path, timeout: float, leg: str):
        self.ctx, self.binary, self.timeout = ctx, binary, timeout
        self.root = ctx.artifact_dir / leg
        self.root.mkdir(parents=True)
        self.stack = contextlib.ExitStack()
        self.evidence: dict[str, Any] = {"clients": {}, "servers": {}}

    def __enter__(self) -> Run:
        return self

    def __exit__(self, *exc: Any) -> None:
        self.stack.__exit__(*exc)

    def _track(self, command: list[str], log: Path, cwd: Path) -> int:
        index = len(self.ctx.commands)
        self.ctx.commands.append(command)
        self.ctx.cwds.append(str(cwd))
        self.ctx.exits.append(None)
        self.ctx.logs.append(str(log))
        return index

    def _stop(self, process: subprocess.Popen[Any], index: int, *, interrupt: bool = False) -> None:
        runtime.stop_process(process, interrupt=interrupt)
        self.ctx.exits[index] = process.returncode

    def server(self, name: str, extra: str, operators: list[str]) -> tuple[str, Path]:
        port = runtime.reserve_port()
        config = self.root / f"{name}.toml"
        source = (runtime.REPO_ROOT / "playable.toml").read_text()
        for old, new in (("port = 25565", f"port = {port}"),
                         ('world_dir = ".analysis/test-world-v12"', f"world_dir = {json.dumps(str(self.root / (name + '-world')))}"),
                         ('directory = "plugins"', f"directory = {json.dumps(str(self.root / (name + '-plugins')))}"),
                         ("seed = 0", "seed = 100")):
            if source.count(old) != 1:
                raise RuntimeError(f"expected one config token: {old}")
            source = source.replace(old, new, 1)
        config.write_text(source + extra)
        stage_plugins(plugins_source(self.ctx), self.root / f"{name}-plugins")
        prepare_operator_fixture(replace(self.ctx, artifact_dir=self.root, client_evidence={}), operators)
        log_path = self.root / f"{name}.log"
        command = [str(self.binary), "--config", str(config), "--no-console"]
        index = self._track(command, log_path, runtime.REPO_ROOT)
        log = self.stack.enter_context(log_path.open("wb"))
        env = {**os.environ, "KRASNOKAMEN_HARNESS_LOG_STDOUT": "1"}
        process = runtime.start_server(command, cwd=runtime.REPO_ROOT, stdout=log, stderr=subprocess.STDOUT,
                                       start_new_session=True, env=env)
        self.stack.callback(self._stop, process, index, interrupt=True)
        runtime.wait_port(port, self.timeout, process, log_path=log_path)
        self.evidence["servers"][name] = {"address": f"127.0.0.1:{port}", "config": str(config), "log": str(log_path)}
        return f"127.0.0.1:{port}", log_path

    def display(self, name: str) -> str:
        process, display, log = runtime.start_xvfb(self.root / name)
        self.stack.callback(log.close)
        self.stack.callback(runtime.stop_process, process)
        return display

    def native(self, name: str, address: str, settings: dict[str, Any], env: dict[str, str]) -> ProbeSteps:
        from ..preflight import CLIENT_CONTENT_DIRS, client_template

        sub = replace(self.ctx, artifact_dir=self.root / name, client_evidence={})
        display = self.display(name)
        settings_path = sub.artifact_dir / "xdg" / "krasnokamen-client" / "client-settings.json"
        settings_path.parent.mkdir(parents=True)
        template = client_template()
        for entry in CLIENT_CONTENT_DIRS:
            if (template / entry).exists():
                (settings_path.parent / entry).symlink_to((template / entry).resolve(),
                                                          target_is_directory=(template / entry).is_dir())
        settings_path.write_text(json.dumps({"fov": 70, **settings}, indent=2) + "\n")
        overrides = {"XDG_DATA_HOME": str(settings_path.parent.parent), **env}
        saved = {key: os.environ.get(key) for key in overrides}
        os.environ.update(overrides)
        try:
            process, log, mcp = native_client.start(sub, native_client.resolve_binary(sub), address, name,
                                                    self.timeout, display=display,
                                                    settings=settings_path)
        finally:
            for key, value in saved.items():
                if value is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = value
        index = sub.client_evidence["command_index"]
        self.stack.callback(log.close)
        self.stack.callback(self._stop, process, index)
        self.stack.callback(mcp.close)
        mcp.request_timeout_seconds = min(self.timeout + 10, 130.0)
        probe = self.stack.enter_context(ProbeSteps(mcp, sub.artifact_dir / "probe", self.timeout, name))
        wait_play(probe)
        mcp.p4_window = native_focus.acquire(display, process.pid, probe.remaining())
        self.evidence["clients"][name] = {"kind": "native", "display": display, **sub.client_evidence}
        return probe

    def java(self, name: str, address: str, profile: str, sound_sink: str | None,
             files: dict[str, str] | None = None) -> ProbeSteps:
        base = self.root / name
        game = base / "game"
        game.mkdir(parents=True)
        (game / "options.txt").write_text(java_options(8, 70))
        for relative, text in (files or {}).items():
            (game / relative).parent.mkdir(parents=True, exist_ok=True)
            (game / relative).write_text(text)
        display = self.display(name)
        token, port = secrets.token_urlsafe(32), runtime.reserve_port()
        log_path = base / "client.log"
        index = self._track([str(runtime.sibling_root("loader") / "gradlew"), ":loader-fabric:runClientMcp", f"profile={profile}"],
                            log_path, runtime.REPO_ROOT)
        process, log = runtime.start_client(game_dir=game, log_path=log_path, token=token, mcp_port=port,
                                            username=name, display=display, platform="fabric",
                                            client_profile=profile, sound_sink=sound_sink)
        self.stack.callback(log.close)
        self.stack.callback(self._stop, process, index)
        mcp = McpClient(f"http://127.0.0.1:{port}/mcp", token, self.timeout + 10)
        self.stack.callback(mcp.close)
        mcp.initialize()
        runtime.wait_client_ready_for_connect(mcp, self.timeout)
        mcp.call_tool("minecraft_connect", {"server_addr": address})
        from ..java_clients import _confirm_exact_permission
        title = _confirm_exact_permission(mcp, address, min(self.timeout, 120.0))
        mcp.request_timeout_seconds = min(self.timeout + 10, 130.0)
        probe = self.stack.enter_context(ProbeSteps(mcp, base / "probe", self.timeout, name))
        wait_play(probe)
        opened = re.findall(r"OpenAL initialized on device (.+)", log_path.read_text(errors="replace"))
        if sound_sink is not None and (not opened or opened[-1].strip() != sound_sink):
            raise RuntimeError(f"{name} OpenAL opened {opened[-1:]} instead of its own sink {sound_sink}")
        window = find_java_window(display)
        if window is None:
            tree = subprocess.run(["xwininfo", "-display", display, "-root", "-children"],
                                  capture_output=True, text=True)
            raise RuntimeError(f"{name} opened no window on its Xvfb {display}: {tree.stdout}{tree.stderr}; "
                               f"its processes: {_processes_with(f'KRASNOKAMEN_CLIENT_MCP_TOKEN={token}')}")
        mcp.p4_window = {"display": display, "window_id": window}
        resize_window(mcp.p4_window, WINDOW)
        self.evidence["clients"][name] = {"kind": f"fabric+{profile}", "display": display, "log": str(log_path),
                                          "loader_permission": title, "openal_device": opened[-1:]}
        return probe


def register(probe: ProbeSteps) -> None:
    from ..java_clients import authenticate_client
    authenticate_client(probe.client, secrets.token_urlsafe(12), timeout_seconds=min(probe.remaining(), 90.0))


def place(op: ProbeSteps, client: ProbeSteps, name: str, feet: tuple[float, float, float],
          target: tuple[float, float, float]) -> None:
    yaw, pitch = facing(feet, target)
    op.fixture(f"tp {name} {feet[0]} {feet[1]} {feet[2]} {yaw:.2f} {pitch:.2f}")
    # A teleport is a travel boundary: the client may rebuild its world before it is in play again.
    client.wait(lambda state: abs((state.get("player") or {}).get("x", 1e9) - feet[0]) < 0.3
                and abs((state.get("player") or {}).get("z", 1e9) - feet[2]) < 0.3, f"{name}_placed",
                wait_for_play=True)
    client.call("minecraft_wait_ticks", {"ticks": 20})


def arena(op: ProbeSteps, name: str, centre: tuple[int, int]) -> None:
    """A cleared 31x31 stone pad at y 159 around `centre`, everyone in creative.

    The operator stands over the pad first: fill only changes loaded chunks."""
    x, z = centre
    op.fixture("difficulty peaceful")
    op.fixture("time set day")
    op.fixture("gamemode creative @a")
    place(op, op, name, (x + 0.5, 176.0, z + 0.5), (x + 0.5, 170.0, z + 8.5))
    op.fixture(f"fill {x - 15} 159 {z - 15} {x + 15} 159 {z + 15} minecraft:stone")
    op.fixture(f"fill {x - 15} 160 {z - 15} {x + 15} 175 {z + 15} minecraft:air")
    require_block(op, (x, 159, z), "minecraft:stone")


def require_block(probe: ProbeSteps, position: tuple[int, int, int], block_id: str) -> dict[str, Any]:
    """Wait until the client's own world shows a fixture block (commands are fire-and-forget)."""
    seen: dict[str, Any] = {}

    def shown(_state: dict[str, Any]) -> bool:
        seen.update(probe.call("minecraft_read_block", dict(zip("xyz", position))))
        return seen.get("block_id") == block_id

    probe.wait(shown, f"block_{block_id}_at_{'_'.join(map(str, position))}", timeout_seconds=120.0)
    return seen


def dialog_nodes(state: dict[str, Any]) -> dict[str, str]:
    return {str(node.get("id")): str(node.get("label", node.get("text", "")))
            for node in state.get("ui_nodes", []) if str(node.get("id", "")).startswith("desktop/")}


def native_shot(probe: ProbeSteps, name: str) -> str:
    return native_client.capture(probe.client, probe.artifact_dir.parent, name)["retained_path"]


def consent_dialog_open(state: dict[str, Any]) -> bool:
    """A consent dialog is open by its structure, never by its wording.

    The wording is what the row measures (`owner_consent_names_app_audio`), so a detector that
    matches the fixed title can never see the pre-fix dialog: the control would wait out its
    whole deadline instead of observing the old strings.
    """
    nodes = dialog_nodes(state)
    return "desktop/title" in nodes and any(key.startswith("desktop/choice/") for key in nodes)


def java_shot(probe: ProbeSteps, name: str) -> str:
    from .java_p5 import capture
    return capture(probe, name)


# ---------------------------------------------------------------- screens leg

def screens_leg(ctx: ProfileContext, binary: Path, timeout: float, verdicts: dict[str, bool],
                evidence: dict[str, Any]) -> None:
    owner_name, viewer_name, fabric_name = "ScreenOwner", "ScreenViewer", "FabricViewer"
    with Run(ctx, binary, timeout, "screens") as run:
        leg: dict[str, Any] = run.evidence
        evidence["screens"] = leg
        tone = write_tone(run.root / "screen-tone-440.wav", 440, 600)
        address, _ = run.server("server", "", [owner_name, viewer_name, fabric_name])
        settings = {"desktop_computer": True}
        # krasnokamen-auth kicks a player who has not registered within its login window: register on join.
        owner = run.native(owner_name, address, settings, {"KRASNOKAMEN_SCREEN_TEST_SOURCE": "video:600",
                                                           "KRASNOKAMEN_SCREEN_AUDIO_WAV": str(tone)})
        register(owner)
        viewer = run.native(viewer_name, address, settings, {})
        register(viewer)
        fabric_out = run.stack.enter_context(contextlib.closing(
            SoundCapture(run.root, sink="fabric_viewer_out", description="fabric_viewer_out")))
        fabric = run.java(fabric_name, address, "loader", fabric_out.sink)
        register(fabric)
        # Manifest gate: the native client admits the computer bundle (loaders lists "native").
        for probe in (owner, viewer):
            state = probe.wait(lambda s: (s.get("loader") or {}).get("acknowledged") is True
                               and (s.get("loader") or {}).get("transfer_pending") is False,
                               "native_loader_acknowledged")
            leg.setdefault("loader", {})[probe.report["id"]] = state.get("loader")
            admitted = "computer:monitor" in state["loader"].get("registered_blocks", [])
            verdicts["native_loads_computer_monitor"] = admitted
            probe.check("native_loader_registered_computer_monitor", admitted)
        arena(owner, owner_name, (1000, 1000))
        owner.fixture("setblock {} {} {} minecraft:gray_glazed_terracotta[facing=north]".format(*MONITOR))
        owner.fixture("setworldspawn 1001 160 1001")
        leg["monitor_carrier"] = require_block(owner, MONITOR, "minecraft:gray_glazed_terracotta")
        place(owner, owner, owner_name, (999.5, 160.0, 1002.0), SCREEN)
        place(owner, viewer, viewer_name, (1001.5, 160.0, 1002.0), SCREEN)
        place(owner, fabric, fabric_name, (1000.5, 160.0, 1000.5), SCREEN)
        owner.wait(lambda s: (s.get("loader") or {}).get("acknowledged") is True, "owner_ready")

        # The owner right-clicks the monitor and picks the Test pattern (first entry).
        click(owner.client.p4_window, WINDOW[0] // 2, WINDOW[1] // 2, button=3)
        picker = owner.wait(lambda s: dialog_nodes(s).get("desktop/choice/0") == "Test pattern", "owner_picker_open")
        leg["picker"] = dialog_nodes(picker)
        keys(owner.client.p4_window["display"], "Return")
        viewer.wait(lambda s: len((s.get("screen_sharing") or {}).get("screens", [])) == 1, "viewer_sees_active_screen")
        fabric.call("minecraft_wait_ticks", {"ticks": 40})

        def request_and_answer(requester: ProbeSteps, allow: bool, label: str) -> dict[str, str]:
            if requester is fabric:
                fabric.call("minecraft_use_item_on", {"x": MONITOR[0], "y": MONITOR[1], "z": MONITOR[2], "face": "north"})
            else:
                click(requester.client.p4_window, WINDOW[0] // 2, WINDOW[1] // 2, button=3)
            asked = owner.wait(consent_dialog_open, f"{label}_consent_dialog")
            nodes = dialog_nodes(asked)
            leg.setdefault("consent_dialogs", {})[label] = {"nodes": nodes, "screenshot": native_shot(owner, f"{label}-consent")}
            keys(owner.client.p4_window["display"], *(("Down", "Return") if allow else ("Return",)))
            owner.wait(lambda s: "desktop/title" not in dialog_nodes(s), f"{label}_consent_answered")
            return nodes

        # Deny: the dialog discloses audio, and no audio capture or picture follows.
        nodes = request_and_answer(viewer, False, "viewer-deny")
        body = " ".join(text for key, text in nodes.items() if key.startswith("desktop/body/"))
        verdicts["owner_consent_names_app_audio"] = ("audio" in nodes.get("desktop/title", "").lower()
                                                     and "app audio" in body
                                                     and "audio" in nodes.get("desktop/choice/1", "").lower())
        before = viewer.observe()
        viewer.call("minecraft_wait_ticks", {"ticks": 100})
        after_deny = viewer.observe()
        owner_after_deny = owner.observe()
        leg["deny"] = {"viewer_before": before.get("screen_sharing"), "viewer_after": after_deny.get("screen_sharing"),
                       "viewer_audio": after_deny.get("screen_audio"), "owner_audio": owner_after_deny.get("screen_audio")}
        verdicts["owner_deny_sends_no_audio"] = (
            owner_after_deny["screen_audio"]["packets_sent"] == 0
            and owner_after_deny["screen_audio"]["capturing"] is False
            and not after_deny["screen_audio"]["heard"]
            and after_deny["screen_sharing"]["decoded_frames"] == before["screen_sharing"]["decoded_frames"])

        # Allow: picture and app audio reach the native viewer.
        request_and_answer(viewer, True, "viewer-allow")
        allowed = viewer.wait(lambda s: any(h["decoded_frames"] > 0 for h in s["screen_audio"]["heard"])
                              and s["screen_sharing"]["decoded_frames"] > before["screen_sharing"]["decoded_frames"],
                              "viewer_receives_picture_and_audio")
        owner_streaming = owner.wait(lambda s: s["screen_audio"]["capturing"] is True
                                     and s["screen_audio"]["packets_sent"] > 0, "owner_streams_audio")
        leg["allow"] = {"viewer": {"screen_sharing": allowed["screen_sharing"], "screen_audio": allowed["screen_audio"]},
                        "owner_audio": owner_streaming["screen_audio"],
                        "viewer_screenshot": native_shot(viewer, "viewer-allowed")}
        verdicts["owner_allow_streams_picture_and_audio"] = True

        # Fabric viewer: real OpenAL output before and after the owner allows it.
        quiet = fabric_out.sample(2.0, (440,))
        request_and_answer(fabric, True, "fabric-allow")
        fabric.call("minecraft_wait_ticks", {"ticks": 40})
        playing = fabric_out.sample(TONE_SECONDS, (440,))
        leg["fabric"] = {"before_allow": quiet, "after_allow": playing,
                         "screenshot": java_shot(fabric, "fabric-viewer-allowed"), "audio_file": str(fabric_out.path)}
        verdicts["fabric_loader_plays_screen_audio"] = silent(quiet, 440) and heard(playing, 440)

        # Same-dimension death and respawn keeps the monitor catalog and the stream.
        frames = viewer.observe()["screen_sharing"]["decoded_frames"]
        owner.fixture(f"kill {viewer_name}")
        state = runtime.current_client_state(viewer.client, viewer.remaining())
        while (state.get("player") or {}).get("health", 1) > 0:
            state = runtime.next_client_state(viewer.client, state, viewer.deadline)
        leg["death_state_catalog"] = state.get("screen_sharing")
        viewer.call("minecraft_respawn", {"timeout_seconds": min(viewer.remaining(), 60.0)})
        respawned = viewer.observe(wait_for_play=True)
        place(owner, viewer, viewer_name, (1001.5, 160.0, 1002.0), SCREEN)
        resumed = viewer.wait(lambda s: s["screen_sharing"]["decoded_frames"] > frames, "viewer_frames_after_respawn")
        leg["respawn"] = {"catalog_right_after_respawn": respawned["screen_sharing"],
                          "after": resumed["screen_sharing"], "screenshot": native_shot(viewer, "viewer-respawned")}
        verdicts["viewer_keeps_screen_after_same_dimension_respawn"] = len(respawned["screen_sharing"]["screens"]) == 1


# ---------------------------------------------------------------- voice leg

def svc_config(speaker: str, microphone: str) -> str:
    """SVC client settings: always transmitting, on this client's own private devices.

    config_version=1 is SVC 2.6's current format; without it SVC migrates the file and resets
    onboarding_finished and the denoiser (which would suppress the test tones)."""
    return "\n".join((
        "config_version=1",
        "microphone_activation_type=VOICE", "voice_activation_threshold=-50", "voice_activity_detection=false",
        "denoiser=false", "automatic_gain_control=false", "onboarding_finished=true", "muted=false",
        "disabled=false", "mute_on_join=false", f"microphone={microphone}", f"speaker={speaker}",
        "java_microphone_implementation=false", "voice_chat_volume=1.0", "",
    ))


def wait_svc_connected(log: Path, count: int, timeout: float) -> None:
    runtime.wait_for_path_condition(
        log, timeout, lambda: (log.read_text(errors="replace").count("voice chat client connected") >= count, None))


def voice_leg(ctx: ProfileContext, binary: Path, timeout: float, verdicts: dict[str, bool],
              evidence: dict[str, Any]) -> None:
    a_name, b_name, native_name = "SvcSpeakerA", "SvcSpeakerB", "NativeOutsider"
    with Run(ctx, binary, timeout, "voice") as run:
        leg = run.evidence
        evidence["voice"] = leg
        tones = {freq: write_tone(run.root / f"tone-{freq}.wav", freq, TONE_SECONDS) for freq in (440, 660)}
        native_tone = write_tone(run.root / "native-mic-880.wav", 880, 30)
        svc_port = runtime.reserve_port()
        voice = f'\n[voice]\nenabled = true\nsvc = true\nsvc_port = {svc_port}\nsvc_bind_address = "127.0.0.1"\n'
        address, server_log = run.server("server", voice, [a_name, b_name, native_name])
        mics = {}
        outs = {}
        for name in (a_name, b_name):
            mics[name] = Microphone(f"{name.lower()}_mic")
            run.stack.callback(mics[name].close)
            outs[name] = run.stack.enter_context(contextlib.closing(
                SoundCapture(run.root, sink=f"{name.lower()}_out", description=f"{name.lower()}_out")))
        clients = {}
        for name in (a_name, b_name):
            # The pinned SVC version leaves its group key unbound by default.
            files = {"config/voicechat/voicechat-client.properties": svc_config(outs[name].sink, mics[name].source),
                     "options.txt": java_options(8, 70) + "\nkey_key.voice_chat_group:key.keyboard.g\n"}
            clients[name] = run.java(name, address, "svc", outs[name].sink, files)
            register(clients[name])
        a, b = clients[a_name], clients[b_name]
        record = Recording(run.root / "native-heard.wav")
        native = run.native(native_name, address, {"voice_chat": True},
                            {"KRASNOKAMEN_VOICE_MIC_WAV": str(native_tone), "KRASNOKAMEN_VOICE_PTT_ALWAYS": "1",
                             "KRASNOKAMEN_VOICE_RECORD": str(record.path)})
        register(native)
        wait_svc_connected(server_log, 2, min(timeout, 120.0))
        arena(native, native_name, (1000, 1000))
        centre = (1002.5, 161.6, 1002.5)
        place(native, a, a_name, (1000.5, 160.0, 1000.5), centre)
        place(native, b, b_name, (1004.5, 160.0, 1000.5), centre)
        place(native, native, native_name, (1002.5, 160.0, 1004.5), centre)
        talking = native.wait(lambda s: s["voice"]["registered"] and s["voice"]["server_enabled"]
                              and s["voice"]["listening"] and voice_hud(s), "native_voice_negotiated")
        leg["native_on_voice_server"] = {"voice": talking["voice"], "voice_hud": voice_hud(talking),
                                         "screenshot": native_shot(native, "native-voice-server")}
        frequencies = (440, 660, 880)

        def phase(name: str, speaker: str) -> dict[str, Any]:
            start = {key: capture.position() for key, capture in outs.items()}
            heard_start = record.size()
            mics[speaker].speak(tones[440 if speaker == a_name else 660])
            drain = int(DRAIN_SECONDS * RATE) * SoundCapture.BYTES_PER_FRAME
            windows = {key: capture.segment(start[key], capture.position() + drain, frequencies)
                       for key, capture in outs.items()}
            windows[native_name] = record.segment(heard_start, record.size(), frequencies)
            leg.setdefault("phases", {})[name] = windows
            return windows

        proximity_a = phase("proximity_a_speaks", a_name)
        proximity_b = phase("proximity_b_speaks", b_name)
        verdicts["svc_proximity_fabric_to_fabric"] = (heard(proximity_a[b_name], 440)
                                                      and heard(proximity_b[a_name], 660))
        verdicts["svc_proximity_fabric_and_native"] = (heard(proximity_a[native_name], 440)
                                                       and heard(proximity_b[native_name], 660)
                                                       and heard(proximity_a[a_name], 880)
                                                       and heard(proximity_a[b_name], 880))

        # SVC's own group screens: A creates a NORMAL group, B joins it from the list.
        a.call("minecraft_press_inputs", {"keys": ["key.keyboard.g"], "ticks": 2})
        listing = a.wait(lambda s: s["screen"]["class"] == "de.maxhenkel.voicechat.gui.group.JoinGroupScreen",
                         "a_group_list_open")["screen"]
        a.call("minecraft_click_screen_button", {"expected_screen_class": listing["class"],
                                                 "expected_title": listing["title"], "button_label": "Create Group"})
        creating = a.wait(lambda s: s["screen"]["class"] == "de.maxhenkel.voicechat.gui.CreateGroupScreen",
                          "a_create_group_open")["screen"]
        # Group name box: guiLeft 7, guiTop 30 of the 195x124 screen, GUI scale 3 (java_options).
        scale = 3
        left, top = (WINDOW[0] // scale - 195) // 2, (WINDOW[1] // scale - 124) // 2
        click(a.client.p4_window, (left + 7 + 90) * scale, (top + 30 + 7) * scale)
        type_text(a.client.p4_window["display"], "krasnokamenqa")
        leg["create_group_screenshot"] = java_shot(a, "a-create-group")
        # CreateGroupScreen's confirm button is "Create" (message.voicechat.create); the list's is "Create Group".
        a.call("minecraft_click_screen_button", {"expected_screen_class": creating["class"],
                                                 "expected_title": creating["title"], "button_label": "Create"})
        created = a.wait(lambda s: s["screen"]["class"] == SVC_GROUP_SCREEN, "a_in_group")
        leg["a_group_screen"] = created["screen"]
        a.call("minecraft_close_screen")
        b.call("minecraft_press_inputs", {"keys": ["key.keyboard.g"], "ticks": 2})
        b.wait(lambda s: s["screen"]["class"] == "de.maxhenkel.voicechat.gui.group.JoinGroupScreen", "b_group_list_open")
        leg["join_list_screenshot"] = java_shot(b, "b-join-list")
        # First list row: list top guiTop(32)+16, 4 px padding, 36 px rows.
        click(b.client.p4_window, WINDOW[0] // 2, (32 + 16 + 4 + 18) * scale)
        joined = b.wait(lambda s: s["screen"]["class"] == SVC_GROUP_SCREEN, "b_in_group")
        leg["b_group_screen"] = joined["screen"]
        leg["b_group_screenshot"] = java_shot(b, "b-in-group")
        b.call("minecraft_close_screen")
        place(native, b, b_name, (1004.5, 160.0, 1200.5), (1004.5, 161.6, 1210.5))
        group_a = phase("group_a_speaks", a_name)
        group_b = phase("group_b_speaks", b_name)
        verdicts["svc_group_fabric_to_fabric"] = heard(group_a[b_name], 440) and heard(group_b[a_name], 660)
        verdicts["native_outsider_hears_no_group_audio"] = (silent(group_a[native_name], 440)
                                                            and silent(group_b[native_name], 660))

        # HC-1: reconnect to a server without voice; nothing from the old one survives.
        quiet_address, _ = run.server("no-voice-server", "\n[voice]\nenabled = false\n", [native_name])
        verdicts["native_voice_resets_on_non_voice_server"] = voice_reconnect(native, quiet_address, leg)


def voice_reconnect(native: ProbeSteps, quiet_address: str, leg: dict[str, Any]) -> bool:
    native.call("minecraft_disconnect")
    disconnected = native.observe(outside_play=True)
    leg["native_after_disconnect"] = {"voice": disconnected["voice"], "voice_hud": voice_hud(disconnected)}
    native.call("minecraft_connect", {"server_addr": quiet_address})
    wait_play(native)
    register(native)
    native.call("minecraft_wait_ticks", {"ticks": 100})
    after = native.observe()
    leg["native_on_non_voice_server"] = {"voice": after["voice"], "voice_hud": voice_hud(after),
                                         "screenshot": native_shot(native, "native-non-voice-server")}
    # Registration advertises this client's capability anew on every Play connection,
    # even when the next server has no voice support. Prove the old state cleared first.
    return (disconnected.get("in_play") is False
            and disconnected["voice"] == {"registered": False, "server_enabled": False, "listening": False}
            and not voice_hud(disconnected)
            and after["voice"]["server_enabled"] is False
            and after["voice"]["listening"] is False
            and not voice_hud(after))


def voice_hud(state: dict[str, Any]) -> list[dict[str, Any]]:
    """The native microphone indicator (MIC/TALK/...), shown only while a capture is open."""
    return [node for node in state.get("ui_nodes", []) if node.get("id") == "hud/voice_mic"]


if __name__ == "__main__":
    sys.exit(_main(sys.argv[1:]))
