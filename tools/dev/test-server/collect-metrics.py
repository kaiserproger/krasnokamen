#!/usr/bin/env python3
"""Fold the dashboard tick ring into per-minute aggregates (one JSONL per UTC day)."""
import json, os, time, urllib.request
from collections import defaultdict

BASE = os.environ.get("KRASNOKAMEN_DASHBOARD_URL", "http://localhost:25580")
OUT = os.environ.get("KRASNOKAMEN_METRICS_DIR", "/opt/solaris/metrics")
CURSOR = os.path.join(OUT, ".cursor")


def get(path):
    with urllib.request.urlopen(BASE + path, timeout=15) as r:
        return json.load(r)


def pct(values, p):
    return values[min(len(values) - 1, int(len(values) * p))] if values else 0


def main():
    os.makedirs(OUT, exist_ok=True)
    stats = get("/stats")
    try:
        instance, after = open(CURSOR).read().split()
    except (OSError, ValueError):
        instance, after = "", "-1"
    load = get("/load-metrics" + ("" if after == "-1" else f"?after_tick={after}"))
    if str(load["instance_started_unix_ns"]) != instance:
        load = get("/load-metrics")  # server restarted: take the whole ring
    current_minute = time.time_ns() // 60_000_000_000
    minutes = defaultdict(list)
    for s in load.get("samples", []):
        minute = s["wall_start_unix_ns"] // 60_000_000_000
        if minute < current_minute:  # closed minutes only; the open tail is re-read next run
            minutes[minute].append(s)
    day = time.strftime("%Y-%m-%d", time.gmtime())
    with open(os.path.join(OUT, f"{day}.jsonl"), "a") as f:
        for minute, ticks in sorted(minutes.items()):
            totals = sorted(t["total_us"] for t in ticks)
            slow = sorted(ticks, key=lambda t: t["total_us"], reverse=True)[:3]
            phases = ticks[0]["phase_us"].keys()
            f.write(json.dumps({
                "minute_unix": minute * 60,
                "ticks": len(ticks),
                "tick_us": {"p50": pct(totals, .5), "p95": pct(totals, .95),
                            "p99": pct(totals, .99), "max": totals[-1]},
                "phase_mean_us": {p: sum(t["phase_us"][p] for t in ticks) // len(ticks) for p in phases},
                "phase_max_us": {p: max(t["phase_us"][p] for t in ticks) for p in phases},
                "slowest": [{"tick": t["tick"], "total_us": t["total_us"], "phase_us": t["phase_us"]} for t in slow],
            }) + "\n")
        f.write(json.dumps({"stats_unix": int(time.time()), "stats": stats}) + "\n")
    emitted = [t["tick"] for ticks in minutes.values() for t in ticks]
    if emitted:
        with open(CURSOR, "w") as c:
            c.write(f'{load["instance_started_unix_ns"]} {max(emitted)}')


main()
