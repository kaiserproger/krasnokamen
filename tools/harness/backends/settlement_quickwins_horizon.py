"""Owner-path acceptance of the settlement quick wins on the owner's client: Horizon 26.3.

A bounded adapter, not a harness profile. It starts a release server (KRASNOKAMEN_SERVER_BINARY), an
Xvfb display and the pinned Horizon client (`--offscreen`, control socket), then runs the SAME walk
(`settlement_quickwins_qa.Walk`, `settlement_quickwins_steps.BLOCKS`, the same 33 criteria) with the
perception and input of Horizon's own agent-control tools (`horizon-ctl mcp`): `state`/`screen`/`ui`
for the open dialog or chest menu, `click_at` / `click` (a mouse press at GUI pixels) for every GUI
press, `mouse right click` on the bell, `key esc` to close. Fixtures (pad, bell, house, chest stock)
go through `command`, as in the other clients. Horizon has no `ui_nodes`: the dialog's body, buttons
and inputs come from `state.dialog.open` and the button rectangles from `ui`.

    python3 -m tools.harness.backends.settlement_quickwins_horizon OUT_DIR [--survey]
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import time
import uuid
from pathlib import Path
from typing import Any, Callable

W = Path("/run/media/kaiserroman/games/solaris-work")
HZ_TOOLS = W / "targets/hz-bots/tools"
CLIENT = W / "receipts/HzBattlePrep/client-pin/horizon-client"
CTL = W / "receipts/HzBattlePrep/client-pin/horizon-ctl"
RESOURCES = W / "targets/hzbattleprep-hz/.local/horizon-client/26.3"
NAME = "HzQw"
SIZE = (1280, 720)
SCALE = 3
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

    def __init__(self, hz: Any, artifact: Path) -> None:
        self.hz, self.artifact_dir = hz, artifact
        self.report: dict[str, Any] = {"id": "quickwins-horizon", "assertions": {}, "fixture_commands": []}
        self.transcript: list[dict[str, Any]] = []
        self.client = None
        self.deadline = time.monotonic() + 3300

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
                if widget["kind"] != "button" or widget["label"] == "warning":
                    continue
                x, y, width, height = widget["bounds"]
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

    def wait(self, predicate: Callable[[dict[str, Any]], bool], label: str, *, wait_for_play: bool = False,
             outside_play: bool = False, timeout_seconds: float | None = None) -> dict[str, Any]:
        deadline = time.monotonic() + (timeout_seconds or 120)
        while True:
            state = self.observe()
            if predicate(state):
                self.report["assertions"][label] = True
                return state
            if time.monotonic() >= deadline:
                self.report["assertions"][label] = False
                raise TimeoutError(f"timed out waiting for {label}")
            time.sleep(0.25)


def shown_title(hz: Any, kind: str) -> str:
    return str(hz("screen").get("title", ""))


# ----------------------------------------------------------------------------- hosting


def host(out: Path):
    """Server + Xvfb + Horizon client. Returns (processes, address, socket, env, cwd)."""
    sys.path[:0] = [str(W / "solaris"), str(HZ_TOOLS)]
    from tools.harness import runtime
    core = W / "solaris"
    pin = Path(os.environ["KRASNOKAMEN_SERVER_BINARY"])
    port = runtime.reserve_port()
    for sub in ("home", "xdg-data", "xdg-config", "runtime", "plugins"):
        (out / sub).mkdir(parents=True, exist_ok=True, mode=0o700)
    conf = (core / "example.toml").read_text()
    for old, new in [('\nbind_address = "0.0.0.0"', '\nbind_address = "127.0.0.1"'),
                     ("\nport = 25565\n# Per-address", f"\nport = {port}\n# Per-address"),
                     ('world_dir = "world"', f'world_dir = {json.dumps(str(out / "world"))}'),
                     ('directory = "plugins"', f'directory = {json.dumps(str(out / "plugins"))}'),
                     ("seed = 0", "seed = 100")]:
        assert conf.count(old) == 1, old
        conf = conf.replace(old, new, 1)
    (out / "server.toml").write_text(conf + "\n")
    offline = uuid.UUID(bytes=hashlib.md5(f"OfflinePlayer:{NAME}".encode(), usedforsecurity=False).digest(), version=3)
    (out / "ops.json").write_text(json.dumps([{"uuid": str(offline), "name": NAME, "level": 4, "bypassesPlayerLimit": False}]))
    (out / "home/options.txt").write_text("onboardAccessibility:false\nlang:ru_ru\nguiScale:3\n")
    env = os.environ.copy()
    sock = Path("/tmp") / f"hzqw-{uuid.uuid4().hex}.sock"
    env.update(HOME=str(out / "home"), HORIZON_HOME=str(out / "home"), HORIZON_LAUNCHER_HOME=str(out / "home/launcher"),
               XDG_DATA_HOME=str(out / "xdg-data"), XDG_CONFIG_HOME=str(out / "xdg-config"),
               XDG_RUNTIME_DIR=str(out / "runtime"), KRASNOKAMEN_CONTENT_CACHE=str(core / "data/vanilla/26.3"),
               HORIZON_NO_AUDIO="1", ALSOFT_DRIVERS="null", HORIZON_NO_DISCORD="1", HORIZON_TELEMETRY_URL="off",
               HORIZON_TEST_LOOPBACK="1")
    sl, cl = (out / "server.log").open("wb"), (out / "client.log").open("wb")
    server = runtime.start_server([str(pin), "--config", str(out / "server.toml"), "--no-console"], cwd=out, env=env,
                                  stdout=sl, stderr=subprocess.STDOUT, start_new_session=True)
    runtime.wait_port(port, 180, server, log_path=out / "server.log")
    display, display_name_, _ = runtime.start_xvfb(out)
    env["DISPLAY"] = display_name_
    game = subprocess.Popen([str(CLIENT), "--resources", str(RESOURCES), "--offscreen", "--control-socket", str(sock)],
                            cwd=out / "home", env=env, stdout=cl, stderr=subprocess.STDOUT, start_new_session=True)
    runtime._wait_for_file(sock, 300, game)
    return runtime, (server, display, game), f"127.0.0.1:{port}", sock, env, out / "home"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("out", type=Path)
    parser.add_argument("--survey", action="store_true")
    args = parser.parse_args()
    args.language = "ru_ru"
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    runtime, (server, display, game), address, sock, env, home = host(out)
    hz = None
    verdict = {"status": "error"}
    try:
        import agent_control_e2e as upstream
        raw = upstream.Mcp(CTL, sock, home, env)

        def hz_call(tool: str, **arguments: Any) -> dict[str, Any]:
            reply = raw(tool, **arguments)
            image = reply.pop("_image", None)
            if tool == "screenshot" and image is not None:  # the MCP tool returns the PNG; keep it where the walk asked
                Path(arguments["path"]).write_bytes(image)
            return reply
        hz = hz_call
        hz("connect", address=address, name=NAME)
        hz("wait_for", condition="loaded", timeout=400)
        hz("wait_ready", seconds=400)
        shots = out / "screenshots"
        shots.mkdir(exist_ok=True)
        probe = Probe(hz, shots)
        from . import settlement_quickwins_qa as qa
        from . import settlement_quickwins_steps as steps
        gui = Gui(probe)
        walk = qa.Walk(probe, shots, gui, args)
        walk.evidence["client"] = "horizon"
        try:
            steps.setup(walk)
            for name, criteria, block in steps.BLOCKS:
                steps.run_block(walk, name, criteria, block)
            unmet = [cid for cid in qa.CRITERIA if not walk.results.get(cid, {}).get("ok")]
            probe.check("QW all", not unmet)
            verdict = {"status": "passed"}
        except BaseException as error:
            verdict = {"status": "failed", "error": f"{type(error).__name__}: {error}"}
            raise
        finally:
            (out / "scenario.json").write_text(json.dumps({"quickwins": probe.report["quickwins"],
                                                           "assertions": probe.report["assertions"],
                                                           "fixture_commands": probe.report["fixture_commands"],
                                                           "gui_clicks": probe.report.get("gui_clicks", []),
                                                           "fixture_retries": probe.report.get("fixture_retries", []),
                                                           **verdict}, ensure_ascii=False, indent=1, default=str) + "\n")
    finally:
        if hz is not None:
            for stop in (lambda: hz("quit"), lambda: raw.close()):
                try:
                    stop()
                except Exception:
                    pass
        runtime.stop_process(game)
        sock.unlink(missing_ok=True)
        runtime.stop_process(display)
        runtime.stop_process(server, interrupt=True)
        (out / "result.json").write_text(json.dumps(verdict) + "\n")
    print(json.dumps(verdict))
    return 0 if verdict["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
