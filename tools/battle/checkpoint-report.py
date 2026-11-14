#!/usr/bin/env python3
"""Join tick telemetry with MC_CORE_PHASE_LOG checkpoint captures.

This reports observations only; it cannot prove the population or dirty count.
"""
import argparse
import json
import re
import statistics
from pathlib import Path

CAPTURE = re.compile(r"checkpoint capture ms: tick=(\d+) data=([0-9.]+) owners=([0-9.]+) scheduler=([0-9.]+) chunks=([0-9.]+) absent=([0-9.]+) dimensions=([0-9.]+) job=([0-9.]+)")
STATUS = re.compile(r"Battle (running|paused) tick=(\d+) side1=(\d+) side2=(\d+) resident=(\d+) active=(\d+) nonzero_velocity=(\d+) horses=(\d+) regiments=(\d+)")
FIELDS = ("data", "owners", "scheduler", "chunks", "absent", "dimensions", "job")


def phase_summary(ticks, start, end):
    if start is None and end is None:
        return None
    if type(start) is not int or type(end) is not int or end < start:
        raise ValueError("invalid phase boundaries")
    values = sorted(wall for tick, wall in ticks.items() if start <= tick <= end)
    result = {"start_tick": start, "end_tick": end, "samples": len(values),
              "missing_ticks": end - start + 1 - len(values)}
    if values:
        result.update({"p50_ms": statistics.median(values),
                       "p95_ms": values[(95 * len(values) + 99) // 100 - 1],
                       "p99_ms": values[(99 * len(values) + 99) // 100 - 1],
                       "max_ms": values[-1]})
    return result


def report(telemetry: Path, server_log: Path, boundaries: Path | None = None) -> dict:
    ticks = {}
    journal = {"journal_build_us": [], "journal_queue_us": []}
    last = None
    gaps = []
    with telemetry.open() as source:
        for line_number, line in enumerate(source, 1):
            row = json.loads(line)
            tick, wall_us = row["tick"], row["wall_us"]
            if not isinstance(tick, int) or not isinstance(wall_us, (int, float)) or wall_us < 0:
                raise ValueError(f"invalid telemetry at line {line_number}")
            if last is not None:
                if tick <= last:
                    raise ValueError(f"out-of-order or duplicate tick {tick}")
                if tick != last + 1:
                    gaps.append([last + 1, tick - 1])
            phase = row.get("phase_us") or {}
            for name, values in journal.items():
                if name in phase:
                    value = phase[name]
                    if not isinstance(value, (int, float)) or value < 0:
                        raise ValueError(f"invalid {name} at line {line_number}")
                    values.append(value)
            ticks[tick] = wall_us / 1000
            last = tick
    if not ticks:
        raise ValueError("empty telemetry")
    log = server_log.read_text(errors="replace")
    samples = []
    for match in STATUS.finditer(log):
        values = list(map(int, match.groups()[1:]))
        samples.append(dict(zip(("tick", "side1", "side2", "resident", "active", "nonzero_velocity", "horses", "regiments"), values), running=match.group(1) == "running"))
    captures = []
    for match in CAPTURE.finditer(log):
        tick = int(match.group(1))
        phases = dict(zip(FIELDS, map(float, match.groups()[1:])))
        nearest = min(samples, key=lambda sample: (abs(sample["tick"] - tick), sample["tick"])) if samples else None
        captures.append({"tick": tick, "capture_ms": sum(phases.values()),
                         "phases_ms": phases, "wall_ms": ticks.get(tick),
                         "nearest_status": nearest,
                         "status_tick_gap": None if nearest is None else nearest["tick"] - tick})
    if not captures:
        raise ValueError("no checkpoint captures; enable MC_CORE_PHASE_LOG")
    capture_ticks = {item["tick"] for item in captures}
    ordinary = [wall for tick, wall in ticks.items() if tick not in capture_ticks]
    journal_summary = {name: {"samples": len(values), "median_us": statistics.median(values),
                              "max_us": max(values)} if values else None
                       for name, values in journal.items()}
    phase_report = None
    if boundaries is not None:
        limits = json.loads(boundaries.read_text())
        phase_report = {
            "paused": phase_summary(ticks, limits["paused_sample_start_tick"], limits["paused_sample_end_tick"]),
            "battle": phase_summary(ticks, limits["battle_sample_start_tick"], limits["recorded_last_tick"]),
        }
    return {"telemetry_ticks": len(ticks), "first_tick": min(ticks), "last_tick": max(ticks),
            "missing_ranges": gaps, "ordinary_median_ms": statistics.median(ordinary) if ordinary else None,
            "journal_owner_phase_us": journal_summary,
            "measurement_phases": phase_report,
            "captures": captures, "unmatched_capture_ticks": [c["tick"] for c in captures if c["wall_ms"] is None],
            "population_and_dirty_count_verified": False}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("telemetry", type=Path)
    parser.add_argument("server_log", type=Path)
    parser.add_argument("--boundaries", type=Path, help="phase-boundaries.json from run-battle.sh")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = report(args.telemetry, args.server_log, args.boundaries)
    with args.output.open("x") as target:
        json.dump(result, target, indent=2, allow_nan=False)
        target.write("\n")


if __name__ == "__main__":
    main()
