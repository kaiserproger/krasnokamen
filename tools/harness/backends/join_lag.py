"""Join-lag gate: the edge pump behind the tick, on real native clients.

Rows (docs/bugs/rows):
- EDGE-SLOW-PUMP-HOLDS-JOIN-BEHIND-STALE-OUTPUTS: a pump slower than the tick
  sits at the oldest retained broadcast output, so a `Joined` waits a whole
  window of tick periods and times out at the 10 s join deadline.
- EDGE-OUTBOX-STALL-WATCH-HALTS-LIVE-LAGGING-PUMP: the 10 s reliable outbox
  stall watch halts the core under a live pump that only lags.

The condition is a pump behind the tick: `KRASNOKAMEN_PUMP_LAG_TICKS=K` (a
test/dev knob, unset or 0 = today's pump) is a delay line in front of the pump —
output n is processed only after n+K arrived, no clock read. The gate sets K=8 at
`/tick rate 3`: about 2.7 s per publication — past the 1.6 s publish budget so
the skip applies, under the 10 s outbox watch so the fixed pump keeps draining,
and the pre-fix queue still grows to the whole broadcast window. The gate reads its own engagement from the server's
one warn line, so an unengaged run is inconclusive instead of a guess.

Criteria:

- `join_behind_slow_pump`: with the delay line engaged, a fresh
  client's join has to land inside the 10 s deadline. The interval is the
  server's own `login admitted` -> `join landed` (`RUST_LOG` sets the pump to
  debug for that line), never the client's process startup.
- `live_pump_does_not_halt`: with the delay line engaged, `/stats` keeps
  advancing its simulation tick and the server log carries no
  `edge pump not draining`.

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
JOINER = "JoinLagJoiner"
TIMEOUT = 1800.0
JOIN_BOUND_SECONDS = 10.0  # the server's join deadline the row names
ENGAGED_LINE = "pump lag enabled"  # the server's warn line for the test/dev knob
CRITERIA = ("join_behind_slow_pump", "live_pump_does_not_halt")

REQUIRED_TOOLS = frozenset({
    "minecraft_observe",
    "minecraft_wait_for_play",
    "minecraft_send_chat",
})


def _int(pattern: str, text: str) -> int | None:
    match = re.search(pattern, text)
    return int(match.group(1)) if match else None


def join_record(log_path: Path, username: str) -> dict[str, Any]:
    """What the server log says about one client's join (pure helper).

    `admitted` is the server's own admission line; `landed_seconds` the interval to its
    `join landed`; `timed_out` the server's deadline kick with the ticks it names; `lag` the pump's
    distance behind the core at that moment (`core_tick_seen - pump_tick`), which is the witness
    that the pump really was delayed — the knob's warn line alone proves nothing.
    """
    record: dict[str, Any] = {"admitted": False, "landed_seconds": None, "timed_out": False,
                              "lag": None, "knockout": None}
    try:
        text = log_path.read_text(errors="replace")
    except OSError:
        return record

    def stamp(line: str) -> datetime | None:
        head = line.split(" ", 1)[0]
        try:
            return datetime.fromisoformat(head.replace("Z", "+00:00"))
        except ValueError:
            return None

    admitted_at: datetime | None = None
    connection: str | None = None
    for line in text.splitlines():
        if "login admitted" in line and f"name={username}" in line:
            match = re.search(r"connection=(\d+)", line)
            if match:
                connection = match.group(1)
                admitted_at = stamp(line)
                record["admitted"] = True
            continue
        if connection is None or not re.search(rf"\bconnection\s*=\s*{connection}\b", line):
            continue
        if "join landed" in line:
            landed = stamp(line)
            if landed is not None and admitted_at is not None:
                record["landed_seconds"] = (landed - admitted_at).total_seconds()
            core = _int(r"core_tick_seen=Some\((\d+)\)", line)
            pump = _int(r"pump_tick=(\d+)", line)
            if core is not None and pump is not None:
                record["lag"] = max(0, core - pump)
            record["knockout"] = "landed"
            continue
        if "timed out waiting for join" in line:
            record["timed_out"] = True
            record["knockout"] = "timeout"
            now = _int(r"\b(\d+) now\b", line)
            pump = _int(r"pump published tick (\d+)", line)
            if now is not None and pump is not None:
                record["lag"] = max(0, now - pump)
    return record


def precondition(record: dict[str, Any], knob_on: bool) -> tuple[bool, str]:
    """The gate's causal precondition: the knob ran, the server admitted the client, and the log
    witnessed the pump actually delayed (`lag` = core tick minus pump tick). A missing piece is
    inconclusive, never a symptom."""
    if not knob_on:
        return False, "the pump lag knob never ran: the lagging condition did not occur"
    if not record.get("admitted"):
        return False, "the joining client never reached the server (no admission line): infrastructure, not a verdict"
    lag = record.get("lag")
    if lag is None or lag < 1:
        return False, "no pump delay was witnessed at the join (pump tick equals the core tick)"
    return True, f"the pump {lag} ticks behind"


def join_verdict(record: dict[str, Any], knob_on: bool, halt: str | None) -> tuple[bool | None, str]:
    """Row 1 verdict: False only for a held join behind a witnessed delay; otherwise True when the
    join landed in time; everything unreached is None."""
    witnessed, info = precondition(record, knob_on)
    if not witnessed:
        return None, info
    if halt:
        return False, f"the core halted during the delayed pump run: {halt}"
    landing = record.get("landed_seconds")
    if landing is not None:
        if float(landing) > JOIN_BOUND_SECONDS:
            return False, (f"the join landed after {float(landing):.1f} s with {info}, "
                           f"past the {JOIN_BOUND_SECONDS:.0f} s deadline")
        return True, f"the join landed in {float(landing):.1f} s with {info}"
    if record.get("timed_out"):
        return False, f"the join missed the {JOIN_BOUND_SECONDS:.0f} s deadline with {info}"
    return None, "the join neither landed nor timed out inside the run: no verdict"


def halt_verdict(knob_on: bool, record: dict[str, Any], tick_before: int, tick_after: int,
                 halt: str | None) -> tuple[bool | None, str]:
    """Row 2 verdict: False only for a halt or a stuck core behind the same witnessed delay."""
    witnessed, info = precondition(record, knob_on)
    if not witnessed:
        return None, info
    if halt:
        return False, f"the core halted under a live lagging pump: {halt}"
    if tick_after <= tick_before:
        return False, f"the simulation tick did not advance ({tick_before} -> {tick_after})"
    return True, f"the core kept ticking {tick_before} -> {tick_after} with {info}"


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


def engaged(log_path: Path) -> bool:
    """Whether the server's warn line says the pump delay line ran (pure helper)."""
    try:
        return ENGAGED_LINE in log_path.read_text(errors="replace")
    except OSError:
        return False


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


