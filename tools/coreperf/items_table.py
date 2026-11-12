"""Per-item cost table from coreperf runs recorded with MC_CORE_PHASE_LOG=1.

Each leg is a run directory holding `run/result.json`, `run/samples.jsonl` and `run/server.log` (the
layout `tools.coreperf.profiles` writes). The server log carries, per simulated dimension tick, the
`items census:`, `items merge:`, `items expiry:` and `items views:` lines followed by that tick's
`phase ms:` line, and, per published tick, one `edge items:` line (joined on the tick number).

Usage:
    python3 -m tools.coreperf.items_table --leg n0=<dir> --leg n10k=<dir> --leg n30k=<dir> \
        [--leg kelp1=<dir> ...] [--json out.json] [--md out.md]

Legs whose name is `n<digits>[k]` form the synthetic ladder: their per-tick means are regressed on the
mean resident item count, giving microseconds per item per tick for every column. Every leg also gets a
within-run regression of each phase on that tick's item count (the kelp world's population drifts).
"""
from __future__ import annotations

import argparse
import json
import re
import statistics
import sys
from pathlib import Path
from typing import Any

KV = re.compile(r"(\w+)=([-\w.]+)")
LAPS = re.compile(r"^\s+(.+?) detail ms:(.*)$")
PHASE_COLUMNS = ("input", "chunks", "entity", "merge", "block", "commit", "snapshot", "views",
                 "entity_tail", "block_prepare", "publish_tail", "before_return")
EDGE_COLUMNS = ("prelude_ms", "index_ms", "plan_ms", "fill_ms", "total_ms", "probe_ms")
EDGE_COUNTS = ("packets", "bytes", "item_pairs", "all_pairs", "item_visits", "all_visits",
               "item_spawn_pkts", "item_spawn_bytes", "item_move_pkts", "item_move_bytes",
               "item_meta_pkts", "item_meta_bytes")


def numbers(text: str) -> dict[str, Any]:
    out: dict[str, Any] = {}
    for key, value in KV.findall(text):
        try:
            out[key] = float(value)
        except ValueError:
            out[key] = value
    return out


def parse_log(text: str) -> tuple[dict[int, dict[str, Any]], dict[int, dict[str, Any]], dict[str, list[dict[str, Any]]]]:
    """Overworld tick records keyed by phase-log tick, edge publish records keyed by tick, and the
    loop-level `tick outer ms:` and `checkpoint capture ms:` lines (outside any dimension tick)."""
    ticks: dict[int, dict[str, Any]] = {}
    edge: dict[int, dict[str, Any]] = {}
    loop: dict[str, list[dict[str, Any]]] = {"outer": [], "checkpoint": []}
    pending: dict[str, Any] = {}
    for line in text.splitlines():
        if line.startswith("tick outer ms:"):
            loop["outer"].append(numbers(line))
        elif line.startswith("checkpoint capture ms:"):
            loop["checkpoint"].append(numbers(line))
        elif line.startswith("items "):
            kind, _, rest = line[len("items "):].partition(":")
            row = numbers(rest)
            if kind == "expiry":
                pending.setdefault("expiry", []).append(row)
            elif kind in ("census", "merge", "views"):
                pending[kind] = row
        elif line.startswith("edge items:"):
            row = numbers(line)
            edge[int(row["tick"])] = row
        elif "phase ms:" in line and line.lstrip().startswith("phase ms:"):
            row = numbers(line.split("phase ms:", 1)[1])
            if row.get("dimension") == "Overworld":
                pending["phase"] = row
                ticks[int(row["tick"])] = pending
            pending = {}
        else:
            match = LAPS.match(line)
            if match:
                pending.setdefault("laps", {})[match.group(1)] = numbers(match.group(2))
    return ticks, edge, loop


def pct(values: list[float], q: float) -> float:
    ordered = sorted(values)
    if not ordered:
        return float("nan")
    position = (len(ordered) - 1) * q
    low = int(position)
    high = min(low + 1, len(ordered) - 1)
    return ordered[low] + (ordered[high] - ordered[low]) * (position - low)


def mean(values: list[float]) -> float:
    return statistics.fmean(values) if values else 0.0


