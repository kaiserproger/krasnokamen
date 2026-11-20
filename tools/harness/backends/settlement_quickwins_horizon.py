"""Settlement quick wins on the owner's client, Horizon 26.3 — harness profile `settlement-quickwins-horizon`.

`python3 -m tools.harness run settlement-quickwins-horizon --timeout-seconds N -- [args]` starts the
server through the harness `Server` (pinned `KRASNOKAMEN_SERVER_BINARY`), an Xvfb display and the
pinned Horizon client (`--offscreen`, control socket), then runs the SAME walk
(`settlement_quickwins_qa.Walk`, `settlement_quickwins_steps.BLOCKS`, the same criteria) with the
perception and input of Horizon's own agent-control tools (`horizon-ctl mcp`): `state`/`screen`/`ui`
for the open dialog or chest menu, `click_at` / `click` (a mouse press at GUI pixels) for every GUI
press, `mouse right click` on the bell, `key esc` to close. Fixtures (pad, bell, house, chest stock)
go through `command`, as in the other clients. Horizon has no `ui_nodes`: the dialog's body, buttons
and inputs come from `state.dialog.open` and the button rectangles from `ui`.

Every process lives in the `ExitStack` of `run`, so a failure at any step (including the server or
Xvfb start) stops what was started; the harness writes `result.json` with the tree fingerprint and
this module records the server binary path and its sha256, so the receipt names the pin the walk ran
against. `Probe.wait` is a push: it observes, and if the predicate still fails it blocks on the
client's `wait-for state-change --since REV` (the `state.revision` the QA kit adds over the film pin),
which moves when the dialog, the position, a tracked entity or a block write changes. A `ru_ru` run
refuses before connect when the client's resources lack `assets/minecraft/lang/ru_ru.json`, because
such a client shows English item names next to the server's Russian text.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import uuid
from contextlib import ExitStack
from pathlib import Path
from typing import Any, Callable

from .. import runtime
from ..profiles import ProfileContext
from .gameplay import prepare_operator_fixture
from .horizon_lod import Server

W = Path("/run/media/kaiserroman/games/solaris-work")
HZ_TOOLS = W / "targets/hz-bots/tools"
CLIENT = Path(os.environ.get("KRASNOKAMEN_HORIZON_CLIENT",
                             W / "receipts/HzBattlePrep/client-pin/horizon-client"))
CTL = Path(os.environ.get("KRASNOKAMEN_HORIZON_CTL", W / "receipts/HzBattlePrep/client-pin/horizon-ctl"))
RESOURCES = Path(os.environ.get("KRASNOKAMEN_HORIZON_RESOURCES",
                                W / "targets/hzbattleprep-hz/.local/horizon-client/26.3"))
NAME = "HzQw"
SIZE = (1280, 720)
SCALE = 3
SEED = 100
DIALOG_CLASS = "net.minecraft.client.gui.screens.dialog.MultiButtonDialogScreen"
CONTAINER_CLASS = "net.minecraft.client.gui.screens.inventory.ContainerScreen"
CHEST_MENU = "net.minecraft.world.inventory.ChestMenu"


def to_menu_slot(hz: int) -> int | None:
    """Horizon slot id -> the chest menu's own numbering (0..53 chest, 54..80 inventory, 81..89 hotbar)."""
    if 300 <= hz < 354:
        return hz - 300
    if 9 <= hz <= 35:
        return 54 + hz - 9
    if 0 <= hz <= 8:
        return 81 + hz
    return None


def to_hz_slot(slot: int) -> int:
    if slot < 54:
        return 300 + slot
    return 9 + slot - 54 if slot < 81 else slot - 81


