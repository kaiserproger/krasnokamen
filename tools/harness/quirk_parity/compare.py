"""Snapshot comparison and oracle self-check for the quirk/farm parity gate."""
from __future__ import annotations

import hashlib
from collections import Counter
from typing import Any

MAX_DIFFS = 40


def entity_key(entity: dict) -> str:
    if entity.get("item"):
        return f"{entity['type']}|{entity['item']}"
    return str(entity.get("type"))


def entity_counts(entities: list[dict], spec: dict) -> Counter:
    """Count per type (item entities per item id, weighted by stack count)."""
    wanted = set(spec.get("entity_types", []))
    ignored = set(spec.get("ignore_entity_types", []))
    counts: Counter = Counter()
    for entity in entities:
        kind = entity.get("type")
        if wanted and kind not in wanted:
            continue
        if kind in ignored:
            continue
        counts[entity_key(entity)] += int(entity.get("count", 1)) if entity.get("item") else 1
    return counts


def block_diff(vanilla: dict, krasnokamen: dict, ignore: list[str]) -> list[dict]:
    diffs = []
    for pos in sorted(set(vanilla) | set(krasnokamen)):
        a, b = vanilla.get(pos), krasnokamen.get(pos)
        if a == b:
            continue
        if any(a and a.startswith(prefix) or b and b.startswith(prefix) for prefix in ignore):
            continue
        diffs.append({"pos": pos, "vanilla": a, "krasnokamen": b})
    return diffs


def _none_last(values: tuple) -> tuple:
    return tuple((value is None, value if value is not None else 0) for value in values)


def _close(a: Any, b: Any, epsilon: float) -> bool:
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return abs(a - b) <= epsilon
    return a == b


Row = tuple[str, Any, Any]  # (observable path, vanilla value, Krasnokamen value)


def compare_snapshot(spec: dict, vanilla: dict, krasnokamen: dict, rows: list[Row] | None = None,
                     prefix: str = "", wildcards: frozenset[str] = frozenset()) -> dict[str, Any]:
    """Compare one snapshot; multi-cell snapshots are compared cell by cell.

    A cell list present on only one side, or with a different length, is itself a
    difference: zipping unequal lists would drop the unmatched tail observations
    and pass a truncated or padded receipt as a match.

    `rows` (when given) receives every difference as an observable path with both values;
    a path listed in `wildcards` (differs between two vanilla recordings) is no difference.
    """
    rows = [] if rows is None else rows
    va_cells = vanilla.get("cells")
    so_cells = krasnokamen.get("cells")
    if va_cells is not None or so_cells is not None:
        if va_cells is None or so_cells is None:
            missing = "vanilla" if va_cells is None else "Krasnokamen"
            rows.append((f"{prefix}cells", va_cells is not None, so_cells is not None))
            return {"cells": {"error": f"per-cell snapshot missing from {missing}"}}
        if len(va_cells) != len(so_cells):
            rows.append((f"{prefix}cells", len(va_cells), len(so_cells)))
            return {"cells": {"error":
                              f"cell count differs: vanilla {len(va_cells)}, Krasnokamen {len(so_cells)}"}}
        cells = {}
        for index, (va, so) in enumerate(zip(va_cells, so_cells)):
            diff = compare_cell(spec, va, so, rows, f"{prefix}cells/{index}/", wildcards)
            if diff:
                cells[str(index)] = diff
        return {"cells": cells} if cells else {}
    return compare_cell(spec, vanilla, krasnokamen, rows, prefix, wildcards)


