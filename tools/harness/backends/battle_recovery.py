"""Battle crash-recovery real-client gate (profile ``battle-crash-recovery``).

Three named scenarios share one lifecycle; each asserts its own verdict:

* ``battle-kit-census`` — PovRed drops a kit piece, then SIGKILL -> recovery ->
  rejoin, death -> respawn and disconnect -> reconnect. A full crash census
  (docs/QA_AGENT_PROTOCOL.md#crash-census-evidence) reconciles every kit piece
  across the whole durable world at each phase: units never exceed the issued kit.
* ``battle-crash-handoff`` — SIGKILL before staging and staged-before-start: the
  battle runner keeps ``crash-handoff.json`` and the crash copy, recovery uses
  them and the real clients rejoin the recovered battle.
* ``battle-auth-hydration`` — a server with the first-party ``krasnokamen-auth``
  package; holder and spectator register, SIGKILL mid-battle, both rejoin locked
  (no kit, saved camera kept), then type ``/login`` through the real client: the
  kit returns exactly once and the camera binds again; a later re-entry issues
  nothing twice.

Census method (all on derived copies, never on the primary evidence): the stopped
SIGKILL image is hashed and copied before any reopen; hot ``backup`` images are
verified by ``krasnokamen restore``. ``world compact`` repairs only the copy's
redb file (no Core open, no tick); every other file of the copy (WAL, journal)
must hash the same before and after it, and both ``native/state.redb`` hashes are
recorded. ``world export-anvil`` projects checkpoint plus WAL tail into Anvil,
read back here: every player record (offline ones too), every entity of every
dimension and every block entity, with UUIDs and item components. The archived
WAL batches (CRC-checked JSON lines) give the ordered removals and lifecycle
clocks. Independent of ``world compact`` (which repacks rows) and of the export's
offline WAL replay, a second hash-verified copy of every image goes through the
production recovery: the server opens it (writable redb open, WAL replay into the
live core), is stopped before any player can join and writes its own checkpoint,
which is exported without repair. Both exports must agree on every kit item's
holder (UUID, dimension), slot, id, count and components; an item entity may only
age, never past the ticks since its durable birth (the reopened server may repay
simulation debt the image carries). Disagreement makes the census
incomplete. The crash export's checkpoint and replay ticks are compared with the
real recovery's own WAL: it reopens with exactly the crash tail and journals its
first new tick at ``replay_tick + 1``.

Inputs come from the environment and fail closed; the server only from
``KRASNOKAMEN_SERVER_BINARY`` (a pin directory with ``manifest.json``).
"""
from __future__ import annotations

import copy
import gzip
import hashlib
import importlib.util
import json
import os
import queue
import re
import secrets
import shutil
import signal
import struct
import subprocess
import sys
import threading
import time
import uuid
import zlib
from pathlib import Path
from typing import Any, Callable

from .. import runtime
from ..profiles import ProfileContext, _run_command
from ..quirk_parity import anvil
from ..runtime import REPO_ROOT

SCENARIOS = ("battle-kit-census", "battle-crash-handoff", "battle-auth-hydration")
HOLDER, VIEWER, RECORDER = "PovRed", "CamSpec", "BattleRecorder"
BATTLE_SEED, BATTLE_SCENARIO, PER_SIDE = 42, "field-1k", 1500
VIEW_DISTANCE = 18
# Vanilla item entity lifetime: an item at this age is despawned.
ITEM_LIFETIME_TICKS = 6000
STRIKE_TICK_LIMIT = 12000
RESUMED_STRIKE_TICK_LIMIT = 2000

COMMON_INPUTS = {
    "world": "KRASNOKAMEN_BATTLE_WORLD",
    "world_seed": "KRASNOKAMEN_BATTLE_WORLD_SEED",
    "horizon_client": "KRASNOKAMEN_BATTLE_HORIZON_CLIENT",
    "horizon_ctl": "KRASNOKAMEN_BATTLE_HORIZON_CTL",
    "horizon_resources": "KRASNOKAMEN_BATTLE_HORIZON_RESOURCES",
}
RUNNER_INPUTS = {
    "recorder": "KRASNOKAMEN_CLIENT_BINARY",
    "recorder_data": "KRASNOKAMEN_BATTLE_RECORDER_DATA",
}
EXECUTABLES = ("horizon_client", "horizon_ctl", "recorder")

BACKUP_DONE = re.compile(r"Backup (\S+) complete: checkpoint tick (\d+), recoverable to tick (\d+)")
BACKUP_FAILED = re.compile(r"Backup failed: .*")
FOUND = re.compile(r"Found (\d+) matching item\(s\) on player (\S+)")
EMPTY = re.compile(r"No items were found on player (\S+)")
STATE_REDB = "native/state.redb"


class Incomplete(RuntimeError):
    """A census case that cannot be verified: never a pass, never a red observation."""


def offline_uuid(name: str) -> str:
    digest = hashlib.md5(("OfflinePlayer:" + name).encode(), usedforsecurity=False).digest()
    return str(uuid.UUID(bytes=digest, version=3))