def load_window(run: Path) -> tuple[dict[int, dict[str, Any]], dict[str, Any]]:
    """Exact samples whose wall start lies in the measured window, keyed by sample tick."""
    result = json.loads((run / "result.json").read_text())
    start, end = result["start_unix_ns"], result["end_unix_ns"]
    samples: dict[int, dict[str, Any]] = {}
    for line in (run / "samples.jsonl").read_text().splitlines():
        for tick in json.loads(line).get("metrics", {}).get("samples", []):
            if start <= tick["wall_start_unix_ns"] < end:
                samples[tick["tick"]] = tick
    return samples, result


def tick_offset(samples: dict[int, dict[str, Any]], ticks: dict[int, dict[str, Any]]) -> int:
    """Phase-log tick minus sample tick, picked by equating the entity phase times."""
    def score(offset: int) -> int:
        hits = 0
        for tick, sample in samples.items():
            row = ticks.get(tick + offset)
            if row:
                phase = row["phase"]
                if abs(phase["entity"] + phase["entity_tail"] - sample["phase_us"]["entity_us"] / 1000) < 0.2:
                    hits += 1
        return hits
    return max(range(-3, 4), key=score)


def regression(xs: list[float], ys: list[float]) -> dict[str, float] | None:
    if len(xs) < 3 or max(xs) == min(xs):
        return None
    slope, intercept = statistics.linear_regression(xs, ys)
    predicted = [slope * x + intercept for x in xs]
    spread = sum((y - statistics.fmean(ys)) ** 2 for y in ys)
    residual = sum((y - p) ** 2 for y, p in zip(ys, predicted))
    return {"us_per_item": slope * 1000, "intercept_ms": intercept,
            "r2": 1 - residual / spread if spread else 0.0}