def compare_cell(spec: dict, vanilla: dict, krasnokamen: dict, rows: list[Row] | None = None,
                 prefix: str = "", wildcards: frozenset[str] = frozenset()) -> dict[str, Any]:
    rows = [] if rows is None else rows
    out: dict[str, Any] = {}
    if spec.get("blocks", "exact") == "exact":
        diffs = [d for d in block_diff(vanilla["blocks"], krasnokamen["blocks"], spec.get("ignore_blocks", []))
                 if f"{prefix}blocks/{d['pos']}" not in wildcards]
        rows += [(f"{prefix}blocks/{d['pos']}", d["vanilla"], d["krasnokamen"]) for d in diffs]
        if diffs:
            out["blocks"] = {"count": len(diffs), "first": diffs[:MAX_DIFFS]}
    if spec.get("containers", "exact") == "exact":
        a, b = vanilla.get("containers", {}), krasnokamen.get("containers", {})
        found = {pos: {"vanilla": a.get(pos), "krasnokamen": b.get(pos)}
                 for pos in sorted(set(a) | set(b))
                 if a.get(pos) != b.get(pos) and f"{prefix}containers/{pos}" not in wildcards}
        rows += [(f"{prefix}containers/{pos}", row["vanilla"], row["krasnokamen"]) for pos, row in found.items()]
        if found:
            out["containers"] = found
    if spec.get("entities", "counts") == "counts":
        a = entity_counts(vanilla["entities"], spec)
        b = entity_counts(krasnokamen["entities"], spec)
        tolerance = spec.get("entity_tolerance", 0)
        found = {}
        for key in sorted(set(a) | set(b)):
            if abs(a[key] - b[key]) > tolerance and f"{prefix}entities/{key}" not in wildcards:
                found[key] = {"vanilla": a[key], "krasnokamen": b[key]}
                rows.append((f"{prefix}entities/{key}", a[key], b[key]))
        if found:
            out["entities"] = found
    positions = spec.get("entity_positions")
    if positions and f"{prefix}entity_positions" not in wildcards:
        epsilon = positions.get("epsilon", 0.01)
        want = set(positions["types"])
        pa = sorted(tuple(round(c, 6) for c in e["pos"]) for e in vanilla["entities"] if e["type"] in want)
        pb = sorted(tuple(round(c, 6) for c in e["pos"]) for e in krasnokamen["entities"] if e["type"] in want)
        if len(pa) != len(pb) or any(
                any(abs(x - y) > epsilon for x, y in zip(u, v)) for u, v in zip(pa, pb)):
            out["entity_positions"] = {"vanilla": pa[:MAX_DIFFS], "krasnokamen": pb[:MAX_DIFFS]}
            rows.append((f"{prefix}entity_positions", pa[:MAX_DIFFS], pb[:MAX_DIFFS]))
    fields = spec.get("entity_fields")
    if fields:
        epsilon = fields.get("epsilon", 0.01)
        for kind in fields["types"]:
            path = f"{prefix}entity_fields/{kind}"
            if path in wildcards:
                continue
            va, so = (sorted((tuple(e.get(name) for name in fields["fields"])
                              for e in side["entities"] if e["type"] == kind), key=_none_last)
                      for side in (vanilla, krasnokamen))
            if len(va) != len(so) or not all(_close(x, y, epsilon) for u, v in zip(va, so) for x, y in zip(u, v)):
                out.setdefault("entity_fields", {})[kind] = {"vanilla": va[:MAX_DIFFS], "krasnokamen": so[:MAX_DIFFS]}
                rows.append((path, va[:MAX_DIFFS], so[:MAX_DIFFS]))
    return out