def plain_lore(components: str) -> list[str]:
    """Lore lines (plain text) of an SNBT `components` string: the `text:"..."` parts of each lore line, joined."""
    match = re.search(r'"minecraft:lore":\[(.*)\]', components or "")
    if not match:
        return []
    lines, depth, current = [], 0, ""
    for char in match.group(1):
        if char == "{":
            depth += 1
        if depth:
            current += char
        if char == "}":
            depth -= 1
            if depth == 0:
                lines.append("".join(re.findall(r'text:"((?:[^"\\]|\\.)*)"', current)).replace('\\"', '"'))
                current = ""
    return lines


def _braced(text: str, start: int) -> str:
    """The `{...}` object of an SNBT string starting at `start` (quotes respected)."""
    depth, quoted, index = 0, False, start
    while index < len(text):
        char = text[index]
        if char == "\\" and quoted:
            index += 1
        elif char == '"':
            quoted = not quoted
        elif not quoted and char == "{":
            depth += 1
        elif not quoted and char == "}":
            depth -= 1
            if depth == 0:
                return text[start:index + 1]
        index += 1
    return text[start:]


def display_name(components: str) -> str:
    """Plain custom name of an SNBT `components` string (own text, then `extra` parts)."""
    marker = '"minecraft:custom_name":'
    at = (components or "").find(marker)
    if at < 0:
        return ""
    obj = _braced(components, at + len(marker))
    return "".join(re.findall(r'text:"((?:[^"\\]|\\.)*)"', obj)).replace('\\"', '"')


def plain_title(title: Any) -> str:
    """Horizon reports a container title as a list of styled parts."""
    if isinstance(title, str):
        return title
    if isinstance(title, list):
        return "".join(plain_title(part) for part in title)
    if isinstance(title, dict):
        return plain_title(title.get("text", "")) + plain_title(title.get("extra", []))
    return ""


class Input:
    """Mouse and keys of the Horizon client (through its agent-control socket), in window pixels."""

    def __init__(self, hz: Any) -> None:
        self.hz = hz

    def click(self, point: tuple[int, int], button: int = 1) -> None:
        if button == 3:
            self.hz("mouse", button="right", action="click")
        else:
            self.hz("click_at", x=point[0] / SCALE, y=point[1] / SCALE)

    def tap(self, key: str) -> None:
        self.hz("key", key={"Escape": "esc"}.get(key, key.lower()), action="tap")

    def wheel(self, point: tuple[int, int], down: bool = True, notches: int = 1) -> None:
        self.hz("scroll_at", x=point[0] / SCALE, y=point[1] / SCALE, steps=-notches if down else notches)

    def close(self) -> None:
        pass


class Gui:
    """What `Walk` and `_enter_hub` take from the GUI object."""
    client = "horizon"

    def __init__(self, probe: "Probe") -> None:
        self.probe, self.size, self.scale, self.serial = probe, SIZE, SCALE, 0
        self.artifact = probe.artifact_dir
        self.input = Input(probe.hz)

    def capture(self, probe: "Probe", artifact: Path, name: str) -> str:
        path = artifact / f"{name}.png"
        probe.hz("screenshot", path=str(path))
        return str(path)

    def slot(self, slot: int, *, shift: bool = False) -> None:
        hz = to_hz_slot(slot)
        node = next((w for w in self.probe.hz("ui")["widgets"] if w.get("id") == f"slot-{hz}"), None)
        if node is None:
            raise RuntimeError(f"Horizon shows no slot-{hz} (menu slot {slot})")
        before = self.probe.hz("screen")
        entry = {"slot": slot, "hz_slot": hz, "shift": shift, "bounds": node["bounds"], "carried": before.get("carried"),
                 "title": plain_title(before.get("title", "")),
                 "item": next((r.get("item") for r in before["slots"] if r["slot"] == hz), None)}
        self.probe.report.setdefault("gui_clicks", []).append(entry)
        if shift:
            self.probe.hz("click", id=f"slot-{hz}", shift=True)
        else:
            x, y, width, height = node["bounds"]
            self.input.click((round((x + width / 2) * SCALE), round((y + height / 2) * SCALE)))
        self.probe.hz("wait", ticks=10)
        after = self.probe.hz("state")
        entry["after"] = {"screen": after.get("screen"), "dialog_title": ((after.get("dialog") or {}).get("open") or {}).get("title"),
                          "title": plain_title(self.probe.hz("screen").get("title", "")) if after.get("screen") == "container" else None}

    def close(self) -> None:
        pass