def _sha256(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def file_hashes(root: Path) -> dict[str, str]:
    """SHA-256 of every regular file under root, keyed by root-relative POSIX path."""
    return {path.relative_to(root).as_posix(): _sha256(path)
            for path in sorted(root.rglob("*")) if path.is_file() and not path.is_symlink()}


def _short(value: Any, limit: int = 600) -> str:
    text = value if isinstance(value, str) else json.dumps(value, sort_keys=True, default=str)
    return text if len(text) <= limit else text[:limit] + "..."


def check(report: dict[str, Any], name: str, condition: bool, detail: Any = None) -> None:
    """Record one named assertion; a false one fails the scenario with a ledger-readable error."""
    report["assertions"][name] = bool(condition)
    if detail is not None:
        report.setdefault("observations", {})[name] = detail
    if not condition:
        raise AssertionError(f"{report['scenario']} assertion failed: {name}: {_short(detail)}")


# ----------------------------------------------------------------------------- inputs


def resolve_inputs(ctx: ProfileContext, scenario: str, env: dict[str, str] | None = None) -> dict[str, Any]:
    """Fail-closed external inputs; every file's SHA-256 enters the receipt."""
    env = os.environ if env is None else env
    names = dict(COMMON_INPUTS)
    if scenario != "battle-auth-hydration":
        names.update(RUNNER_INPUTS)
    inputs: dict[str, Any] = {}
    evidence: dict[str, Any] = {}
    for key, variable in names.items():
        value = env.get(variable)
        if not value:
            raise ValueError(f"{scenario} requires {variable}")
        if key == "world_seed":
            if not re.fullmatch(r"-?\d+", value):
                raise ValueError(f"{variable} must be the battle world's integer seed")
            inputs[key] = int(value)
            evidence[variable] = inputs[key]
            continue
        path = Path(value).expanduser().resolve()
        if key in EXECUTABLES:
            if not path.is_file() or not os.access(path, os.X_OK):
                raise ValueError(f"{variable} is missing or not executable: {path}")
            evidence[variable] = {"path": str(path), "sha256": _sha256(path)}
        else:
            if not path.is_dir():
                raise ValueError(f"{variable} is not a directory: {path}")
            evidence[variable] = {"path": str(path)}
        inputs[key] = path
    if not (inputs["world"] / "native" / "state.redb").is_file():
        raise ValueError(f"KRASNOKAMEN_BATTLE_WORLD is not a stopped native world: {inputs['world']}")
    server = env.get("KRASNOKAMEN_SERVER_BINARY")
    if not server:
        raise ValueError("KRASNOKAMEN_SERVER_BINARY is required (a pin-release server)")
    binary = Path(server).expanduser().resolve()
    if not (binary.parent / "manifest.json").is_file():
        raise ValueError(f"KRASNOKAMEN_SERVER_BINARY must sit in a pin directory with manifest.json: {binary}")
    if scenario != "battle-auth-hydration" and json.loads((binary.parent / "manifest.json").read_text()).get("incremental"):
        raise ValueError(f"{scenario} runs tools/battle/run-battle.py, which refuses an incremental server pin: "
                         f"pin the tree with `tools/dev/pin-release --no-incremental <tree>` ({binary.parent})")
    inputs["server"] = binary
    evidence["KRASNOKAMEN_SERVER_BINARY"] = {"path": str(binary), "sha256": _sha256(binary)}
    cache = REPO_ROOT / "data" / "vanilla"
    if not cache.is_dir():
        raise ValueError(f"vanilla content cache missing: {cache}")
    inputs["cache"] = cache
    if scenario == "battle-auth-hydration":
        package = runtime.sibling_root("plugins") / "krasnokamen-auth"
        for name in ("plugin.toml", "plugin.wasm"):
            if not (package / name).is_file():
                raise ValueError(f"krasnokamen-auth package incomplete: {package / name}")
        inputs["auth_package"] = package
        evidence["auth_package"] = {"path": str(package), "plugin.wasm": _sha256(package / "plugin.wasm"),
                                    "plugin.toml": _sha256(package / "plugin.toml")}
    ctx.client_evidence["battle_inputs"] = evidence
    ctx.client_evidence.setdefault("server_binary_path", str(binary))
    ctx.client_evidence.setdefault("server_binary_sha256", evidence["KRASNOKAMEN_SERVER_BINARY"]["sha256"])
    return inputs


def _runner_module():
    spec = importlib.util.spec_from_file_location("battle_runner", REPO_ROOT / "tools" / "battle" / "run-battle.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# ----------------------------------------------------------------------------- processes and consoles


def _launch(ctx: ProfileContext, argv: list[str], log: Path | None, **popen: Any) -> tuple[subprocess.Popen, int]:
    """Start a long-lived child and record it in the receipt; returns (process, evidence index)."""
    ctx.commands.append([str(part) for part in argv])
    ctx.cwds.append(str(popen.get("cwd", REPO_ROOT)))
    ctx.exits.append(None)
    if log is not None:
        ctx.logs.append(str(log))
    return subprocess.Popen([str(part) for part in argv], **popen), len(ctx.exits) - 1


def _record_exit(ctx: ProfileContext, index: int, process: subprocess.Popen | None) -> None:
    if process is not None and process.poll() is not None:
        ctx.exits[index] = process.returncode


class Console:
    """A server whose stdin we own: every output line is logged and waited on by condition, never by timer."""

    def __init__(self, process: subprocess.Popen, log_path: Path):
        self.process, self.log_path = process, log_path
        self.lines: list[str] = []
        self.closed = False
        self.condition = threading.Condition()
        self.thread = threading.Thread(target=self._read, daemon=True)
        self.thread.start()

    def _read(self) -> None:
        try:
            with self.log_path.open("x") as output:
                for line in self.process.stdout:
                    output.write(line)
                    output.flush()
                    with self.condition:
                        self.lines.append(line.rstrip("\n"))
                        self.condition.notify_all()
        finally:
            with self.condition:
                self.closed = True
                self.condition.notify_all()

    def send(self, command: str) -> None:
        with (self.log_path.parent / f"{self.log_path.stem}-commands.txt").open("a") as record:
            record.write(command + "\n")
        self.process.stdin.write(command + "\n")
        self.process.stdin.flush()

    def mark(self) -> int:
        with self.condition:
            return len(self.lines)

    def text_since(self, mark: int) -> str:
        with self.condition:
            return "\n".join(self.lines[mark:])

    def count(self, pattern: str, mark: int = 0) -> int:
        regex = re.compile(pattern)
        with self.condition:
            return sum(1 for line in self.lines[mark:] if regex.search(line))

    def wait(self, pattern: str, mark: int = 0, timeout: float = 300, fail: re.Pattern | None = None) -> re.Match:
        regex = re.compile(pattern)
        deadline = time.monotonic() + timeout
        seen = mark
        with self.condition:
            while True:
                for line in self.lines[seen:]:
                    found = regex.search(line)
                    if found:
                        return found
                    if fail is not None and fail.search(line):
                        raise RuntimeError(f"server reported {line.strip()!r} while waiting for {pattern!r}")
                seen = len(self.lines)
                if self.closed:
                    raise RuntimeError(f"server exited before {pattern!r}")
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError(f"server did not log {pattern!r} within {timeout} s")
                self.condition.wait(remaining)

    def close(self) -> None:
        self.thread.join(timeout=10)


class FifoConsole:
    """The battle runner's server: commands through RUN_DIR/console.fifo, replies read from server.log."""

    def __init__(self, root: Path):
        self.root, self.log_path = root, root / "server.log"

    def send(self, command: str) -> None:
        with (self.root / "console.fifo").open("w") as fifo:
            fifo.write(command + "\n")

    def mark(self) -> int:
        return self.log_path.stat().st_size if self.log_path.exists() else 0

    def text_since(self, mark: int) -> str:
        with self.log_path.open("rb") as source:
            source.seek(mark)
            return source.read().decode(errors="replace")

    def count(self, pattern: str, mark: int = 0) -> int:
        return len(re.findall(pattern, self.text_since(mark)))

    def wait(self, pattern: str, mark: int = 0, timeout: float = 300, fail: re.Pattern | None = None) -> re.Match:
        regex = re.compile(pattern)

        def find() -> tuple[bool, Any]:
            text = self.text_since(mark) if self.log_path.exists() else ""
            found = regex.search(text)
            if found is None and fail is not None and (bad := fail.search(text)):
                raise RuntimeError(f"server reported {bad.group(0)!r} while waiting for {pattern!r}")
            return found is not None, found

        return runtime.wait_for_path_condition(self.log_path, timeout, find)


def online_census(console: Any, items: list[str], phase: str) -> dict[str, dict[str, int]]:
    """Auxiliary: `clear @a <item> 0` per kit item, delimited by `say` markers so replies group exactly."""
    token = uuid.uuid4().hex[:8]
    mark = console.mark()
    for index, item in enumerate(items):
        console.send(f"clear @a {item} 0")
        console.send(f"say A11Census{token}x{index}")
    console.wait(re.escape(f"[Server] A11Census{token}x{len(items) - 1}"), mark, timeout=120)
    text = console.text_since(mark)
    counts: dict[str, dict[str, int]] = {}
    start = 0
    for index, item in enumerate(items):
        end = text.index(f"[Server] A11Census{token}x{index}", start)
        group: dict[str, int] = {}
        for line in text[start:end].splitlines():
            if found := FOUND.search(line):
                group[found.group(2)] = int(found.group(1))
            elif empty := EMPTY.search(line):
                group[empty.group(1)] = 0
        counts[item] = group
        start = end + 1
    return counts


def hot_backup(console: Any, phase: str) -> dict[str, Any]:
    mark = console.mark()
    console.send("backup")
    found = console.wait(BACKUP_DONE.pattern, mark, timeout=600, fail=BACKUP_FAILED)
    return {"phase": phase, "path": found.group(1), "checkpoint_tick": int(found.group(2)),
            "wal_tick": int(found.group(3))}


# ----------------------------------------------------------------------------- Horizon real clients


class HorizonCtl:
    """`horizon-ctl mcp` over JSON-RPC. Arguments of a secret call never reach a log."""

    def __init__(self, ctl: Path, socket: Path, cwd: Path, env: dict[str, str], log: Path):
        self.log = log.open("a")
        self.proc = subprocess.Popen([str(ctl), "--socket", str(socket), "mcp"], cwd=cwd, env=env,
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log, text=True)
        self.replies: queue.Queue = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()
        self.next_id = 0
        init = self._rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                        "clientInfo": {"name": "battle_recovery", "version": "1"}}, 60)
        if init.get("serverInfo", {}).get("name") != "horizon-ctl":
            raise RuntimeError(f"not a horizon-ctl MCP server: {init}")
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.proc.stdin.flush()
        self.tools = {tool["name"] for tool in self._rpc("tools/list", {}, 60)["tools"]}

    def _read(self) -> None:
        for line in self.proc.stdout:
            self.replies.put(line)
        self.replies.put(None)

    def _rpc(self, method: str, params: dict[str, Any], timeout: float) -> dict[str, Any]:
        self.next_id += 1
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.next_id, "method": method,
                                          "params": params}) + "\n")
        self.proc.stdin.flush()
        try:
            line = self.replies.get(timeout=timeout)
        except queue.Empty:
            raise TimeoutError(f"horizon-ctl {method} gave no reply within {timeout} s") from None
        if line is None:
            raise RuntimeError(f"horizon-ctl exited during {method}")
        reply = json.loads(line)
        if reply.get("id") != self.next_id:
            raise RuntimeError(f"horizon-ctl reply out of order: {reply.get('id')} != {self.next_id}")
        if "error" in reply:
            raise RuntimeError(f"horizon-ctl {method}: {reply['error']}")
        return reply["result"]

    def __call__(self, tool: str, *, secret: bool = False, timeout: float = 120, **args: Any) -> dict[str, Any]:
        if tool not in self.tools:
            raise RuntimeError(f"horizon-ctl has no tool {tool}")
        if "timeout" in args:
            timeout = max(timeout, float(args["timeout"]) + 60)
        result = self._rpc("tools/call", {"name": tool, "arguments": args}, timeout)
        texts = [part for part in result.get("content", []) if part.get("type") == "text"]
        reply = json.loads(texts[-1]["text"]) if texts else {}
        images = [part for part in result.get("content", []) if part.get("type") == "image"]
        if images:
            import base64
            reply["_image"] = base64.b64decode(images[0]["data"])
        shown = "[redacted]" if secret else args
        self.log.write(json.dumps({"tool": tool, "args": shown, "error": bool(result.get("isError"))}) + "\n")
        self.log.flush()
        if result.get("isError"):
            raise RuntimeError(f"horizon-ctl {tool} failed: {'[redacted]' if secret else _short(reply)}")
        return reply

    def close(self) -> None:
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=30)
        finally:
            if self.proc.poll() is None:
                self.proc.kill()
                self.proc.wait()
            self.log.close()


class Horizon:
    """One real Horizon client process under the run's private Xvfb."""

    def __init__(self, ctx: ProfileContext, inputs: dict[str, Any], root: Path, name: str, display: str):
        self.ctx, self.inputs, self.name = ctx, inputs, name
        self.home = root / name
        self.home.mkdir(parents=True)
        self.socket = Path("/tmp") / f"battle-recovery-{uuid.uuid4().hex}.sock"
        self.process = self.driver = self.log = None
        self.index = -1
        self.env = os.environ.copy()
        for key, sub in (("HOME", "home"), ("HORIZON_HOME", "home"), ("HORIZON_LAUNCHER_HOME", "launcher"),
                         ("XDG_DATA_HOME", "data"), ("XDG_CONFIG_HOME", "config"), ("XDG_RUNTIME_DIR", "runtime")):
            path = self.home / sub
            path.mkdir(mode=0o700, exist_ok=True)
            self.env[key] = str(path)
        self.env.update(DISPLAY=display, HORIZON_NO_AUDIO="1", ALSOFT_DRIVERS="null", HORIZON_NO_DISCORD="1",
                        HORIZON_TELEMETRY_URL="off", HORIZON_TEST_LOOPBACK="1", HORIZON_LAUNCHER_NO_SELF_UPDATE="1")

    def launch(self, phase: str) -> None:
        """Start the client process and its control channel, not yet connected."""
        command = [self.inputs["horizon_client"], "--resources", self.inputs["horizon_resources"], "--offscreen",
                   "--control-socket", self.socket]
        self.log = (self.home / f"{phase}-client.log").open("xb")
        self.process, self.index = _launch(self.ctx, command, self.home / f"{phase}-client.log", cwd=self.home,
                                           env=self.env, stdout=self.log, stderr=subprocess.STDOUT,
                                           start_new_session=True)
        runtime._wait_for_file(self.socket, 300, self.process)
        self.driver = HorizonCtl(self.inputs["horizon_ctl"], self.socket, self.home, self.env,
                                 self.home / f"{phase}-ctl.jsonl")

    def connect(self, address: str) -> None:
        self.driver("connect", address=address, name=self.name)

    def ready(self) -> None:
        self.driver("wait_for", condition="loaded", timeout=400)
        self.driver("wait_ready", seconds=400)

    def start(self, address: str, phase: str) -> None:
        self.launch(phase)
        self.connect(address)
        self.ready()

    def last_event(self) -> int:
        return self.driver("events")["last"]

    def wait_kit(self, timeout: float = 60) -> None:
        """Until this client shows the kit's spear and shield, or the deadline: the server logs a rearm
        before its slot packets reach the client. The capture that follows reports what arrived."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            counts = self.driver("inventory")["counts"]
            if counts.get("minecraft:iron_spear") and counts.get("minecraft:shield"):
                return
            time.sleep(0.25)

    def wait_camera(self, entity: int, timeout: float = 60) -> None:
        """Until this spectator's camera is bound to `entity`, or the deadline (same reason as wait_kit)."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if (self.driver("state").get("camera") or {}).get("entity_id") == entity:
                return
            time.sleep(0.25)

    def capture(self, phase: str) -> dict[str, Any]:
        data = {key: self.driver(key) for key in ("state", "inventory")}
        shot = self.driver("screenshot", path=str(self.home / f"{phase}.png"))
        if "_image" in shot:
            (self.home / f"{phase}.png").write_bytes(shot["_image"])
        if not (self.home / f"{phase}.png").is_file():
            raise RuntimeError(f"{self.name} screenshot {phase} missing")
        (self.home / f"{phase}.json").write_text(json.dumps(data, indent=2) + "\n")
        return data

    def stop(self) -> None:
        try:
            if self.driver is not None:
                try:
                    self.driver("quit")
                except Exception as error:  # the client may already be gone; teardown continues
                    print(f"{self.name} quit: {error}", flush=True)
                self.driver.close()
        finally:
            self.driver = None
            runtime.stop_process(self.process)
            _record_exit(self.ctx, self.index, self.process)
            self.process = None
            self.socket.unlink(missing_ok=True)
            if self.log is not None:
                self.log.close()
                self.log = None


