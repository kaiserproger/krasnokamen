"""Lease clocks and agent-reported phase edges; claims.log owns timing history."""
from __future__ import annotations

import json
import math
import os
import re
import time
from dataclasses import dataclass, field
from pathlib import Path

import wtlib as w

NS = 1_000_000_000
ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,95}")
PHASES = ("active", "build-wait", "build-run", "review-wait", "peer-wait", "owner-wait", "unknown")
TERMINAL = ("blocked", "done")
EVENTS = ("start", *PHASES, *TERMINAL)
SOURCE = "agent check-in edges; same-boot CLOCK_BOOTTIME, not CPU/model/harness utilization"


@dataclass(frozen=True)
class Clock:
    at: int
    boot: str
    ticks: int


def clock() -> Clock:
    # Tests already inject the lease clock; a private fleet can decouple the two clocks.
    fixed = os.environ.get("KRASNOKAMEN_FLEET_NOW")
    if fixed is not None and os.environ.get("KRASNOKAMEN_FLEET_DIR"):
        at = int(fixed)
        return Clock(at, os.environ.get("KRASNOKAMEN_FLEET_BOOT_ID", "test-boot"),
                     int(os.environ.get("KRASNOKAMEN_FLEET_MONO_NS", at * NS)))
    at = int(fixed) if fixed is not None else int(time.time())
    boot = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
    return Clock(at, boot, time.clock_gettime_ns(time.CLOCK_BOOTTIME))


def lease_id(claim: dict) -> str:
    # A live claim from before this tool can opt in without rewriting its file.
    return claim.get("lease_id") or f"legacy-{claim['start']}"


def lease_left(claim: dict, now: Clock) -> int:
    ref = claim.get("lease_clock")
    if ref and ref["boot"] == now.boot and now.ticks >= ref["ticks"]:
        span = (claim["timebox_min"] + claim["grace_min"]) * 60 * NS
        return ref["ticks"] + span - now.ticks
    # Old claims and lease ownership after reboot retain the wall-clock lease policy.
    return (claim["expires"] - now.at) * NS


def lease_state(claim: dict, now: Clock) -> str:
    if claim.get("released") is not None:
        return "released"
    return "live" if lease_left(claim, now) > 0 else "expired"


def append(fleet: Path, record: dict) -> None:
    """Caller holds claims/.lock exclusively. Preserve, but never complete, a torn row."""
    fd = os.open(fleet / "claims.log", os.O_RDWR | os.O_APPEND | os.O_CREAT, 0o600)
    try:
        end = os.fstat(fd).st_size
        prefix = b" !incomplete\n" if end and os.pread(fd, 1, end - 1) != b"\n" else b""
        data = prefix + (json.dumps(record, separators=(",", ":")) + "\n").encode()
        if os.write(fd, data) != len(data):
            raise w.ToolError("short write to claims.log; check-in history is incomplete")
        if record["op"] == "check-in":
            os.fsync(fd)
    finally:
        os.close(fd)