class Probe:
    """The `ProbeSteps` surface the walk uses, over Horizon's tools."""

    def __init__(self, hz: Any, artifact: Path, deadline_seconds: float = 3300) -> None:
        self.hz, self.artifact_dir = hz, artifact
        self.report: dict[str, Any] = {"id": "quickwins-horizon", "assertions": {}, "fixture_commands": []}
        self.transcript: list[dict[str, Any]] = []
        self.client = None
        self.deadline = time.monotonic() + deadline_seconds

    def remaining(self) -> float:
        left = self.deadline - time.monotonic()
        if left <= 0:
            raise TimeoutError("scenario deadline")
        return left

    def check(self, name: str, condition: bool) -> None:
        self.report["assertions"][name] = bool(condition)
        if not condition:
            raise RuntimeError(f"{self.report['id']} assertion failed: {name}")

    def fixture(self, command: str) -> None:
        """A privileged command. The server answers "Position is not loaded" while the pad's chunks are still
        generating (seen on Horizon: a fill that did nothing, then a founding without ground); retry such a
        refusal after 2 s, recording the retries."""
        self.report["fixture_commands"].append(command)
        for attempt in range(30):
            events = self.hz("events", limit=1)["events"]
            seq = events[-1]["seq"] if events else 0
            self.hz("command", command=command)
            self.hz("wait", ticks=2)
            replies = [e.get("text", "") for e in self.hz("events", since=seq)["events"] if e.get("event") == "chat"]
            if not any("Position is not loaded" in text for text in replies):
                return
            self.report.setdefault("fixture_retries", []).append({"command": command, "attempt": attempt})
            self.hz("wait", ticks=40)
        raise RuntimeError(f"fixture still refused as not loaded after 30 tries: {command}")

    # ---- the minecraft_* calls the walk makes
    def call(self, name: str, arguments: dict[str, Any] | None = None) -> dict[str, Any]:
        a = arguments or {}
        hz = self.hz
        if name == "minecraft_observe":
            return self.observe()
        if name == "minecraft_send_chat":
            return hz("command", command=a["message"]) if a.get("command") else hz("chat", text=a["message"])
        if name == "minecraft_wait_ticks":
            return hz("wait", ticks=int(a["ticks"]))
        if name == "minecraft_look":
            return hz("look", yaw=a["yaw_deg"], pitch=a["pitch_deg"])
        if name == "minecraft_scan_blocks":
            box = hz("blocks", x1=a["min_x"], y1=a["min_y"], z1=a["min_z"], x2=a["max_x"], y2=a["max_y"], z2=a["max_z"])
            rows = [{"x": x, "y": y, "z": z, "block_id": state.split("[")[0]} for x, y, z, state in box["blocks"]]
            return {"count": len(rows), "blocks": rows}
        if name == "minecraft_list_entities":
            rows = hz("entities", radius=a.get("radius", 32))["entities"]
            return {"entities": [{"entity_type": r["type"], "entity_uuid": r.get("uuid"), "x": r["pos"][0],
                                  "y": r["pos"][1], "z": r["pos"][2]} for r in rows]}
        if name == "minecraft_read_item_components":
            screen = hz("screen")
            row = next((r for r in screen["slots"] if r["slot"] == to_hz_slot(int(a["slot"]))), None)
            return {"components": {"minecraft:lore": plain_lore(row["components"]) if row and row.get("components") else []}}
        raise NotImplementedError(f"Horizon adapter has no mapping for {name}")

    def observe(self, *, wait_for_play: bool = False, outside_play: bool = False) -> dict[str, Any]:
        hz = self.hz
        state = hz("state")
        player = state["player"]
        out: dict[str, Any] = {"in_play": True, "player": {"x": player["position"][0], "y": player["position"][1],
                                                           "z": player["position"][2], "yaw": player["yaw"],
                                                           "pitch": player["pitch"]}}
        aim = (state.get("looking_at") or {}).get("block")
        if aim:
            x, y, z = aim["pos"]
            out["target"] = {"block_id": aim["block"], "x": x, "y": y, "z": z}
        out["recent_chat"] = hz("messages", n=30)["chat"]
        dialog = (state.get("dialog") or {}).get("open")
        kind = state.get("screen")
        screen: dict[str, Any] = {"open": False, "class": "none", "title": ""}
        nodes: list[dict[str, Any]] = []
        if dialog:
            screen = {"open": True, "class": DIALOG_CLASS, "title": dialog.get("title", "")}
            nodes.append({"id": "dialog/title/0", "role": "text", "label": dialog.get("title", ""), "enabled": True})
            for index, text in enumerate(dialog.get("body", [])):
                nodes.append({"id": f"dialog/body/{index}", "role": "text", "label": text, "enabled": True})
            for widget in hz("ui")["widgets"]:
                self.report.setdefault("widget_samples", {}).setdefault(widget["kind"], widget)
                if widget["kind"] != "button" or widget["label"] == "warning":
                    continue
                x, y, width, height = widget["bounds"]
                if re.match(r"(Тип постройки|Building type): ", str(widget["label"])):
                    # Horizon lists the dialog's building-type cycle button as a plain button: give it the input shape.
                    nodes.append({"id": "dialog/input/kind", "role": "button", "label": widget["label"],
                                  "enabled": bool(widget["enabled"]),
                                  "bounds": (round(x * SCALE), round(y * SCALE), round(width * SCALE), round(height * SCALE))})
                    continue
                nodes.append({"id": f"dialog/action/{len(nodes)}", "role": "button", "label": widget["label"],
                              "enabled": bool(widget["enabled"]),
                              "bounds": (round(x * SCALE), round(y * SCALE), round(width * SCALE), round(height * SCALE))})
        elif kind == "container":
            shown = hz("screen")
            if shown.get("screen") != "container":  # the screen changed between the two reads: not ready this poll
                out["screen"], out["ui_nodes"] = screen, nodes
                return out
            screen = {"open": True, "class": CONTAINER_CLASS, "title": plain_title(shown.get("title", ""))}
            slots = []
            for row in shown["slots"]:
                menu_slot = to_menu_slot(row["slot"])
                if menu_slot is None or not row.get("item"):
                    continue
                slots.append({"slot": menu_slot, "item_id": row["item"], "count": row["count"],
                              "name": display_name(row.get("components", "")) or row["item"]})
            out["container"] = {"menu_class": CHEST_MENU, "slot_count": 90, "slots": slots}
        elif kind not in ("playing", None):
            screen = {"open": True, "class": f"horizon.{kind}", "title": str(shown_title(hz, kind))}
        out["screen"] = screen
        out["ui_nodes"] = nodes
        return out

    def revision(self) -> int:
        """The observable-state revision `state` reports (dialog, position, entities, blocks)."""
        state = self.hz("state")
        revision = state.get("revision") if isinstance(state, dict) else None
        if revision is None:
            raise RuntimeError(f"Horizon state reports no revision: {state}")
        return int(revision)

    def push(self, observed: int, remaining: float) -> None:
        """Block until the client publishes the next observable-state revision.

        Horizon's `wait-for state-change` is a push: the client answers once its dialog, the
        player's position, a tracked entity or a block write changes, so the walk never polls a
        wall clock. The tool's own timeout is the only error, and the caller re-checks its step
        deadline after it.
        """
        reply = self.hz("wait_for", condition="state-change", since=observed,
                        timeout=min(30.0, max(0.1, remaining)))
        if not isinstance(reply, dict):
            raise RuntimeError(f"Horizon wait_for state-change returned {reply!r}")
        if reply.get("kind") == "timeout":
            return
        if reply.get("error"):
            raise RuntimeError(f"Horizon wait_for state-change failed: {reply['error']}")

    def wait(self, predicate: Callable[[dict[str, Any]], bool], label: str, *, wait_for_play: bool = False,
             outside_play: bool = False, timeout_seconds: float | None = None) -> dict[str, Any]:
        deadline = time.monotonic() + (timeout_seconds or 120)
        while True:
            observed = self.revision()
            state = self.observe()
            if predicate(state):
                self.report["assertions"][label] = True
                return state
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                self.report["assertions"][label] = False
                raise TimeoutError(f"timed out waiting for {label}")
            self.push(observed, remaining)