def roles(report: dict[str, Any], name: str, holder: dict[str, Any], viewer: dict[str, Any], entity: int) -> None:
    """Holder plays survival with exactly one of every kit piece; the spectator's camera is bound to him."""
    state, counts = holder["state"], holder["inventory"]["counts"]
    bound = viewer["state"].get("camera")
    check(report, name,
          state.get("screen") == "playing" and state.get("gamemode") == "survival" and bool(state.get("player"))
          and viewer["state"].get("screen") == "playing" and viewer["state"].get("gamemode") == "spectator"
          and bound is not None and bound.get("entity_id") == entity
          and counts.get("minecraft:iron_spear") == 1 and counts.get("minecraft:shield") == 1
          and all(count == 1 for count in counts.values()),
          {"holder_state": {k: state.get(k) for k in ("screen", "gamemode")}, "holder_counts": counts,
           "viewer_state": {k: viewer["state"].get(k) for k in ("screen", "gamemode", "camera")},
           "holder_entity": entity})


# ----------------------------------------------------------------------------- export reading


ITEM_KEYS = frozenset({"Item", "krasnokamen:Carried", "RecordItem", "Book", "item", "SaddleItem", "body_armor_item"})
ITEM_LIST_KEYS = frozenset({"Inventory", "Items", "EnderItems", "krasnokamen:CraftingGrid", "HandItems", "ArmorItems"})


def nbt_uuid(value: Any) -> str:
    if isinstance(value, list) and len(value) == 4:
        return str(uuid.UUID(bytes=struct.pack(">4i", *value)))
    if isinstance(value, str):
        return str(uuid.UUID(value))
    raise ValueError(f"not an NBT UUID: {value!r}")


def _is_stack(value: Any) -> bool:
    return isinstance(value, dict) and isinstance(value.get("id"), str) and ":" in value["id"]


def stacks_in(node: Any, path: str = "", item_context: bool = False):
    """Yield (path, stack) for every item stack under an NBT value, nested containers included.

    A stack is a compound with a namespaced `id` that either sits under a known item key or carries
    `count`; vanilla omits `count` when it is 1, so a known item key alone suffices."""
    if isinstance(node, dict):
        if item_context or (_is_stack(node) and "count" in node):
            if _is_stack(node):
                yield path, node
        for key, value in node.items():
            if key in ("Passengers",):
                continue
            child = f"{path}/{key}"
            if key == "equipment" and isinstance(value, dict):
                for slot, stack in value.items():
                    if _is_stack(stack):
                        yield from stacks_in(stack, f"{child}/{slot}", True)
            elif key in ITEM_KEYS and _is_stack(value):
                yield from stacks_in(value, child, True)
            elif key in ITEM_LIST_KEYS and isinstance(value, list):
                for index, stack in enumerate(value):
                    if _is_stack(stack):
                        yield from stacks_in(stack, f"{child}/{stack.get('Slot', index)}", True)
            elif isinstance(value, (dict, list)) and not (key == "id"):
                yield from stacks_in(value, child)
    elif isinstance(node, list):
        for index, value in enumerate(node):
            yield from stacks_in(value, f"{path}/{index}")


def kit_key(stack: dict[str, Any]) -> str:
    """Identity of a kit piece: item id and components, wear excluded (wear is tracked, not identity)."""
    components = {k: v for k, v in (stack.get("components") or {}).items() if k != "minecraft:damage"}
    return json.dumps([stack["id"], components], sort_keys=True)


def _stack_record(holder: str, kind: str, dimension: str | None, place: str, stack: dict[str, Any],
                  age: int | None = None) -> dict[str, Any]:
    components = stack.get("components") or {}
    return {"holder": holder, "kind": kind, "dimension": dimension, "place": place, "id": stack["id"],
            "count": int(stack.get("count", 1)), "components": components,
            "damage": components.get("minecraft:damage"), "key": kit_key(stack), "age": age}


def _entities(node: dict[str, Any], dimension: str):
    """An entity compound and its passengers, each with its own identity."""
    yield node
    for passenger in node.get("Passengers", []) or []:
        yield from _entities(passenger, dimension)


def read_export(root: Path) -> dict[str, Any]:
    """Every item stack of an exported Anvil world: players, entities and block entities of every dimension."""
    stacks: list[dict[str, Any]] = []
    entities: dict[str, dict[str, Any]] = {}
    players = []
    for path in sorted((root / "players" / "data").glob("*.dat")):
        raw = path.read_bytes()
        nbt = anvil.parse_nbt(gzip.decompress(raw) if raw[:2] == b"\x1f\x8b" else raw)
        holder = f"player:{uuid.UUID(path.stem)}"
        dimension = nbt.get("Dimension")
        players.append({"uuid": str(uuid.UUID(path.stem)), "dimension": dimension, "pos": nbt.get("Pos"),
                        "game_type": nbt.get("playerGameType")})
        for place, stack in stacks_in(nbt):
            stacks.append(_stack_record(holder, "player", dimension, place, stack))
    dimensions_root = root / "dimensions"
    dimensions = sorted(path for path in dimensions_root.glob("*/*") if path.is_dir())
    for directory in dimensions:
        dimension = f"{directory.parent.name}:{directory.name}"
        for mca in sorted((directory / "entities").glob("*.mca")):
            for _, _, chunk in anvil.read_region(mca):
                for top in chunk.get("Entities", []):
                    for entity in _entities(top, dimension):
                        ident = f"{dimension}/{nbt_uuid(entity['UUID'])}"
                        if ident in entities:
                            raise Incomplete(f"export holds entity {ident} twice")
                        kind = entity.get("id")
                        age = entity.get("Age") if kind == "minecraft:item" else None
                        entities[ident] = {"type": kind, "pos": entity.get("Pos"), "age": age}
                        holder = f"entity:{ident}"
                        for place, stack in stacks_in({k: v for k, v in entity.items() if k != "Passengers"}):
                            stacks.append(_stack_record(holder, "item_entity" if kind == "minecraft:item" else "entity",
                                                        dimension, place, stack, age))
        for mca in sorted((directory / "region").glob("*.mca")):
            for _, _, chunk in anvil.read_region(mca):
                for block in chunk.get("block_entities", []) or []:
                    holder = f"block:{dimension}/{block.get('x')},{block.get('y')},{block.get('z')}"
                    for place, stack in stacks_in({k: v for k, v in block.items() if k != "id"}):
                        stacks.append(_stack_record(holder, "container", dimension, place, stack))
    if not dimensions:
        raise Incomplete(f"export {root} has no dimension directories")
    return {"stacks": stacks, "entities": entities, "players": players,
            "dimensions": [f"{d.parent.name}:{d.name}" for d in dimensions]}


# ----------------------------------------------------------------------------- WAL history


def _visit(entry: dict[str, Any], dimension: str | None = None):
    if "Dimension" in entry:
        wrapped = entry["Dimension"]
        for child in wrapped["entries"]:
            yield from _visit(child, wrapped["dimension"])
        return
    yield dimension or "minecraft:overworld", entry


def read_batches(directories: list[Path]) -> dict[tuple[int, int], Path]:
    """Complete WAL batches of one server epoch; equal names in two sources must be byte-equal."""
    batches: dict[tuple[int, int], Path] = {}
    for directory in directories:
        if not directory.is_dir():
            continue
        for path in sorted(directory.glob("batch-*.bin")):
            found = re.fullmatch(r"batch-(\d+)-(\d+)\.bin", path.name)
            if not found:
                continue
            span = (int(found.group(1)), int(found.group(2)))
            data = path.read_bytes()
            if len(data) < 32:
                continue
            magic, version, low, high, length, crc = struct.unpack("<IIQQII", data[:32])
            body = data[32:]
            if magic != 0x534F4C4A or (low, high) != span or len(body) != length or zlib.crc32(body) != crc:
                continue  # torn tail: recovery replays only complete batches
            if span in batches and batches[span].read_bytes() != data:
                raise Incomplete(f"WAL batch {path.name} differs between {batches[span]} and {path}")
            batches.setdefault(span, path)
    return batches


def wal_history(batches: dict[tuple[int, int], Path]) -> dict[str, Any]:
    """Entity identity, removal tombstones and item clocks from the WAL; motion and battle lines are skipped."""
    spawns: list[dict[str, Any]] = []
    removals: list[dict[str, Any]] = []
    motions: dict[str, list[tuple[int, int]]] = {}
    for (low, high), path in sorted(batches.items()):
        for line in path.read_bytes()[32:].splitlines():
            if b'"Entity"' not in line and b'"RemoveEntity"' not in line and b'"ItemMotion"' not in line:
                continue
            for dimension, entry in _visit(json.loads(line)):
                if "Entity" in entry:
                    record = entry["Entity"]
                    spawns.append({"dimension": dimension, "id": record["id"], "uuid": record["uuid"],
                                   "tick": record.get("tick", high), "alive": record.get("alive"),
                                   "age": record.get("age"), "has_item": record.get("item") is not None})
                elif "RemoveEntity" in entry:
                    record = entry["RemoveEntity"]
                    removals.append({"dimension": dimension, "id": record["id"], "tick": record["tick"]})
                elif "ItemMotion" in entry:
                    record = entry["ItemMotion"]
                    motions.setdefault(f"{dimension}/{record['id']}", []).append((record["tick"], record["age"]))
    return {"spans": sorted(batches), "spawns": spawns, "removals": removals, "motions": motions}


def coverage_gaps(spans: list[tuple[int, int]], first: int, last: int) -> list[list[int]]:
    """Tick ranges of (first, last] no complete batch covers."""
    gaps, cursor = [], first + 1
    for low, high in sorted(spans):
        if high < cursor:
            continue
        if low > last:
            break
        if low > cursor:
            gaps.append([cursor, min(low - 1, last)])
        cursor = max(cursor, high + 1)
    if cursor <= last:
        gaps.append([cursor, last])
    return gaps


