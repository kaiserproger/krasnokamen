"""Join-lag gate: the edge pump behind the tick, on real native clients.

Rows (docs/bugs/rows):
- EDGE-SLOW-PUMP-HOLDS-JOIN-BEHIND-STALE-OUTPUTS: a pump slower than the tick
  sits at the oldest retained broadcast output, so a `Joined` waits a whole
  window of tick periods and times out at the 10 s join deadline.
- EDGE-OUTBOX-STALL-WATCH-HALTS-LIVE-LAGGING-PUMP: the 10 s reliable outbox
  stall watch halts the core under a live pump that only lags.

The condition is the pump publishing fewer outputs than the tick:
`KRASNOKAMEN_PUMP_PUBLISH_PER_SECOND` (a test/dev knob, unset or 0 = unlimited)
paces the pump, and the gate sets it to 3 at the default 20 TPS. The dashboard
is the sensor: the pump lag is `simulation_tick - publish.source_tick`, measured
in both trees, so a verdict never needs the fixed behaviour's own skip counter.

Criteria:

- `join_behind_slow_pump`: with the pump behind by at least `LAG_TICKS`, a fresh
  client's join has to land inside the 10 s deadline. The interval is the
  server's own `login admitted` -> `join landed` (`RUST_LOG` sets the pump to
  debug for that line), never the client's process startup.
- `live_pump_does_not_halt`: while the pump lags, `/stats` keeps advancing its
  simulation tick and the server log carries no `edge pump not draining`.

A verdict whose condition never occurred is `null` (inconclusive: the run ends
failed with that reason), never `false` — `ledger._failing_verdict` reads
`false` as the row's symptom, so an unreached condition must not look like a red
control.

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
from .gameplay import peak_rss_kib, prepare_operator_fixture, require_tools
from .probe_steps import ProbeSteps

WITNESS = "JoinLagWitness"
FLOOD = "JoinLagFlood"
JOINER = "JoinLagJoiner"
TIMEOUT = 1800.0
JOIN_BOUND_SECONDS = 10.0  # the server's join deadline the row names
LAG_TICKS = 8              # the pump must be at least this many ticks behind
LAG_WAIT_SECONDS = 20.0    # how long the gate waits for the pump to fall behind
LAG_SAMPLE_SECONDS = 0.1
FLOOD_DX = 512             # a flood client's area sits far outside the witness
FLOOD_RADIUS = 72          # a 145x145 slab covers the whole 9x9 chunk view
FILL_PAUSE = 0.35          # pacing between a flood client's fills
CRITERIA = ("join_behind_slow_pump", "live_pump_does_not_halt")

REQUIRED_TOOLS = frozenset({
    "minecraft_observe",
    "minecraft_wait_for_play",
    "minecraft_send_chat",
})


def join_verdict(elapsed: float | None, lag_max: int, halt: str | None) -> tuple[bool | None, str]:
    """Row 1 verdict: None when the pump never fell behind, False only for a real symptom."""
    if halt:
        return False, f"the core halted during the lag: {halt}"
    if lag_max < LAG_TICKS:
        return None, f"the pump was never {LAG_TICKS} ticks behind (max lag {lag_max}): the condition did not occur"
    if elapsed is None:
        return False, f"the join never landed with the pump {lag_max} ticks behind"
    if elapsed > JOIN_BOUND_SECONDS:
        return False, (f"the join landed after {elapsed:.1f} s with the pump {lag_max} ticks behind, "
                       f"past the {JOIN_BOUND_SECONDS:.0f} s deadline")
    return True, f"the join landed in {elapsed:.1f} s with the pump {lag_max} ticks behind"


def halt_verdict(lag_max: int, tick_before: int, tick_after: int,
                 halt: str | None) -> tuple[bool | None, str]:
    """Row 2 verdict: None when the pump never fell behind, False only for a real symptom."""
    if halt:
        return False, f"the core halted under a live lagging pump: {halt}"
    if lag_max < LAG_TICKS:
        return None, f"the pump was never {LAG_TICKS} ticks behind (max lag {lag_max}): the condition did not occur"
    if tick_after <= tick_before:
        return False, f"the simulation tick did not advance ({tick_before} -> {tick_after})"
    return True, (f"the core kept ticking {tick_before} -> {tick_after} through a lag of "
                  f"{lag_max} ticks")


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


def pump_lag(stats: dict[str, Any]) -> int:
    """How many ticks the pump's last publication is behind the core (pure helper).

    No publication sample yet (or a publish ahead of the core's rollup) means the lag is unknown,
    reported as zero so it can never stand in for the condition.
    """
    publish = stats.get("publish") or {}
    if publish.get("source_tick") is None:
        return 0
    return max(0, int(stats.get("simulation_tick") or 0) - int(publish["source_tick"]))


def _lag(dashboard_port: int) -> int:
    return pump_lag(_stat(dashboard_port))


def _tick(stats: dict[str, Any]) -> int:
    return int(stats.get("simulation_tick") or 0)


def validate_args(ctx: ProfileContext) -> tuple[float, int, int, int, int]:
    if ctx.client != "native":
        raise ValueError("join-lag-gate requires --client native")
    if ctx.platform is not None:
        raise ValueError("join-lag-gate accepts only --client, --windowed, --timeout-seconds and its own args")
    parser = argparse.ArgumentParser(prog="join-lag-gate")
    # 20 TPS is the row's own rate: the pump must outpace the tick for the queue to build in seconds.
    parser.add_argument("--tick-rate", type=int, default=20)
    parser.add_argument("--flood-clients", type=int, default=1)
    parser.add_argument("--flood-steps", type=int, default=40,
                        help="cap on view-wide fills per flood client while the pump falls behind")
    # The condition lever: the server publishes at most this many outputs per second for the run
    # (KRASNOKAMEN_PUMP_PUBLISH_PER_SECOND, a test/dev knob, never an operator setting).
    parser.add_argument("--pump-publish-per-second", type=int, default=3)
    args = parser.parse_args(ctx.extra_args)
    if not 1 <= args.tick_rate <= 20:
        raise ValueError("join-lag-gate --tick-rate must be in [1, 20]")
    if not 0 <= args.flood_clients <= 4:
        raise ValueError("join-lag-gate --flood-clients must be in [0, 4]")
    if not 4 <= args.flood_steps <= 128:
        raise ValueError("join-lag-gate --flood-steps must be in [4, 128]")
    if not 1 <= args.pump_publish_per_second <= 100:
        raise ValueError("join-lag-gate --pump-publish-per-second must be in [1, 100]")
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else TIMEOUT
    if not 0 < timeout <= 2400:
        raise ValueError("join-lag-gate --timeout-seconds must be in (0, 2400]")
    return (timeout, args.tick_rate, args.flood_clients, args.flood_steps,
            args.pump_publish_per_second)


def run(ctx: ProfileContext) -> dict[str, Any]:
    timeout, tick_rate, flood_clients, flood_steps, pump_rate = validate_args(ctx)
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
    source = (runtime.REPO_ROOT / "playable.toml").read_text()
    for old, new in (
        ("port = 25565", f"port = {server_port}"),
        ('world_dir = ".analysis/test-world-v12"',
         f'world_dir = "{(artifact / "world").relative_to(runtime.REPO_ROOT).as_posix()}"'),
        ('directory = "plugins"', f'directory = {json.dumps(str(artifact / "plugins"))}'),
        ("seed = 0", "seed = 81"),
    ):
        if source.count(old) != 1:
            raise RuntimeError(f"expected one join-lag-gate config token: {old}")
        source = source.replace(old, new, 1)
    # The dashboard is read-only and loopback-only; the gate reads its tick and publish counters.
    source += f'\n[dashboard]\nenabled = true\nbind_address = "127.0.0.1"\nport = {dashboard_port}\n'
    config.write_text(source)
    names = [WITNESS, JOINER] + [f"{FLOOD}{index}" for index in range(1, flood_clients + 1)]
    prepare_operator_fixture(ctx, names)
    evidence = ctx.client_evidence
    with binary.open("rb") as server_executable:
        server_hash = hashlib.file_digest(server_executable, "sha256").hexdigest()
    evidence.update(server_address=address, server_binary_path=str(binary),
                    server_binary_sha256=server_hash, world_dir=str(artifact / "world"),
                    seed=81, dashboard_address=f"127.0.0.1:{dashboard_port}",
                    flood_clients=names[2:], pump_publish_per_second=pump_rate)
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

        def launch(owner: ProfileContext, username: str, *,
                   display: str | None) -> tuple[subprocess.Popen[Any], Any]:
            process, _log, mcp = native_client.start(
                owner, native_client.resolve_binary(owner), address, username, timeout, display=display)
            mcp.request_timeout_seconds = min(timeout + 10, 300.0)
            clients.append((process, mcp))
            return process, mcp

        def peer_context(name: str) -> ProfileContext:
            return replace(ctx, artifact_dir=artifact / name, client_evidence={}, windowed=False)

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
            detail, verdicts = _gate(a, flooders, ctx, artifact, tick_rate, flood_steps,
                                     address, server_log_path, dashboard_port,
                                     lambda: launch(peer_context(JOINER), JOINER, display=None))
            detail["pump_publish_per_second"] = pump_rate
            a.report["details"] = detail
            evidence["flood_client"] = flood_evidence
            outcome, names_out = disposition(verdicts)
            reasons = detail.get("verdicts", {})
            if outcome == "failed":
                for name in names_out:
                    a.check(name, verdicts[name] is True)
            if outcome == "inconclusive":
                raise RuntimeError("join-lag-gate inconclusive: " + "; ".join(
                    f"{name}: {reasons.get(name, 'condition not reached')}" for name in names_out))
        rss = peak_rss_kib(process_a.pid)
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
          ctx: ProfileContext, artifact: Path, tick_rate: int, flood_steps: int,
          address: str, server_log_path: Path, dashboard_port: int,
          launch_joiner: Any) -> tuple[dict[str, Any], dict[str, bool | None]]:
    """The walk: the pump falls behind under a paced publish rate, then a fresh client joins."""
    verdicts: dict[str, bool | None] = {}
    detail: dict[str, Any] = {"tick_rate": tick_rate, "flood_clients": len(flooders),
                              "flood_steps": flood_steps}
    a.report["details"] = detail  # kept even when a later step fails
    a.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float), "player_present")
    a.fixture(f"tick rate {tick_rate}")
    a.fixture("gamemode creative @s")

    def flood(probe: ProbeSteps, index: int, steps: int) -> None:
        """Keep one flood client's whole view dirty: each publish to it re-encodes 81 chunks."""
        cx, cy, cz = (a_pos[0] + FLOOD_DX * (index + 1), a_pos[1] + 40, a_pos[2])
        probe.fixture(f"tp @s {cx} {cy} {cz}")
        for step in range(1, steps + 1):
            block = "minecraft:stone" if step % 2 else "minecraft:air"
            probe.fixture(f"fill {cx - FLOOD_RADIUS} {cy} {cz - FLOOD_RADIUS} "
                          f"{cx + FLOOD_RADIUS} {cy} {cz + FLOOD_RADIUS} {block}")
            time.sleep(FILL_PAUSE)

    state = a.call("minecraft_observe")
    player = state.get("player") or {}
    a_pos = (int(player.get("x") or 0), int(player.get("y") or 0), int(player.get("z") or 0))
    a.report["origin_block"] = list(a_pos)

    samples: list[int] = []
    stop = threading.Event()

    def sample() -> None:
        while not stop.is_set():
            try:
                samples.append(_lag(dashboard_port))
            except OSError:
                pass
            time.sleep(LAG_SAMPLE_SECONDS)

    threads = [threading.Thread(target=flood, args=(probe, index, flood_steps), daemon=True)
               for index, (_process, probe) in enumerate(flooders)]
    for thread in threads:
        thread.start()
    sampler = threading.Thread(target=sample, daemon=True)
    sampler.start()
    # The condition: the pump has to be clearly behind before the join is queued.
    lag_deadline = time.monotonic() + LAG_WAIT_SECONDS
    while time.monotonic() < lag_deadline and not any(lag >= LAG_TICKS for lag in samples):
        time.sleep(0.1)
    detail["lag_at_join"] = max(samples) if samples else 0
    tick_before = _tick(_stat(dashboard_port))

    joiner_process, joiner_mcp = launch_joiner()
    elapsed: float | None = None
    try:
        joiner = ProbeSteps(joiner_mcp, artifact / "join-lag-joiner",
                            max(60.0, a.deadline - time.monotonic()), "native-join-lag-joiner")
        joiner.__enter__()
        joiner.call("minecraft_wait_for_play", {"timeout_seconds": JOIN_BOUND_SECONDS})
        joiner.check("joiner_entered_play", joiner.call("minecraft_observe").get("in_play") is True)
    except Exception as error:
        # A kicked or stalled join is the symptom, not an infrastructure failure.
        detail["join_error"] = str(error)
    elapsed = join_timing(server_log_path, JOINER)
    detail["join_elapsed_seconds"] = None if elapsed is None else round(elapsed, 2)
    for thread in threads:
        thread.join(timeout=5)
    time.sleep(0.5)
    stop.set()
    sampler.join(timeout=2)
    lag_max = max(samples) if samples else 0
    tick_after = _tick(_stat(dashboard_port))
    halt = halt_line(server_log_path)
    stats_end = _stat(dashboard_port)
    detail["lag_max"] = lag_max
    detail["lag_samples"] = len(samples)
    detail["join_ticks"] = [tick_before, tick_after]
    detail["publish"] = stats_end.get("publish")
    detail["tps_at_end"] = stats_end.get("tps")
    detail["halt_line"] = halt
    ok, why = join_verdict(elapsed, lag_max, halt)
    detail["join_verdict"] = why
    detail.setdefault("verdicts", {})["join_behind_slow_pump"] = why
    verdicts["join_behind_slow_pump"] = ok
    ok2, why2 = halt_verdict(lag_max, tick_before, tick_after, halt)
    detail["halt_verdict"] = why2
    detail.setdefault("verdicts", {})["live_pump_does_not_halt"] = why2
    verdicts["live_pump_does_not_halt"] = ok2
    if ctx.windowed:
        native_client.capture(a.client, artifact, "join-lag-after-join")
        native_client.capture(a.client, artifact, "join-lag-final")
    return detail, verdicts