@dataclass
class Timing:
    last: dict | None = None
    until: int = 0
    renewal: int = -1
    cutoffs: dict[tuple[str, int], int] = field(default_factory=dict)
    totals: dict[str, int] = field(default_factory=dict)
    seen: set[str] = field(default_factory=set)
    unmeasured: bool = False
    incomplete: bool = False

    def add(self, phase: str, ns: int) -> None:
        self.totals[phase] = self.totals.get(phase, 0) + ns

    def apply(self, event: dict, claim: dict) -> None:
        """Check the complete candidate edge before mutating this fold."""
        phase, session, seq = (event.get(k) for k in ("phase", "session", "seq"))
        ticks, boot, cutoff = (event.get(k) for k in ("ticks", "boot", "until"))
        previous = event.get("previous")
        if (phase not in EVENTS or not isinstance(session, str) or not ID.fullmatch(session)
                or type(seq) is not int or type(ticks) is not int or ticks < 0
                or not isinstance(boot, str) or not boot):
            raise w.ToolError("invalid timing event")
        last = self.last
        if phase == "start":
            if (seq != 0 or session in self.seen or type(cutoff) is not int or cutoff <= ticks
                    or previous != (last["session"] if last else None)
                    or (last and last["phase"] == "done")):
                raise w.ToolError("start needs a new session, seq 0 and the current previous session, if any")
            renewal = event.get("renewals")
            if type(renewal) is not int or not max(0, self.renewal) <= renewal <= claim["renewals"]:
                raise w.ToolError("invalid lease renewal epoch")
            epoch = (boot, renewal)
            if cutoff > self.cutoffs.get(epoch, cutoff):
                raise w.ToolError("session restart cannot extend the lease cutoff")
            # No phase is inferred across checkout/worker/boot boundaries, even on the same lease.
            origin = last or claim.get("created_clock")
            if origin and origin["boot"] == boot and ticks < origin["ticks"]:
                raise w.ToolError("monotonic clock regressed")
            if (not self.incomplete and origin and origin["boot"] == boot and ticks >= origin["ticks"]):
                self.add("unknown", ticks - origin["ticks"])
            else:
                self.unmeasured = True
            self.until = cutoff
            self.renewal = renewal
            self.cutoffs[epoch] = cutoff
            self.seen.add(session)
            self.incomplete = False
        else:
            if not last or last["session"] != session or seq != last["seq"] + 1:
                raise w.ToolError("stale session or out-of-order sequence; read fleet-status --task")
            if last["phase"] in TERMINAL:
                raise w.ToolError("session already ended")
            if self.incomplete or boot != last["boot"] or ticks >= self.until:
                raise w.ToolError("observation incomplete, expired or on another boot; start a new observation session")
            if ticks < last["ticks"]:
                raise w.ToolError("monotonic clock regressed")
            if phase == ("active" if last["phase"] == "start" else last["phase"]):
                raise w.ToolError("phase is unchanged; only check in on transitions")
            self.add("active" if last["phase"] == "start" else last["phase"], ticks - last["ticks"])
        self.last = event


def fold(fleet: Path, claims: list[dict]) -> dict[str, Timing]:
    """Caller holds claims/.lock. Historical generations are kept but not merged."""
    current = {c["task"]: c for c in claims}
    timings = {task: Timing() for task in current}
    if not timings:
        return timings
    try:
        stream = (fleet / "claims.log").open("rb")
    except FileNotFoundError:
        return timings
    last_raw: dict[str, bytes] = {}
    with stream:
        for raw in stream:
            try:
                if not raw.endswith(b"\n"):
                    raise ValueError("torn row")
                event = json.loads(raw)
                if not isinstance(event, dict):
                    raise ValueError("not an event")
            except (UnicodeError, ValueError):
                # A broken line has no reliable task ID. Invalidate open spans, not
                # already closed totals. A new explicit start is the recovery edge.
                for timing in timings.values():
                    if not timing.last or timing.last["phase"] not in TERMINAL:
                        timing.incomplete = True
                continue
            task = event.get("task")
            if event.get("op") != "check-in" or not isinstance(task, str) or task not in current:
                continue
            claim = current[task]
            timing = timings[task]
            if event.get("lease") != lease_id(claim):
                # Older generations precede the current start. A mismatched
                # edge after that start cannot be part of a valid current chain.
                if timing.last or not isinstance(event.get("lease"), str):
                    timing.incomplete = True
                continue
            try:
                if event.get("agent") != claim["agent"]:
                    raise w.ToolError("timing owner mismatch")
                if raw == last_raw.get(task):
                    continue  # An exact replay is not another edge or a new terminal result.
                timing.apply(event, claim)
                last_raw[task] = raw
            except w.ToolError:
                timing.incomplete = True
                timing.unmeasured = True
    return timings