def _tick(stats: dict[str, Any]) -> int:
    return int(stats.get("simulation_tick") or 0)


def _safe_tick(dashboard_port: int) -> int:
    """The core tick, or 0 when the dashboard is not answering (a halt ends the process)."""
    try:
        return _tick(_stat(dashboard_port))
    except OSError:
        return 0


def validate_args(ctx: ProfileContext) -> tuple[float, int, int]:
    if ctx.client != "native":
        raise ValueError("join-lag-gate requires --client native")
    if ctx.platform is not None:
        raise ValueError("join-lag-gate accepts only --client, --windowed, --timeout-seconds and its own args")
    parser = argparse.ArgumentParser(prog="join-lag-gate")
    # 20 TPS is the row's own rate: the pump must outpace the tick for the queue to build in seconds.
    parser.add_argument("--tick-rate", type=int, default=3)
    # The condition lever: the pump is delayed by K core ticks (KRASNOKAMEN_PUMP_LAG_TICKS, a
    # test/dev knob, never an operator setting).
    parser.add_argument("--pump-lag-ticks", type=int, default=8)
    args = parser.parse_args(ctx.extra_args)
    if not 1 <= args.tick_rate <= 20:
        raise ValueError("join-lag-gate --tick-rate must be in [1, 20]")
    if not 1 <= args.pump_lag_ticks <= 64:
        raise ValueError("join-lag-gate --pump-lag-ticks must be in [1, 64]")
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else TIMEOUT
    if not 0 < timeout <= 2400:
        raise ValueError("join-lag-gate --timeout-seconds must be in (0, 2400]")
    return timeout, args.tick_rate, args.pump_lag_ticks


