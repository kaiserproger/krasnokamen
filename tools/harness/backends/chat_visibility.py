"""Native graphical chat-visibility gate: what the server sends a client that asked not to hear.

The native client has no chat setting (it always sends "full"), so a TCP proxy
between the real windowed client and the release server rewrites the chat-mode
byte of the client's own ClientInformation packets, in Configuration and in
Play, to the mode under test. The server therefore sees a client that asked for
"commands only" or "hidden"; the client then shows only what the server
actually sent. A vanilla Java client cannot prove this: it also filters chat
locally.

Per connection (operator fixture commands, ordered by a give/inventory fence):

* full: `/say` (chat) and `/tellraw` (system text) both reach the client;
* commands only: only the system text arrives;
* hidden: no text line at all arrives (not even the join tip, `/help` or an unknown command's
  error, which the edge writes itself), also after the player left in the Nether and joins
  into it again (the setting must follow the join into another dimension).
"""
from __future__ import annotations

import hashlib
import json
import os
import socket
import subprocess
import threading
from pathlib import Path
from typing import Any, Callable

from .. import runtime
from ..profiles import ProfileContext
from . import native_client
from .chisel import rewrite_config
from .gameplay import prepare_operator_fixture, require_tools
from .probe_steps import ProbeSteps, item_count

USERNAME = "ChatGate"
SEED = 83
FULL, COMMANDS_ONLY, HIDDEN = 0, 1, 2
# One entry per connection the client makes, in order.
MODES = (FULL, COMMANDS_ONLY, HIDDEN, HIDDEN)
CONFIGURATION_CLIENT_INFORMATION = 0x00
PLAY_CLIENT_INFORMATION = 0x0E
TOOLS = frozenset({"minecraft_observe", "minecraft_wait_for_play", "minecraft_wait_for_state_change",
                   "minecraft_send_chat", "minecraft_wait_for_inventory", "minecraft_connect",
                   "minecraft_disconnect"})


def read_varint(data: bytes | bytearray, pos: int) -> tuple[int, int] | None:
    """(value, next position), or None while the varint is incomplete."""
    value = 0
    for shift in range(0, 35, 7):
        if pos >= len(data):
            return None
        byte = data[pos]
        pos += 1
        value |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return value, pos
    raise ValueError("varint longer than 5 bytes")


def write_varint(value: int) -> bytes:
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        out.append(byte | (0x80 if value else 0))
        if not value:
            return bytes(out)


class ClientInformationRewriter:
    """Rewrite the chat mode of the client's ClientInformation frames; every other byte passes untouched.

    `mode` is the chat mode, or a callable that picks it (given this rewriter) when the handshake asks to log in: the native
    client also opens status (server list ping) connections, which carry no ClientInformation and must
    not use up one of the planned modes.
    """

    def __init__(self, mode: int | Callable[[ClientInformationRewriter], int]) -> None:
        self.pick = mode if callable(mode) else lambda _: mode
        self.mode = -1  # until the handshake logs in
        self.threshold: int | None = None
        self.phase = "handshake"
        self.buffer = bytearray()
        self.rewrites: list[dict[str, Any]] = []

    @staticmethod
    def check(mode: int) -> None:
        if not 0 <= mode < 0x80:
            raise ValueError("chat mode must fit one varint byte")

    def handshake(self, payload: bytearray, body: int) -> None:
        """Status connections are forwarded untouched; a login or transfer one claims its chat mode."""
        protocol = read_varint(payload, body)
        address = read_varint(payload, protocol[1]) if protocol else None
        intent = read_varint(payload, address[1] + address[0] + 2) if address else None
        if intent is None:
            raise ValueError("malformed handshake")
        if intent[0] == 1:
            self.phase = "status"
            return
        self.mode = self.pick(self)
        self.check(self.mode)
        self.phase = "login"

    def feed(self, data: bytes) -> bytes:
        """Bytes to forward now: every complete frame, rewritten where it carries the setting."""
        self.buffer += data
        out = bytearray()
        while True:
            length = read_varint(self.buffer, 0)
            if length is None or len(self.buffer) < length[1] + length[0]:
                break
            start, end = length[1], length[1] + length[0]
            payload = bytearray(self.buffer[start:end])
            del self.buffer[:end]
            out += write_varint(length[0]) + self.frame(payload)
        return bytes(out)

    def frame(self, payload: bytearray) -> bytes:
        offset = 0
        if self.threshold is not None:
            data_length = read_varint(payload, 0)
            if data_length is None or data_length[0] != 0:
                return bytes(payload)  # compressed frames are never a ClientInformation (those are tiny)
            offset = data_length[1]
        parsed = read_varint(payload, offset)
        if parsed is None:
            return bytes(payload)
        packet_id, body = parsed
        if self.phase == "handshake":
            self.handshake(payload, body)
        elif self.phase == "login":
            if packet_id == 3:  # login acknowledged
                self.phase = "configuration"
        elif self.phase == "configuration":
            if packet_id == CONFIGURATION_CLIENT_INFORMATION:
                self.rewrite(payload, body, "configuration")
            elif packet_id == 3:  # finish configuration acknowledged
                self.phase = "play"
        elif self.phase == "play" and packet_id == PLAY_CLIENT_INFORMATION:
            self.rewrite(payload, body, "play")
        return bytes(payload)

    def rewrite(self, payload: bytearray, body: int, phase: str) -> None:
        locale = read_varint(payload, body)
        if locale is None:
            return
        mode_at = locale[1] + locale[0] + 1  # locale bytes, then the view distance byte
        mode = read_varint(payload, mode_at)
        if mode is None or mode[1] != mode_at + 1:
            raise ValueError("chat mode is not a one-byte varint")
        self.rewrites.append({"phase": phase, "was": mode[0], "now": self.mode})
        payload[mode_at] = self.mode


