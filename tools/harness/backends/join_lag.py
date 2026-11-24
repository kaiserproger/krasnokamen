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

Walk order (review REVIEW-wave2-2 B1/N3): the joiner is launched right after `/tick rate` and
before the `/tick query` reply is awaited, because at 3 TPS that reply travels through the same
lagging pump — awaiting it first spent the whole join deadline before any join was attempted, so
the pre-fix tree's control recorded no join outcome at all. The rate confirmation is best effort
(a reply the pump held past `RATE_WAIT_SECONDS` leaves the interval to the joiner record's own
ticks: `join_record` reads the core ticks the server itself printed around the wait). Both verdicts
are read from the joiner's record; a halt that disconnects the witness after `player_present` is
recorded in `details`, never as the client's `in_play` check (the ledger refuses a receipt whose
client never reached play, which is exactly the halt's own disconnect).

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
import math
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
JOIN_BOUND_SECONDS = 10.0      # the server's join deadline the row names
JOIN_RECORD_SECONDS = JOIN_BOUND_SECONDS + 15.0  # the server's own kick/landing line lands late
RATE_WAIT_SECONDS = 12.0       # `/tick query`'s reply shares the pump queue it describes
PUBLISH_BUDGET_SECONDS = 1.6
OUTBOX_STALL_SECONDS = 10.0     # the reliable outbox stall watch row 2 names
HALT_WINDOW_MARGIN_SECONDS = 2.0  # the verdict needs the whole stall window, not a fragment of it
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
    that the pump really was delayed — the knob's warn line alone proves nothing. `rate` is the
    core rate the server itself printed around this client's wait: the ticks it advanced divided by
    the wait's own seconds, so a join whose `/tick query` reply the pump held still carries the
    interval the row is about.
    """
    record: dict[str, Any] = {"admitted": False, "landed_seconds": None, "timed_out": False,
                              "lag": None, "rate": None, "knockout": None, "landed_at": None}
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
                record["landed_at"] = landed
            core = _int(r"core_tick_seen=Some\((\d+)\)", line)
            pump = _int(r"pump_tick=(\d+)", line)
            if core is not None and pump is not None:
                record["lag"] = max(0, core - pump)
            rate = _wait_rate(line, r"core_tick_seen_at_wait=Some\((\d+)\)",
                              r"core_tick_seen=Some\((\d+)\)", record.get("landed_seconds"))
            if rate is not None:
                record["rate"] = rate
            record["knockout"] = "landed"
            continue
        if "timed out waiting for join" in line:
            record["timed_out"] = True
            record["knockout"] = "timeout"
            now = _int(r"\b(\d+) now\b", line)
            pump = _int(r"pump published tick (\d+)", line)
            if now is not None and pump is not None:
                record["lag"] = max(0, now - pump)
            rate = _wait_rate(line, r"core tick (\d+) seen when the wait began",
                              r"\b(\d+) now\b", JOIN_BOUND_SECONDS)
            if rate is not None:
                record["rate"] = rate
    return record


def _wait_rate(line: str, start: str, end: str, seconds: float | None) -> float | None:
    """The core rate over one join wait: ticks between its two printed marks, over its seconds."""
    if seconds is None or seconds <= 0:
        return None
    first = re.search(start, line)
    last = re.search(end, line)
    if first is None or last is None:
        return None
    ticks = int(last.group(1)) - int(first.group(1))
    return ticks / seconds if ticks > 0 else None


def witness(record: dict[str, Any], knob_on: bool, applied_rate: float | None,
            stale_skips: int, lagged_skips: int = 0) -> tuple[bool, str]:
    """Why an output is known to have aged past the publish budget, on either tree.

    The fixed pump proves it with `pump_stale_skipped_ticks`: that counter moves only at the queue
    budget (`stale`), never for a broadcast `Lagged` run. A pump without the budget skip proves it
    by its measured lag behind the core (`core_tick_seen - pump_tick`) over the join's rate: the
    `/tick query` confirmation when it arrived, otherwise the rate the server's own join record
    prints for that wait (its ticks over its seconds). A `Lagged`-only skip, the knob's warn line
    alone, a missing admission or a wait with no printed interval are inconclusive, never a symptom.
    """
    if not knob_on:
        return False, "the pump lag knob never ran: the lagging condition did not occur"
    if not record.get("admitted"):
        return False, "the joining client never reached the server (no admission line): infrastructure, not a verdict"
    # The record's own measured interval is the only rate that belongs to *this* wait; the
    # `/tick query` reply is valid only as a fallback when the wait printed no interval (a witness
    # admitted before `/tick rate` would lend its 20 TPS to a 3 TPS verdict).
    rate = record.get("rate")
    if rate is None or rate <= 0:
        rate = applied_rate if applied_rate is not None and applied_rate > 0 else None
    if rate is None or rate <= 0:
        return False, ("neither an applied tick rate nor the join's own printed interval "
                       "was witnessed: the delay age in seconds is unknown")
    if stale_skips > 0:
        return True, (f"the pump skipped {stale_skips} outputs on the publish budget at "
                      f"{rate:g} TPS")
    lag = record.get("lag")
    if lag is not None and lag / rate > PUBLISH_BUDGET_SECONDS:
        return True, (f"the pump was {lag} ticks = {lag / rate:.2f} s behind at "
                      f"{rate:g} TPS, past the {PUBLISH_BUDGET_SECONDS:.1f} s budget")
    if lagged_skips > 0:
        return False, (f"only broadcast Lagged skips were counted ({lagged_skips}), not one on the "
                       f"{PUBLISH_BUDGET_SECONDS:.1f} s budget: the condition did not occur")
    return False, (f"neither a budget skip nor a delay past the {PUBLISH_BUDGET_SECONDS:.1f} s budget "
                   f"was witnessed (lag {lag}, rate {rate:g} TPS): the condition did not occur")


def join_verdict(record: dict[str, Any], knob_on: bool, applied_rate: float | None,
                 stale_skips: int, lagged_skips: int = 0, *,
                 halt_at: datetime | None = None,
                 joiner_in_play: bool = True) -> tuple[bool | None, str]:
    """Row 1 verdict: False only for a join held past the deadline or timed out behind a witnessed
    over-budget publication; True for a join inside the deadline behind it; else None. A core halt is
    row 2's symptom and never makes row 1 false."""
    witnessed, info = witness(record, knob_on, applied_rate, stale_skips, lagged_skips)
    if not witnessed:
        return None, info
    landing = record.get("landed_seconds")
    if landing is not None:
        landed_at = record.get("landed_at")
        if halt_at is not None and isinstance(landed_at, datetime) and landed_at >= halt_at:
            # The halt drains the outbox, so the join it "admits" was never served by the pump the
            # row is about: the symptom is row 2's and row 1 has no outcome (review wave2-8 M2).
            return None, (f"the join landed only after the core halt drained the queue "
                          f"(halted at {halt_at.isoformat()}, landed {landed_at.isoformat()}): "
                          f"row 1's deadline was never exercised")
        if not joiner_in_play:
            return None, (f"the joining client never reached play inside {float(landing):.1f} s: "
                          f"a disconnected join is infrastructure or the halt's own kick, not an "
                          f"undrained pump")
        if float(landing) > JOIN_BOUND_SECONDS:
            return False, (f"the join landed after {float(landing):.1f} s with {info}, "
                           f"past the {JOIN_BOUND_SECONDS:.0f} s deadline")
        return True, f"the join landed in {float(landing):.1f} s with {info}"
    if record.get("timed_out"):
        return False, f"the join missed the {JOIN_BOUND_SECONDS:.0f} s deadline with {info}"
    return None, "the join neither landed nor timed out inside the run: no verdict"


