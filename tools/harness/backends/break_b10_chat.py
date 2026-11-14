"""Breaker attack run for AUDIT-CHAT-VISIBILITY (QA glue, not a gate).

Real windowed native client behind the chat-visibility proxy plus a second
player (offline protocol bot, chat setting full) on the same server. The proxy
can also inject a Play ClientInformation mid-session to switch the mode in
place, the way a vanilla client does when its chat option changes.
"""
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import threading
import traceback
from pathlib import Path
from typing import Any, Callable

from .. import runtime, wire
from ..matrix_execution import offline_uuid
from ..profiles import ProfileContext
from . import native_client
from .chat_visibility import (COMMANDS_ONLY, FULL, HIDDEN, PLAY_CLIENT_INFORMATION, ClientInformationRewriter,
                              RewritingProxy, read_varint, validate_args, write_varint)
from .chisel import rewrite_config
from .gameplay import prepare_operator_fixture
from .probe_steps import ProbeSteps, item_count

USERNAME = "ChatGate"
BOT = "BreakBot"
SEED = 83
NAMES = {FULL: "full", COMMANDS_ONLY: "commands_only", HIDDEN: "hidden"}
# c0 full (switched in place several times), c1 hidden, c2 hidden after a server restart.
MODES = (FULL, HIDDEN, HIDDEN, HIDDEN, HIDDEN)


class InjectingRewriter(ClientInformationRewriter):
    """Remembers the configuration ClientInformation body so a Play one can be injected later."""

    def __init__(self, mode: Any) -> None:
        super().__init__(mode)
        self.template: bytes | None = None
        self.upstream = None
        self.lock = threading.Lock()

    def rewrite(self, payload: bytearray, body: int, phase: str) -> None:
        super().rewrite(payload, body, phase)
        if phase == "configuration":
            self.template = bytes(payload[body:])

    def inject(self, mode: int) -> None:
        if self.template is None or self.upstream is None:
            raise RuntimeError("no ClientInformation template to inject")
        locale = read_varint(self.template, 0)
        mode_at = locale[1] + locale[0] + 1
        body = bytearray(self.template)
        body[mode_at] = mode
        self.mode = mode  # later client-sent ClientInformation keeps the injected mode
        packet = write_varint(PLAY_CLIENT_INFORMATION) + bytes(body)
        if self.threshold is not None:
            packet = write_varint(0) + packet
        with self.lock:
            self.upstream.sendall(write_varint(len(packet)) + packet)
        self.rewrites.append({"phase": "play-injected", "was": -1, "now": mode})


class InjectingProxy(RewritingProxy):
    def accept_loop(self) -> None:
        import socket
        try:
            while True:
                client, _ = self.listener.accept()
                rewriter = InjectingRewriter(self.claim)
                upstream = socket.create_connection(("127.0.0.1", self.target_port))
                rewriter.upstream = upstream
                self.sockets += [client, upstream]
                threading.Thread(target=self.client_to_server, args=(client, upstream, rewriter), daemon=True).start()
                threading.Thread(target=self.server_to_client, args=(upstream, client, rewriter), daemon=True).start()
        except OSError:
            return
        except BaseException as error:
            self.error = error

    def client_to_server(self, client, upstream, rewriter) -> None:
        try:
            while data := client.recv(65536):
                out = rewriter.feed(data)
                with rewriter.lock:
                    upstream.sendall(out)
        except OSError:
            pass
        except ValueError as error:
            self.error = error
        finally:
            self.shutdown(client, upstream)