def shown_title(hz: Any, kind: str) -> str:
    return str(hz("screen").get("title", ""))


# ----------------------------------------------------------------------------- lifecycle


def validate_args(ctx: ProfileContext) -> tuple[argparse.Namespace, float]:
    parser = argparse.ArgumentParser(prog="settlement-quickwins-horizon")
    parser.add_argument("--horizon-client", type=Path, default=CLIENT)
    parser.add_argument("--horizon-ctl", type=Path, default=CTL)
    parser.add_argument("--horizon-resources", type=Path, default=RESOURCES)
    parser.add_argument("--scenario", choices=("quickwins", "stage2", "stage2op"), default="quickwins",
                        help="stage2: the stage-2 walk (certify by standing inside) with a non-operator player and "
                             "console fixtures; stage2op: the same player at ops.json level 2")
    parser.add_argument("--survey", action="store_true",
                        help="run every block and record unmet criteria instead of stopping at the first")
    try:
        args = parser.parse_args(ctx.extra_args[1:] if ctx.extra_args[:1] == ["--"] else ctx.extra_args)
    except SystemExit as error:
        raise ValueError("invalid settlement-quickwins-horizon arguments") from error
    for key in ("client", "ctl"):
        path = getattr(args, f"horizon_{key}")
        if not path.is_file() or not os.access(path, os.X_OK):
            raise ValueError(f"--horizon-{key} (or KRASNOKAMEN_HORIZON_{key.upper()}) must name an "
                             f"existing Horizon executable: {path}")
    if not args.horizon_resources.is_dir():
        raise ValueError("--horizon-resources (or KRASNOKAMEN_HORIZON_RESOURCES) must name Horizon's "
                         f"26.3 resources folder: {args.horizon_resources}")
    if ctx.platform is not None or ctx.plugins_dir is not None:
        raise ValueError("settlement-quickwins-horizon runs the bare server without Loader/plugins")
    args.language = "ru_ru"
    if args.language != "en_us" and not language_file(args.horizon_resources, args.language).is_file():
        raise RuntimeError(
            f"Horizon resources have no assets/minecraft/lang/{args.language}.json "
            f"({args.horizon_resources}): the client would translate its own text (item names, "
            f"tooltips) into en_us while the server sends {args.language} literals, so the run is "
            "not evidence; repair the resources (the QA kit ships the repaired language file)")
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else 3600.0
    if not timeout > 0:
        raise ValueError("timeout must be positive")
    return args, timeout