def halt_verdict(knob_on: bool, record: dict[str, Any], applied_rate: float | None,
                 stale_skips: int, tick_before: int, tick_after: int, halt: str | None,
                 window_start: int | None = None) -> tuple[bool | None, str]:
    """Row 2 verdict: False only for a halt or a stuck core behind the same witnessed publication.

    A pass needs a window at least as long as the outbox stall watch the row names: the halt fires
    10 s after the queue stopped draining, so a shorter observation would "pass" a core that had
    simply not been given the chance to halt yet (review wave2-8 M3)."""
    witnessed, info = witness(record, knob_on, applied_rate, stale_skips)
    if not witnessed:
        return None, info
    if halt:
        return False, f"the core halted under a live lagging pump: {halt}"
    if tick_after <= tick_before:
        return False, f"the simulation tick did not advance ({tick_before} -> {tick_after})"
    needed = OUTBOX_STALL_SECONDS + HALT_WINDOW_MARGIN_SECONDS
    if window_start is not None:
        rate = record.get("rate") or applied_rate
        elapsed = ((tick_after - window_start) / rate) if rate and rate > 0 else None
        if elapsed is None or elapsed < needed:
            return None, (f"the halt watch had only {elapsed if elapsed is not None else 'unknown'} s "
                          f"of observation ({window_start} -> {tick_after}), shorter than the "
                          f"{needed:.0f} s stall window: a pass proves nothing")
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


