"""Join-lag gate: the edge pump lagging behind the tick, on real native clients.

Rows (docs/bugs/rows):
- EDGE-SLOW-PUMP-HOLDS-JOIN-BEHIND-STALE-OUTPUTS: a pump slower than the tick
  sits at the oldest retained broadcast output, so a `Joined` waits a whole
  window of tick periods and times out at the 10 s join deadline.
- EDGE-OUTBOX-STALL-WATCH-HALTS-LIVE-LAGGING-PUMP: the 10 s reliable outbox
  stall watch halts the core under a live pump that only lags.
- ANTI-XRAY-BORDER-REVEAL-LOST-ACROSS-SKIPPED-PUBLISH: a border ore's mask
  changed by its side neighbour during a publish the pump skips stays wrong on
  the client.

The pump is made slower than the tick by real client work, not by a knob: each
flood client fills a slab across its own 9x9 chunk view, and a block change
evicts its chunk from the pump's framed-chunk cache, so every pump publish
re-encodes its viewers' views (measured about 143 ms per cold view against a
50 ms tick at 20 TPS). Several flood clients keep every publish longer than a
tick, the broadcast queue grows, and the pump falls behind exactly as the rows
describe. The dashboard's `pump_skipped_ticks` says whether the condition was
reached: a run that never skips reports `inconclusive` (null, never false, so
it can never read as the row's symptom).

Clients: A the witness (windowed, holds the border ore and stays put), flood
clients (headless) and a fresh joiner launched while the pump is behind.
Criteria:

- `border_reveal_after_skipped_publish`: A holds a chunk whose border diamond
  ore is masked by a stone side cell in the neighbouring chunk; while the pump
  skips, that side cell becomes air, and A's own loaded world must then hold
  the diamond ore (the skip repair re-sent the held chunk for a side change).
- `join_behind_slow_pump`: with the flood running and skips observed, a fresh
  client enters play inside the 10 s join deadline.
- `live_pump_does_not_halt`: the whole time the flood runs, `/stats` keeps
  advancing its simulation tick and the server log carries no
  `edge pump not draining` / core-halt line.

Xvfb + `--windowed` is the graphical evidence (`native_client.capture` at each
step); a headless run is received-state evidence only.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import threading
import time
import urllib.request
from dataclasses import replace
from datetime import datetime
from pathlib import Path
from typing import Any

from .. import runtime
from ..profiles import ProfileContext
from . import native_client
from .chisel import _pad_commands, rewrite_config
from .gameplay import peak_rss_kib, prepare_operator_fixture, require_tools
from .probe_steps import CLIENT_STATE_EVENT_EXPIRED, ProbeSteps

WITNESS = "JoinLagWitness"
FLOOD = "JoinLagFlood"
JOINER = "JoinLagJoiner"
TIMEOUT = 1800.0
SEED = 81
STONE = "minecraft:stone"
DIAMOND = "minecraft:diamond_ore"
AIR = "minecraft:air"
JOIN_BOUND_SECONDS = 10.0  # the server's join deadline the row names
SKIP_WAIT_SECONDS = 20.0   # how long the gate waits for the pump to start skipping
FLOOD_DX = 512             # flood areas sit far outside the witness fixtures
FLOOD_RADIUS = 72          # a 145x145 slab covers the whole 9x9 chunk view
FILL_PAUSE = 0.35          # pacing between a flood client's fills
SCENARIOS = ("all", "join-stall", "border-reveal")
CRITERIA = ("border_reveal_after_skipped_publish", "join_behind_slow_pump", "live_pump_does_not_halt")

REQUIRED_TOOLS = frozenset({
    "minecraft_observe",
    "minecraft_wait_for_play",
    "minecraft_wait_for_state_change",
    "minecraft_wait_for_loaded_block",
    "minecraft_read_block",
    "minecraft_send_chat",
    "minecraft_connect",
    "minecraft_disconnect",
})


def border_layout(ox: int, oy: int, oz: int) -> dict[str, Any]:
    """The border-ore fixture around the witness (pure helper).

    The ore sits in the witness's own chunk at its east edge; the cell that
    masks it lies in the neighbouring chunk, so changing that cell changes the
    ore's mask without touching the ore chunk's own revision (no block, no
    light change inside it).
    """
    border = (ox // 16) * 16 + 15  # last block of the witness's chunk along x
    y = oy - 5                     # below the pad: no sky light reaches it
    z = oz
    return {
        "ore": (border, y, z),
        "side": (border + 1, y, z),
        "fill": (border - 2, y - 1, z - 1, border + 3, y + 1, z + 1),
    }


def join_verdict(elapsed: float | None, skips_before: int, skips_after: int,
                 halt: str | None) -> tuple[bool | None, str]:
    """Row 1 verdict: None when the lagging condition never occurred, False only for a real symptom."""
    if halt:
        return False, f"the core halted during the flood: {halt}"
    if skips_after <= skips_before:
        return None, "no pump skip was observed: the lagging condition did not occur"
    if elapsed is None:
        return False, "the joining client never entered play"
    if elapsed > JOIN_BOUND_SECONDS:
        return False, f"the join landed after {elapsed:.1f} s, past the {JOIN_BOUND_SECONDS:.0f} s deadline"
    return True, f"the join landed in {elapsed:.1f} s while the pump skipped {skips_after - skips_before} outputs"


def halt_verdict(skips_before: int, skips_after: int, tick_before: int, tick_after: int,
                 halt: str | None) -> tuple[bool | None, str]:
    """Row 2 verdict: None when the lagging condition never occurred, False only for a real symptom."""
    if halt:
        return False, f"the core halted during the flood: {halt}"
    if skips_after <= skips_before:
        return None, "no pump skip was observed: the lagging condition did not occur"
    if tick_after <= tick_before:
        return False, f"the simulation tick did not advance ({tick_before} -> {tick_after})"
    return True, (f"the core kept ticking {tick_before} -> {tick_after} through "
                  f"{skips_after - skips_before} skipped pump outputs")


def border_verdict(client_block: str | None, skips_before: int, skips_after: int) -> tuple[bool | None, str]:
    """Row 3 verdict: None when the skipped publish never occurred, False only for a stale mask."""
    if skips_after <= skips_before:
        return None, "no pump skip was observed: the skipped-publish condition did not occur"
    if client_block == DIAMOND:
        return True, f"the client holds {DIAMOND} after {skips_after - skips_before} skipped outputs"
    return False, f"the client still holds {client_block!r}, not {DIAMOND}"


def disposition(verdicts: dict[str, bool | None]) -> tuple[str, list[str]]:
    """How a run ends: a false symptom fails it; an unreached condition is inconclusive, never false.

    `ledger._failing_verdict` reads a false verdict as the row's symptom, so a run that never
    reached the condition must not report false: it ends failed with an inconclusive reason and
    leaves the row's verdict out.
    """
    failed = [name for name, value in verdicts.items() if value is False]
    if failed:
        return "failed", failed
    unproven = [name for name, value in verdicts.items() if value is None]
    if unproven:
        return "inconclusive", unproven
    return "passed", []


def halt_line(log_path: Path) -> str | None:
    """The core-halt line of a server log, if the fail-closed halt happened (pure helper)."""
    try:
        text = log_path.read_text(errors="replace")
    except OSError:
        return None
    for line in text.splitlines():
        if "edge pump not draining" in line or "core halted" in line:
            return line.strip()
    return None


def join_timing(log_path: Path, username: str) -> float | None:
    """Seconds from the server's `login admitted` for `username` to its `join landed`, or None.

    Server-side timestamps only: a client's own startup never counts as join latency.
    """
    def stamp(line: str) -> datetime | None:
        head = line.split(" ", 1)[0]
        try:
            return datetime.fromisoformat(head.replace("Z", "+00:00"))
        except ValueError:
            return None

    admitted: datetime | None = None
    connection: str | None = None
    for line in log_path.read_text(errors="replace").splitlines():
        if admitted is None and "login admitted" in line and f"name={username}" in line:
            match = re.search(r"connection=(\d+)", line)
            if match:
                connection = match.group(1)
                admitted = stamp(line)
            continue
        if admitted is not None and connection and "join landed" in line and \
                re.search(rf"\bconnection\s*=\s*{connection}\b", line):
            landed = stamp(line)
            if landed is not None:
                return (landed - admitted).total_seconds()
    return None


def private_xdg_root(artifact: Path) -> str:
    """A private XDG_DATA_HOME whose `krasnokamen-client/` links the content dirs, never copies.

    The native client reads its facts and content under `$XDG_DATA_HOME/krasnokamen-client`;
    a shared root leaks settings between runs and a missing one exits before MCP is ready
    (docs/development/real-client-gates.md).
    """
    from ..preflight import CLIENT_CONTENT_DIRS, client_template

    source = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share") / "krasnokamen-client"
    if not (source / "facts").is_dir():
        source = client_template()
    root = artifact / "xdg-data"
    target = root / "krasnokamen-client"
    target.mkdir(parents=True, exist_ok=True)
    for name in CLIENT_CONTENT_DIRS:
        link = target / name
        if link.exists() or link.is_symlink():
            continue
        if (source / name).exists():
            link.symlink_to((source / name).resolve(), target_is_directory=(source / name).is_dir())
    return str(root)


def _stat(dashboard_port: int) -> dict[str, Any]:
    with urllib.request.urlopen(f"http://127.0.0.1:{dashboard_port}/stats", timeout=5) as reply:
        return json.loads(reply.read().decode())


def _skips(stats: dict[str, Any]) -> int:
    return int(stats.get("pump_skipped_ticks") or 0)


def _tick(stats: dict[str, Any]) -> int:
    return int(stats.get("simulation_tick") or 0)


def validate_args(ctx: ProfileContext) -> tuple[float, int, int, int, int, str]:
    if ctx.client != "native":
        raise ValueError("join-lag-gate requires --client native")
    if ctx.platform is not None:
        raise ValueError("join-lag-gate accepts only --client, --windowed, --timeout-seconds and its own args")
    parser = argparse.ArgumentParser(prog="join-lag-gate")
    parser.add_argument("--scenario", choices=SCENARIOS, default="all")
    # 20 TPS is the row's own rate: the pump must outpace the tick for the queue to build in
    # seconds; a slower tick needs a queue of dozens of periods and takes minutes to fill.
    parser.add_argument("--tick-rate", type=int, default=20)
    parser.add_argument("--flood-clients", type=int, default=1)
    # The gate's own condition lever: the server publishes at most this many outputs per second for
    # the run (KRASNOKAMEN_PUMP_PUBLISH_PER_SECOND, a test/dev knob, never an operator setting).
    parser.add_argument("--pump-publish-per-second", type=int, default=3)
    parser.add_argument("--flood-steps", type=int, default=60,
                        help="cap on view-wide fills per flood client while waiting for pump skips")
    args = parser.parse_args(ctx.extra_args)
    if not 1 <= args.tick_rate <= 20:
        raise ValueError("join-lag-gate --tick-rate must be in [1, 20]")
    if not 0 <= args.flood_clients <= 4:
        raise ValueError("join-lag-gate --flood-clients must be in [0, 4]")
    if not 0 <= args.pump_publish_per_second <= 100:
        raise ValueError("join-lag-gate --pump-publish-per-second must be in [0, 100]")
    if not 4 <= args.flood_steps <= 128:
        raise ValueError("join-lag-gate --flood-steps must be in [4, 128]")
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else TIMEOUT
    if not 0 < timeout <= 2400:
        raise ValueError("join-lag-gate --timeout-seconds must be in (0, 2400]")
    return (timeout, args.tick_rate, args.flood_clients, args.flood_steps,
            args.pump_publish_per_second, args.scenario)


def run(ctx: ProfileContext) -> dict[str, Any]:
    timeout, tick_rate, flood_clients, flood_steps, pump_rate, scenario = validate_args(ctx)
    artifact = ctx.artifact_dir
    override = os.environ.get("KRASNOKAMEN_SERVER_BINARY")
    if not override:
        raise RuntimeError("join-lag-gate requires KRASNOKAMEN_SERVER_BINARY (a krasnokamen built from this tree)")
    binary = Path(override).expanduser().resolve()
    if not binary.is_file():
        raise RuntimeError(f"missing executable server binary: {binary}")
    server_port = runtime.reserve_port()
    dashboard_port = runtime.reserve_port()
    address = f"127.0.0.1:{server_port}"
    config = artifact / "join-lag-gate.toml"
    (artifact / "plugins").mkdir(exist_ok=True)
    text = rewrite_config(
        (runtime.REPO_ROOT / "playable.toml").read_text(),
        port=server_port,
        world=(artifact / "world").relative_to(runtime.REPO_ROOT).as_posix(),
        plugins=str(artifact / "plugins"),
        seed=SEED,
    )
    # The dashboard is read-only and loopback-only; the gate reads its pump and tick counters.
    text += f'\n[dashboard]\nenabled = true\nbind_address = "127.0.0.1"\nport = {dashboard_port}\n'
    config.write_text(text)
    names = [WITNESS, JOINER] + [f"{FLOOD}{index}" for index in range(1, flood_clients + 1)]
    prepare_operator_fixture(ctx, names)
    evidence = ctx.client_evidence
    with binary.open("rb") as server_executable:
        server_hash = hashlib.file_digest(server_executable, "sha256").hexdigest()
    evidence.update(server_address=address, server_binary_path=str(binary),
                    server_binary_sha256=server_hash, world_dir=str(artifact / "world"),
                    seed=SEED, dashboard_address=f"127.0.0.1:{dashboard_port}",
                    flood_clients=names[2:])
    previous_xdg = os.environ.get("XDG_DATA_HOME")
    os.environ["XDG_DATA_HOME"] = private_xdg_root(artifact)
    server = None
    xvfb = None
    clients: list[tuple[subprocess.Popen[Any], Any]] = []
    server_log = None
    server_log_path = artifact / "server.log"
    verdicts: dict[str, bool | None] = {}
    try:
        server_log = server_log_path.open("wb")
        env = os.environ.copy()
        env["KRASNOKAMEN_HARNESS_LOG_STDOUT"] = "1"
        # The pump's debug `join landed` line is the server-side join timing this gate reads.
        env.setdefault("RUST_LOG", "info,mc_edge::server::pump=debug")
        if pump_rate:
            # The pump publishes at most N outputs per second: the deterministic condition of these
            # rows. Unset or 0 keeps the shipped unlimited pump.
            env["KRASNOKAMEN_PUMP_PUBLISH_PER_SECOND"] = str(pump_rate)
        server = runtime.start_server([str(binary), "--config", str(config), "--no-console"],
                                      cwd=runtime.REPO_ROOT, stdout=server_log,
                                      stderr=subprocess.STDOUT, start_new_session=True, env=env)
        ctx.logs.append(str(server_log_path))
        runtime.wait_port(server_port, timeout, server, log_path=server_log_path)
        display = None
        if ctx.windowed:
            xvfb, display, _ = runtime.start_xvfb(artifact)
            ctx.logs.append(str(artifact / "xvfb.log"))

        def launch(owner: ProfileContext, username: str, *, display: str | None,
                   server_addr: str | None = None) -> tuple[subprocess.Popen[Any], Any]:
            process, _log, mcp = native_client.start(
                owner, native_client.resolve_binary(owner), address if server_addr is None else server_addr,
                username, timeout, display=display)
            mcp.request_timeout_seconds = min(timeout + 10, 300.0)
            clients.append((process, mcp))
            return process, mcp

        def peer_context(name: str) -> ProfileContext:
            return replace(ctx, artifact_dir=artifact / name, client_evidence={}, windowed=False)

        # The witness first: it builds the fixture and reaches play before any load exists.
        process_a, mcp_a = launch(ctx, WITNESS, display=display)
        with ProbeSteps(mcp_a, artifact / "join-lag-witness", timeout, "native-join-lag-witness") as a:
            require_tools(mcp_a.list_tools(), REQUIRED_TOOLS, "join-lag-gate")
            a.call("minecraft_wait_for_play", {"timeout_seconds": a.remaining()})
            flooders = []
            flood_evidence: list[dict[str, Any]] = []
            for name in names[2:]:
                owner = peer_context(name)
                process, mcp = launch(owner, name, display=None)
                require_tools(mcp.list_tools(), REQUIRED_TOOLS, "join-lag-gate")
                probe = ProbeSteps(mcp, artifact / name, timeout, f"native-{name.lower()}")
                probe.__enter__()
                probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
                flooders.append((process, probe))
                flood_evidence.append(dict(owner.client_evidence))
            detail, verdicts = _gate(a, flooders, ctx, artifact, tick_rate, scenario, flood_steps,
                                     address, server_log_path, dashboard_port,
                                     lambda: launch(peer_context(JOINER), JOINER, display=None))
            detail["pump_publish_per_second"] = pump_rate
            a.report["details"] = detail
            evidence["flood_client"] = flood_evidence
            outcome, failed = disposition(verdicts)
            reasons = detail.get("verdicts", {})
            if outcome == "failed":
                for name in failed:
                    a.check(name, verdicts[name] is True)
            if outcome == "inconclusive":
                raise RuntimeError("join-lag-gate inconclusive: " + "; ".join(
                    f"{name}: {reasons.get(name, 'condition not reached')}" for name in failed))
        for process in (process_a,):
            rss = peak_rss_kib(process.pid)
            if rss is not None:
                evidence.setdefault("peak_client_rss_kib", rss)
        return {"scenario": a.report, "verdicts": dict(verdicts)}
    finally:
        for process, mcp in clients:
            try:
                mcp.close()
            except Exception:
                pass
            runtime.stop_process(process)
        runtime.stop_process(server, interrupt=True)
        runtime.stop_process(xvfb)
        if server_log is not None:
            server_log.close()
        if previous_xdg is None:
            os.environ.pop("XDG_DATA_HOME", None)
        else:
            os.environ["XDG_DATA_HOME"] = previous_xdg


def _gate(a: ProbeSteps, flooders: list[tuple[subprocess.Popen[Any], ProbeSteps]],
          ctx: ProfileContext, artifact: Path, tick_rate: int, scenario: str, flood_steps: int,
          address: str, server_log_path: Path, dashboard_port: int,
          launch_joiner: Any) -> tuple[dict[str, Any], dict[str, bool | None]]:
    """The whole gate: fixtures and the border reveal, then the flood and a fresh join."""
    verdicts: dict[str, bool | None] = {}
    detail: dict[str, Any] = {"tick_rate": tick_rate, "scenario": scenario,
                              "flood_clients": len(flooders), "flood_steps": flood_steps}
    a.report["details"] = detail  # kept even when a later step fails
    rooted = a.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float), "player_present")
    ox, oy, oz = (int((rooted["player"] or {})[axis]) for axis in ("x", "y", "z"))
    a.report["origin_block"] = [ox, oy, oz]
    plan = border_layout(ox, oy, oz)
    a.report["layout"] = {name: list(value) for name, value in plan.items()}

    def at(name: str) -> dict[str, int]:
        x, y, z = plan[name]
        return {"x": x, "y": y, "z": z}

    def read(probe: ProbeSteps, name: str) -> dict[str, Any]:
        return probe.call("minecraft_read_block", at(name))

    def loaded(probe: ProbeSteps, name: str) -> None:
        probe.call("minecraft_wait_for_loaded_block", {**at(name), "timeout_seconds": probe.remaining()})

    def wait_skips(skips_before: int, budget_seconds: float = SKIP_WAIT_SECONDS) -> int:
        """Skips so far; the pump must actually fall behind for any verdict to mean anything."""
        deadline = time.monotonic() + budget_seconds
        while time.monotonic() < deadline:
            current = _skips(_stat(dashboard_port))
            if current > skips_before:
                return current
            time.sleep(0.2)
        return _skips(_stat(dashboard_port))

    a.fixture(f"tick rate {tick_rate}")
    a.fixture("gamemode creative @s")
    for command in _pad_commands(ox, oy, oz):
        a.fixture(command)
    x0, y0, z0, x1, y1, z1 = plan["fill"]
    a.fixture(f"fill {x0} {y0} {z0} {x1} {y1} {z1} {STONE}")
    ox_ore, oy_ore, oz_ore = plan["ore"]
    a.fixture(f"setblock {ox_ore} {oy_ore} {oz_ore} {DIAMOND}")
    sx, sy, sz = plan["side"]
    a.fixture(f"setblock {sx} {sy} {sz} {STONE}")

    # Block updates carry the real state (the anti-xray fixture rule): only a fresh encode masks the
    # ore, so the witness reconnects after the fixture and the server encodes these chunks for it.
    a.call("minecraft_disconnect")
    a.check("disconnect_left_play", a.call("minecraft_observe").get("in_play") is False)
    a.call("minecraft_connect", {"server_addr": address})
    rejoined = a.call("minecraft_wait_for_play", {"timeout_seconds": a.remaining()})
    a.check("reconnect_entered_play", rejoined.get("in_play") is True)
    loaded(a, "ore")
    before = read(a, "ore")
    detail["client_read_ore_before"] = before
    a.report["assertions"]["border_ore_is_masked_before"] = before.get("block_id") == STONE
    if before.get("block_id") != STONE:
        raise RuntimeError("join-lag-gate: the border ore is not masked before the flood; "
                           "anti-xray must be on (playable.toml defaults) for this gate")

    def flood(probe: ProbeSteps, index: int, steps: int) -> None:
        """Keep one flood client's whole view dirty: every publish to it re-encodes 81 chunks."""
        cx, cy, cz = ox + FLOOD_DX * (index + 1), oy + 40, oz
        probe.fixture(f"tp @s {cx} {cy} {cz}")
        for step in range(1, steps + 1):
            block = STONE if step % 2 else AIR
            probe.fixture(f"fill {cx - FLOOD_RADIUS} {cy} {cz - FLOOD_RADIUS} "
                          f"{cx + FLOOD_RADIUS} {cy} {cz + FLOOD_RADIUS} {block}")
            time.sleep(FILL_PAUSE)

    skips_before = _skips(_stat(dashboard_port))
    threads = [threading.Thread(target=flood, args=(probe, index, flood_steps), daemon=True)
               for index, (_process, probe) in enumerate(flooders)]
    for thread in threads:
        thread.start()
    if scenario in ("all", "border-reveal"):
        skips_mid = wait_skips(skips_before)
        detail["border_skips_while_flooding"] = [skips_before, skips_mid]
        # The side cell changes while the flanking publishes are skipped: the ore must be revealed.
        a.fixture(f"setblock {sx} {sy} {sz} {AIR}")
        revealed: dict[str, Any] | None = None
        reveal_deadline = time.monotonic() + 20.0
        while time.monotonic() < reveal_deadline:
            block = read(a, "ore")
            if block.get("block_id") == DIAMOND:
                revealed = block
                break
            state = a.observe()
            try:
                a.call("minecraft_wait_for_state_change",
                       {"observed_version": state["state_version"],
                        "timeout_seconds": min(2.0, a.remaining())})
            except RuntimeError as error:
                # An idle client reports "no state change"; the deadline above owns the verdict.
                if CLIENT_STATE_EVENT_EXPIRED not in str(error):
                    raise
        skips_after = _skips(_stat(dashboard_port))
        detail["border_skips"] = [skips_before, skips_after]
        detail["client_read_ore_after"] = revealed or read(a, "ore")
        ok, why = border_verdict((revealed or {}).get("block_id"), skips_before, skips_after)
        detail["border_verdict"] = why
        detail.setdefault("verdicts", {})["border_reveal_after_skipped_publish"] = why
        verdicts["border_reveal_after_skipped_publish"] = ok
        if ctx.windowed:
            native_client.capture(a.client, artifact, "join-lag-border-ore")

    if scenario in ("all", "join-stall"):
        detail["join_skips_before"] = _skips(_stat(dashboard_port))
        tick_before = _tick(_stat(dashboard_port))
        # A fresh client joins while the pump is behind: its `Joined` is a reliable change.
        joiner_process, joiner_mcp = launch_joiner()
        elapsed: float | None = None
        try:
            joiner = ProbeSteps(joiner_mcp, artifact / "join-lag-joiner",
                                max(60.0, a.deadline - time.monotonic()), "native-join-lag-joiner")
            joiner.__enter__()
            joiner.call("minecraft_wait_for_play", {"timeout_seconds": JOIN_BOUND_SECONDS})
            in_play = joiner.call("minecraft_observe").get("in_play") is True
            joiner.check("joiner_entered_play", in_play)
        except Exception as error:
            detail["join_error"] = str(error)
            runtime.tag_failure(error, "join")
        elapsed = join_timing(server_log_path, JOINER)
        detail["join_elapsed_seconds"] = None if elapsed is None else round(elapsed, 2)
        for thread in threads:
            thread.join(timeout=5)
        time.sleep(0.5)
        skips_after = _skips(_stat(dashboard_port))
        tick_after = _tick(_stat(dashboard_port))
        halt = halt_line(server_log_path)
        stats_end = _stat(dashboard_port)
        detail["join_skips"] = [detail["join_skips_before"], skips_after]
        detail["join_ticks"] = [tick_before, tick_after]
        detail["publish"] = stats_end.get("publish")
        detail["tps_at_end"] = stats_end.get("tps")
        detail["halt_line"] = halt
        ok, why = join_verdict(elapsed, detail["join_skips_before"], skips_after, halt)
        detail["join_verdict"] = why
        detail.setdefault("verdicts", {})["join_behind_slow_pump"] = why
        verdicts["join_behind_slow_pump"] = ok
        ok2, why2 = halt_verdict(detail["join_skips_before"], skips_after, tick_before, tick_after, halt)
        detail["halt_verdict"] = why2
        detail.setdefault("verdicts", {})["live_pump_does_not_halt"] = why2
        verdicts["live_pump_does_not_halt"] = ok2
        if ctx.windowed:
            native_client.capture(a.client, artifact, "join-lag-after-join")
    for thread in threads:
        thread.join(timeout=5)
    if ctx.windowed:
        native_client.capture(a.client, artifact, "join-lag-final")
    return detail, verdicts