def durable_removal(history: dict[str, Any], ident: str, after: int, until: int) -> dict[str, Any]:
    """The ordered durable removal of an item entity `<dimension>/<uuid>` in (after, until], or why none is proven."""
    dimension, entity_uuid = ident.rsplit("/", 1)
    ids = {spawn["id"] for spawn in history["spawns"]
           if spawn["uuid"] == entity_uuid and spawn["dimension"] == dimension}
    if len(ids) != 1:
        return {"proven": False, "reason": f"WAL maps {ident} to entity ids {sorted(ids)}"}
    entity_id = ids.pop()
    others = [spawn for spawn in history["spawns"] if spawn["id"] == entity_id
              and spawn["dimension"] == dimension and spawn["uuid"] != entity_uuid and spawn["tick"] <= until]
    removals = [removal for removal in history["removals"] if removal["id"] == entity_id
                and removal["dimension"] == dimension and after < removal["tick"] <= until]
    if others and removals and min(o["tick"] for o in others) <= max(r["tick"] for r in removals):
        return {"proven": False, "reason": f"entity id {entity_id} reused by {others[0]['uuid']} in the window"}
    if not removals:
        return {"proven": False, "reason": f"no RemoveEntity for id {entity_id} in ({after}, {until}]"}
    tick = removals[0]["tick"]
    ages = [age for motion_tick, age in history["motions"].get(f"{dimension}/{entity_id}", []) if motion_tick <= tick]
    return {"proven": True, "id": entity_id, "tick": tick, "last_age": ages[-1] if ages else None,
            "expired": bool(ages) and ages[-1] >= ITEM_LIFETIME_TICKS - 1}


# ----------------------------------------------------------------------------- reconciliation


def issued_kit(census: dict[str, Any], holder_uuid: str) -> dict[str, int]:
    kit: dict[str, int] = {}
    for stack in census["stacks"]:
        if stack["holder"] == f"player:{holder_uuid}":
            kit[stack["key"]] = kit.get(stack["key"], 0) + stack["count"]
    return kit


def durable_issued_kit(report: dict[str, Any], census: dict[str, Any], items: list[str]) -> dict[str, int]:
    """The issued kit as the durable holder record has it; it must be the kit the real client shows."""
    kit = issued_kit(census, offline_uuid(HOLDER))
    ids = sorted({json.loads(key)[0] for key in kit})
    check(report, "issued_kit_durable_matches_client", bool(kit) and ids == items
          and all(count == 1 for count in kit.values()), {"durable": kit, "client_items": items})
    report["issued_kit"] = kit
    return kit


def _custody(stacks: list[dict[str, Any]], kit: dict[str, int], roster: set[str]):
    """Kit stacks in custody: everything but battle soldiers' equipment."""
    return (stack for stack in stacks if stack["key"] in kit
            and not (stack["kind"] == "entity" and stack["holder"].rsplit("/", 1)[-1] in roster))


def kit_units(census: dict[str, Any], kit: dict[str, int], roster: set[str]) -> dict[str, Any]:
    """Units of each kit piece in custody (players, item entities, containers, non-soldier entities)."""
    units = {key: 0 for key in kit}
    holders: dict[str, list[dict[str, Any]]] = {key: [] for key in kit}
    soldiers = {key: 0 for key in kit}
    for stack in census["stacks"]:
        if stack["key"] in kit and stack["kind"] == "entity" and stack["holder"].rsplit("/", 1)[-1] in roster:
            soldiers[stack["key"]] += stack["count"]
    for stack in _custody(census["stacks"], kit, roster):
        units[stack["key"]] += stack["count"]
        holders[stack["key"]].append({k: stack[k] for k in ("holder", "kind", "place", "count", "damage", "age")})
    return {"units": units, "holders": holders, "soldier_equipment": soldiers}


def entity_births(histories: list[dict[str, Any]]) -> dict[str, int]:
    """Earliest implied birth tick (record tick - age) per dimension/UUID over every durable Entity record.

    An entity never ages more than the ticks since its birth: a skipped (unadmitted) tick is only repaid
    later as simulation debt, so no record has `tick - age` below the birth and the drop record gives it."""
    births: dict[str, int] = {}
    for history in histories:
        for spawn in history["spawns"]:
            if spawn.get("age") is not None:
                key = f"{spawn['dimension']}/{spawn['uuid']}"
                births[key] = min(births.get(key, spawn["tick"] - spawn["age"]), spawn["tick"] - spawn["age"])
    return births


def reopen_agreement(census: dict[str, Any], kit: dict[str, int], roster: set[str],
                     births: dict[str, int]) -> dict[str, Any]:
    """Kit custody of the offline export against the production-recovery export of the same image.

    Every kit item agrees on holder (UUID and dimension), slot, item id, count and all components. An
    item entity's age may only grow, and neither image may hold it older than the ticks since its
    durable birth (`entity_births`): the reopened server can repay simulation debt the image carries, so
    the growth may exceed the ticks it ran (`bound_ticks`) but never the birth bound (`max_age`)."""
    reopen = census.get("reopen")
    if reopen is None:
        return {"agree": False, "reason": "no production-recovery export of this image"}
    bound = reopen["replay_tick"] - census["replay_tick"]

    def index(stacks: list[dict[str, Any]]) -> dict[str, list[int | None]]:
        found: dict[str, list[int | None]] = {}
        for stack in _custody(stacks, kit, roster):
            ident = json.dumps([stack["holder"], stack["kind"], stack["dimension"], stack["place"], stack["id"],
                                stack["count"], stack["components"]], sort_keys=True)
            found.setdefault(ident, []).append(stack["age"])
        return found

    offline, reopened = index(census["stacks"]), index(reopen["stacks"])
    ages = []
    for ident in sorted(offline.keys() & reopened.keys()):
        before = sorted(offline[ident], key=lambda age: (age is not None, age or 0))
        after = sorted(reopened[ident], key=lambda age: (age is not None, age or 0))
        birth = births.get(json.loads(ident)[0].split(":", 1)[1])
        limits = None if birth is None else [census["replay_tick"] - birth, reopen["replay_tick"] - birth]
        if len(before) != len(after) or any(
                not (a is None and b is None or a is not None and b is not None and limits is not None
                     and a <= b <= limits[1] and a <= limits[0])
                for a, b in zip(before, after)):
            ages.append({"item": ident, "offline": before, "reopen": after, "max_age": limits})
    result = {"bound_ticks": bound, "items": sum(len(found) for found in offline.values()),
              "only_offline": sorted(offline.keys() - reopened.keys()),
              "only_reopen": sorted(reopened.keys() - offline.keys()), "ages": ages}
    result["agree"] = bound >= 0 and not (result["only_offline"] or result["only_reopen"] or ages)
    return result


DISAGREE = "offline and production-recovery exports disagree"


def _kit_item_entities(census: dict[str, Any], kit: dict[str, int]) -> dict[str, dict[str, Any]]:
    found = {}
    for stack in census["stacks"]:
        if stack["kind"] == "item_entity" and stack["key"] in kit:
            found[stack["holder"].split(":", 1)[1]] = stack
    return found


def reconcile(phases: list[dict[str, Any]], kit: dict[str, int], roster: set[str],
              histories: list[dict[str, Any]]) -> dict[str, Any]:
    """Phase-by-phase custody verdict: `fail` if units exceed the issued kit, `incomplete` when a loss,
    identity or clock cannot be proven from durable evidence or the image's offline and
    production-recovery exports disagree (`reopen_agreement`), else `pass`.

    `phases[i]` = {phase, replay_tick, stacks, entities, epoch, reopen}; `histories[epoch]` is that
    server epoch's WAL history. A piece may leave custody only through an ordered durable removal."""
    violations, incomplete, rows, removals = [], [], [], []
    previous = None
    births = entity_births(histories)
    for phase in phases:
        units = kit_units(phase, kit, roster)
        agreement = reopen_agreement(phase, kit, roster, births)
        rows.append({"phase": phase["phase"], "replay_tick": phase["replay_tick"], **units, "reopen": agreement})
        if not agreement["agree"]:
            incomplete.append({"phase": phase["phase"], "reason": DISAGREE, "detail": agreement})
        for key, count in units["units"].items():
            if count > kit[key]:
                violations.append({"phase": phase["phase"], "key": key, "units": count, "issued": kit[key],
                                   "holders": units["holders"][key]})
        if previous is not None:
            history = histories[phase["epoch"]]
            after, until = previous["replay_tick"], phase["replay_tick"]
            if until <= after:
                incomplete.append({"phase": phase["phase"], "reason": f"replay tick {until} not after {after}"})
            gaps = coverage_gaps(history["spans"], after if phase["epoch"] == previous["epoch"]
                                 else phase.get("epoch_start", after), until)
            if gaps:
                incomplete.append({"phase": phase["phase"], "reason": f"WAL history gaps {gaps}"})
            before_entities = _kit_item_entities(previous, kit)
            after_entities = _kit_item_entities(phase, kit)
            removed_by_key: dict[str, int] = {}
            for ident, stack in before_entities.items():
                now = after_entities.get(ident)
                if now is not None:
                    if now["key"] != stack["key"]:
                        incomplete.append({"phase": phase["phase"], "reason": f"identity reuse: {ident} changed image"})
                    elif (now["age"] is not None and stack["age"] is not None and now["age"] < stack["age"]):
                        incomplete.append({"phase": phase["phase"], "reason": f"{ident} age fell {stack['age']} -> {now['age']}"})
                    continue
                if ident in phase["entities"]:
                    incomplete.append({"phase": phase["phase"], "reason": f"identity reuse: {ident} no longer holds the kit piece"})
                    continue
                proof = durable_removal(history, ident, after, until)
                if proof["proven"]:
                    removals.append({"phase": phase["phase"], "entity": ident, **proof})
                    removed_by_key[stack["key"]] = removed_by_key.get(stack["key"], 0) + stack["count"]
                else:
                    incomplete.append({"phase": phase["phase"], "entity": ident, "reason": "missing kit piece: "
                                       + proof["reason"]})
            for key, count in units["units"].items():
                lost = rows[-2]["units"][key] - count
                if lost > removed_by_key.get(key, 0):
                    incomplete.append({"phase": phase["phase"], "key": key,
                                       "reason": f"{lost} unit(s) left custody with {removed_by_key.get(key, 0)} ordered removal(s)"})
        previous = phase
    # A phase whose two exports disagree has no verified image: nothing about it is a pass or a red.
    unverified = any(entry["reason"] == DISAGREE for entry in incomplete)
    verdict = "incomplete" if unverified else "fail" if violations else "incomplete" if incomplete else "pass"
    return {"verdict": verdict, "violations": violations, "incomplete": incomplete, "phases": rows,
            "removals": removals}