def compare_scenario(scenario: dict, vanilla: dict, krasnokamen: dict, rows: list[Row] | None = None,
                     wildcards: frozenset[str] = frozenset()) -> dict[str, Any]:
    """Return {"phases": {name: diff}, "match": bool}.

    Traces are compared only when the scenario root enables ``trace`` — the same
    key the drivers read to record them; ``compare`` holds comparison rules, not
    the trace switch.

    `rows` receives the differences as observable paths in timeline order: phases in
    receipt order, a phase's per-tick trace before its end-of-phase snapshot, and inside
    one snapshot blocks, containers, entity counts, entity positions. The first row of a
    timeline is where the servers started to differ (`mismatch_groups`).
    """
    rows = [] if rows is None else rows
    spec = scenario.get("compare", {})
    only = set(spec.get("phases", []))
    phases: dict[str, Any] = {}
    by_name = {phase["name"]: phase for phase in krasnokamen["phases"]}
    for index, phase in enumerate(vanilla["phases"]):
        name = phase["name"]
        if only and name not in only:
            continue
        other = by_name.get(name)
        if other is None:
            phases[name] = {"error": "phase missing from Krasnokamen receipt"}
            rows.append((name, "present", None))
            continue
        trace_diff: dict[str, Any] | None = None
        # Both drivers record no trace for the initial "setup" phase (always the
        # first receipt entry) but record one for every declared phase when the
        # scenario enables traces, even when the phase has zero ticks. A
        # one-sided trace stays a difference; so does a declared phase where
        # both receipts omit the trace entirely.
        if scenario.get("trace"):
            va_trace, so_trace = phase.get("trace"), other.get("trace")
            if va_trace is None and so_trace is None:
                if not (index == 0 and name == "setup"):
                    trace_diff = {"error": "trace missing from both phase receipts"}
                    rows.append((f"{name}/trace", None, None))
            elif va_trace is None or so_trace is None:
                missing = "vanilla" if va_trace is None else "Krasnokamen"
                trace_diff = {"error": f"trace missing from {missing} phase receipt"}
                rows.append((f"{name}/trace", va_trace is not None, so_trace is not None))
            elif len(va_trace) != len(so_trace):
                trace_diff = {"error":
                              f"trace length differs: vanilla {len(va_trace)}, Krasnokamen {len(so_trace)}"}
                rows.append((f"{name}/trace", len(va_trace), len(so_trace)))
            else:
                ticks = {}
                for tick, (va, so) in enumerate(zip(va_trace, so_trace)):
                    tick_diff = compare_snapshot(spec, va, so, rows, f"{name}/trace/{tick + 1}/", wildcards)
                    if tick_diff:
                        ticks[str(tick + 1)] = tick_diff
                if ticks:
                    trace_diff = ticks
        diff = compare_snapshot(spec, phase["snapshot"], other["snapshot"], rows, f"{name}/", wildcards)
        if trace_diff is not None:
            diff["trace"] = trace_diff
        if diff:
            phases[name] = diff
    # Both drivers emit the same phase names for one scenario; an extra phase on
    # the Krasnokamen side is a receipt-shape difference, not a silent skip.
    # compare.phases still limits the check to the selected phase names.
    vanilla_names = {phase["name"] for phase in vanilla["phases"]}
    for phase in krasnokamen["phases"]:
        name = phase["name"]
        if name in vanilla_names or (only and name not in only):
            continue
        phases[name] = {"error": "phase not present in vanilla receipt"}
        rows.append((name, None, "present"))
    return {"phases": phases, "match": not phases}


def mismatch_id(case_id: str, path: str) -> str:
    """Stable id of one divergence: the same first differing path of the same case keeps its id."""
    return hashlib.sha256(f"{case_id}\n{path}".encode()).hexdigest()[:12]


def mismatch_groups(case_id: str, rows: list[Row], limit: int = 20) -> list[dict[str, Any]]:
    """Group difference rows by the first differing path of each independent timeline.

    Cells of a multi-box scenario are independent timelines; everything else is one.
    Later rows of a timeline are usually consequences of its first one, so they are
    listed under it instead of becoming separate mismatches.
    """
    groups: dict[str, dict[str, Any]] = {}
    for path, vanilla, krasnokamen in rows:
        parts = path.split("/")
        cell = next((f"cells/{parts[i + 1]}" for i, part in enumerate(parts[:-1])
                     if part == "cells" and parts[i + 1].isdigit()), "")
        group = groups.get(cell)
        if group is None:
            groups[cell] = {"mismatch_id": mismatch_id(case_id, path), "path": path,
                            "vanilla": vanilla, "krasnokamen": krasnokamen, "rows": 1, "later_paths": []}
            continue
        group["rows"] += 1
        if len(group["later_paths"]) < limit:
            group["later_paths"].append(path)
    return list(groups.values())