def analyze_leg(root: Path, label: str) -> dict[str, Any]:
    run = root / "run" if (root / "run").is_dir() else root
    samples, result = load_window(run)
    ticks, edge, loop = parse_log((run / "server.log").read_text(errors="replace"))
    offset = tick_offset(samples, ticks)
    window = {tick: ticks[tick + offset] for tick in samples if tick + offset in ticks}
    wall = [samples[tick]["wall_us"] / 1000 for tick in window]
    out: dict[str, Any] = {
        "label": label, "receipt": str(root), "status": result["status"],
        "tps": result["summary"]["tps"], "mspt": result["summary"]["mspt"],
        "window_ticks": len(samples), "matched_ticks": len(window), "offset": offset,
        "wall_ms": {"mean": mean(wall), "p50": pct(wall, .5), "p95": pct(wall, .95), "max": max(wall, default=0.0)},
        "items_after": result.get("entity_counts_after", {}).get("minecraft:item"),
        "items_before": result.get("entity_counts_before", {}).get("minecraft:item"),
    }
    census = [row["census"] for row in window.values() if "census" in row]
    items = [row.get("items", 0.0) for row in census]
    out["items"] = {"mean": mean(items), "min": min(items, default=0.0), "max": max(items, default=0.0),
                    "moving_mean": mean([row.get("moving", 0.0) for row in census]),
                    "ground_mean": mean([row.get("ground", 0.0) for row in census]),
                    "mature_mean": mean([row.get("mature", 0.0) for row in census])}
    out["phases_ms"] = {key: mean([row["phase"][key] for row in window.values() if key in row["phase"]])
                        for key in PHASE_COLUMNS}
    # What the attribution itself costs inside the timed phases (diagnostic overhead, subtracted below):
    # the per-tick census runs in the entity commit, the views probe inside the views phase.
    out["overhead_ms"] = {
        "census": sum(row.get("census_ms", 0.0) for row in census) / len(window) if window else 0.0,
        "views_probe": sum(row["views"].get("probe_ms", 0.0) for row in window.values() if "views" in row)
        / len(window) if window else 0.0}
    out["phases_net_ms"] = {"entity": out["phases_ms"]["entity"] - out["overhead_ms"]["census"],
                            "views": out["phases_ms"]["views"] - out["overhead_ms"]["views_probe"]}
    laps: dict[str, dict[str, list[float]]] = {}
    for row in window.values():
        for label_, values in row.get("laps", {}).items():
            for key, value in values.items():
                laps.setdefault(label_, {}).setdefault(key, []).append(value)
    out["laps_ms"] = {label_: {key: mean(values) for key, values in group.items()} for label_, group in laps.items()}
    # Item-owned stages. Step CPU is summed over rayon workers (not wall); each timed item carries one
    # clock read pair, subtracted here.
    step_items = sum(row.get("step_items", 0.0) for row in census)
    step_cpu_ms = sum(row.get("step_cpu_ms", 0.0) for row in census)
    clock_ns = mean([row.get("clock_ns", 0.0) for row in census])
    out["step"] = {"cpu_ms_per_tick": step_cpu_ms / len(census) if census else 0.0,
                   "items_per_tick": step_items / len(census) if census else 0.0,
                   "ns_per_item": ((step_cpu_ms * 1e6 - step_items * clock_ns) / step_items
                                   if step_items else 0.0), "clock_ns": clock_ns}
    merges = [row["merge"] for row in window.values() if "merge" in row]
    examined = sum(row.get("examined", 0.0) for row in merges)
    scan_ms = sum(row.get("scan_ms", 0.0) + row.get("gather_sort_ms", 0.0) for row in merges)
    out["merge"] = {"passes": len(merges),
                    "ms_per_pass": mean([row.get("scan_ms", 0.0) + row.get("gather_sort_ms", 0.0) for row in merges]),
                    "max_ms": max([row.get("scan_ms", 0.0) + row.get("gather_sort_ms", 0.0) for row in merges], default=0.0),
                    "gather_sort_ms": mean([row.get("gather_sort_ms", 0.0) for row in merges]),
                    "scan_ms": mean([row.get("scan_ms", 0.0) for row in merges]),
                    "ms_per_tick": scan_ms / len(window) if window else 0.0,
                    "ns_per_item_per_pass": scan_ms * 1e6 / examined if examined else 0.0,
                    "candidates_per_item": sum(row.get("candidates", 0.0) for row in merges) / examined if examined else 0.0,
                    "overlapping_per_item": sum(row.get("overlapping", 0.0) for row in merges) / examined if examined else 0.0,
                    "merged_total": sum(row.get("merged", 0.0) for row in merges)}
    expiry = [entry for row in window.values() for entry in row.get("expiry", [])]
    removed = sum(entry.get("removed", 0.0) for entry in expiry)
    remove_ms = sum(entry.get("remove_ms", 0.0) for entry in expiry)
    out["expiry"] = {"events": len(expiry), "removed": removed, "remove_ms": remove_ms,
                     "us_per_removed": remove_ms * 1000 / removed if removed else 0.0}
    views = [row["views"] for row in window.values() if "views" in row]
    out["views"] = {"ticks_with_publish": len(views),
                    "pending_mean": sum(row.get("pending", 0.0) for row in views) / len(window) if window else 0.0,
                    "items_pending_mean": sum(row.get("items_pending", 0.0) for row in views) / len(window) if window else 0.0,
                    "changed_mean": sum(row.get("changed", 0.0) for row in views) / len(window) if window else 0.0,
                    "items_changed_mean": sum(row.get("items_changed", 0.0) for row in views) / len(window) if window else 0.0}
    low, high = min(window, default=0) + offset, max(window, default=0) + offset
    outer = [row for row in loop["outer"] if low <= row.get("tick", -1) <= high]
    out["outer_ms"] = {key: mean([row.get(key, 0.0) for row in outer])
                       for key in ("pre", "run", "journal", "seal_send_generated", "post")}
    captures = [row for row in loop["checkpoint"] if low <= row.get("tick", -1) <= high]
    capture_ms = [sum(value for key, value in row.items() if key != "tick" and isinstance(value, float))
                  for row in captures]
    out["checkpoint"] = {"captures": len(captures), "ms_per_capture": mean(capture_ms),
                         "max_ms": max(capture_ms, default=0.0),
                         "ms_per_tick": sum(capture_ms) / len(window) if window else 0.0,
                         "data_ms": mean([row.get("data", 0.0) for row in captures])}
    joined = [edge[tick] for tick in window if tick in edge]
    out["edge"] = {"ticks": len(joined)}
    out["edge"].update({key: mean([row.get(key, 0.0) for row in joined]) for key in (*EDGE_COLUMNS, *EDGE_COUNTS, "viewers")})
    # Within-run regression of each phase on that tick's item count, merge-cadence ticks excluded.
    steady = [(row["census"]["items"], row) for row in window.values()
              if "census" in row and "merge" not in row]
    xs = [item for item, _ in steady]
    out["within_run_slope"] = {key: regression(xs, [row["phase"].get(key, 0.0) for _, row in steady])
                               for key in ("entity", "block", "input", "views", "commit", "before_return")}
    out["within_run_slope"]["edge_plan_ms"] = regression(
        [row["census"]["items"] for tick, row in window.items() if "census" in row and tick in edge and "merge" not in row],
        [edge[tick]["plan_ms"] for tick, row in window.items() if "census" in row and tick in edge and "merge" not in row])
    return out