def language_file(resources: Path, language: str) -> Path:
    """Where Horizon keeps one of its translations (`assets/minecraft/lang/<locale>.json`)."""
    return resources / "assets" / "minecraft" / "lang" / f"{language}.json"


def server_config_text(port: int, world: Path, plugins: Path) -> str:
    """The walk's server: its own loopback port, world and empty plugins folder."""
    config = (runtime.REPO_ROOT / "example.toml").read_text()
    for old, new in [
        ('\nbind_address = "0.0.0.0"', '\nbind_address = "127.0.0.1"'),
        ("\nport = 25565\n# Per-address", f"\nport = {port}\n# Per-address"),
        ('world_dir = "world"', f'world_dir = {json.dumps(str(world))}'),
        ('directory = "plugins"', f'directory = {json.dumps(str(plugins))}'),
        ("seed = 0", f"seed = {SEED}"),
    ]:
        assert config.count(old) == 1, old
        config = config.replace(old, new, 1)
    return config + "\n"


def client_env(artifact: Path, display: str) -> dict[str, str]:
    """Horizon's own HOME/XDG tree under the artifact, Russian GUI at scale 3, null audio."""
    for sub in ("home", "xdg-data", "xdg-config", "runtime"):
        (artifact / sub).mkdir(parents=True, exist_ok=True, mode=0o700)
    home = artifact / "home"
    (home / "options.txt").write_text("onboardAccessibility:false\nlang:ru_ru\nguiScale:3\n")
    env = os.environ.copy()
    env.update(
        HOME=str(home), HORIZON_HOME=str(home), HORIZON_LAUNCHER_HOME=str(home / "launcher"),
        XDG_DATA_HOME=str(artifact / "xdg-data"), XDG_CONFIG_HOME=str(artifact / "xdg-config"),
        XDG_RUNTIME_DIR=str(artifact / "runtime"), DISPLAY=display,
        HORIZON_NO_AUDIO="1", ALSOFT_DRIVERS="null", HORIZON_NO_DISCORD="1",
        HORIZON_TELEMETRY_URL="off", HORIZON_TEST_LOOPBACK="1",
    )
    return env