def removal_control(phases: list[dict[str, Any]], index: int, kit: dict[str, int], roster: set[str],
                    histories: list[dict[str, Any]]) -> dict[str, Any]:
    """Remove one observed, nonexpired kit piece from a derived copy of census `index`; the census must catch it."""
    target = phases[index]
    observed = [stack for stack in target["stacks"] if stack["key"] in kit
                and not (stack["kind"] == "entity" and stack["holder"].rsplit("/", 1)[-1] in roster)
                and (stack["age"] is None or stack["age"] < ITEM_LIFETIME_TICKS)]
    if not observed:
        raise Incomplete(f"removal control: census {target['phase']} holds no observed kit piece")
    removed = next((stack for stack in observed if stack["kind"] == "item_entity"), observed[0])
    derived = copy.deepcopy(target)
    derived["stacks"] = [stack for stack in derived["stacks"] if stack != removed]
    if "reopen" in derived:  # the loss is in the durable image, so both exports of it lack the piece
        derived["reopen"]["stacks"] = [stack for stack in derived["reopen"]["stacks"] if not all(
            stack[k] == removed[k] for k in ("holder", "place", "key"))]
    if removed["kind"] == "item_entity":
        derived["entities"].pop(removed["holder"].split(":", 1)[1], None)
    derived["phase"] = target["phase"] + "-removal-control"
    result = reconcile(phases[:index] + [derived], kit, roster, histories)
    detected = result["verdict"] == "incomplete" and any(
        entry.get("reason") != DISAGREE
        and (removed["holder"].split(":", 1)[1] in json.dumps(entry) or removed["key"] == entry.get("key"))
        for entry in result["incomplete"])
    return {"removed": {k: removed[k] for k in ("holder", "kind", "place", "id", "count", "age")},
            "verdict": result["verdict"], "incomplete": result["incomplete"], "detected": detected,
            "derived_phase": derived}


# ----------------------------------------------------------------------------- census of one image


def census_config(server_toml: str, world: Path) -> str:
    """The run's server config pointed at a derived world, without a backup directory."""
    lines = []
    for line in server_toml.splitlines():
        if line.startswith("backup_dir ="):
            continue
        if line.startswith("world_dir ="):
            line = f"world_dir = {json.dumps(str(world))}"
        lines.append(line)
    text = "\n".join(lines) + "\n"
    if f"world_dir = {json.dumps(str(world))}" not in text:
        raise ValueError("server config has no world_dir line")
    return text


def take_census(ctx: ProfileContext, binary: Path, server_toml: Path, out: Path, *, phase: str, epoch: int,
                backup: dict[str, Any] | None = None, stopped: Path | None = None,
                hashes: dict[str, str] | None = None) -> dict[str, Any]:
    """Census of one durable image through a derived copy; the source is never opened."""
    out.mkdir(parents=True)
    derived = out / "world"
    relative = out.relative_to(ctx.artifact_dir).as_posix()
    if backup is not None:
        _run_command([str(binary), "restore", "--from", backup["path"], "--world-dir", str(derived)], ctx,
                     f"{relative}/restore.log")
    else:
        shutil.copytree(stopped, derived, symlinks=False)
        if file_hashes(derived) != hashes:
            raise Incomplete(f"derived copy of {stopped} does not match its recorded SHA-256")
    before = file_hashes(derived)
    reopen = reopen_image(ctx, binary, server_toml, out / "reopen", derived, before)
    config = out / "census.toml"
    config.write_text(census_config(server_toml.read_text(), derived))
    _run_command([str(binary), "--config", str(config), "world", "compact"], ctx, f"{relative}/compact.log")
    after = file_hashes(derived)
    compact = {"files": len(before), "state_redb_before": before.get(STATE_REDB),
               "state_redb_after": after.get(STATE_REDB),
               "changed": sorted(name for name in before.keys() | after.keys() if before.get(name) != after.get(name))}
    (out / "compact.json").write_text(json.dumps({**compact, "before": before, "after": after}, indent=1,
                                                 sort_keys=True) + "\n")
    if not set(compact["changed"]) <= {STATE_REDB} or STATE_REDB not in after:
        raise Incomplete(f"{phase}: world compact changed more than {STATE_REDB}: {compact['changed']}")
    receipt, records = export_image(ctx, binary, config, derived, out, phase)
    if backup is not None and receipt.get("replay_tick") != backup["wal_tick"]:
        raise Incomplete(f"{phase}: export replay tick {receipt.get('replay_tick')} != backup recoverable tick {backup['wal_tick']}")
    census = {"phase": phase, "epoch": epoch, "replay_tick": receipt["replay_tick"],
              "checkpoint_tick": receipt["checkpoint_tick"], "source": backup["path"] if backup else str(stopped),
              "export_receipt": str(out / "export.json"), "compact": compact, "reopen": reopen, **records}
    (out / "census.json").write_text(json.dumps(census, indent=1, sort_keys=True) + "\n")
    return census


def export_image(ctx: ProfileContext, binary: Path, config: Path, world: Path, out: Path,
                 label: str) -> tuple[dict[str, Any], dict[str, Any]]:
    """`world export-anvil` of one world into out/anvil, refused unless clean; returns (receipt, records)."""
    relative = out.relative_to(ctx.artifact_dir).as_posix()
    receipt_path = out / "export.json"
    _run_command([str(binary), "--config", str(config), "world", "export-anvil", "--source", str(world),
                  "--destination", str(out / "anvil"), "--receipt", str(receipt_path)], ctx, f"{relative}/export.log")
    receipt = json.loads(receipt_path.read_text())
    if receipt.get("status") != "PASS" or receipt.get("counts", {}).get("duplicate_uuid_dropped", 0) != 0 \
            or receipt.get("known_gaps"):
        raise Incomplete(f"census export of {label} not clean: {_short({k: receipt.get(k) for k in ('status', 'error', 'known_gaps', 'counts')})}")
    return receipt, read_export(out / "anvil")


def reopen_config(server_toml: str, world: Path, plugins: Path) -> tuple[str, Path | None]:
    """census_config with fresh listen ports and the plugin directory moved to a private copy."""
    lines, source = [], None
    for line in census_config(server_toml, world).splitlines():
        if line.startswith("port ="):
            line = f"port = {runtime.reserve_port()}"
        elif line.startswith("directory ="):
            source = Path(json.loads(line.split("=", 1)[1].strip()))
            line = f"directory = {json.dumps(str(plugins))}"
        lines.append(line)
    return "\n".join(lines) + "\n", source


def reopen_image(ctx: ProfileContext, binary: Path, server_toml: Path, out: Path, source: Path,
                 hashes: dict[str, str]) -> dict[str, Any]:
    """The same durable image through production recovery, independent of `world compact` and of the
    export's offline WAL replay: a second hash-verified copy is opened by the server (writable redb open,
    WAL replay into the live core, the server's own final checkpoint), stopped before any player can
    join (fresh ports, no client), and exported without repair."""
    out.mkdir()
    world = out / "world"
    shutil.copytree(source, world, symlinks=False)
    if file_hashes(world) != hashes:
        raise Incomplete(f"reopen copy of {source} does not match its recorded SHA-256")
    text, plugins = reopen_config(server_toml.read_text(), world, out / "plugins")
    if plugins is not None:
        shutil.copytree(plugins, out / "plugins", symlinks=False)
    config = out / "server.toml"
    config.write_text(text)
    process, index = _launch(ctx, [str(binary), "--config", str(config)], out / "server.log", cwd=out,
                             stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                             bufsize=1)
    console = Console(process, out / "server.log")
    try:
        console.wait(r"Krasnokamen is listening \(edge v2\)", timeout=600)
        console.send("stop")
        code = process.wait(timeout=300)
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=30)
        _record_exit(ctx, index, process)
        console.close()
    joined = console.count(r"joined the game|Battle player online:")
    if code != 0 or joined:
        raise Incomplete(f"reopen of {source}: exit {code}, {joined} join line(s)")
    receipt, records = export_image(ctx, binary, config, world, out, f"reopen of {source}")
    return {"replay_tick": receipt["replay_tick"], "checkpoint_tick": receipt["checkpoint_tick"],
            "server_log": str(out / "server.log"), "export_receipt": str(out / "export.json"),
            "stacks": records["stacks"]}


def crash_evidence(ctx: ProfileContext, root: Path, crash_copy: Path) -> dict[str, Any]:
    """Before any reopen: hashes of the stopped world (WAL, checkpoint, everything) and its preserved copy.

    The real recovery then reopens the stopped world in place (the production path); the preserved
    copy is the crash image every census derives from and must hash the same after the run."""
    world_hashes = file_hashes(root / "world")
    copy_hashes = file_hashes(crash_copy)
    evidence = {"world": str(root / "world"), "crash_copy": str(crash_copy), "world_files": world_hashes,
                "files": copy_hashes,
                "wal": {k: v for k, v in copy_hashes.items() if k.startswith("journal/")},
                "checkpoint": {k: v for k, v in copy_hashes.items() if k.startswith("native/")},
                "copy_equals_world": world_hashes == copy_hashes}
    (root / "crash-evidence.json").write_text(json.dumps(evidence, indent=1, sort_keys=True) + "\n")
    return evidence


def recovery_tick_evidence(pre: dict[tuple[int, int], Path], recovery: dict[tuple[int, int], Path],
                           crash: dict[str, Any]) -> dict[str, Any]:
    """The crash export's ticks against the real recovery's own WAL.

    The server logs no recovered tick: it sets `max(replay.tick, manifest.tick)` (core/recovery.rs) and
    journals one batch per tick, so it reopens with the crash tail (checkpoint_tick, replay_tick] and its
    first batch absent from the crash image is recovered tick + 1."""
    low, high = crash["checkpoint_tick"], crash["replay_tick"]
    tail = sorted(span for span in pre if span[0] > low and span[1] <= high)
    carried = sorted(span for span in recovery if span in pre)
    differing = [list(span) for span in carried if pre[span].read_bytes() != recovery[span].read_bytes()]
    new = sorted(span for span in recovery if span not in pre)
    first = new[0][0] if new else None
    return {"export_checkpoint_tick": low, "export_replay_tick": high,
            "reopened_tail": [carried[0][0], carried[-1][1]] if carried else None,
            "reopened_tail_batches": len(carried), "crash_tail_batches": len(tail), "differing": differing,
            "first_new_tick": first, "recovered_tick": None if first is None else first - 1,
            "match": bool(tail) and carried == tail and not differing and tail[0][0] == low + 1
            and tail[-1][1] == high and first == high + 1}


def check_recovery_tick(report: dict[str, Any], root: Path, pre: dict[tuple[int, int], Path],
                        recovery: dict[tuple[int, int], Path], crash: dict[str, Any]) -> None:
    evidence = recovery_tick_evidence(pre, recovery, crash)
    (root / "recovery-tick.json").write_text(json.dumps(evidence, indent=1, sort_keys=True) + "\n")
    check(report, "export_ticks_match_real_recovery", evidence["match"], evidence)


def summarize(result: dict[str, Any]) -> dict[str, Any]:
    return {"verdict": result["verdict"], "violations": result["violations"], "incomplete": result["incomplete"],
            "removals": result["removals"],
            "units": [{"phase": row["phase"], "replay_tick": row["replay_tick"], "units": row["units"],
                       "soldier_equipment": row["soldier_equipment"], "holders": row["holders"],
                       "reopen": row["reopen"]}
                      for row in result["phases"]]}