def halt_moment(log_path: Path) -> datetime | None:
    """The stamp of the core-halt line: a join landed after it only rode the halt's queue drain."""
    line = halt_line(log_path)
    if line is None:
        return None
    try:
        return datetime.fromisoformat(line.split(" ", 1)[0].replace("Z", "+00:00"))
    except ValueError:
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


def applied_tick_rate(chat: list[Any]) -> float | None:
    """The rate from the server's own `/tick query` reply ("Target tick rate: 3.0"), or None."""
    for line in reversed([str(item) for item in chat]):
        match = re.search(r"Target tick rate:?\s*([0-9.]+)", line)
        if match:
            return float(match.group(1))
    return None


def _skip_counters(dashboard_port: int) -> tuple[int, int]:
    """(budget skips, all pump skips) from the dashboard, or zeros when it is silent.

    The edge keeps `pump_stale_skipped_ticks` for the publish-budget `stale` path only, so the
    difference `pump_skipped_ticks - stale` is the broadcast `Lagged` share.
    """
    try:
        stats = _stat(dashboard_port)
    except (OSError, ValueError):
        return 0, 0
    return int(stats.get("pump_stale_skipped_ticks") or 0), int(stats.get("pump_skipped_ticks") or 0)


def _safe_tick(dashboard_port: int) -> int:
    """The core tick, or 0 when the dashboard is not answering (a halt ends the process)."""
    try:
        return _tick(_stat(dashboard_port))
    except OSError:
        return 0


def validate_args(ctx: ProfileContext) -> tuple[float, float, int]:
    if ctx.client != "native":
        raise ValueError("join-lag-gate requires --client native")
    if ctx.platform is not None:
        raise ValueError("join-lag-gate accepts only --client, --windowed, --timeout-seconds and its own args")
    parser = argparse.ArgumentParser(prog="join-lag-gate")
    # 20 TPS is the row's own rate: the pump must outpace the tick for the queue to build in seconds.
    parser.add_argument("--tick-rate", type=float, default=3.0)
    # The condition lever: the pump is delayed by K core ticks (KRASNOKAMEN_PUMP_LAG_TICKS, a
    # test/dev knob, never an operator setting).
    parser.add_argument("--pump-lag-ticks", type=int, default=8)
    args = parser.parse_args(ctx.extra_args)
    if not 1.0 <= args.tick_rate <= 20.0:
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
            # A failed receipt must carry the gate's own verdicts: the aggregate assertion below
            # fails first and, without them, a promotion control cannot read the row's own red
            # (ledger._failing_verdict reads details.verdicts for a boolean profile).
            evidence["verdicts"] = dict(verdicts)
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


def _note_play_lost(a: ProbeSteps, detail: dict[str, Any], error: Exception,
                    server_log_path: Path) -> None:
    """Keep a tolerated post-play failure from reading as "the client never reached play".

    `ProbeSteps.observe` records an `in_play` check on every observation, so an observation taken
    after the halt fails it. The client did reach play (`player_present`), and the ledger refuses
    every receipt whose client never reached play — exactly the halt's own disconnect. The loss is
    recorded in `details` and the verdict-hiding check is dropped; a client still in play keeps its
    own green check and no note. Only the core halt excuses it: any other lost play (a keepalive
    kick, a crashed client) stays the client's own failed check (review wave2-8 m1).
    """
    assertions = a.report["assertions"]
    if assertions.get("in_play") is not False:
        return
    if halt_line(server_log_path) is None:
        return
    detail.setdefault("play_lost", []).append(str(error))
    assertions.pop("in_play", None)
    assertions["client_reached_play"] = True