def control(ctl: Path, socket: Path, home: Path, env: dict[str, str]) -> Any:
    """`horizon-ctl mcp` over the control socket (a seam: tests replace it)."""
    sys.path[:0] = [str(HZ_TOOLS)]
    import agent_control_e2e as upstream

    return upstream.Mcp(ctl, socket, home, env)


def walk(ctx: ProfileContext, args: argparse.Namespace, raw: Any, address: str, artifact: Path,
         timeout: float) -> dict[str, Any]:
    """The quick-wins walk over Horizon's tools; `scenario.json` is written even when it stops."""
    verdict: dict[str, Any] = {"status": "error"}
    probe: Probe | None = None

    def hz_call(tool: str, **arguments: Any) -> dict[str, Any]:
        reply = raw(tool, **arguments)
        image = reply.pop("_image", None)
        if tool == "screenshot" and image is not None:
            Path(arguments["path"]).write_bytes(image)
        return reply

    try:
        hz_call("connect", address=address, name=NAME)
        hz_call("wait_for", condition="loaded", timeout=timeout)
        hz_call("wait_ready", seconds=timeout)
        shots = artifact / "screenshots"
        shots.mkdir(exist_ok=True)
        probe = Probe(hz_call, shots, deadline_seconds=timeout)
        from . import settlement_quickwins_qa as qa
        from . import settlement_quickwins_steps as steps
        gui = Gui(probe)
        stage2 = args.scenario in ("stage2", "stage2op")
        if stage2:
            from . import settlement_quickwins_stage2 as stage
            from .settlement_quickwins_console import ConsoleFixtures
            criteria, blocks = stage.suite(args.scenario, "horizon")
        else:
            criteria, blocks = qa.CRITERIA, steps.BLOCKS
        session = qa.Walk(probe, shots, gui, args, criteria if stage2 else None)
        session.evidence["client"] = "horizon"
        if stage2:
            session.console = ConsoleFixtures(artifact, NAME)
            session.evidence["operator_fixture"] = ctx.client_evidence.get("operator_fixture")
        try:
            steps.setup(session)
            for name, owed, block in blocks:
                steps.run_block(session, name, owed, block)
            unmet = [cid for cid in criteria if not session.results.get(cid, {}).get("ok")]
            probe.check("QW all", not unmet)
            verdict = {"status": "passed"}
        except BaseException as error:
            verdict = {"status": "failed", "error": f"{type(error).__name__}: {error}"}
            raise
    finally:
        report = probe.report if probe is not None else {"quickwins": {}, "assertions": {}, "fixture_commands": []}
        (artifact / "scenario.json").write_text(json.dumps({
            "quickwins": report.get("quickwins", {}), "assertions": report.get("assertions", {}),
            "fixture_commands": report.get("fixture_commands", []),
            "gui_clicks": report.get("gui_clicks", []),
            "fixture_retries": report.get("fixture_retries", []),
            "widget_samples": report.get("widget_samples", {}),
            **verdict,
        }, ensure_ascii=False, indent=1, default=str) + "\n")
    return verdict