def finish_census(report: dict[str, Any], phases: list[dict[str, Any]], kit: dict[str, int], roster: set[str],
                  histories: list[dict[str, Any]], root: Path, control_index: int, exact: set[str]) -> None:
    """The verdict over every phase, its removal control, and the exact-count phases."""
    result = reconcile(phases, kit, roster, histories)
    report["census"] = summarize(result)
    (root / "census-reconcile.json").write_text(json.dumps(report["census"], indent=1, sort_keys=True) + "\n")
    compact = {phase["phase"]: {k: phase["compact"][k] for k in ("changed", "state_redb_before", "state_redb_after")}
               for phase in phases}
    check(report, "census_compact_changed_only_state_redb",
          all(set(row["changed"]) <= {STATE_REDB} for row in compact.values()), compact)
    agreement = {row["phase"]: row["reopen"] for row in result["phases"]}
    report["assertions"]["census_offline_and_reopen_agree"] = all(row["agree"] for row in agreement.values())
    report.setdefault("observations", {})["census_offline_and_reopen_agree"] = agreement
    if not report["assertions"]["census_offline_and_reopen_agree"]:
        raise Incomplete(f"crash census incomplete: {DISAGREE}: "
                         f"{_short({k: v for k, v in agreement.items() if not v['agree']})}")
    for row in result["phases"]:
        check(report, f"kit_units_within_issued_{row['phase']}",
              all(row["units"][key] <= kit[key] for key in kit), {"units": row["units"], "issued": kit})
        if row["phase"] in exact:
            check(report, f"kit_exactly_once_{row['phase']}", row["units"] == kit,
                  {"units": row["units"], "issued": kit})
    control = removal_control(phases, control_index, kit, roster, histories)
    (root / "census-removal-control.json").write_text(json.dumps(control, indent=1, sort_keys=True) + "\n")
    report["removal_control"] = {k: v for k, v in control.items() if k != "derived_phase"}
    check(report, "census_removal_control_detects_loss", control["detected"], report["removal_control"])
    if result["verdict"] == "incomplete":
        raise Incomplete(f"crash census incomplete: {_short(result['incomplete'])}")


# ----------------------------------------------------------------------------- battle-runner sessions


class RunnerSession:
    """One `tools/battle/run-battle.py --crash-handoff` battle with two real clients."""

    def __init__(self, ctx: ProfileContext, inputs: dict[str, Any], root: Path, display: str, *,
                 paused_ticks: int = 0, console_fifo: bool = False):
        self.ctx, self.inputs, self.root, self.display = ctx, inputs, root, display
        self.battle = _runner_module()
        self.port, self.dashboard = runtime.reserve_port(), runtime.reserve_port()
        self.address = f"127.0.0.1:{self.port}"
        self.command = [sys.executable, str(REPO_ROOT / "tools" / "battle" / "run-battle.py"),
                        "--pin", str(inputs["server"].parent), "--client", str(inputs["recorder"]),
                        "--client-data", str(inputs["recorder_data"]), "--cache", str(inputs["cache"]),
                        "--run-dir", str(root), "--world", str(inputs["world"]),
                        "--world-seed", str(inputs["world_seed"]), "--seed", str(BATTLE_SEED),
                        "--scenario", BATTLE_SCENARIO, "--count", str(PER_SIDE), "--duration", "1800",
                        "--view-distance", str(VIEW_DISTANCE), "--pre-stage-ticks", "1200",
                        *(["--paused-ticks", str(paused_ticks)] if paused_ticks else []),
                        "--port", str(self.port), "--dashboard-port", str(self.dashboard),
                        "--live-player", f"{HOLDER}:red", "--live-spectator", VIEWER, "--live-timeout", "600",
                        "--crash-handoff", *(["--console-fifo"] if console_fifo else []),
                        "--allow-marker-gaps", "--no-phase-log"]
        self.runner = self.recovery = self.console = self.archive = None
        self.runner_index = self.recovery_index = -1
        self.clients: list[Horizon] = []
        self.driver_log = None

    def start(self) -> None:
        self.driver_log = (self.root.parent / f"{self.root.name}-runner.log").open("x")
        self.runner, self.runner_index = _launch(self.ctx, self.command, self.root.parent / f"{self.root.name}-runner.log",
                                                 cwd=REPO_ROOT, stdout=self.driver_log, stderr=subprocess.STDOUT)
        self.battle.watch_ready(self.root, self.runner, timeout=120, label="new run directory")
        runtime.wait_port(self.port, 300, self.runner, log_path=self.root / "server.log")
        for name in (HOLDER, VIEWER):
            client = Horizon(self.ctx, self.inputs, self.root, name, self.display)
            self.clients.append(client)
            client.start(self.address, "before")

    @property
    def holder(self) -> Horizon:
        return self.clients[0]

    @property
    def viewer(self) -> Horizon:
        return self.clients[1]

    def first_strike(self, dashboard: int | None = None, limit: int = STRIKE_TICK_LIMIT,
                     need_fighting: bool = True) -> tuple[dict[str, Any], list[dict[str, Any]]]:
        after, rows_seen = None, []
        while True:
            text = self.battle.telemetry_get(dashboard or self.dashboard, after, 1)
            rows = [json.loads(line) for line in text.splitlines() if line]
            rows_seen.extend(rows)
            after = rows[-1]["tick"]
            hits = [row for row in rows if row.get("battle", {}).get("strikes", 0) > 0
                    and (not need_fighting or row["battle"].get("fighting", 0) > 0)]
            if hits:
                return hits[-1], rows_seen
            if after - rows_seen[0]["tick"] > limit:
                raise RuntimeError(f"battle produced no ordinary native strike within {limit} ticks")

    def sigkill(self, report: dict[str, Any], name: str) -> dict[str, Any]:
        server_pid = json.loads((self.root / "server-pid.json").read_text())["pid"]
        os.kill(server_pid, signal.SIGKILL)
        code = self.runner.wait(timeout=300)
        _record_exit(self.ctx, self.runner_index, self.runner)
        handoff_path = self.root / "crash-handoff.json"
        handoff = json.loads(handoff_path.read_text()) if handoff_path.is_file() else None
        archive_errors = (json.loads((self.root / "wal-archive-errors.json").read_text())
                          if (self.root / "wal-archive-errors.json").is_file() else None)
        detail = {"runner_exit": code, "handoff": handoff, "wal_archive_errors": archive_errors,
                  "crash_copy_world": (self.root / "crash-copy" / "world").is_dir(),
                  "archived_batches": len(list((self.root / "crash-copy" / "wal-archive").glob("batch-*.bin")))}
        check(report, name, code == 0 and handoff is not None and handoff.get("exit") == -signal.SIGKILL
              and archive_errors == [] and detail["crash_copy_world"], detail)
        for client in self.clients:
            client.stop()
        return detail

    def recover(self, archive: bool = False) -> Console:
        if archive:
            self.archive = self.battle.WalArchive(self.root / "world" / "journal", self.root / "wal-archive-recovery")
        command = [sys.executable, str(REPO_ROOT / "tools" / "battle" / "run-battle.py"), "--recover-from-crash",
                   str(self.root)]
        (self.root / "recovery-command.json").write_text(json.dumps(command) + "\n")
        self.recovery, self.recovery_index = _launch(self.ctx, command, self.root / "recovery.log", cwd=self.root,
                                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                                     stderr=subprocess.STDOUT, text=True, bufsize=1)
        self.console = Console(self.recovery, self.root / "recovery.log")
        self.console.wait(r"Krasnokamen is listening \(edge v2\)", timeout=600)
        return self.console

    def stop_recovery(self, report: dict[str, Any], name: str) -> None:
        mark = self.console.mark()
        self.console.send("save-all")
        self.console.wait(r"Saved the game", mark, timeout=300)
        self.console.send("stop")
        code = self.recovery.wait(timeout=300)
        _record_exit(self.ctx, self.recovery_index, self.recovery)
        self.console.close()
        errors = None
        if self.archive is not None:
            self.archive.close()
            errors = self.archive.errors
            (self.root / "wal-archive-recovery-errors.json").write_text(json.dumps(errors, indent=2) + "\n")
            self.archive = None
        check(report, name, code == 0 and not errors, {"recovery_exit": code, "wal_archive_errors": errors})

    def close(self) -> None:
        for client in self.clients:
            client.stop()
        if self.archive is not None:
            self.archive.close()
        self.battle.stop_child(self.recovery, 30)
        _record_exit(self.ctx, self.recovery_index, self.recovery)
        self.battle.stop_child(self.runner, 30)
        _record_exit(self.ctx, self.runner_index, self.runner)
        if self.driver_log is not None:
            self.driver_log.close()


def _holder_entity(console: Any, mark: int = 0) -> int:
    return int(console.wait(rf"Battle player online: {HOLDER} entity=(\d+)", mark).group(1))


def _roster(path: Path) -> set[str]:
    return {line.strip() for line in path.read_text().splitlines() if line.strip()}


def _phase_census(console: Any, items: list[str], phase: str, aux: dict[str, Any],
                  clients: list[Horizon] | None = None) -> dict[str, Any]:
    """Auxiliary live counts plus the hot backup that the durable census reads later."""
    aux[phase] = {"clear": online_census(console, items, phase)}
    if clients:
        aux[phase]["client_inventories"] = {client.name: client.driver("inventory")["counts"] for client in clients
                                            if client.driver is not None}
        viewer = next((client for client in clients if client.name == VIEWER and client.driver is not None), None)
        if viewer is not None:
            aux[phase]["radius64_items"] = viewer.driver("entities", radius=64, type="minecraft:item")
    return hot_backup(console, phase)


# ----------------------------------------------------------------------------- verdict 1