def run(ctx: ProfileContext) -> dict[str, Any]:
    timeout = validate_args(ctx)
    binary = Path(os.environ["SOLARIS_SERVER_BINARY"]).expanduser().resolve()
    artifact = ctx.artifact_dir
    port = runtime.reserve_port()
    plugins = artifact / "plugins"
    plugins.mkdir()
    config = artifact / "chat-visibility.toml"
    vanilla = os.environ.get("SOLARIS_SERVER_VANILLA_DATA_DIR", "data/vanilla")
    source = (runtime.REPO_ROOT / "playable.toml").read_text()
    source = source.replace('vanilla_data_dir = "data/vanilla"', f"vanilla_data_dir = {json.dumps(vanilla)}")
    world = artifact / "world"
    config.write_text(rewrite_config(source, port=port,
        world=world.relative_to(runtime.REPO_ROOT).as_posix() if world.is_relative_to(runtime.REPO_ROOT) else str(world),
        plugins=str(plugins), seed=SEED))
    prepare_operator_fixture(ctx, [USERNAME])
    with binary.open("rb") as handle:
        digest = hashlib.file_digest(handle, "sha256").hexdigest()
    ctx.client_evidence.update(server_binary_path=str(binary), server_binary_sha256=digest, world_dir=str(world))
    procs: dict[str, Any] = {"server": None}
    logs: list[Any] = []
    client = xvfb = mcp = proxy = None

    def start_server(n: int) -> None:
        log_path = artifact / f"server-{n}.log"
        log = log_path.open("wb")
        logs.append(log)
        ctx.logs.append(str(log_path))
        procs["server"] = runtime.start_server([str(binary), "--config", str(config), "--no-console"],
            cwd=runtime.REPO_ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        runtime.wait_port(port, timeout, procs["server"], log_path=log_path)

    try:
        start_server(0)
        proxy = InjectingProxy(port, MODES)
        address = f"127.0.0.1:{proxy.start()}"
        ctx.client_evidence["server_address"] = address
        xvfb, display, _ = runtime.start_xvfb(artifact)
        client, _, mcp = native_client.start(ctx, native_client.resolve_binary(ctx), address,
                                              USERNAME, timeout, display=display)
        mcp.p4_window = ctx.client_evidence.get("window_focus")
        mcp.request_timeout_seconds = min(timeout + 10, 300.0)
        with ProbeSteps(mcp, artifact / "break-chat", timeout, "break-b10-chat") as probe:
            try:
                attacks(probe, proxy, address, port, artifact,
                        restart=lambda n: (runtime.stop_process(procs["server"], interrupt=True), start_server(n)))
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
        runtime.stop_process(procs["server"], interrupt=True)
        runtime.stop_process(xvfb)
        for log in logs:
            log.close()


def lines(state: dict[str, Any]) -> list[str]:
    return [str(line) for line in state.get("recent_chat", [])]


def attacks(probe: ProbeSteps, proxy: InjectingProxy, address: str, port: int, artifact: Path,
            restart: Callable[[int], Any]) -> None:
    report = probe.report
    report["attacks"] = {}
    report["errors"] = {}
    counters = {"dirt": 0, "fence": 0}
    bot: dict[str, Any] = {"o": None, "gen": 0}

    def soft(name: str, ok: bool, detail: Any = None) -> bool:
        report["attacks"][name] = {"ok": bool(ok), "detail": detail}
        return ok

    only = os.environ.get("BREAK_B10_ONLY", "").split(",") if os.environ.get("BREAK_B10_ONLY") else None

    def step(name: str, body: Callable[[], None]) -> None:
        if only is not None and name not in only and not name.startswith("c3_"):
            return
        try:
            body()
        except Exception as error:  # keep attacking; the failure is a finding or a harness gap
            report["errors"][name] = "".join(traceback.format_exception_only(type(error), error)).strip()
            report["errors"][name + "_trace"] = traceback.format_exc()[-1500:]

    def connect_bot() -> None:
        bot["gen"] += 1
        protocol = json.loads((runtime.REPO_ROOT / "data/vanilla/version.json").read_text())["protocol_version"]
        bot["o"] = wire.PlayObserver(("127.0.0.1", port), BOT, account_uuid=offline_uuid(BOT), timeout=30,
                                     trace_path=artifact / f"bot-{bot['gen']}.wire.jsonl", protocol=protocol)
        bot["o"].ready()

    def bot_events() -> list[str]:
        return list(bot["o"].events)

    def bot_chat(text: str) -> None:
        import struct
        import time
        bot["o"].send(0x09, wire.mc_string(text) + struct.pack(">qq", int(time.time() * 1000), 0)
                      + b"\x00\x00\x00\x00\x00\x00")

    def bot_command(text: str) -> None:
        bot["o"].send(0x07, wire.mc_string(text))
        bot["o"].send(0x0D)

    def bot_wait(marker: str, label: str) -> None:
        bot["o"].wait(lambda _: any(marker in event for event in bot["o"].events), description=label, timeout=30)

    def fence(label: str) -> dict[str, Any]:
        """Order everything sent before: the operator's give is processed after it, then read the client."""
        count = item_count(probe.observe(), "minecraft:dirt") + 1
        probe.fixture("give @s minecraft:dirt 1")
        probe.call("minecraft_wait_for_inventory", {"item_id": "minecraft:dirt", "count": count,
                                                    "timeout_seconds": probe.remaining(limit=40)})
        state = probe.observe()
        report.setdefault("observations", []).append({"label": label, "new_lines": fresh(state),
                                                       "dimension": state.get("dimension")})
        return state

    def mark() -> None:
        """Later checks only look at chat lines that arrived after this point.

        recent_chat is newest first and keeps 20 lines, so the new lines are the shortest prefix of the current log
        after which the rest lines up with the snapshot taken here."""
        counters["snapshot"] = lines(probe.observe())

    def fresh(state: dict[str, Any]) -> list[str]:
        now = lines(state)
        old = counters.get("snapshot", [])
        for k in range(len(now) + 1):
            if now[k:] == old[:len(now) - k]:
                return now[:k]
        return now

    def heard(state: dict[str, Any], marker: str) -> bool:
        return any(marker in line for line in fresh(state))

    def bot_speaks_and_fence(marker: str) -> None:
        """Bot chats; the bot's own echo (it is full) orders it before the native fence."""
        bot_chat(marker)
        bot_wait(marker, f"bot_echo_{marker}")

    def dimension_is(name: str) -> Callable[[dict[str, Any]], bool]:
        return lambda s: name in json.dumps(s.get("dimension", s.get("player", {})))

    def inject(mode: int) -> None:
        """Switch in place, then a fence on the same connection proves the server applied it."""
        proxy.logins[-1].inject(mode)
        fence(f"inject_{NAMES[mode]}_applied")
        mark()

    # ---------- c0: full, in-place switches ----------
    probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
    home = probe.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float), "c0_player_present")["player"]
    HOME = f"{home['x']:.2f} {home['y'] + 0.5:.2f} {home['z']:.2f}"

    def protect() -> None:
        """Dimension hops must not kill the player; death clears effects, so this runs again after respawns."""
        probe.fixture("effect give @s minecraft:resistance infinite 4 true")
        probe.fixture("effect give @s minecraft:fire_resistance infinite 0 true")

    protect()
    mark()
    connect_bot()

    def c0_full_baseline() -> None:
        bot_speaks_and_fence("BotChat_c0_full")
        probe.fixture("say Say_c0_full")
        probe.fixture("me Me_c0_full")
        bot_command(f"msg {USERNAME} BotMsg_c0_full")
        bot_wait("BotMsg_c0_full", "bot_msg_outgoing_full")
        state = fence("c0_full")
        soft("full_hears_other_player_chat", heard(state, "BotChat_c0_full"))
        soft("full_hears_say", heard(state, "Say_c0_full"))
        soft("full_hears_me", heard(state, "Me_c0_full"))
        soft("full_hears_incoming_msg", heard(state, "BotMsg_c0_full"))
        soft("full_hears_bot_join", heard(state, f"{BOT} joined the game"))
    step("c0_full_baseline", c0_full_baseline)

    def c0_inplace_commands_only() -> None:
        inject(COMMANDS_ONLY)
        # The setting change is a Play input like any other: it is ordered before the next command.
        bot_speaks_and_fence("BotChat_c0_cmd")
        probe.fixture("say Say_c0_cmd")
        probe.fixture("me Me_c0_cmd")
        probe.fixture('tellraw @s {"text":"Sys_c0_cmd"}')
        probe.fixture("time query daytime")
        bot_command(f"msg {USERNAME} BotMsg_c0_cmd")
        bot_wait("BotMsg_c0_cmd", "bot_msg_outgoing_cmd")
        probe.fixture(f"msg {BOT} OutMsg_c0_cmd")
        bot_wait("OutMsg_c0_cmd", "bot_receives_out_msg_cmd")
        probe.call("minecraft_send_chat", {"message": "CmdOnlySpeak_c0", "command": False})
        state = fence("c0_inplace_commands_only")
        bot_wait("CmdOnlySpeak_c0", "bot_hears_commands_only_sender")
        soft("cmd_misses_other_player_chat", not heard(state, "BotChat_c0_cmd"))
        soft("cmd_misses_say", not heard(state, "Say_c0_cmd"))
        soft("cmd_misses_me", not heard(state, "Me_c0_cmd"))
        soft("cmd_misses_incoming_msg", not heard(state, "BotMsg_c0_cmd"))
        soft("cmd_hears_tellraw", heard(state, "Sys_c0_cmd"))
        soft("cmd_hears_command_feedback", heard(state, "The time is"))
        soft("cmd_hears_give_feedback", heard(state, "Gave 1 Dirt"))
        soft("cmd_outgoing_msg_copy_hidden_like_vanilla", not heard(state, "OutMsg_c0_cmd"),
             "vanilla MsgCommand sends the sender's copy as chat (MSG_COMMAND_OUTGOING)")
        soft("cmd_own_public_chat_not_echoed", not heard(state, "CmdOnlySpeak_c0"))
        soft("cmd_sender_reaches_full_bot", "CmdOnlySpeak_c0" in " ".join(bot_events()))
    step("c0_inplace_commands_only", c0_inplace_commands_only)

    def c0_inplace_hidden() -> None:
        inject(HIDDEN)
        bot_speaks_and_fence("BotChat_c0_hidden")
        probe.fixture("say Say_c0_hidden")
        probe.fixture('tellraw @s {"text":"Sys_c0_hidden"}')
        probe.fixture("time query daytime")
        probe.fixture("help")  # edge-handled command
        probe.fixture("breakb10nosuchcommand")  # unknown command feedback
        before = lines(probe.observe())
        probe.call("minecraft_send_chat", {"message": "HiddenSpeak_c0", "command": False})
        probe.fixture("say FenceSay_c0_hidden")
        bot_wait("FenceSay_c0_hidden", "bot_fence_after_hidden_speak")
        state = fence("c0_inplace_hidden")
        new = fresh(state)
        soft("hidden_misses_other_player_chat", not heard(state, "BotChat_c0_hidden"))
        soft("hidden_misses_say", not heard(state, "Say_c0_hidden"))
        soft("hidden_misses_tellraw", not heard(state, "Sys_c0_hidden"))
        soft("hidden_misses_core_feedback", not heard(state, "The time is"))
        soft("hidden_misses_give_feedback_after_switch", not heard(state, "Gave 1 Dirt"), {"new_lines": new})
        soft("hidden_misses_edge_help_and_unknown_command", not any("Unknown" in l or "/help" in l or "Incorrect" in l
                                                                 for l in new), {"new_lines": new})
        soft("hidden_speaker_told_chat_disabled", any("isabled" in l for l in new), {"new_lines": new})
        soft("hidden_speaker_chat_not_broadcast", "HiddenSpeak_c0" not in " ".join(bot_events()))
    step("c0_inplace_hidden", c0_inplace_hidden)

    def c0_back_to_full() -> None:
        inject(FULL)
        bot_speaks_and_fence("BotChat_c0_full_again")
        state = fence("c0_back_to_full")
        soft("switch_back_full_hears_chat", heard(state, "BotChat_c0_full_again"))
    step("c0_back_to_full", c0_back_to_full)

    # ---------- c0 in the Nether: cross-dimension relay and in-place switch there ----------
    def c0_nether() -> None:
        probe.fixture("execute in minecraft:the_nether run tp @s 0 80 0")
        probe.wait(dimension_is("the_nether"), "c0_in_nether", wait_for_play=True, timeout_seconds=60)
        mark()
        bot_speaks_and_fence("BotChat_nether_full")
        bot_command(f"msg {USERNAME} BotMsg_nether_full")
        state = fence("c0_nether_full")
        soft("nether_full_hears_overworld_chat", heard(state, "BotChat_nether_full"))
        soft("nether_full_hears_cross_dimension_msg", heard(state, "BotMsg_nether_full"))
        inject(COMMANDS_ONLY)
        bot_speaks_and_fence("BotChat_nether_cmd")
        probe.fixture('tellraw @s {"text":"Sys_nether_cmd"}')
        state = fence("c0_nether_cmd")
        soft("nether_switch_cmd_misses_overworld_chat", not heard(state, "BotChat_nether_cmd"))
        soft("nether_switch_cmd_hears_tellraw", heard(state, "Sys_nether_cmd"))
        # Death in the Nether as commands-only: the death message is system text and arrives.
        probe.fixture("kill @s")
        probe.wait(lambda s: (s.get("player") or {}).get("health", 1) <= 0 or "death" in json.dumps(s.get("screen", "")).lower(),
                   "c0_dead_in_nether", timeout_seconds=30)
        dead = probe.observe()
        report["death_screen"] = dead.get("screen")
        soft("cmd_hears_own_death_message", any(USERNAME in l and ("killed" in l or "died" in l or "fell" in l)
                                                 for l in lines(dead)), lines(dead)[:5])
        bot_wait(f"{USERNAME}", "bot_sees_death")
        probe.call("minecraft_respawn", {})
        probe.wait(lambda s: (s.get("player") or {}).get("health", 0) > 0 and dimension_is("overworld")(s),
                   "c0_respawned_overworld", wait_for_play=True, timeout_seconds=60)
        protect()
        bot_speaks_and_fence("BotChat_after_respawn")
        probe.fixture('tellraw @s {"text":"Sys_after_respawn"}')
        state = fence("c0_after_respawn")
        soft("respawn_keeps_commands_only_misses_chat", not heard(state, "BotChat_after_respawn"))
        soft("respawn_keeps_commands_only_hears_system", heard(state, "Sys_after_respawn"))
        # Hidden and die again: no death message for the hidden player, the bot still sees it.
        inject(HIDDEN)
        mark()
        probe.fixture("kill @s")
        probe.wait(lambda s: (s.get("player") or {}).get("health", 1) <= 0 or "death" in json.dumps(s.get("screen", "")).lower(),
                   "c0_dead_hidden", timeout_seconds=30)
        probe.call("minecraft_respawn", {})
        probe.wait(lambda s: (s.get("player") or {}).get("health", 0) > 0, "c0_respawned_hidden",
                   wait_for_play=True, timeout_seconds=60)
        protect()
        state = fence("c0_after_hidden_death")
        new = fresh(state)
        soft("hidden_misses_own_death_message", not any(USERNAME in l and ("killed" in l or "died" in l) for l in new),
             {"new_lines": new})
    step("c0_nether_switch_death_respawn", c0_nether)

    def c0_end() -> None:
        # Still hidden (set above). Travel to the End and back by teleport.
        probe.fixture("execute in minecraft:the_end run tp @s 10 80 10")
        probe.wait(dimension_is("the_end"), "c0_in_end", wait_for_play=True, timeout_seconds=60)
        mark()
        bot_speaks_and_fence("BotChat_end_hidden")
        probe.fixture('tellraw @s {"text":"Sys_end_hidden"}')
        state = fence("c0_end_hidden")
        soft("end_hidden_misses_chat", not heard(state, "BotChat_end_hidden"))
        soft("end_hidden_misses_system", not heard(state, "Sys_end_hidden"))
        probe.fixture(f"execute in minecraft:overworld run tp @s {HOME}")
        probe.wait(dimension_is("overworld"), "c0_back_from_end", wait_for_play=True, timeout_seconds=60)
        mark()
        bot_speaks_and_fence("BotChat_from_end_hidden")
        probe.fixture('tellraw @s {"text":"Sys_from_end_hidden"}')
        state = fence("c0_from_end_hidden")
        soft("from_end_hidden_misses_chat", not heard(state, "BotChat_from_end_hidden"))
        soft("from_end_hidden_misses_system", not heard(state, "Sys_from_end_hidden"))
    step("c0_end_round_trip", c0_end)

    # ---------- c1: reconnect hidden; join-time text ----------
    def c1_hidden_rejoin() -> None:
        probe.call("minecraft_disconnect", {})
        probe.call("minecraft_connect", {"server_addr": address})
        probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
        probe.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float), "c1_player_present")
        counters["snapshot"] = []  # a new connection starts an empty chat log; join-time text counts
        soft("c1_claimed_hidden", len(proxy.logins) == 2 and proxy.logins[1].mode == HIDDEN,
             [r.rewrites for r in proxy.logins])
        bot["o"].close()
        connect_bot()  # BreakBot joined the game: system text, hidden must not see it
        bot_speaks_and_fence("BotChat_c1_hidden")
        state = fence("c1_hidden_join")
        soft("c1_hidden_misses_bot_join", not heard(state, f"{BOT} joined the game"))
        soft("c1_hidden_misses_chat", not heard(state, "BotChat_c1_hidden"))
        soft("c1_hidden_receives_no_text_at_all", not fresh(state), fresh(state))
    step("c1_hidden_rejoin", c1_hidden_rejoin)

    # ---------- c2: server restart while hidden ----------
    def c2_restart() -> None:
        bot["o"].close()
        restart(1)
        probe.call("minecraft_connect", {"server_addr": address})
        probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
        probe.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float), "c2_player_present")
        counters["snapshot"] = []
        report["c2_logins"] = len(proxy.logins)
        connect_bot()
        bot_speaks_and_fence("BotChat_c2_hidden")
        probe.fixture('tellraw @s {"text":"Sys_c2_hidden"}')
        state = fence("c2_after_restart")
        soft("restart_hidden_misses_chat", not heard(state, "BotChat_c2_hidden"))
        soft("restart_hidden_misses_system", not heard(state, "Sys_c2_hidden"))
        soft("restart_hidden_receives_no_text_at_all", not fresh(state), fresh(state))
        inject(FULL)
        bot_speaks_and_fence("BotChat_c2_full")
        state = fence("c2_full_positive_control")
        soft("restart_switch_full_hears_chat", heard(state, "BotChat_c2_full"))
    step("c2_restart", c2_restart)

    # ---------- c3: edge-answered commands x chat mode x send_command_feedback ----------
    EDGE_SUCCESS = {"list": "players online", "help": "/help", "save-all": "Sav", "whitelist list": "whitelisted",
                    "banlist players": "banned players"}
    CORE_SUCCESS = {"time query daytime": "The time is"}
    FAILURE = {"breakb10nosuchcommand": "Unknown or incomplete command", "whitelist frobnicate": "Incorrect argument"}

    def matrix_cell(label: str, mode: int, feedback_on: bool) -> None:
        probe.fixture(f"gamerule send_command_feedback {'true' if feedback_on else 'false'}")
        fence(f"{label}_gamerule_applied")  # the core applies it on its next tick; the edge answers at once
        inject(mode)
        for command in [*EDGE_SUCCESS, *CORE_SUCCESS, *FAILURE]:
            probe.fixture(command)
        state = fence(label)
        new = fresh(state)
        report.setdefault("edge_matrix", {})[label] = new
        for command, needle in {**EDGE_SUCCESS, **CORE_SUCCESS}.items():
            want = mode != HIDDEN and feedback_on
            soft(f"{label}_{command.replace(' ', '_')}_success_{'shown' if want else 'silent'}",
                 any(needle in l for l in new) == want, new)
        for command, needle in FAILURE.items():
            want = mode != HIDDEN
            soft(f"{label}_{command.split()[0]}_failure_{'shown' if want else 'silent'}",
                 any(needle in l for l in new) == want, new)
        if mode == HIDDEN:
            soft(f"{label}_hidden_no_text_at_all", not new, new)

    def c3_edge_matrix() -> None:
        for mode in (FULL, COMMANDS_ONLY, HIDDEN):
            for on in (True, False):
                step(f"c3_{NAMES[mode]}_{'on' if on else 'off'}",
                     lambda mode=mode, on=on: matrix_cell(f"c3_{NAMES[mode]}_{'on' if on else 'off'}", mode, on))
        probe.fixture("gamerule send_command_feedback true")
        fence("c3_reset")
    step("c3_edge_matrix", c3_edge_matrix)

    def c4_gamerule_from_nether() -> None:
        """The edge reads a flag the core publishes; set the rule from a player standing in the Nether."""
        inject(FULL)
        probe.fixture("execute in minecraft:the_nether run tp @s 0 80 0")
        probe.wait(dimension_is("the_nether"), "c4_in_nether", wait_for_play=True, timeout_seconds=60)
        probe.fixture("gamerule send_command_feedback false")
        fence("c4_gamerule_off_applied")
        mark()
        probe.fixture("list")
        probe.fixture("time query daytime")
        state = fence("c4_nether_off")
        new = fresh(state)
        report["c4_nether_off"] = new
        soft("c4_core_success_silent_after_nether_gamerule", not any("The time is" in l for l in new), new)
        soft("c4_edge_list_silent_after_nether_gamerule", not any("players online" in l for l in new), new)
        probe.fixture(f"execute in minecraft:overworld run tp @s {HOME}")
        probe.wait(dimension_is("overworld"), "c4_back", wait_for_play=True, timeout_seconds=60)
        mark()
        probe.fixture("list")
        state = fence("c4_overworld_off")
        report["c4_overworld_off"] = fresh(state)
        soft("c4_edge_list_silent_in_overworld_after_nether_gamerule",
             not any("players online" in l for l in fresh(state)), fresh(state))
        probe.fixture("gamerule send_command_feedback true")
        fence("c4_reset")
    step("c4_gamerule_from_nether", c4_gamerule_from_nether)

    def c5_rule_off_survives_restart() -> None:
        """Overworld path and persistence: rule off, /list silent; restart; still silent; Nether again after restart."""
        inject(FULL)
        probe.fixture("gamerule send_command_feedback false")
        fence("c5_off_applied")
        mark()
        probe.fixture("list")
        state = fence("c5_overworld_off")
        soft("c5_overworld_gamerule_silences_list", not any("players online" in l for l in fresh(state)), fresh(state))
        bot["o"].close()
        restart(2)
        probe.call("minecraft_connect", {"server_addr": address})
        probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
        probe.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float), "c5_rejoined")
        inject(FULL)
        probe.fixture("list")
        probe.fixture("time query daytime")
        state = fence("c5_after_restart")
        soft("c5_list_silent_after_restart", not any("players online" in l for l in fresh(state)), fresh(state))
        soft("c5_core_silent_after_restart", not any("The time is" in l for l in fresh(state)), fresh(state))
        # Turn it back on from the Nether: does /list come back?
        probe.fixture("execute in minecraft:the_nether run tp @s 0 80 0")
        probe.wait(dimension_is("the_nether"), "c5_in_nether", wait_for_play=True, timeout_seconds=60)
        probe.fixture("gamerule send_command_feedback true")
        fence("c5_on_from_nether")
        mark()
        probe.fixture("list")
        probe.fixture("time query daytime")
        state = fence("c5_nether_on")
        soft("c5_core_back_after_nether_gamerule_on", any("The time is" in l for l in fresh(state)), fresh(state))
        soft("c5_list_back_after_nether_gamerule_on", any("players online" in l for l in fresh(state)), fresh(state))
    step("c5_rule_off_survives_restart", c5_rule_off_survives_restart)

    def c6_teammsg_commands_only() -> None:
        probe.fixture("team add breakteam")
        probe.fixture(f"team join breakteam {USERNAME}")
        probe.fixture(f"team join breakteam {BOT}")
        inject(COMMANDS_ONLY)
        probe.fixture('tellraw @s {"text":"Sys_c6"}')
        probe.fixture("teammsg TeamMark_c6")
        state = fence("c6_teammsg")
        report["c6_lines"] = fresh(state)
        soft("c6_commands_only_teammsg_sender_copy_hidden_like_vanilla", not heard(state, "TeamMark_c6")
             and heard(state, "Sys_c6"), fresh(state))
        bot_wait("TeamMark_c6", "c6_bot_teammate_hears")
        soft("c6_full_teammate_hears_teammsg", "TeamMark_c6" in " ".join(bot_events()))
        inject(FULL)
        probe.fixture("teammsg TeamMark_c6_full")
        state = fence("c6_teammsg_full")
        soft("c6_full_sender_sees_own_copy", heard(state, "TeamMark_c6_full"), fresh(state))
    step("c6_teammsg_commands_only", c6_teammsg_commands_only)

    def c7_end_exit_portal() -> None:
        """Hidden player leaves the End through an exit portal (credits path), not by teleport."""
        inject(HIDDEN)
        probe.fixture("execute in minecraft:the_end run tp @s 10 80 10")
        probe.wait(dimension_is("the_end"), "c7_in_end", wait_for_play=True, timeout_seconds=60)
        probe.fixture("execute at @s run setblock ~ ~ ~ minecraft:end_portal")
        try:
            probe.wait(dimension_is("overworld"), "c7_left_end", wait_for_play=True, timeout_seconds=25)
        except Exception as error:
            report["c7_first_wait"] = {"error": str(error), "screen": probe.observe().get("screen")}
            probe.call("minecraft_respawn", {})
            probe.wait(dimension_is("overworld"), "c7_left_end_after_respawn", wait_for_play=True, timeout_seconds=30)
        protect()
        mark()
        bot_speaks_and_fence("BotChat_c7")
        probe.fixture('tellraw @s {"text":"Sys_c7"}')
        state = fence("c7_after_exit")
        soft("c7_hidden_after_end_exit_portal", not fresh(state), fresh(state))
        inject(FULL)
        bot_speaks_and_fence("BotChat_c7_full")
        state = fence("c7_full_control")
        soft("c7_full_control_after_exit", heard(state, "BotChat_c7_full"), fresh(state))
    step("c7_end_exit_portal", c7_end_exit_portal)

    def c8_gamerule_from_end() -> None:
        """send_command_feedback set from the End, both directions; /list read in the End and in the overworld."""
        inject(FULL)
        probe.fixture("execute in minecraft:the_end run tp @s 10 80 10")
        probe.wait(dimension_is("the_end"), "c8_in_end", wait_for_play=True, timeout_seconds=60)
        probe.fixture("gamerule send_command_feedback false")
        fence("c8_off_applied")
        mark()
        probe.fixture("list")
        probe.fixture("time query daytime")
        state = fence("c8_end_off")
        soft("c8_end_off_core_silent", not heard(state, "The time is"), fresh(state))
        soft("c8_end_off_list_silent", not heard(state, "players online"), fresh(state))
        probe.fixture(f"execute in minecraft:overworld run tp @s {HOME}")
        probe.wait(dimension_is("overworld"), "c8_back_off", wait_for_play=True, timeout_seconds=60)
        mark()
        probe.fixture("list")
        state = fence("c8_overworld_off")
        soft("c8_overworld_list_silent_after_end_off", not heard(state, "players online"), fresh(state))
        probe.fixture("execute in minecraft:the_end run tp @s 10 80 10")
        probe.wait(dimension_is("the_end"), "c8_in_end_again", wait_for_play=True, timeout_seconds=60)
        probe.fixture("gamerule send_command_feedback true")
        fence("c8_on_applied")
        mark()
        probe.fixture("list")
        probe.fixture("time query daytime")
        state = fence("c8_end_on")
        soft("c8_end_on_core_back", heard(state, "The time is"), fresh(state))
        soft("c8_end_on_list_back", heard(state, "players online"), fresh(state))
        probe.fixture(f"execute in minecraft:overworld run tp @s {HOME}")
        probe.wait(dimension_is("overworld"), "c8_back_on", wait_for_play=True, timeout_seconds=60)
        mark()
        probe.fixture("list")
        state = fence("c8_overworld_on")
        soft("c8_overworld_list_back_after_end_on", heard(state, "players online"), fresh(state))
    step("c8_gamerule_from_end", c8_gamerule_from_end)

    failed = sorted([name for name, row in report["attacks"].items() if not row["ok"]]
                    + [name for name in report["errors"] if not name.endswith("_trace")])
    report["failed"] = failed
    if failed:
        raise RuntimeError(f"break-b10-chat failures: {failed}")