def run(ctx: ProfileContext) -> dict[str, Any]:
    """`settlement-quickwins-horizon`: the walk on the pinned Horizon client, owned by the harness."""
    args, timeout = validate_args(ctx)
    from ..profiles import _server_binary

    artifact = ctx.artifact_dir
    artifact.mkdir(parents=True, exist_ok=True)
    binary = _server_binary(ctx, "server-build.log")
    with binary.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    ctx.client_evidence.update(server_binary_path=str(binary), server_binary_sha256=digest,
                               horizon_client=str(args.horizon_client),
                               horizon_resources=str(args.horizon_resources))
    if args.scenario == "quickwins":
        prepare_operator_fixture(ctx, [NAME])
    elif args.scenario == "stage2op":
        prepare_operator_fixture(ctx, [NAME], 2)  # level 2 before the server starts (the console `op` grants 4)
    world, plugins = artifact / "world", artifact / "plugins"
    world.mkdir(exist_ok=True)
    plugins.mkdir(exist_ok=True)
    port = runtime.reserve_port()
    report: dict[str, Any] = {
        "scope": "the settlement quick wins walk on the pinned Horizon client (offscreen, harness Xvfb)",
        "verdict": {"status": "error"},
    }
    ctx.client_evidence["settlement_quickwins_horizon"] = report
    with ExitStack() as cleanup:
        socket_dir = Path(cleanup.enter_context(tempfile.TemporaryDirectory(prefix="hzqw-ctl-", dir="/tmp")))
        socket = socket_dir / "ctl.sock"
        server = Server(ctx, binary, artifact, "server", server_config_text(port, world, plugins), port,
                        console=args.scenario != "quickwins")
        with runtime.phase("server_start"):
            server.start()
            cleanup.callback(server.stop)
            server.wait_ready(timeout)
        xvfb, display, xvfb_log = runtime.start_xvfb(artifact)
        cleanup.callback(xvfb_log.close)
        cleanup.callback(runtime.stop_process, xvfb)
        env = client_env(artifact, display)
        home = artifact / "home"
        client_log = cleanup.enter_context((artifact / "client.log").open("wb"))
        command = [str(args.horizon_client), "--resources", str(args.horizon_resources),
                   "--offscreen", "--control-socket", str(socket)]
        ctx.commands.append(command)
        ctx.cwds.append(str(home))
        ctx.exits.append(None)
        ctx.logs.append(str(artifact / "client.log"))
        index = len(ctx.commands) - 1
        client = subprocess.Popen(command, cwd=home, env=env, stdout=client_log,
                                  stderr=subprocess.STDOUT, start_new_session=True)

        def stop_client() -> None:
            runtime.stop_process(client)
            ctx.exits[index] = client.returncode

        cleanup.callback(stop_client)
        runtime._wait_for_file(socket, timeout, client)
        raw = control(args.horizon_ctl, socket, home, env)
        cleanup.callback(raw.close)
        report["verdict"] = walk(ctx, args, raw, f"127.0.0.1:{port}", artifact, timeout)
    if report["verdict"]["status"] != "passed":
        raise RuntimeError("settlement quick wins on Horizon failed: "
                           + str(report["verdict"].get("error", report["verdict"]["status"])))
    return report