def _gate(a: ProbeSteps, ctx: ProfileContext, artifact: Path, tick_rate: float, address: str,
          server_log_path: Path, dashboard_port: int, server: subprocess.Popen[Any],
          launch_joiner: Any) -> tuple[dict[str, Any], dict[str, bool | None]]:
    """The walk: the pump runs behind the tick, a fresh client joins while it is behind.

    Order (review REVIEW-wave2-2 B1): `/tick rate`, then the joiner is launched, then the
    `/tick query` reply is awaited as best effort, then the joiner's own record is judged. Awaiting
    the reply first spent the whole join deadline inside the lagging pump, so the pre-fix control
    never attempted a join. Both verdicts read the joiner's record; the witness's own record
    witnesses the delay when the joiner never got admitted.
    """
    verdicts: dict[str, bool | None] = {}
    detail: dict[str, Any] = {"tick_rate": tick_rate}
    a.report["details"] = detail  # kept even when a later step fails
    # The knob's own warn line is written before the server listens; wait for it with inotify so a
    # late flush is an event, not a sleep.
    delayed = runtime.wait_for_path_condition(
        server_log_path, 10.0, lambda: (engaged(server_log_path), engaged(server_log_path)), server)
    detail["pump_lag_enabled"] = delayed
    try:
        a.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float), "player_present")
        a.fixture(f"tick rate {tick_rate:g}")
    except Exception as error:
        # Before `/tick rate` is even sent a halt is row 2's symptom, judged from the witness's own
        # landing record and the halt line; row 1 has no join outcome because none was attempted.
        return _early_halt(detail, verdicts, a, error, server_log_path, delayed, tick_rate, server)
    tick_before = _safe_tick(dashboard_port)
    # The halt watch's own origin: the stall watch runs for 10 s from the queue stalling, so the
    # observation window is measured from the tick the rate applied (review wave2-8 M3).
    try:
        observed_at_rate = (a.observe(outside_play=True).get("time") or {}).get("game_time")
    except Exception:
        observed_at_rate = None
    skips_before = _skip_counters(dashboard_port)
    joiner: ProbeSteps | None = None
    try:
        _joiner_process, joiner_mcp = launch_joiner()
        joiner = ProbeSteps(joiner_mcp, artifact / "join-lag-joiner",
                            max(60.0, a.deadline - time.monotonic()), "native-join-lag-joiner")
        joiner.__enter__()
    except Exception as error:  # a client that cannot start is not a verdict
        detail["join_launch_error"] = str(error)
    # `/tick query` while the join is in flight: its reply shares the pump queue it describes, so it
    # is best effort — the row's own interval then comes from the joiner record's core ticks.
    applied_rate: float | None = None
    try:
        a.fixture("tick query")
        applied = a.wait(lambda s: applied_tick_rate(s.get("recent_chat") or []) is not None,
                         "tick_rate_applied", timeout_seconds=RATE_WAIT_SECONDS, record=False)
        applied_rate = applied_tick_rate(applied.get("recent_chat") or [])
    except Exception as error:
        # The reply is best effort: a held reply must not be written as a failed scenario assertion
        # (review wave2-8 m2); the joiner record's own interval carries the rate.
        detail["rate_wait_error"] = str(error)
        _note_play_lost(a, detail, error, server_log_path)
    detail["applied_tick_rate"] = applied_rate
    joiner_in_play = False
    if joiner is not None:
        try:
            joiner.call("minecraft_wait_for_play", {"timeout_seconds": JOIN_BOUND_SECONDS})
            joiner_in_play = joiner.call("minecraft_observe").get("in_play") is True
            joiner.check("joiner_entered_play", joiner_in_play)
        except Exception as error:
            # A kicked or stalled join is the symptom, not an infrastructure failure.
            detail["join_error"] = str(error)

    def settled() -> tuple[bool, dict[str, Any]]:
        current = join_record(server_log_path, JOINER)
        return current.get("knockout") is not None or current.get("timed_out") is True, current

    try:
        # The server's own landing/kick line may be written just after the client's tool deadline:
        # wait for the log event (inotify) with the server pidfd, then judge.
        record = runtime.wait_for_path_condition(server_log_path, JOIN_RECORD_SECONDS,
                                                 settled, server)
    except (TimeoutError, runtime.ChildExited):
        record = join_record(server_log_path, JOINER)
    witness_record = join_record(server_log_path, WITNESS)
    detail["join"] = record
    detail["witness_join"] = witness_record
    detail["join_elapsed_seconds"] = (None if record.get("landed_seconds") is None
                                      else round(float(record["landed_seconds"]), 2))
    detail["join_in_play"] = joiner_in_play
    halt = halt_line(server_log_path)
    # A pass needs the whole stall window, not the fragment a fast join leaves: hold the witness
    # until the client's own observed clock has advanced past OUTBOX_STALL_SECONDS + margin at the
    # applied rate. Event-driven (`minecraft_wait_for_state_change`), never a sleep.
    bound_rate = applied_rate if applied_rate is not None and applied_rate > 0 else tick_rate
    needed_ticks = math.ceil((OUTBOX_STALL_SECONDS + HALT_WINDOW_MARGIN_SECONDS) * bound_rate)
    if halt is None and observed_at_rate is not None:
        try:
            a.wait(lambda s: (s.get("time") or {}).get("game_time", -1) >= observed_at_rate + needed_ticks,
                   "halt_window_elapsed", outside_play=True, timeout_seconds=OUTBOX_STALL_SECONDS + 20.0,
                   record=False)
            detail["halt_window_ticks"] = needed_ticks
        except Exception as error:
            # A halt inside the window is row 2's own symptom and is read below; a silent client
            # leaves the window check to halt_verdict (a None verdict, never a pass).
            detail["halt_window_error"] = str(error)
            if halt is None:
                halt = halt_line(server_log_path)
    tick_after = _safe_tick(dashboard_port)
    stats_end = None
    try:
        stats_end = _stat(dashboard_port)
    except OSError:
        pass
    skips_after = _skip_counters(dashboard_port)
    stale_delta = max(0, skips_after[0] - skips_before[0])
    lagged_delta = max(0, (skips_after[1] - skips_before[1]) - stale_delta)
    detail["join_stale_skips"] = stale_delta
    detail["join_lagged_skips"] = lagged_delta
    detail["join_ticks"] = [tick_before, tick_after]
    detail["halt_window"] = {"start_tick": tick_before, "end_tick": tick_after,
                             "seconds": (None if not bound_rate or tick_after <= tick_before
                                         else round((tick_after - tick_before) / bound_rate, 2))}
    detail["publish"] = (stats_end or {}).get("publish")
    detail["tps_at_end"] = (stats_end or {}).get("tps")
    detail["halt_line"] = halt
    ok, why = join_verdict(record, delayed, applied_rate, stale_delta, lagged_delta,
                           halt_at=halt_moment(server_log_path), joiner_in_play=joiner_in_play)
    detail["join_verdict"] = why
    detail.setdefault("verdicts", {})["join_behind_slow_pump"] = why
    verdicts["join_behind_slow_pump"] = ok
    # Row 2 is judged from the *joiner's* record only: the witness joined before `/tick rate`, so
    # its lag and its 20 TPS rate are evidence about another phase (review wave2-8 M4).
    ok2, why2 = halt_verdict(delayed, record, applied_rate, stale_delta,
                             tick_before, tick_after, halt, window_start=tick_before)
    detail["halt_verdict"] = why2
    detail.setdefault("verdicts", {})["live_pump_does_not_halt"] = why2
    verdicts["live_pump_does_not_halt"] = ok2
    if ctx.windowed:
        try:
            native_client.capture(a.client, artifact, "join-lag-after-join")
            native_client.capture(a.client, artifact, "join-lag-final")
        except Exception as error:  # a halted core closed the screen; the verdicts stand
            _note_play_lost(a, detail, error, server_log_path)
    return detail, verdicts