def kit_census(ctx: ProfileContext, inputs: dict[str, Any], report: dict[str, Any], display: str) -> None:
    root = ctx.artifact_dir / "battle"
    session = RunnerSession(ctx, inputs, root, display, console_fifo=True)
    aux: dict[str, Any] = {}
    report["auxiliary"] = aux
    backups: list[dict[str, Any]] = []
    try:
        session.start()
        session.battle.watch_ready(root / "roster.ready", session.runner, timeout=1200, label="battle roster")
        cut, before = session.first_strike()
        (root / "cut-observation.json").write_text(json.dumps(cut, indent=2) + "\n")
        fifo = FifoConsole(root)
        fifo.wait(rf"Battle spectator placed: {VIEWER} target={HOLDER}")
        entity = _holder_entity(fifo)
        pre = [client.capture("before-drop") for client in session.clients]
        roles(report, "roles_before_drop", pre[0], pre[1], entity)
        items = sorted(pre[0]["inventory"]["counts"])
        roster = _roster(root / "roster.txt")
        backups.append(_phase_census(fifo, items, "issued", aux, session.clients))
        seq = session.holder.last_event()
        session.holder.driver("drop")
        session.holder.driver("wait_for", condition="event", value="item", since=seq, timeout=120)
        mark = fifo.mark()
        fifo.send("save-all")
        fifo.wait(r"Saved the game", mark)
        session.holder.capture("after-drop")
        backups.append(_phase_census(fifo, items, "dropped", aux, session.clients))
        session.sigkill(report, "crash_handoff_preserved")
        crash_copy = root / "crash-copy" / "world"
        evidence = crash_evidence(ctx, root, crash_copy)
        check(report, "crash_copy_equals_stopped_world", evidence["copy_equals_world"], None)
        binary, toml = inputs["server"], root / "server.toml"
        census_root = root / "census"
        early = [take_census(ctx, binary, toml, census_root / backup["phase"], phase=backup["phase"], epoch=0,
                             backup=backup) for backup in backups]
        crash = take_census(ctx, binary, toml, census_root / "crash", phase="crash", epoch=0, stopped=crash_copy,
                            hashes=evidence["files"])
        kit = durable_issued_kit(report, early[0], items)
        pre_crash_batches = read_batches([root / "crash-copy" / "wal-archive", crash_copy / "journal"])
        pre_crash_history = wal_history(pre_crash_batches)
        original = reconcile(early + [crash], kit, roster, [pre_crash_history])
        verdict_path = root / "crash-verdict.json"
        verdict_path.write_text(json.dumps({"recorded_before_reopen": True, "evidence": "crash-evidence.json",
                                            "evidence_sha256": _sha256(root / "crash-evidence.json"),
                                            **summarize(original)}, indent=1, sort_keys=True) + "\n")
        report["crash_original_verdict"] = {"path": str(verdict_path), "sha256": _sha256(verdict_path),
                                            "verdict": original["verdict"]}
        console = session.recover(archive=True)
        epoch_start = crash["replay_tick"]
        mark = 0
        session.viewer.start(session.address, "after-crash")
        session.holder.start(session.address, "after-crash")
        console.wait(rf"Battle live rearmed: {HOLDER} side=1", mark)
        console.wait(rf"Battle spectator placed: {VIEWER} target={HOLDER}", mark)
        session.holder.wait_kit()
        session.holder.capture("after-crash-rejoin")
        backups.append(_phase_census(console, items, "crash-rejoin", aux, session.clients))
        mark = console.mark()
        console.send(f"kill {HOLDER}")
        session.holder.driver("wait_for", condition="screen", value="dead", timeout=120)
        session.holder.capture("dead")
        session.holder.driver("respawn")
        session.holder.driver("wait_for", condition="screen", value="playing", timeout=120)
        console.wait(rf"Battle live respawned: {HOLDER} side=1", mark)
        session.holder.wait_kit()
        session.holder.capture("after-respawn")
        backups.append(_phase_census(console, items, "death-respawn", aux, session.clients))
        mark = console.mark()
        session.holder.stop()
        console.wait(rf"Battle player offline: {HOLDER}", mark)
        session.holder.start(session.address, "after-reconnect")
        console.wait(rf"Battle live rearmed: {HOLDER} side=1", mark)
        session.holder.wait_kit()
        session.holder.capture("after-reconnect")
        backups.append(_phase_census(console, items, "reconnect", aux, session.clients))
        for client in session.clients:
            client.stop()
        session.stop_recovery(report, "recovery_stopped_cleanly")
        late = [take_census(ctx, binary, toml, census_root / backup["phase"], phase=backup["phase"], epoch=1,
                            backup=backup) for backup in backups[2:]]
        stopped_hashes = file_hashes(root / "world")
        final = take_census(ctx, binary, toml, census_root / "stopped", phase="stopped", epoch=1,
                            stopped=root / "world", hashes=stopped_hashes)
        check(report, "crash_copy_unchanged", file_hashes(crash_copy) == evidence["files"], None)
        late[0]["epoch_start"] = epoch_start
        recovery_batches = read_batches([root / "wal-archive-recovery", root / "world" / "journal"])
        check_recovery_tick(report, root, pre_crash_batches, recovery_batches, crash)
        recovery_history = wal_history(recovery_batches)
        phases = early + [crash] + late + [final]
        finish_census(report, phases, kit, roster, [pre_crash_history, recovery_history], root,
                      control_index=len(early), exact=set())
    finally:
        session.close()


# ----------------------------------------------------------------------------- verdict 2


def crash_handoff(ctx: ProfileContext, inputs: dict[str, Any], report: dict[str, Any], display: str) -> None:
    for cut in ("prestage", "staged"):
        root = ctx.artifact_dir / f"battle-{cut}"
        session = RunnerSession(ctx, inputs, root, display, paused_ticks=1200)
        try:
            session.start()
            if cut == "prestage":
                session.battle.watch_ready(root / "take.ready", session.runner, timeout=1200,
                                           label="recorder ready before pre-stage")
            else:
                session.battle.watch_ready(root / "roster.ready", session.runner, timeout=1200, label="battle roster")
            for client in session.clients:
                client.capture(f"before-{cut}-cut")
            log = (root / "server.log").read_text(errors="replace")
            check(report, f"cut_point_{cut}", ("Battle staged:" not in log) if cut == "prestage"
                  else ("Battle staged:" in log and "Battle started tick=" not in log), None)
            session.sigkill(report, f"crash_handoff_preserved_{cut}")
            console = session.recover()
            session.viewer.start(session.address, f"after-{cut}")
            session.holder.start(session.address, f"after-{cut}")
            joined = [client.capture(f"after-{cut}-rejoined") for client in session.clients]
            check(report, f"clients_rejoin_recovered_server_{cut}",
                  all(data["state"].get("screen") == "playing" for data in joined),
                  [data["state"].get("screen") for data in joined])
            mark = console.mark()
            console.send("battle status")
            status = console.wait(r"Battle paused tick=.*", mark, timeout=120).group(0)
            (root / "recovered-status.txt").write_text(status + "\n")
            if cut == "prestage":
                check(report, "recovered_prestage_world_without_battle",
                      "side1=0 side2=0" in status and "regiments=0" in status, status)
            else:
                check(report, "recovered_staged_battle", f"side1={PER_SIDE} side2={PER_SIDE}" in status, status)
                for command, reply in ((f"battle live {HOLDER} red", f"Battle live placed: {HOLDER}"),
                                       (f"battle spectate {VIEWER} {HOLDER}", f"Battle spectator placed: {VIEWER}"),
                                       ("battle start", "Battle started tick=")):
                    mark = console.mark()
                    console.send(command)
                    console.wait(re.escape(reply), mark, timeout=120, fail=re.compile(r"Command failed:|Battle error:"))
                entity = _holder_entity(console)
                session.holder.wait_kit()
                session.viewer.wait_camera(entity)
                post = [client.capture("after-manual-start") for client in session.clients]
                roles(report, "clients_hold_roles_in_recovered_battle", post[0], post[1], entity)
                strike, rows = session.first_strike(limit=RESUMED_STRIKE_TICK_LIMIT, need_fighting=False)
                (root / "after-observer.jsonl").write_text("".join(json.dumps(row) + "\n" for row in rows))
                check(report, "recovered_battle_strikes", strike["battle"]["strikes"] > 0, strike["battle"])
            for client in session.clients:
                client.stop()
            session.stop_recovery(report, f"recovery_stopped_cleanly_{cut}")
        finally:
            session.close()


# ----------------------------------------------------------------------------- verdict 3


def auth_config(inputs: dict[str, Any], root: Path, port: int, dashboard: int) -> str:
    q = json.dumps
    return (f"[server]\nname = {q('battle-auth-' + uuid.uuid4().hex)}\nview_distance = 12\n"
            f"simulation_distance = 12\npause_when_empty_seconds = 0\n"
            f"[network]\nbind_address = \"127.0.0.1\"\nport = {port}\n"
            f"[auth]\nonline_mode = false\nrequire_plugin_login = true\n"
            f"[dashboard]\nenabled = true\nbind_address = \"127.0.0.1\"\nport = {dashboard}\n"
            f"[plugins]\ndirectory = {q(str(root / 'plugins'))}\n"
            f"[data]\nworld_dir = {q(str(root / 'world'))}\nvanilla_data_dir = {q(str(inputs['cache']))}\n"
            f"seed = {inputs['world_seed']}\ncheckpoint_every_ticks = 1200\n"
            f"backup_dir = {q(str(root / 'backups'))}\n")


def _authenticate(client: Horizon, password: str, register: bool) -> None:
    seq = client.last_event()
    command = f"register {password} {password}" if register else f"login {password}"
    client.driver("command", secret=True, command=command)
    if register:
        client.driver("wait_for", condition="chat", value="Successfully registered", since=seq, timeout=60)
    client.driver("wait_for", condition="chat", value="Successfully logged in", since=seq, timeout=60)


def battle_image(history_batches: dict[tuple[int, int], Path], until: int) -> dict[str, Any] | None:
    """The last durable battle history image at or before `until` (the WAL `Battle` entry)."""
    for (low, high), path in sorted(history_batches.items(), reverse=True):
        if high > until:
            continue
        for line in reversed(path.read_bytes()[32:].splitlines()):
            if b'"Battle"' not in line:
                continue
            for _, entry in _visit(json.loads(line)):
                if "Battle" in entry:
                    return {"tick": high, "image": json.loads(entry["Battle"]["value"])}
    return None


def _kit_counts(counts: dict[str, int], items: list[str]) -> dict[str, int]:
    return {item: counts.get(item, 0) for item in items}