def check_vanilla(scenario: dict, vanilla: dict) -> list[str]:
    """Evaluate vanilla_expect: proves the oracle really shows the quirk."""
    failures = []
    by_name = {phase["name"]: phase["snapshot"] for phase in vanilla["phases"]}
    spec = scenario.get("compare", {})
    for check in scenario.get("vanilla_expect", []):
        snapshot = by_name.get(check["phase"])
        if snapshot is None:
            failures.append(f"{check}: no such phase")
            continue
        if "cell" in check:
            snapshot = snapshot["cells"][check["cell"]]
        if check["check"] == "block":
            pos = ",".join(str(v) for v in check["pos"])
            actual = snapshot["blocks"].get(pos)
            if actual != check.get("equals"):
                failures.append(f"block {pos}: expected {check.get('equals')!r}, vanilla has {actual!r}")
        elif check["check"] == "count":
            actual = entity_counts(snapshot["entities"], spec)[check["key"]]
            if actual < check.get("min", 0) or actual > check.get("max", 10**9):
                failures.append(
                    f"count {check['key']}: expected {check.get('min', 0)}..{check.get('max', '*')}, vanilla has {actual}")
        else:
            failures.append(f"unknown vanilla_expect check {check['check']!r}")
    return failures


def rate_metric(spec: dict, snapshot: dict) -> float:
    """One scalar from a snapshot: container/item/entity totals for farm production."""
    kind = spec["kind"]
    if kind == "container_items":
        item = spec["item"]
        return float(sum(totals.get(item, 0) for totals in snapshot.get("containers", {}).values()))
    if kind == "item_entities":
        item = spec["item"]
        return float(sum(int(e.get("count", 1)) for e in snapshot["entities"] if e.get("item") == item))
    if kind == "entity_count":
        return float(sum(1 for e in snapshot["entities"] if e["type"] == spec["entity"]))
    if kind == "block_count":
        return float(sum(1 for name in snapshot["blocks"].values() if name.split("[")[0] == spec["block"]))
    if kind == "block_state_count":
        return float(sum(1 for name in snapshot["blocks"].values() if name == spec["equals"]))
    raise ValueError(f"unknown rate metric kind {kind!r}")


def rate_series(scenario: dict, result: dict) -> list[float]:
    """The metric after every phase (setup included) of one run."""
    return [rate_metric(scenario["rate"]["metric"], phase["snapshot"]) for phase in result["phases"]]


def _mean_sd(values: list[float]) -> tuple[float, float]:
    mean = sum(values) / len(values)
    if len(values) < 2:
        return mean, 0.0
    return mean, (sum((v - mean) ** 2 for v in values) / (len(values) - 1)) ** 0.5


def rate_verdict(scenario: dict, vanilla_runs: list[dict], krasnokamen_runs: list[dict]) -> dict[str, Any]:
    """Farm-rate parity: mean production per run, Krasnokamen vs vanilla, over several seeds.

    Passes when Krasnokamen' mean is within `tolerance_pct` of vanilla's, or the gap is
    inside 2 standard errors of the difference (seed noise), whichever is wider.
    `"direction": "drop"` measures consumption instead (baseline minus end value).
    A farm that produces nothing in vanilla is a scenario bug, not a pass.
    """
    spec = scenario["rate"]
    phase = spec.get("phase")
    names = [p["name"] for p in vanilla_runs[0]["phases"]]
    index = names.index(phase) if phase else len(names) - 1
    base = spec.get("baseline_phase")
    base_index = names.index(base) if base else 0

    def totals(runs):
        values = []
        for run in runs:
            series = rate_series(scenario, run)
            delta = series[index] - series[base_index]
            values.append(-delta if spec.get("direction") == "drop" else delta)
        return values

    v, s = totals(vanilla_runs), totals(krasnokamen_runs)
    (vm, vsd), (sm, ssd) = _mean_sd(v), _mean_sd(s)
    out: dict[str, Any] = {
        "metric": spec["metric"], "window": [names[base_index], names[index]],
        "vanilla": {"runs": v, "mean": vm, "sd": vsd}, "krasnokamen": {"runs": s, "mean": sm, "sd": ssd},
    }
    if vm < spec.get("min_vanilla", 1):
        out["error"] = f"vanilla production {vm} below min_vanilla {spec.get('min_vanilla', 1)}: scenario does not run the farm"
        out["match"] = False
        return out
    tolerance = spec.get("tolerance_pct", 5.0) / 100.0 * vm
    stderr = (vsd ** 2 / len(v) + ssd ** 2 / len(s)) ** 0.5
    gap = abs(sm - vm)
    out["ratio"] = sm / vm
    out["gap"] = gap
    out["allowed"] = max(tolerance, 2.0 * stderr)
    out["match"] = gap <= out["allowed"]
    return out