def _early_halt(detail: dict[str, Any], verdicts: dict[str, bool | None], a: ProbeSteps,
                error: Exception, server_log_path: Path, delayed: bool, tick_rate: float,
                server: subprocess.Popen[Any]) -> tuple[dict[str, Any], dict[str, bool | None]]:
    """The core halted before `/tick rate`: row 2's symptom, and row 1 never had a join to judge."""
    try:
        early_halt = runtime.wait_for_path_condition(
            server_log_path, 20.0,
            lambda: (halt_line(server_log_path) is not None, halt_line(server_log_path)), server)
    except (TimeoutError, runtime.ChildExited):
        early_halt = halt_line(server_log_path)
    if early_halt is None:
        raise error
    detail["early_error"] = str(error)
    detail["halt_line"] = early_halt
    record = join_record(server_log_path, WITNESS)
    detail["witness_record"] = record
    witnessed, info = witness(record, delayed, tick_rate, 0)
    ok, why = (None, info) if not witnessed else (False, f"the core halted before the join: {early_halt}")
    detail["halt_verdict"] = why
    detail.setdefault("verdicts", {})["live_pump_does_not_halt"] = why
    verdicts["live_pump_does_not_halt"] = ok
    detail["join_verdict"] = "no join was attempted before the halt: row 1 has no outcome here"
    detail.setdefault("verdicts", {})["join_behind_slow_pump"] = detail["join_verdict"]
    verdicts["join_behind_slow_pump"] = None
    return detail, verdicts