def run(ctx: ProfileContext) -> dict[str, Any]:
    timeout, tick_rate, lag_ticks = validate_args(ctx)
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
    names = [WITNESS, JOINER]
    prepare_operator_fixture(ctx, names)
    evidence = ctx.client_evidence
    with binary.open("rb") as server_executable:
        server_hash = hashlib.file_digest(server_executable, "sha256").hexdigest()
    evidence.update(server_address=address, server_binary_path=str(binary),
                    server_binary_sha256=server_hash, world_dir=str(artifact / "world"),
                    seed=81, dashboard_address=f"127.0.0.1:{dashboard_port}",
                    pump_lag_ticks=lag_ticks)
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
        env["KRASNOKAMEN_PUMP_LAG_TICKS"] = str(lag_ticks)
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
            detail, verdicts = _gate(a, ctx, artifact, tick_rate, address, server_log_path,
                                     dashboard_port, server,
                                     lambda: launch(peer_context(JOINER), JOINER, display=None))
            detail["pump_lag_ticks"] = lag_ticks
            a.report["details"] = detail
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


def _gate(a: ProbeSteps, ctx: ProfileContext, artifact: Path, tick_rate: int, address: str,
          server_log_path: Path, dashboard_port: int, server: subprocess.Popen[Any],
          launch_joiner: Any) -> tuple[dict[str, Any], dict[str, bool | None]]:
    """The walk: the pump runs behind the tick, then a fresh client joins."""
    verdicts: dict[str, bool | None] = {}
    detail: dict[str, Any] = {"tick_rate": tick_rate}
    a.report["details"] = detail  # kept even when a later step fails
    a.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float), "player_present")
    a.fixture(f"tick rate {tick_rate}")
    a.fixture("gamemode creative @s")
    # The knob's own warn line is written before the server listens; wait for it with inotify so a
    # late flush is an event, not a sleep.
    delayed = runtime.wait_for_path_condition(
        server_log_path, 10.0, lambda: (engaged(server_log_path), engaged(server_log_path)), server)
    detail["pump_lag_enabled"] = delayed
    tick_before = _safe_tick(dashboard_port)

    joiner_process, joiner_mcp = launch_joiner()
    try:
        joiner = ProbeSteps(joiner_mcp, artifact / "join-lag-joiner",
                            max(60.0, a.deadline - time.monotonic()), "native-join-lag-joiner")
        joiner.__enter__()
        joiner.call("minecraft_wait_for_play", {"timeout_seconds": JOIN_BOUND_SECONDS})
        joiner.check("joiner_entered_play", joiner.call("minecraft_observe").get("in_play") is True)
    except Exception as error:
        # A kicked or stalled join is the symptom, not an infrastructure failure.
        detail["join_error"] = str(error)
    def settled() -> tuple[bool, dict[str, Any]]:
        current = join_record(server_log_path, JOINER)
        return current.get("knockout") is not None or current.get("timed_out") is True, current

    try:
        # The server's own kick line may be written just after the client's tool deadline: wait for
        # the log event (inotify) with the server pidfd, then judge.
        record = runtime.wait_for_path_condition(server_log_path, JOIN_BOUND_SECONDS + 15.0,
                                                 settled, server)
    except (TimeoutError, runtime.ChildExited):
        record = join_record(server_log_path, JOINER)
    detail["join"] = record
    detail["join_elapsed_seconds"] = (None if record.get("landed_seconds") is None
                                      else round(float(record["landed_seconds"]), 2))
    tick_after = _safe_tick(dashboard_port)
    halt = halt_line(server_log_path)
    stats_end = None
    try:
        stats_end = _stat(dashboard_port)
    except OSError:
        pass
    detail["join_ticks"] = [tick_before, tick_after]
    detail["publish"] = (stats_end or {}).get("publish")
    detail["tps_at_end"] = (stats_end or {}).get("tps")
    detail["halt_line"] = halt
    ok, why = join_verdict(record, delayed, halt)
    detail["join_verdict"] = why
    detail.setdefault("verdicts", {})["join_behind_slow_pump"] = why
    verdicts["join_behind_slow_pump"] = ok
    ok2, why2 = halt_verdict(delayed, record, tick_before, tick_after, halt)
    detail["halt_verdict"] = why2
    detail.setdefault("verdicts", {})["live_pump_does_not_halt"] = why2
    verdicts["live_pump_does_not_halt"] = ok2
    if ctx.windowed:
        native_client.capture(a.client, artifact, "join-lag-after-join")
        native_client.capture(a.client, artifact, "join-lag-final")
    return detail, verdicts