def auth_hydration(ctx: ProfileContext, inputs: dict[str, Any], report: dict[str, Any], display: str) -> None:
    battle = _runner_module()
    root = ctx.artifact_dir / "battle"
    root.mkdir()
    shutil.copytree(inputs["world"], root / "world", symlinks=False)
    package = root / "plugins" / "krasnokamen-auth"
    package.mkdir(parents=True)
    for name in ("plugin.toml", "plugin.wasm"):
        shutil.copy2(inputs["auth_package"] / name, package / name)
    (root / "backups").mkdir()
    port, dashboard = runtime.reserve_port(), runtime.reserve_port()
    address = f"127.0.0.1:{port}"
    config = root / "server.toml"
    config.write_text(auth_config(inputs, root, port, dashboard))
    passwords = {HOLDER: secrets.token_urlsafe(12), VIEWER: secrets.token_urlsafe(12)}
    aux: dict[str, Any] = {}
    report["auxiliary"] = aux
    clients: list[Horizon] = []
    server = console = archive = None
    server_index = -1
    backups: list[dict[str, Any]] = []
    try:
        archive = battle.WalArchive(root / "world" / "journal", root / "wal-archive")
        argv = [str(inputs["server"]), "--config", str(config)]
        server, server_index = _launch(ctx, argv, root / "server.log", cwd=root, stdin=subprocess.PIPE,
                                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
        console = Console(server, root / "server.log")
        console.wait(r"Krasnokamen is listening \(edge v2\)", timeout=600)
        mark = console.mark()
        console.send(f"battle watch {HOLDER} {VIEWER}")
        console.wait(r"Battle watching:", mark)
        for name in (HOLDER, VIEWER):
            client = Horizon(ctx, inputs, root, name, display)
            clients.append(client)
            client.start(address, "before")
            _authenticate(client, passwords[name], register=True)
        holder, viewer = clients
        console.wait(rf"Battle player online: {HOLDER} entity=")
        console.wait(rf"Battle player online: {VIEWER} entity=")
        for command, reply in ((f"battle setup {BATTLE_SEED} {BATTLE_SCENARIO} {PER_SIDE}", "Battle staged:"),
                               ("battle roster", "Battle roster:")):
            mark = console.mark()
            console.send(command)
            console.wait(re.escape(reply), mark, timeout=600, fail=re.compile(r"Command failed:|Battle error:"))
        roster_line = console.wait(r"Battle roster: (\S+)").group(1)
        roster = {str(uuid.UUID(name)) for name in roster_line.split(",")}
        after, first = None, None
        while True:
            rows = [json.loads(line) for line in battle.telemetry_get(dashboard, after).splitlines() if line]
            after = rows[-1]["tick"]
            first = rows[0]["tick"] if first is None else first
            mark = console.mark()
            console.send("battle status")
            status = battle.parse_status(console.wait(r"Battle paused tick=.*", mark, timeout=120).group(0))
            if status and status["resident"] == 2 * PER_SIDE and status["active"] == 2 * PER_SIDE:
                break
            if after - first > STRIKE_TICK_LIMIT:
                raise RuntimeError(f"staged battle not fully resident and active within {STRIKE_TICK_LIMIT} ticks: {status}")
        for command, reply in ((f"battle live {HOLDER} red", f"Battle live placed: {HOLDER}"),
                               (f"battle spectate {VIEWER} {HOLDER}", f"Battle spectator placed: {VIEWER}"),
                               ("battle start", "Battle started tick=")):
            mark = console.mark()
            console.send(command)
            console.wait(re.escape(reply), mark, timeout=120, fail=re.compile(r"Command failed:|Battle error:"))
        after, seen = None, []
        while True:
            rows = [json.loads(line) for line in battle.telemetry_get(dashboard, after, 1).splitlines() if line]
            seen.extend(rows)
            after = rows[-1]["tick"]
            if any(row.get("battle", {}).get("strikes", 0) > 0 and row["battle"].get("fighting", 0) > 0 for row in rows):
                break
            if after - seen[0]["tick"] > STRIKE_TICK_LIMIT:
                raise RuntimeError(f"battle produced no ordinary native strike within {STRIKE_TICK_LIMIT} ticks")
        entity = _holder_entity(console)
        pre = [client.capture("before-cut") for client in clients]
        roles(report, "roles_before_cut", pre[0], pre[1], entity)
        items = sorted(pre[0]["inventory"]["counts"])
        backups.append(_phase_census(console, items, "issued", aux, clients))
        os.kill(server.pid, signal.SIGKILL)
        server.wait(timeout=120)
        _record_exit(ctx, server_index, server)
        console.close()
        archive.close()
        archive_errors = archive.errors
        archive = None
        battle.preserve_crash(root, server)
        check(report, "crash_image_preserved", server.returncode == -signal.SIGKILL and not archive_errors
              and (root / "crash-copy" / "world").is_dir(), {"exit": server.returncode, "wal_archive_errors": archive_errors})
        for client in clients:
            client.stop()
        crash_copy = root / "crash-copy" / "world"
        evidence = crash_evidence(ctx, root, crash_copy)
        check(report, "crash_copy_equals_stopped_world", evidence["copy_equals_world"], None)
        census_root = root / "census"
        early = [take_census(ctx, inputs["server"], config, census_root / "issued", phase="issued", epoch=0,
                             backup=backups[0])]
        crash = take_census(ctx, inputs["server"], config, census_root / "crash", phase="crash", epoch=0,
                            stopped=crash_copy, hashes=evidence["files"])
        kit = durable_issued_kit(report, early[0], items)
        pre_crash_batches = read_batches([root / "crash-copy" / "wal-archive", crash_copy / "journal"])
        pre_crash_history = wal_history(pre_crash_batches)
        original = reconcile(early + [crash], kit, roster, [pre_crash_history])
        verdict_path = root / "crash-verdict.json"
        verdict_path.write_text(json.dumps({"recorded_before_reopen": True, "evidence_sha256": _sha256(root / "crash-evidence.json"),
                                            **summarize(original)}, indent=1, sort_keys=True) + "\n")
        report["crash_original_verdict"] = {"path": str(verdict_path), "sha256": _sha256(verdict_path),
                                            "verdict": original["verdict"]}
        archive = battle.WalArchive(root / "world" / "journal", root / "wal-archive-recovery")
        server, server_index = _launch(ctx, argv, root / "recovery.log", cwd=root, stdin=subprocess.PIPE,
                                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
        console = Console(server, root / "recovery.log")
        console.wait(r"Krasnokamen is listening \(edge v2\)", timeout=600)
        # Both rejoin together: the auth package kicks an unauthenticated session after 1,200 ticks.
        for client in (viewer, holder):
            client.launch("after-crash")
        for client in (viewer, holder):
            client.connect(address)
        for client in (viewer, holder):
            client.ready()
        locked = [client.capture("locked") for client in clients]
        lines = console.text_since(0)
        holder_kit = _kit_counts(locked[0]["inventory"]["counts"], items)
        check(report, "locked_holder_has_no_kit",
              all(count == 0 for count in holder_kit.values())
              and f"Battle live rearmed: {HOLDER}" not in lines and f"Battle player online: {HOLDER}" not in lines,
              {"client_kit_counts": holder_kit, "rearmed_logged": f"Battle live rearmed: {HOLDER}" in lines})
        camera = locked[1]["state"].get("camera")
        check(report, "locked_spectator_not_yet_bound",
              f"Battle spectator placed: {VIEWER}" not in lines
              and not (locked[1]["state"].get("gamemode") == "spectator" and camera is not None),
              {"viewer_state": {k: locked[1]["state"].get(k) for k in ("screen", "gamemode", "camera")}})
        backups.append(_phase_census(console, items, "locked", aux, clients))
        locked_tick = backups[-1]["wal_tick"]
        mark = console.mark()
        _authenticate(holder, passwords[HOLDER], register=False)
        _authenticate(viewer, passwords[VIEWER], register=False)
        console.wait(rf"Battle live rearmed: {HOLDER} side=1", mark)
        console.wait(rf"Battle spectator placed: {VIEWER} target={HOLDER}", mark)
        entity = _holder_entity(console, mark)
        holder.wait_kit()
        viewer.wait_camera(entity)
        unlocked = [client.capture("after-login") for client in clients]
        roles(report, "roles_restored_after_login", unlocked[0], unlocked[1], entity)
        check(report, "holder_rearmed_once_after_login", console.count(rf"Battle live rearmed: {HOLDER} ") == 1,
              console.count(rf"Battle live rearmed: {HOLDER} "))
        backups.append(_phase_census(console, items, "login", aux, clients))
        mark = console.mark()
        holder.stop()
        console.wait(rf"Battle player offline: {HOLDER}", mark)
        holder.start(address, "reentry")
        _authenticate(holder, passwords[HOLDER], register=False)
        console.wait(rf"Battle player online: {HOLDER} entity=", mark)
        holder.wait_kit()
        reentered = holder.capture("after-reentry")
        check(report, "reentry_issues_no_second_kit",
              _kit_counts(reentered["inventory"]["counts"], items) == {item: 1 for item in items},
              reentered["inventory"]["counts"])
        backups.append(_phase_census(console, items, "reentry", aux, clients))
        for client in clients:
            client.stop()
        console.send("stop")
        code = server.wait(timeout=300)
        _record_exit(ctx, server_index, server)
        console.close()
        archive.close()
        recovery_errors = archive.errors
        archive = None
        check(report, "recovery_stopped_cleanly", code == 0 and not recovery_errors,
              {"exit": code, "wal_archive_errors": recovery_errors})
        recovery_batches = read_batches([root / "wal-archive-recovery", root / "world" / "journal"])
        check_recovery_tick(report, root, pre_crash_batches, recovery_batches, crash)
        image = battle_image(recovery_batches, locked_tick)
        live = (image or {}).get("image", {}).get("battle", {}).get("live", {})
        check(report, "locked_saved_camera_and_hold_durable",
              [VIEWER, HOLDER] in live.get("spectators", [])
              and any(hold.get("name") == HOLDER for hold in live.get("holds", [])),
              {"battle_image_tick": (image or {}).get("tick"), "spectators": live.get("spectators"),
               "holds": [hold.get("name") for hold in live.get("holds", [])]})
        late = [take_census(ctx, inputs["server"], config, census_root / backup["phase"], phase=backup["phase"],
                            epoch=1, backup=backup) for backup in backups[1:]]
        final = take_census(ctx, inputs["server"], config, census_root / "stopped", phase="stopped", epoch=1,
                            stopped=root / "world", hashes=file_hashes(root / "world"))
        check(report, "crash_copy_unchanged", file_hashes(crash_copy) == evidence["files"], None)
        late[0]["epoch_start"] = crash["replay_tick"]
        recovery_history = wal_history(recovery_batches)
        finish_census(report, early + [crash] + late + [final], kit, roster, [pre_crash_history, recovery_history],
                      root, control_index=1, exact={"crash", "locked", "login", "reentry", "stopped"})
        leaks = password_leaks(ctx.artifact_dir, list(passwords.values()))
        check(report, "password_never_logged", not leaks, leaks)
    finally:
        for client in clients:
            client.stop()
        if archive is not None:
            archive.close()
        if server is not None and server.poll() is None:
            battle.stop_child(server, 30)
        _record_exit(ctx, server_index, server)


def password_leaks(root: Path, passwords: list[str]) -> list[str]:
    """Files under root that contain any generated password (paths only; the secret is never reported)."""
    needles = [password.encode() for password in passwords]
    leaks = []
    for path in sorted(root.rglob("*")):
        if path.is_file() and not path.is_symlink():
            data = path.read_bytes()
            if any(needle in data for needle in needles):
                leaks.append(str(path))
    return leaks


# ----------------------------------------------------------------------------- entry


VERDICTS: dict[str, Callable[..., None]] = {
    "battle-kit-census": kit_census,
    "battle-crash-handoff": crash_handoff,
    "battle-auth-hydration": auth_hydration,
}


def run(ctx: ProfileContext) -> dict[str, Any]:
    scenario = ctx.scenario_id
    if scenario not in VERDICTS or ctx.extra_args:
        raise ValueError(f"battle-crash-recovery runs only its named scenarios ({', '.join(SCENARIOS)}) without extra arguments")
    inputs = resolve_inputs(ctx, scenario)
    report: dict[str, Any] = {"scenario": scenario, "assertions": {}}
    ctx.failure_scenario = report
    display_process, display, display_log = runtime.start_xvfb(ctx.artifact_dir)
    try:
        VERDICTS[scenario](ctx, inputs, report, display)
    finally:
        runtime.stop_process(display_process)
        display_log.close()
    report["passed"] = all(report["assertions"].values())
    return {"scenario": report}
