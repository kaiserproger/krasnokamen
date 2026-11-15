#!/usr/bin/env python3
"""Extract an honest chronicle from native-record observations and tick telemetry.

Input is the spain_trial run.json observation envelope (not a new world schema).
The first census is baseline, never a founding or construction event. Later
record appearances/certificates are dated observation intervals, not exact ticks.
Only an explicit native audit event can establish parent -> daughter lineage;
matching owner UUIDs alone NEVER establish it. Owner UUID is the teaser faction,
not a persistent culture/lineage field. See docs/architecture/settlements.md.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
from typing import Any


def audit_events(log: str) -> list[dict[str, Any]]:
    """Read native construction/founding audit messages, ignoring unrelated logs."""
    events = []
    for line in log.splitlines():
        match = re.search(r"\bsettlement_(building_completed|daughter_founded|founded)\b", line)
        if not match:
            continue
        fields = {
            key: quoted or bare for key, quoted, bare in
            re.findall(r'\b(\w+)=(?:"([^"\n]*)"|([^\s]+))', line)
        }
        required = ["settlement", "tick", "game_time"]
        if match[1] == "building_completed":
            required += ["building", "blueprint", "level"]
        elif match[1] == "daughter_founded":
            required += ["parent"]
        missing = [key for key in required if key not in fields]
        if missing:
            raise ValueError(f"incomplete {match[0]} audit: {missing}")
        row: dict[str, Any] = {"kind": match[1], "evidence": line}
        for key in required:
            row[key] = fields[key] if key == "blueprint" else int(fields[key])
        events.append(row)
    return events


def extract(run: dict[str, Any], telemetry: list[dict[str, Any]], log: str = "",
            milestone: int = 5) -> dict[str, Any]:
    if milestone <= 0:
        raise ValueError("population milestone must be positive")
    snapshots = run["snapshots"]
    if not snapshots:
        raise ValueError("no settlement census observations")
    events: list[dict[str, Any]] = []
    known: dict[int, dict[str, Any]] = {}
    buildings: dict[tuple[int, int], int] = {}
    milestones: set[tuple[int, int]] = set()
    previous = None
    names: dict[int, str] = {}
    owners: dict[int, str | None] = {}
    for index, snapshot in enumerate(snapshots):
        tick, age = snapshot["tick"], snapshot["game_time"]
        if type(tick) is not int or type(age) is not int or min(tick, age) < 0:
            raise ValueError("census tick/game_time must be nonnegative integers")
        if previous and (tick <= previous["tick"] or age < previous["game_time"]):
            raise ValueError("censuses must advance ticks and never reverse game time")
        identities = [town["id"] for town in snapshot["settlements"]]
        if len(identities) != len(set(identities)):
            raise ValueError("duplicate settlement identity in census")
        for town in snapshot["settlements"]:
            sid = town["id"]
            names[sid], owners[sid] = town["name"], town["owner"]
            population = town["population"]
            if type(population) is not int or population < 0:
                raise ValueError("population must be a nonnegative integer")
            interval = {"tick": tick, "game_time": age, "after_tick": previous["tick"] if previous else None,
                        "settlement": sid, "precision": "observed_interval"}
            if sid not in known and index:
                events.append({**interval, "kind": "founded"})
            old_population = known.get(sid, town)["population"]
            for count in range(milestone, population + 1, milestone):
                key = sid, count
                if index and population >= count > old_population and key not in milestones:
                    events.append({**interval, "kind": "population_milestone", "population": count})
                milestones.add(key)
            for building in town["buildings"]:
                key = sid, building["id"]
                old = buildings.get(key, 0)
                level = building["level"]
                # A new town's inherited/imported certificates are its baseline,
                # not new construction merely because their owner/ID changed.
                if sid in known and level > old and building.get("blueprint"):
                    events.append({**interval, "kind": "building_completed", "building": building["id"],
                                   "blueprint": building["blueprint"], "level": level})
                buildings[key] = max(old, level)
            known[sid] = town
        previous = snapshot
    start, end = snapshots[0]["tick"], snapshots[-1]["tick"]
    audits = audit_events(log)
    exact_buildings = set()
    for event in audits:
        if not start <= event["tick"] <= end:
            continue
        sid = event["settlement"]
        if sid not in names:
            raise ValueError(f"audit names settlement absent from run records: {sid}")
        if event["kind"] == "daughter_founded":
            parent = event["parent"]
            if parent not in owners or owners[parent] is None or owners[parent] != owners[sid]:
                raise ValueError("daughter audit does not match same-owner native records")
        if event["kind"] == "building_completed":
            if buildings.get((sid, event["building"]), 0) < event["level"]:
                raise ValueError("completed-building audit lacks a matching certificate")
            exact_buildings.add((sid, event["building"], event["level"]))
        events.append({**event, "precision": "native_audit"})
    exact_foundings = {e["settlement"] for e in events if e["precision"] == "native_audit" and e["kind"] in ("founded", "daughter_founded")}
    events = [e for e in events if e["precision"] == "native_audit" or not (
        (e["kind"] == "building_completed" and (e["settlement"], e["building"], e["level"]) in exact_buildings)
        or (e["kind"] == "founded" and e["settlement"] in exact_foundings))]
    unique = {}
    for event in events:
        key = (event["kind"], event["settlement"], event.get("building"), event.get("level"), event.get("population"), event["tick"])
        unique[key] = event
    events = sorted(unique.values(), key=lambda e: (e["tick"], e["settlement"], e["kind"]))
    for event in events:
        event["day"] = event["game_time"] // 24000
        town = names[event["settlement"]]
        text = {"founded": f"{town}: settlement founded", "daughter_founded": f"{town}: daughter of {names.get(event.get('parent'), '?')} founded",
                "building_completed": f"{town}: {event.get('blueprint')} L{event.get('level')} completed",
                "population_milestone": f"{town}: population reached {event.get('population')}"}[event["kind"]]
        suffix = " (observed by this time)" if event["precision"] == "observed_interval" else ""
        event["caption"] = f"Day {event['day']} · {text}{suffix}"
    samples = [row for row in telemetry if start <= row["tick"] <= end]
    ticks = [row["tick"] for row in samples]
    if any(b <= a for a, b in zip(ticks, ticks[1:])):
        raise ValueError("telemetry must advance without duplicates")
    gaps = sum(b - a - 1 for a, b in zip(ticks, ticks[1:]))
    return {"schema": 1, "seed": run["seed"], "server": run["server"], "events": events,
            "coverage": {"first_tick": start, "last_tick": end, "telemetry_samples": len(samples),
                         "interior_missing_ticks": gaps, "boundary_coverage": bool(ticks) and ticks[0] == start and ticks[-1] == end},
            "limitations": ["Initial census is baseline, not founding or construction.",
                            "Census changes have interval dates; native audit events have exact simulation ages.",
                            "Same owner is the teaser faction, never proof of daughter lineage."]}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", required=True, type=Path)
    parser.add_argument("--telemetry", required=True, type=Path)
    parser.add_argument("--server-log", type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    result = extract(json.loads(args.run.read_text()),
                     [json.loads(line) for line in args.telemetry.read_text().splitlines() if line],
                     args.server_log.read_text() if args.server_log else "")
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "chronicle.json").write_text(json.dumps(result, indent=2) + "\n")
    (args.out / "captions.txt").write_text("\n".join(e["caption"] for e in result["events"]) + "\n")


if __name__ == "__main__":
    main()