class RewritingProxy:
    """Accepts the client's connections; the Nth one that logs in asks for MODES[N], status pings pass through."""

    def __init__(self, target_port: int, modes: tuple[int, ...]) -> None:
        self.target_port = target_port
        self.modes = modes
        self.logins: list[ClientInformationRewriter] = []
        self.claim_lock = threading.Lock()
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen()
        self.port = self.listener.getsockname()[1]
        self.error: BaseException | None = None
        self.thread = threading.Thread(target=self.accept_loop, daemon=True)
        self.sockets: list[socket.socket] = []

    def start(self) -> int:
        self.thread.start()
        return self.port

    def claim(self, rewriter: ClientInformationRewriter) -> int:
        with self.claim_lock:
            index = len(self.logins)
            if index >= len(self.modes):
                raise ValueError("the client logged in more often than the gate planned")
            self.logins.append(rewriter)
            return self.modes[index]

    def accept_loop(self) -> None:
        try:
            while True:
                client, _ = self.listener.accept()
                rewriter = ClientInformationRewriter(self.claim)
                upstream = socket.create_connection(("127.0.0.1", self.target_port))
                self.sockets += [client, upstream]
                threading.Thread(target=self.client_to_server, args=(client, upstream, rewriter), daemon=True).start()
                threading.Thread(target=self.server_to_client, args=(upstream, client, rewriter), daemon=True).start()
        except OSError:
            return  # the listener was closed
        except BaseException as error:  # reported by the scenario
            self.error = error

    def client_to_server(self, client: socket.socket, upstream: socket.socket, rewriter: ClientInformationRewriter) -> None:
        try:
            while data := client.recv(65536):
                upstream.sendall(rewriter.feed(data))
        except OSError:
            pass
        except ValueError as error:
            self.error = error
        finally:
            self.shutdown(client, upstream)

    def server_to_client(self, upstream: socket.socket, client: socket.socket, rewriter: ClientInformationRewriter) -> None:
        """Forward the server's bytes; read its Set Compression so the client's frames can be framed."""
        buffer = bytearray()
        watching = True
        try:
            while data := upstream.recv(65536):
                if watching:
                    buffer += data
                    while watching:
                        length = read_varint(buffer, 0)
                        if length is None or len(buffer) < length[1] + length[0]:
                            break
                        packet = read_varint(buffer, length[1])
                        if packet is not None and packet[0] == 3:  # login: Set Compression
                            threshold = read_varint(buffer, packet[1])
                            rewriter.threshold = threshold[0] if threshold and threshold[0] >= 0 else None
                            watching = False
                        elif packet is not None and packet[0] == 2:  # login finished without compression
                            watching = False
                        del buffer[:length[1] + length[0]]
                client.sendall(data)
        except OSError:
            pass
        finally:
            self.shutdown(client, upstream)

    @staticmethod
    def shutdown(*sockets: socket.socket) -> None:
        for sock in sockets:
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass

    def close(self) -> None:
        self.listener.close()
        for sock in self.sockets:
            try:
                sock.close()
            except OSError:
                pass