def check_in(fleet: Path, claim: dict, now: Clock, agent: str, lease: str,
             session: str, seq: int, phase: str, previous: str | None) -> str:
    if claim["agent"] != agent or lease_id(claim) != lease:
        raise w.ToolError("stale lease or wrong logical agent; read fleet-status --task")
    timing = fold(fleet, [claim])[claim["task"]]
    last = timing.last
    if (not timing.incomplete and last and (session, seq, phase, previous) ==
            (last["session"], last["seq"], last["phase"], last.get("previous"))):
        return f"already checked in {claim['task']} {session} seq={seq} {phase}"
    if lease_state(claim, now) != "live":
        raise w.ToolError("check-in requires a live lease; no lease was renewed")
    ref = claim.get("lease_clock")
    if ref and ref["boot"] == now.boot and now.ticks < ref["ticks"]:
        raise w.ToolError("lease clock regressed; check-in refused")
    if (not ref or ref["boot"] != now.boot) and now.at < claim["lease_start"]:
        raise w.ToolError("wall clock predates the legacy lease anchor; check-in refused")
    event = {"op": "check-in", "task": claim["task"], "agent": agent, "lease": lease,
             "session": session, "seq": seq, "phase": phase, "at": now.at,
             "boot": now.boot, "ticks": now.ticks}
    if phase == "start":
        renewal = claim["renewals"]
        max_span = (claim["timebox_min"] + claim["grace_min"]) * 60 * NS
        # Wall expiry can bootstrap a legacy/reboot observation once, not renew it.
        cutoff = now.ticks + min(max_span, lease_left(claim, now))
        cutoff = min(cutoff, timing.cutoffs.get((now.boot, renewal), cutoff))
        if now.ticks >= cutoff:
            raise w.ToolError("observation cutoff expired; a new session cannot extend the lease")
        event.update(previous=previous, until=cutoff, renewals=renewal)
    timing.apply(event, claim)
    append(fleet, event)
    return f"checked in {claim['task']} {session} seq={seq} {phase}"


def seconds(ns: int) -> float:
    return round(ns / NS, 3)


def summary(claim: dict, timing: Timing, now: Clock) -> dict:
    last = timing.last
    state = lease_state(claim, now)
    live = (last is not None and state == "live" and not timing.incomplete
            and last["boot"] == now.boot and last["ticks"] <= now.ticks < timing.until)
    phase = "unknown"
    age = None
    reason = "no-check-in"
    if timing.incomplete:
        reason = "incomplete-log"
    elif last and last["phase"] in TERMINAL:
        phase, reason = last["phase"], "terminal-check-in"
    elif live:
        phase = "active" if last["phase"] == "start" else last["phase"]
        age, reason = seconds(now.ticks - last["ticks"]), "unconfirmed-open-interval"
    elif state != "live":
        reason = f"lease-{state}"
    elif last:
        reason = "boot-clock-or-session-boundary"
    return {"task": claim["task"], "agent": claim["agent"], "lease": lease_id(claim),
            "lease_state": state, "session": last["session"] if last else None,
            "next_seq": last["seq"] + 1 if live and phase not in TERMINAL else None,
            "phase": phase, "phase_age_s": age, "phase_evidence": reason,
            "closed_seconds": {p: seconds(timing.totals.get(p, 0)) for p in PHASES},
            "unmeasured_gap": timing.unmeasured or reason not in
            {"terminal-check-in", "unconfirmed-open-interval"}, "source": SOURCE}


def harness_evidence(path: Path) -> dict:
    """Explicitly supplied evidence, not inferred attribution or part of task totals."""
    result = {"source": str(path), "status": "unavailable", "duration_seconds": None}
    try:
        receipt = json.loads(path.read_bytes())
    except (OSError, ValueError):
        return result
    if (not isinstance(receipt, dict) or not isinstance(receipt.get("started_at"), str)
            or not isinstance(receipt.get("ended_at"), str) or not receipt["ended_at"]):
        return result
    status = receipt.get("status")
    if status not in ("passed", "failed", "timeout", "prepared", "interrupted"):
        return result
    duration = receipt.get("duration_seconds")
    if type(duration) in (int, float) and math.isfinite(duration) and duration >= 0:
        result["duration_seconds"] = duration
    result["status"] = status
    details = receipt.get("details")
    if isinstance(details, dict):
        # Some profiles report their own duration separately from the harness envelope.
        stage = details.get("duration_seconds")
        if type(stage) in (int, float) and math.isfinite(stage) and stage >= 0:
            result["details_duration_seconds"] = stage
    return result