LADDER = re.compile(r"^n(\d+)(k?)$")


def ladder_size(label: str) -> int | None:
    match = LADDER.match(label)
    return int(match.group(1)) * (1000 if match.group(2) else 1) if match else None


def flat(leg: dict[str, Any]) -> dict[str, float]:
    """The columns the ladder regresses: every per-tick mean that can scale with the item count."""
    row = {f"phase.{key}": value for key, value in leg["phases_ms"].items()}
    row["wall"] = leg["wall_ms"]["mean"]
    row["phase.entity_net"] = leg["phases_net_ms"]["entity"]
    row["phase.views_net"] = leg["phases_net_ms"]["views"]
    row["step.cpu"] = leg["step"]["cpu_ms_per_tick"]
    row["merge.amortized"] = leg["merge"]["ms_per_tick"]
    row["checkpoint.amortized"] = leg["checkpoint"]["ms_per_tick"]
    for key, value in leg["outer_ms"].items():
        row[f"outer.{key}"] = value
    row["views.items_pending"] = leg["views"]["items_pending_mean"]
    row["views.items_changed"] = leg["views"]["items_changed_mean"]
    for label, group in leg["laps_ms"].items():
        for key, value in group.items():
            row[f"lap.{label}.{key}"] = value
    for key in (*EDGE_COLUMNS, *EDGE_COUNTS):
        row[f"edge.{key}"] = leg["edge"].get(key, 0.0)
    return row


def is_count(key: str) -> bool:
    """Columns that count things (packets, bytes, visits, candidates) rather than milliseconds."""
    return key.startswith("views.") or (key.startswith("edge.") and key[len("edge."):] in EDGE_COUNTS)


def ladder_table(legs: list[dict[str, Any]]) -> dict[str, Any]:
    """Least-squares slope of every column against the mean item count, in microseconds per item per tick."""
    rungs = sorted(((ladder_size(leg["label"]), leg) for leg in legs if ladder_size(leg["label"]) is not None),
                   key=lambda pair: pair[0])
    if len(rungs) < 2:
        return {}
    xs = [leg["items"]["mean"] for _, leg in rungs]
    rows = [flat(leg) for _, leg in rungs]
    columns: dict[str, Any] = {}
    for key in sorted(set().union(*rows)):
        ys = [row.get(key, 0.0) for row in rows]
        if max(xs) == min(xs):
            continue
        slope, intercept = statistics.linear_regression(xs, ys)
        steps = [(ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i]) for i in range(len(xs) - 1) if xs[i + 1] != xs[i]]
        columns[key] = {"values": ys, "per_item_per_tick": slope, "base": intercept,
                        "segment_slopes": steps, "unit": "count" if is_count(key) else "ms"}
    return {"sizes": [size for size, _ in rungs], "items_mean": xs, "columns": columns}