def validate_args(ctx: ProfileContext) -> float:
    if ctx.client != "native" or not ctx.windowed or ctx.extra_args or ctx.platform is not None:
        raise ValueError("chat-visibility requires --client native --windowed")
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else 900.0
    if not 0 < timeout <= 3600:
        raise ValueError("chat-visibility timeout must be in (0, 3600]")
    return timeout


def run(ctx: ProfileContext) -> dict[str, Any]:
    timeout = validate_args(ctx)
    override = os.environ.get("SOLARIS_SERVER_BINARY")
    if not override or not os.environ.get("SOLARIS_CLIENT_BINARY"):
        raise RuntimeError("chat-visibility requires prebuilt SOLARIS_SERVER_BINARY and SOLARIS_CLIENT_BINARY")
    binary = Path(override).expanduser().resolve()
    if not binary.is_file():
        raise RuntimeError(f"missing server binary: {binary}")
    artifact = ctx.artifact_dir
    port = runtime.reserve_port()
    plugins = artifact / "plugins"
    plugins.mkdir()
    config = artifact / "chat-visibility.toml"
    # The server's own content cache, as client_probes uses it; the client's
    # SOLARIS_CLIENT_DATA_DIR is not a server content cache.
    vanilla = os.environ.get("SOLARIS_SERVER_VANILLA_DATA_DIR", "data/vanilla")
    source = (runtime.REPO_ROOT / "playable.toml").read_text()
    token = 'vanilla_data_dir = "data/vanilla"'
    if source.count(token) != 1:
        raise RuntimeError("expected one vanilla_data_dir config token")
    source = source.replace(token, f"vanilla_data_dir = {json.dumps(vanilla)}")
    world = artifact / "world"
    config.write_text(rewrite_config(source, port=port,
        world=world.relative_to(runtime.REPO_ROOT).as_posix() if world.is_relative_to(runtime.REPO_ROOT) else str(world),
        plugins=str(plugins), seed=SEED))
    prepare_operator_fixture(ctx, [USERNAME])
    with binary.open("rb") as handle:
        digest = hashlib.file_digest(handle, "sha256").hexdigest()
    ctx.client_evidence.update(server_binary_path=str(binary), server_binary_sha256=digest,
                               world_dir=str(world), seed=SEED)
    server = client = xvfb = mcp = log = proxy = None
    try:
        log_path = artifact / "server.log"
        log = log_path.open("wb")
        ctx.logs.append(str(log_path))
        server = runtime.start_server([str(binary), "--config", str(config), "--no-console"],
            cwd=runtime.REPO_ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        runtime.wait_port(port, timeout, server, log_path=log_path)
        proxy = RewritingProxy(port, MODES)
        address = f"127.0.0.1:{proxy.start()}"
        ctx.client_evidence["server_address"] = address
        xvfb, display, _ = runtime.start_xvfb(artifact)
        ctx.logs.append(str(artifact / "xvfb.log"))
        client, _, mcp = native_client.start(ctx, native_client.resolve_binary(ctx), address,
                                              USERNAME, timeout, display=display)
        mcp.p4_window = ctx.client_evidence.get("window_focus")
        mcp.request_timeout_seconds = min(timeout + 10, 300.0)
        with ProbeSteps(mcp, artifact / "chat-visibility", timeout, "native-chat-visibility") as probe:
            require_tools(mcp.list_tools(), TOOLS, "chat-visibility")
            try:
                scenario(probe, proxy, address)
            finally:
                probe.report["client_information_rewrites"] = [r.rewrites for r in proxy.logins]
        return {"scenario": probe.report}
    finally:
        if mcp is not None:
            try:
                mcp.close()
            except Exception:
                pass
        runtime.stop_process(client)
        if proxy is not None:
            proxy.close()
        runtime.stop_process(server, interrupt=True)
        runtime.stop_process(xvfb)
        if log is not None:
            log.close()


def heard(state: dict[str, Any], marker: str) -> bool:
    return any(marker in str(line) for line in state.get("recent_chat", []))


def scenario(probe: ProbeSteps, proxy: RewritingProxy, address: str) -> None:
    dirt = 0
    probe.report["connections"] = []

    def join(index: int) -> None:
        if index:
            probe.call("minecraft_connect", {"server_addr": address})
        probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
        probe.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float), f"c{index}_player_present")
        # The server must have been asked for this connection's mode before the player speaks.
        probe.check(f"c{index}_client_information_rewritten", proxy.error is None
                    and len(proxy.logins) == index + 1
                    and any(r["phase"] == "configuration" and r["now"] == MODES[index]
                            for r in proxy.logins[index].rewrites))

    def speak(index: int, label: str) -> dict[str, bool]:
        """Chat and system text to the operator, then an inventory change that orders them."""
        nonlocal dirt
        chat, system = f"ChatMark_{label}", f"SystemMark_{label}"
        if MODES[index] == HIDDEN:
            # Text the edge writes itself, not the core: a command list and an unknown command.
            probe.fixture("help")
            probe.fixture("nosuchcommand")
        probe.fixture(f"say {chat}")
        probe.fixture(f'tellraw @s {{"text":"{system}"}}')
        probe.fixture("give @s minecraft:dirt 1")
        dirt += 1
        probe.call("minecraft_wait_for_inventory", {"item_id": "minecraft:dirt", "count": dirt,
                                                    "timeout_seconds": probe.remaining()})
        state = probe.observe()
        result = {"chat": heard(state, chat), "system": heard(state, system),
                  "lines": len(state.get("recent_chat", []))}
        probe.report["connections"].append({"index": index, "label": label, "mode": MODES[index],
                                            "heard": result, "recent_chat": state.get("recent_chat", [])})
        return result

    def in_nether(state: dict[str, Any]) -> bool:
        return "the_nether" in json.dumps(state.get("dimension", state.get("player", {})))

    # Connection 0 is the harness's first join; each later one reconnects through the proxy.
    join(0)
    full = speak(0, "full")
    probe.check("full_hears_chat", full["chat"])
    probe.check("full_hears_system_text", full["system"])

    # `/list` is answered by the connection layer, not the core: it must still obey send_command_feedback.
    def listed() -> bool:
        return any("players online" in str(line) for line in probe.observe().get("recent_chat", []))

    def fence() -> None:
        nonlocal dirt
        probe.fixture("give @s minecraft:dirt 1")
        dirt += 1
        probe.call("minecraft_wait_for_inventory", {"item_id": "minecraft:dirt", "count": dirt,
                                                    "timeout_seconds": probe.remaining()})

    # The connection layer answers /list at once while the core runs `gamerule` on its next
    # tick, so each rule change is fenced before the next command.
    probe.fixture("gamerule send_command_feedback false")
    fence()
    probe.fixture("list")
    fence()
    probe.check("list_silent_with_command_feedback_off", not listed())
    probe.fixture("gamerule send_command_feedback true")
    fence()
    probe.fixture("list")
    fence()
    probe.check("list_answers_with_command_feedback_on", listed())

    # A team message is chat in every copy (vanilla TeamMsgCommand): the Full sender is shown its own
    # copy here, a Commands-only sender is not (below).
    probe.fixture("team add chatcrew")
    probe.fixture("team join chatcrew @s")
    probe.fixture("teammsg TeamMark_full")
    fence()
    probe.check("full_sender_sees_its_team_message", heard(probe.observe(), "TeamMark_full"))

    probe.call("minecraft_disconnect", {})
    join(1)
    commands_only = speak(1, "commands_only")
    probe.check("commands_only_misses_chat", not commands_only["chat"])
    probe.check("commands_only_hears_system_text", commands_only["system"])
    probe.fixture("teammsg TeamMark_commands_only")
    fence()
    probe.check("commands_only_sender_gets_no_team_message_copy",
                not heard(probe.observe(), "TeamMark_commands_only"))

    probe.call("minecraft_disconnect", {})
    join(2)
    hidden = speak(2, "hidden")
    probe.check("hidden_misses_chat", not hidden["chat"])
    probe.check("hidden_misses_system_text", not hidden["system"])
    probe.check("hidden_receives_no_text_line_at_all", hidden["lines"] == 0)

    # Leave in the Nether: the next join is routed into that dimension, and the setting must follow it.
    probe.fixture("execute in minecraft:the_nether run tp @s 0 80 0")
    probe.wait(in_nether, "left_in_the_nether", wait_for_play=True)  # the client rebuilds its world for the dimension change
    probe.call("minecraft_disconnect", {})
    join(3)
    probe.wait(in_nether, "joined_into_the_nether")
    nether = speak(3, "hidden_nether")
    probe.check("hidden_in_nether_misses_chat", not nether["chat"])
    probe.check("hidden_in_nether_misses_system_text", not nether["system"])
    probe.check("hidden_in_nether_receives_no_text_line_at_all", nether["lines"] == 0)
    probe.check("one_dirt_per_connection", item_count(probe.observe(), "minecraft:dirt") == dirt)
