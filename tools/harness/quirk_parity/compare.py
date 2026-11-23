"""Snapshot comparison and oracle self-check for the quirk/farm parity gate."""
from __future__ import annotations

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


def compare_snapshot(spec: dict, vanilla: dict, krasnokamen: dict) -> dict[str, Any]:
    """Compare one snapshot; multi-cell snapshots are compared cell by cell.

    A cell list present on only one side, or with a different length, is itself a
    difference: zipping unequal lists would drop the unmatched tail observations
    and pass a truncated or padded receipt as a match.
    """
    va_cells = vanilla.get("cells")
    so_cells = krasnokamen.get("cells")
    if va_cells is not None or so_cells is not None:
        if va_cells is None or so_cells is None:
            missing = "vanilla" if va_cells is None else "Krasnokamen"
            return {"cells": {"error": f"per-cell snapshot missing from {missing}"}}
        if len(va_cells) != len(so_cells):
            return {"cells": {"error":
                              f"cell count differs: vanilla {len(va_cells)}, Krasnokamen {len(so_cells)}"}}
        cells = {}
        for index, (va, so) in enumerate(zip(va_cells, so_cells)):
            diff = compare_cell(spec, va, so)
            if diff:
                cells[str(index)] = diff
        return {"cells": cells} if cells else {}
    return compare_cell(spec, vanilla, krasnokamen)


def compare_cell(spec: dict, vanilla: dict, krasnokamen: dict) -> dict[str, Any]:
    out: dict[str, Any] = {}
    if spec.get("blocks", "exact") == "exact":
        diffs = block_diff(vanilla["blocks"], krasnokamen["blocks"], spec.get("ignore_blocks", []))
        if diffs:
            out["blocks"] = {"count": len(diffs), "first": diffs[:MAX_DIFFS]}
    if spec.get("entities", "counts") == "counts":
        a = entity_counts(vanilla["entities"], spec)
        b = entity_counts(krasnokamen["entities"], spec)
        tolerance = spec.get("entity_tolerance", 0)
        rows = {}
        for key in sorted(set(a) | set(b)):
            if abs(a[key] - b[key]) > tolerance:
                rows[key] = {"vanilla": a[key], "krasnokamen": b[key]}
        if rows:
            out["entities"] = rows
    if spec.get("containers", "exact") == "exact":
        a, b = vanilla.get("containers", {}), krasnokamen.get("containers", {})
        rows = {pos: {"vanilla": a.get(pos), "krasnokamen": b.get(pos)}
                for pos in sorted(set(a) | set(b)) if a.get(pos) != b.get(pos)}
        if rows:
            out["containers"] = rows
    positions = spec.get("entity_positions")
    if positions:
        epsilon = positions.get("epsilon", 0.01)
        want = set(positions["types"])
        pa = sorted(tuple(round(c, 6) for c in e["pos"]) for e in vanilla["entities"] if e["type"] in want)
        pb = sorted(tuple(round(c, 6) for c in e["pos"]) for e in krasnokamen["entities"] if e["type"] in want)
        if len(pa) != len(pb) or any(
                any(abs(x - y) > epsilon for x, y in zip(u, v)) for u, v in zip(pa, pb)):
            out["entity_positions"] = {"vanilla": pa[:MAX_DIFFS], "krasnokamen": pb[:MAX_DIFFS]}
    return out


def compare_scenario(scenario: dict, vanilla: dict, krasnokamen: dict) -> dict[str, Any]:
    """Return {"phases": {name: diff}, "match": bool}.

    Traces are compared only when the scenario root enables ``trace`` — the same
    key the drivers read to record them; ``compare`` holds comparison rules, not
    the trace switch.
    """
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
            continue
        diff = compare_snapshot(spec, phase["snapshot"], other["snapshot"])
        # Both drivers record no trace for the initial "setup" phase (always the
        # first receipt entry) but record one for every declared phase when the
        # scenario enables traces, even when the phase has zero ticks. A
        # one-sided trace stays a difference; so does a declared phase where
        # both receipts omit the trace entirely.
        if scenario.get("trace"):
            va_trace, so_trace = phase.get("trace"), other.get("trace")
            if va_trace is None and so_trace is None:
                if not (index == 0 and name == "setup"):
                    diff["trace"] = {"error": "trace missing from both phase receipts"}
            elif va_trace is None or so_trace is None:
                missing = "vanilla" if va_trace is None else "Krasnokamen"
                diff["trace"] = {"error": f"trace missing from {missing} phase receipt"}
            elif len(va_trace) != len(so_trace):
                diff["trace"] = {"error":
                                 f"trace length differs: vanilla {len(va_trace)}, Krasnokamen {len(so_trace)}"}
            else:
                rows = {}
                for tick, (va, so) in enumerate(zip(va_trace, so_trace)):
                    tick_diff = compare_snapshot(spec, va, so)
                    if tick_diff:
                        rows[str(tick + 1)] = tick_diff
                if rows:
                    diff["trace"] = rows
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
    return {"phases": phases, "match": not phases}


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