def markdown(legs: list[dict[str, Any]], ladder: dict[str, Any]) -> str:
    lines = ["# Many-items per-item cost table", ""]
    lines += ["| leg | TPS | wall mean | p50 | p95 | items (mean) | moving | step cpu ms | merge ms/pass | edge plan ms | edge pkts/tick | edge KiB/tick | item pairs | checkpoint ms/tick | outer post ms |",
              "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"]
    for leg in legs:
        lines.append("| {label} | {tps:.2f} | {wall:.1f} | {p50:.1f} | {p95:.1f} | {items:.0f} | {moving:.0f} | {step:.2f} | {merge:.2f} | {plan:.2f} | {pkts:.0f} | {kib:.1f} | {pairs:.0f} | {ckpt:.2f} | {post:.2f} |".format(
            label=leg["label"], tps=leg["tps"], wall=leg["wall_ms"]["mean"], p50=leg["wall_ms"]["p50"],
            p95=leg["wall_ms"]["p95"], items=leg["items"]["mean"], moving=leg["items"]["moving_mean"],
            step=leg["step"]["cpu_ms_per_tick"], merge=leg["merge"]["ms_per_pass"],
            plan=leg["edge"].get("plan_ms", 0.0), pkts=leg["edge"].get("packets", 0.0),
            kib=leg["edge"].get("bytes", 0.0) / 1024, pairs=leg["edge"].get("item_pairs", 0.0),
            ckpt=leg["checkpoint"]["ms_per_tick"], post=leg["outer_ms"]["post"]))
    lines += ["", "## Phases (mean ms per tick, Overworld)", "",
              "| leg | " + " | ".join(PHASE_COLUMNS) + " | entity net of census | views net of probe |",
              "|---|" + "---:|" * (len(PHASE_COLUMNS) + 2)]
    for leg in legs:
        lines.append(f"| {leg['label']} | " + " | ".join(f"{leg['phases_ms'][key]:.2f}" for key in PHASE_COLUMNS)
                     + f" | {leg['phases_net_ms']['entity']:.2f} | {leg['phases_net_ms']['views']:.2f} |")
    lines += ["", "## Item-owned stages", "",
              "| leg | step ns/item (clock-corrected) | merge ns/item/pass | candidates/item | overlapping/item | expiry removed | expiry µs/item | views items pending | views items changed |",
              "|---|---:|---:|---:|---:|---:|---:|---:|---:|"]
    for leg in legs:
        lines.append("| {label} | {step:.0f} | {merge:.0f} | {cand:.2f} | {over:.2f} | {removed:.0f} | {us:.1f} | {pending:.0f} | {changed:.0f} |".format(
            label=leg["label"], step=leg["step"]["ns_per_item"], merge=leg["merge"]["ns_per_item_per_pass"],
            cand=leg["merge"]["candidates_per_item"], over=leg["merge"]["overlapping_per_item"],
            removed=leg["expiry"]["removed"], us=leg["expiry"]["us_per_removed"],
            pending=leg["views"]["items_pending_mean"], changed=leg["views"]["items_changed_mean"]))
    if ladder:
        sizes = ", ".join(f"{x:.0f}" for x in ladder["items_mean"])
        for unit, title, scale, digits in (
                ("ms", f"Ladder slopes, time (items mean {sizes}): µs per item per tick", 1000, 3),
                ("count", f"Ladder slopes, counts (items mean {sizes}): packets, bytes, visits per item per tick", 1, 3)):
            ranked = sorted(((key, column) for key, column in ladder["columns"].items() if column["unit"] == unit),
                            key=lambda pair: -abs(pair[1]["per_item_per_tick"]))
            if not ranked:
                continue
            lines += ["", f"## {title}", "",
                      "| column | values at each rung | per item per tick | base | segments |", "|---|---|---:|---:|---|"]
            for key, column in ranked:
                lines.append("| {key} | {values} | {slope:.{d}f} | {base:.2f} | {segments} |".format(
                    key=key, values=", ".join(f"{value:.2f}" for value in column["values"]),
                    slope=column["per_item_per_tick"] * scale, base=column["base"], d=digits,
                    segments=", ".join(f"{value * scale:.{digits}f}" for value in column["segment_slopes"])))
    lines += ["", "## Within-run slope (µs per item per tick, non-merge ticks)", "",
              "| leg | " + " | ".join(("entity", "block", "input", "views", "commit", "before_return", "edge_plan_ms")) + " |",
              "|---|" + "---:|" * 7]
    for leg in legs:
        cells = []
        for key in ("entity", "block", "input", "views", "commit", "before_return", "edge_plan_ms"):
            fit = leg["within_run_slope"].get(key)
            cells.append("n/a" if fit is None else f"{fit['us_per_item']:.3f} (r²={fit['r2']:.2f})")
        lines.append(f"| {leg['label']} | " + " | ".join(cells) + " |")
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--leg", action="append", required=True, metavar="LABEL=DIR")
    parser.add_argument("--json", type=Path)
    parser.add_argument("--md", type=Path)
    args = parser.parse_args()
    legs = []
    for spec in args.leg:
        label, _, directory = spec.partition("=")
        legs.append(analyze_leg(Path(directory), label))
    ladder = ladder_table(legs)
    report = {"legs": legs, "ladder": ladder}
    text = markdown(legs, ladder)
    if args.json:
        args.json.write_text(json.dumps(report, indent=1))
    if args.md:
        args.md.write_text(text)
    sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
