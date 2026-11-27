"""Vanilla differential oracle: record real vanilla once, replay Krasnokamen against it.

    python3 -m tools.harness.quirk_parity [replay] --out DIR [--scenario ID ...] [--family F ...]
    python3 -m tools.harness.quirk_parity record --out DIR [--scenario ID ...] [--family F ...]
        [--fuzz-seed N --fuzz-cells C --fuzz-style S] [--jar PATH] [--java PATH]
    python3 -m tools.harness.quirk_parity re-record --out DIR --scenario ID --reason TEXT
    python3 -m tools.harness.quirk_parity verify
    (all take [--mc-version 26.3] [--corpus DIR])

`record` runs every not yet recorded case twice on real vanilla and appends it to the
append-only corpus (`corpus.py`); a path where the two recordings differ becomes a
wildcard that accepts only the values they showed. `replay` (the gate) checks corpus
integrity, runs only Krasnokamen, compares observable state and groups mismatches by
first differing path under stable ids. A
mismatch listed in `known-diffs.json` is reported as `known-divergence`; a listed mismatch
that no longer occurs fails the gate, as does any other mismatch, a Krasnokamen command
rejection, an unrecorded or changed scenario, or a corpus integrity problem.
DIR/receipt.json holds the verdicts; the process exits non-zero unless it passes.
"""
from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import subprocess
import sys
import time
from pathlib import Path

from . import compare, seeded
from .corpus import FORMAT, LOCK, Corpus, CorpusError, corpus_root, scenario_sha256, trusted_lock
from .vanilla import (MEM_RUN, REPO_ROOT, VanillaError, VanillaOracle, java_version, resolve_jar,
                      resolve_java)

SCENARIO_DIR = Path(__file__).with_name("scenarios")
KNOWN_DIFFS = Path(__file__).with_name("known-diffs.json")
TEST_NAME = "quirk_parity_tests::quirk_parity_krasnokamen_replay"
DEFAULT_MC = "26.3"
COMMANDS = ("replay", "record", "re-record", "verify")

# Same rules on both servers before any scenario command; scenarios opt out with
# "prelude": false when they need spawning, random ticks or weather.
PRELUDE = [
    "gamerule spawn_mobs false",
    "gamerule random_tick_speed 0",
    "gamerule advance_time false",
    "gamerule advance_weather false",
    "weather clear",
    "time set noon",
]


def load_scenarios(only: list[str], families: list[str]) -> list[dict]:
    scenarios = []
    for path in sorted(SCENARIO_DIR.glob("*.json")):
        scenario = json.loads(path.read_text())
        scenario.setdefault("id", path.stem)
        if only and scenario["id"] not in only and not any(o.startswith(scenario["id"] + "#") for o in only):
            continue
        if families and scenario.get("family") not in families:
            continue
        scenario["_path"] = str(path)
        scenarios.append(scenario)
    return [s for s in expand_seeds(scenarios) if not only or s["id"] in only or s.get("_group") in only]


def expand_seeds(scenarios: list[dict]) -> list[dict]:
    """A scenario with `seeds` runs once per seed on both servers (`_group` = base id)."""
    out = []
    for scenario in scenarios:
        seeds = scenario.get("seeds")
        if not seeds:
            out.append(scenario)
            continue
        for seed in seeds:
            variant = dict(scenario, id=f"{scenario['id']}#s{seed}", seed=seed, _group=scenario["id"])
            variant.pop("seeds")
            out.append(variant)
    return out


def resolved(scenario: dict) -> dict:
    """What both servers execute: the scenario with the common prelude folded into setup."""
    out = {k: v for k, v in scenario.items() if not k.startswith("_")}
    if out.pop("prelude", True):
        out["setup"] = PRELUDE + list(scenario.get("setup", []))
    if scenario.get("_group"):
        out["group"] = scenario["_group"]
    return out


def fuzz_scenarios(args) -> list[dict]:
    return [seeded.timing_scenario(seed, args.fuzz_cells) if args.fuzz_style == "timing"
            else seeded.bud_scenario(seed, args.fuzz_cells, "slime3d" if args.fuzz_style == "slime3d" else "line")
            if args.fuzz_style in ("bud", "slime3d")
            else seeded.fuzz_scenario(seed, args.fuzz_cells, args.fuzz_style) for seed in args.fuzz_seed]


# ----------------------------------------------------------------------------- servers


def _prepare(oracle: VanillaOracle, boxes: list, *, restarted: bool) -> None:
    """Freeze time and make every box resident.

    A fresh world loads its forced chunks while sprinting real ticks (it holds nothing yet).
    A restarted world runs unfrozen from startup (vanilla does not persist `tick freeze`) and
    reloads its persisted forced chunks on its own; it is frozen once they are resident, never
    sprinted, so the recorded state does not advance.
    """
    if not restarted:
        oracle.run_checked(["tick freeze"])
    for x1, _, z1, x2, _, z2 in boxes:
        oracle.run_checked([f"forceload add {x1} {z1} {x2} {z2}"])
    for box in boxes:
        for _ in range(200):
            lines = oracle.run(f"execute if loaded {box[3]} {box[4]} {box[5]} run say QP_LOADED")
            if any("QP_LOADED" in line for line in lines):
                break
            if not restarted:
                oracle.load_ticks(20)
        else:
            raise VanillaError("forceloaded box never became resident")
    if restarted:
        oracle.run_checked(["tick freeze"])
    oracle.step_marker = (boxes[0][0], boxes[0][2])


def run_vanilla(scenario: dict, workdir: Path, jar: Path, java: str) -> dict:
    """One vanilla recording of a resolved scenario."""
    boxes = scenario.get("boxes") or [scenario["box"]]
    started = time.monotonic()
    result = {"id": scenario["id"], "server": "vanilla", "phases": [], "command_log": []}
    with VanillaOracle(workdir, seed=scenario.get("seed", 1), jar=jar, java=java) as oracle:
        _prepare(oracle, boxes, restarted=False)
        result["command_log"] += oracle.run_checked(scenario.get("setup", []))
        oracle.sprint(scenario.get("settle_ticks", 1))
        result["phases"].append({"name": "setup", "tick": 0, "snapshot": oracle.snapshot(boxes)})
        trace = scenario.get("trace", False)
        for phase in scenario.get("phases", []):
            if phase.get("restart"):
                oracle.restart()
                _prepare(oracle, boxes, restarted=True)
            result["command_log"] += oracle.run_checked(phase.get("commands", []))
            rows = []
            ticks = phase.get("ticks", 0)
            if trace:
                for _ in range(ticks):
                    oracle.sprint(1)
                    rows.append(oracle.snapshot(boxes))
            else:
                oracle.sprint(ticks)
            entry = {"name": phase["name"], "tick": oracle.game_ticks, "snapshot": oracle.snapshot(boxes)}
            if trace:
                entry["trace"] = rows
            result["phases"].append(entry)
    result["wall_ms"] = int((time.monotonic() - started) * 1000)
    return result


def run_krasnokamen(scenarios: list[dict], out: Path, restart_fault: str | None = None) -> dict[str, dict]:
    """Krasnokamen observations of resolved scenarios: the in-process native tick driver.

    Restart scenarios keep their native worlds under DIR/krasnokamen/worlds; `restart_fault`
    damages each store between the stop and the reopen (negative control: they must fail)."""
    krasnokamen_dir = out / "krasnokamen"
    krasnokamen_dir.mkdir(parents=True, exist_ok=True)
    paths = []
    for scenario in scenarios:
        path = krasnokamen_dir / f"{scenario['id'].replace('#', '@')}.json"
        path.write_text(json.dumps(scenario, indent=1))
        paths.append(str(path))
    receipt = krasnokamen_dir / "receipt.json"
    env = dict(os.environ, KRASNOKAMEN_QUIRK_SCENARIOS=":".join(paths), KRASNOKAMEN_QUIRK_RECEIPT=str(receipt),
               KRASNOKAMEN_QUIRK_WORLDS=str(krasnokamen_dir / "worlds"))
    env.pop("KRASNOKAMEN_QUIRK_RESTART_FAULT", None)
    if restart_fault:
        env["KRASNOKAMEN_QUIRK_RESTART_FAULT"] = restart_fault
    command = ["cargo", "test", "-p", "mc-core", "--lib", TEST_NAME, "--", "--ignored", "--exact", "--nocapture"]
    if MEM_RUN.is_file():
        command = [str(MEM_RUN), "-m", "12G"] + command
    log = krasnokamen_dir / "krasnokamen.log"
    with log.open("w") as handle:
        proc = subprocess.run(command, cwd=REPO_ROOT, env=env, stdout=handle,
                              stderr=subprocess.STDOUT, check=False)
    if proc.returncode or not receipt.is_file():
        raise RuntimeError(f"Krasnokamen replay failed (exit {proc.returncode}); see {log}")
    return {row["id"]: row for row in json.loads(receipt.read_text())}


# ----------------------------------------------------------------------------- verdicts

# Krasnokamen success/no-op feedback that carries no rejection (vanilla's "Could not set the block"
# no-op is likewise ignored by the vanilla driver).
BENIGN_KRASNOKAMEN = ("Changed ", "No blocks were changed", "Successfully filled", "Summoned ")


def krasnokamen_rejections(run: dict) -> list[str]:
    return [row for row in run["command_log"]
            if not row.split(" => ", 1)[-1].startswith(BENIGN_KRASNOKAMEN)]


def load_known_diffs(path: Path | None = None) -> dict[str, dict]:
    """Case -> {"reason", "mismatches": [ids]}: intended or accepted differences from vanilla."""
    path = KNOWN_DIFFS if path is None else path
    if not path.is_file():
        return {}
    known = {key: value for key, value in json.loads(path.read_text()).items() if not key.startswith("_")}
    for case, value in known.items():
        if not isinstance(value, dict) or not value.get("reason") or not value.get("mismatches"):
            raise ValueError(f"{path}: {case} needs a reason and the mismatch ids it covers")
    return known


def apply_known(entry: dict, groups: list[dict], known: dict | None, reasons: list[str]) -> None:
    """Every mismatch must be listed with a reason; a listed mismatch that is gone fails too."""
    observed = {group["mismatch_id"] for group in groups}
    listed = set(known["mismatches"]) if known else set()
    for group in groups:
        group["status"] = "known" if group["mismatch_id"] in listed else "new"
    if groups:
        entry["mismatches"] = groups
    unknown, stale = observed - listed, sorted(listed - observed)
    if unknown:
        reasons.append("Krasnokamen diverges from vanilla")
    if stale:
        reasons.append(f"known divergence no longer differs, remove it from known-diffs.json: {stale}")
    if observed and not unknown and not stale:
        entry["status"] = "known-divergence"
        entry["known_divergence"] = known["reason"]


def verdict(scenario: dict, vanilla: dict, krasnokamen: dict | None, *,
            wildcards: compare.Wildcards = compare.NO_WILDCARDS, known: dict | None = None) -> dict:
    """Replay verdict of one exact-state case against its first vanilla recording."""
    entry: dict = {"id": scenario["id"], "family": scenario.get("family"), "status": "pass"}
    reasons: list[str] = []
    if krasnokamen is None:
        entry.update(status="fail", reasons=["Krasnokamen produced no observation for this case"])
        return entry
    entry["krasnokamen_wall_ms"] = krasnokamen.get("wall_ms")
    if krasnokamen_rejections(krasnokamen) and not scenario.get("allow_krasnokamen_command_errors"):
        reasons.append("Krasnokamen rejected commands vanilla accepted")
        entry["krasnokamen_command_errors"] = krasnokamen_rejections(krasnokamen)[:20]
    if krasnokamen.get("error"):
        reasons.append(f"Krasnokamen run failed: {krasnokamen['error']}")
    restarted = {phase["name"] for phase in krasnokamen["phases"] if phase.get("restarted")}
    skipped = [phase["name"] for phase in scenario.get("phases", []) if phase.get("restart")
               and phase["name"] not in restarted]
    if skipped:
        reasons.append(f"Krasnokamen did not checkpoint and recover before restart phase(s) {skipped}")
    rows: list = []
    if scenario.get("fuzz"):
        fuzz_verdict(scenario, vanilla, krasnokamen, entry, reasons, wildcards, rows)
        return entry
    diff = compare.compare_scenario(scenario, vanilla, krasnokamen, rows, wildcards)
    if not diff["match"]:
        entry["diff"] = diff["phases"]
    apply_known(entry, compare.mismatch_groups(scenario["id"], rows), known, reasons)
    if reasons:
        entry["status"] = "fail"
        entry["reasons"] = reasons
    return entry


def rate_group_verdict(group: str, variants: list[dict], vanilla: dict[str, list[dict]],
                       krasnokamen: dict[str, dict], known: dict | None = None) -> dict:
    """Farm-rate parity over seed variants; every vanilla recording of every seed is a sample."""
    scenario = variants[0]
    entry: dict = {"id": group, "family": scenario.get("family"), "status": "pass",
                   "seeds": [v["seed"] for v in variants]}
    reasons = []
    vruns = [run for v in variants for run in vanilla.get(v["id"], [])]
    sruns = [krasnokamen[v["id"]] for v in variants if v["id"] in krasnokamen]
    for run in vruns:
        if run["command_log"]:
            reasons.append("vanilla rejected setup commands (scenario bug)")
            entry["vanilla_command_errors"] = run["command_log"]
            break
    for variant in variants:
        for run in vanilla.get(variant["id"], []):
            failures = compare.check_vanilla(variant, run)
            if failures:
                entry.setdefault("vanilla_selfcheck_failures", {})[variant["id"]] = failures
    if entry.get("vanilla_selfcheck_failures"):
        reasons.append("vanilla does not show the quirk this scenario encodes")
    for run in sruns:
        if krasnokamen_rejections(run) and not scenario.get("allow_krasnokamen_command_errors"):
            reasons.append("Krasnokamen rejected commands vanilla accepted")
            entry["krasnokamen_command_errors"] = krasnokamen_rejections(run)[:20]
            break
    if vruns and len(sruns) == len(variants) and all(v["id"] in vanilla for v in variants):
        entry["rate"] = compare.rate_verdict(scenario, vruns, sruns)
        groups = []
        if not entry["rate"]["match"]:
            path = "rate/" + "/".join(str(v) for v in entry["rate"]["metric"].values())
            groups.append({"mismatch_id": compare.mismatch_id(group, path), "path": path,
                           "vanilla": entry["rate"]["vanilla"]["mean"],
                           "krasnokamen": entry["rate"]["krasnokamen"]["mean"], "rows": 1, "later_paths": []})
            if entry["rate"].get("error"):
                reasons.append(entry["rate"]["error"])
        apply_known(entry, groups, known, reasons)
    else:
        reasons.append("rate group incomplete: every seed needs vanilla recordings and a Krasnokamen run")
    if reasons:
        entry["status"], entry["reasons"] = "fail", reasons
    return entry


def fuzz_verdict(scenario: dict, vanilla: dict, krasnokamen: dict, entry: dict, reasons: list,
                 wildcards: compare.Wildcards = compare.NO_WILDCARDS, rows: list | None = None) -> dict:
    """Fuzz gate: every vanilla duplication cell must match; other cell diffs are findings."""
    rows = [] if rows is None else rows
    diff = compare.compare_scenario(scenario, vanilla, krasnokamen, rows, wildcards)
    dupes = seeded.dupe_cells(scenario, vanilla)
    diverging: dict[int, list[str]] = {}
    shape: list[str] = []
    for phase, detail in diff["phases"].items():
        for key, value in detail.items():
            if key == "cells":
                continue
            if isinstance(value, str):
                shape.append(f"{phase}: {key}: {value}")
            elif isinstance(value, dict) and set(value) == {"error"}:
                shape.append(f"{phase}: {key}: {value['error']}")
            else:
                shape.append(f"{phase}: {key} difference outside per-cell detail")
        for index, cell_detail in detail.get("cells", {}).items():
            if index.isdigit():
                diverging.setdefault(int(index), []).append(phase)
            else:
                shape.append(f"{phase}: {cell_detail}")
    cells_meta = {c["index"]: c for c in scenario["fuzz"]["cells"]}
    entry["fuzz"] = {
        "seed": scenario["fuzz"]["seed"], "cells": len(cells_meta),
        "vanilla_duplication_cells": {str(i): p for i, p in sorted(dupes.items())},
        "diverging_cells": {str(i): p for i, p in sorted(diverging.items())},
        "diverging_duplication_cells": sorted(set(dupes) & set(diverging)),
    }
    entry["diff_cells"] = {str(i): {"phases": diverging[i], "blocks": cells_meta[i]["blocks"],
                                    "detail": {ph: diff["phases"][ph]["cells"][str(i)] for ph in diverging[i]}}
                           for i in sorted(diverging)[:25]}
    groups = compare.mismatch_groups(scenario["id"], rows)
    if groups:
        entry["mismatches"] = groups
    if set(dupes) & set(diverging):
        reasons.append("Krasnokamen diverges from vanilla on a vanilla TNT-duplication cell")
    elif diverging:
        entry["status"] = "findings"
    if shape:
        reasons.append("comparison shape differs: " + "; ".join(shape[:5]))
    if reasons:
        entry["status"] = "fail"
        entry["reasons"] = reasons
    return entry


# ----------------------------------------------------------------------------- commands


def _is_shape(path: str) -> bool:
    return "/" not in path or path.rsplit("/", 1)[-1] in ("cells", "trace")


def recording_problems(case: dict, recordings: list[dict]) -> tuple[list[str], dict[str, list]]:
    """(problems that refuse the recording, {path: [value in recording 1, value in recording 2]}).

    The wildcards are the element paths (a block, a container, one entity's position or field)
    where the two recordings differ, with the values each showed: replay accepts only those.
    """
    problems = []
    for index, run in enumerate(recordings, 1):
        if run["command_log"]:
            problems.append(f"recording {index}: vanilla rejected commands (scenario bug): {run['command_log'][:5]}")
        failures = compare.check_vanilla(case, run)
        if failures:
            problems.append(f"recording {index}: vanilla does not show what the scenario encodes: {failures[:5]}")
    if case.get("rate"):  # seed variants compare production distributions, not paths
        return problems, {}
    rows: list = []
    compare.compare_scenario(case, recordings[0], recordings[1], rows)
    shape = [path for path, _, _ in rows if _is_shape(path)]
    if shape:
        problems.append(f"the two vanilla recordings differ in shape, not values: {shape[:5]}")
    return problems, {path: [first, second] for path, first, second in sorted(rows, key=lambda row: row[0])}


def recorded_wildcards(doc: dict) -> dict[str, list]:
    """A corpus case's wildcards; a legacy path list without the recorded values cannot be checked."""
    wildcards = doc["wildcards"]
    if isinstance(wildcards, dict):
        return wildcards
    if wildcards:
        raise CorpusError(f"{doc['case']} v{doc['version']} keeps wildcard paths without the values vanilla "
                          "showed there: re-record it")
    return {}


def _environment(args) -> dict:
    jar = resolve_jar(args.mc_version, args.jar)
    java = resolve_java(args.java)
    return {"jar": str(jar), "jar_sha1": hashlib.sha1(jar.read_bytes()).hexdigest(),
            "jar_sha256": hashlib.sha256(jar.read_bytes()).hexdigest(),
            "java": java, "java_version": java_version(java), "mc_version": args.mc_version}


def _head() -> str | None:
    proc = subprocess.run(["git", "rev-parse", "HEAD"], cwd=REPO_ROOT, capture_output=True, text=True, check=False)
    return proc.stdout.strip() or None


def _record_case(case: dict, version: int, out: Path, env: dict, reason: str | None) -> dict:
    recordings = [run_vanilla(case, out / "vanilla" / case["id"].replace("#", "@") / f"v{version}-rec{n}",
                              Path(env["jar"]), env["java"]) for n in (1, 2)]
    problems, wildcards = recording_problems(case, recordings)
    if problems:
        raise VanillaError("; ".join(problems))
    return {"format": FORMAT, "case": case["id"], "version": version, "mc_version": env["mc_version"],
            "group": case.get("group"), "scenario": case, "scenario_sha256": scenario_sha256(case),
            "recordings": recordings, "wildcards": wildcards, "vanilla": env, "reason": reason,
            "recorded_at": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
            "recorded_from": {"repo": str(REPO_ROOT), "head": _head()}}


def record(args, rerecord: bool = False) -> int:
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    corpus = Corpus(corpus_root(args.corpus), args.mc_version)
    receipt: dict = {"command": "re-record" if rerecord else "record", "mc_version": args.mc_version,
                     "corpus": str(corpus.dir), "lock": str(LOCK), "cases": [], "status": "fail"}
    try:
        state = verify_against_trusted(corpus, args)
        if state["problems"]:
            raise CorpusError("corpus fails verification: " + "; ".join(state["problems"][:5]))
        scenarios = [resolved(s) for s in load_scenarios(args.scenario, args.family)] if (
            args.scenario or args.family or not args.fuzz_seed) else []
        scenarios += [resolved(s) for s in fuzz_scenarios(args)]
        if rerecord and (not args.reason or not args.scenario):
            raise CorpusError("re-record needs --scenario and --reason")
        if not scenarios:
            raise CorpusError("no scenarios selected")
        env = _environment(args)
        receipt["vanilla"] = env
        unanchored = {(e["case"], e["scenario_sha256"]): e for e in state["unanchored"]}
        for case in scenarios:
            row: dict = {"id": case["id"]}
            receipt["cases"].append(row)
            locked = state["latest"].get(case["id"])
            sha = scenario_sha256(case)
            if rerecord:
                if locked is None:
                    row.update(status="fail", reason="not recorded yet: use record")
                    continue
                try:
                    doc = _record_case(case, locked["version"] + 1, out, env, args.reason)
                except VanillaError as error:
                    row.update(status="fail", reason=str(error))
                    continue
                kept = corpus.keep_manifest(f"before-v{locked['version'] + 1}-{case['id'].replace('#', '@')}")
                doc["supersedes"] = {"version": locked["version"], "sha256": locked["sha256"]}
                entry = corpus.append(doc, op="re-record", reason=args.reason)
                row.update(status="re-recorded", version=entry["version"], sha256=entry["sha256"],
                           kept_manifest=str(kept), wildcards=len(doc["wildcards"]))
                continue
            if locked is not None:
                if locked["scenario_sha256"] == sha:
                    row.update(status="recorded-before", version=locked["version"])
                else:
                    row.update(status="fail", reason=f"scenario changed since v{locked['version']} was "
                                                     "recorded: re-record --scenario ID --reason TEXT")
                continue
            other = unanchored.get((case["id"], sha))
            if other is not None:  # recorded from another tree: adopt the same bytes
                corpus.anchor(other)
                row.update(status="anchored", version=other["version"], sha256=other["sha256"])
                continue
            try:
                doc = _record_case(case, 1, out, env, None)
            except VanillaError as error:
                row.update(status="fail", reason=str(error))
                continue
            entry = corpus.append(doc, op="record")
            row.update(status="recorded", version=1, sha256=entry["sha256"], wildcards=len(doc["wildcards"]))
        receipt["status"] = "pass" if all(row["status"] != "fail" for row in receipt["cases"]) else "fail"
    except Exception as error:  # fail closed, keep the receipt
        receipt["error"] = f"{type(error).__name__}: {error}"
    (out / "receipt.json").write_text(json.dumps(receipt, indent=1))
    print(json.dumps({row["id"]: row["status"] for row in receipt["cases"]}, indent=1))
    print(f"quirk-parity {receipt['command']}:", receipt["status"], receipt.get("error", ""))
    return 0 if receipt["status"] == "pass" else 1


def replay(args) -> int:
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    corpus = Corpus(corpus_root(args.corpus), args.mc_version)
    receipt: dict = {"profile": "quirk-parity", "command": "replay", "mc_version": args.mc_version,
                     "corpus": str(corpus.dir), "lock": str(LOCK), "scenarios": [], "mismatches": [],
                     "restart_fault": args.restart_fault, "status": "fail"}
    try:
        state = verify_against_trusted(corpus, args)
        receipt["integrity"] = integrity_summary(state, args)
        if state["problems"]:
            raise CorpusError("corpus integrity: " + "; ".join(state["problems"][:5]))
        known = load_known_diffs()
        docs: dict[str, dict] = {}
        for scenario in load_scenarios(args.scenario, args.family):
            entry = state["latest"].get(scenario["id"])
            if entry is None:
                receipt["scenarios"].append({"id": scenario["id"], "status": "fail",
                                             "reasons": ["not recorded: run `record` first"]})
            elif entry["scenario_sha256"] != scenario_sha256(resolved(scenario)):
                receipt["scenarios"].append({"id": scenario["id"], "status": "fail", "reasons": [
                    f"scenario changed since v{entry['version']} was recorded: a changed setup, compare rule "
                    "or tolerance needs `re-record --scenario ID --reason TEXT`"]})
            else:
                docs[scenario["id"]] = corpus.load(entry)
        files = {scenario["id"] for scenario in load_scenarios([], [])}
        for case, entry in sorted(state["latest"].items()):
            if case in files or case in docs:
                continue
            doc = corpus.load(entry)  # generated (fuzz) cases live only in the corpus
            family = doc["scenario"].get("family")
            if (not args.scenario or case in args.scenario) and (not args.family or family in args.family):
                docs[case] = doc
        if not docs and not receipt["scenarios"]:
            raise CorpusError("no recorded cases selected")
        krasnokamen = run_krasnokamen([doc["scenario"] for doc in docs.values()], out,
                                      args.restart_fault) if docs else {}
        groups: dict[str, list[dict]] = {}
        for case, doc in docs.items():
            if doc.get("group"):
                groups.setdefault(doc["group"], []).append(doc)
                continue
            receipt["scenarios"].append(verdict(
                doc["scenario"], doc["recordings"][0], krasnokamen.get(case),
                wildcards=recorded_wildcards(doc), known=known.get(case)))
            receipt["scenarios"][-1]["recorded_version"] = doc["version"]
        for group, members in groups.items():
            receipt["scenarios"].append(rate_group_verdict(
                group, [doc["scenario"] for doc in members],
                {doc["case"]: doc["recordings"] for doc in members}, krasnokamen, known.get(group)))
        if not args.scenario and not args.family:
            selected = {row["id"] for row in receipt["scenarios"]}
            for case in sorted(set(known) - selected):
                receipt["scenarios"].append({"id": case, "status": "fail", "reasons": [
                    "known-diffs.json names a case that is not recorded"]})
        for row in receipt["scenarios"]:
            for group in row.get("mismatches", []):
                receipt["mismatches"].append({"case": row["id"], **{k: group[k] for k in (
                    "mismatch_id", "path", "vanilla", "krasnokamen", "rows")}, "status": group.get("status", "finding")})
        failed = [row for row in receipt["scenarios"] if row["status"] == "fail"]
        receipt["status"] = "pass" if not failed else "fail"
    except Exception as error:  # fail closed, keep the receipt
        receipt["error"] = f"{type(error).__name__}: {error}"
    (out / "receipt.json").write_text(json.dumps(receipt, indent=1))
    print(json.dumps({row["id"]: row["status"] for row in receipt["scenarios"]}, indent=1))
    print("quirk-parity:", receipt["status"], receipt.get("error", ""))
    return 0 if receipt["status"] == "pass" else 1


def verify_against_trusted(corpus: Corpus, args) -> dict:
    return corpus.verify(trusted=trusted_lock(args.mc_version, args.trusted_ref, REPO_ROOT))


def integrity_summary(state: dict, args) -> dict:
    return {"locked_cases": len(state["latest"]), "manifest_entries": state["entries"],
            "manifest_head": state["manifest_head"], "problems": state["problems"],
            "unanchored": sorted({e["case"] for e in state["unanchored"]}), "trusted_ref": args.trusted_ref,
            "re_recorded_since_trusted": state["re_recorded"], "added_since_trusted": state["added"]}


def verify(args) -> int:
    corpus = Corpus(corpus_root(args.corpus), args.mc_version)
    try:
        state = verify_against_trusted(corpus, args)
    except CorpusError as error:
        print(f"quirk-parity verify: {error}")
        return 1
    print(json.dumps({"corpus": str(corpus.dir), **integrity_summary(state, args)}, indent=1))
    return 1 if state["problems"] else 0


def parse_args(argv: list[str] | None = None) -> tuple[str, argparse.Namespace]:
    argv = list(sys.argv[1:] if argv is None else argv)
    command = argv.pop(0) if argv and argv[0] in COMMANDS else "replay"
    parser = argparse.ArgumentParser(prog=f"quirk_parity {command}", description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--mc-version", default=DEFAULT_MC)
    parser.add_argument("--corpus", default=None, help="corpus root (default KRASNOKAMEN_ORACLE_CORPUS "
                                                       "or <work>/oracle-corpus)")
    parser.add_argument("--trusted-ref", default="main",
                        help="Git ref whose corpus lock this tree may only extend (default main)")
    if command != "verify":
        parser.add_argument("--out", type=Path, required=True)
        parser.add_argument("--scenario", action="append", default=[])
        parser.add_argument("--family", action="append", default=[])
    if command == "replay":
        parser.add_argument("--restart-fault", choices=["delete-store", "garble-store"], default=None,
                            help="negative control: damage each restart scenario's store before it reopens")
    if command in ("record", "re-record"):
        parser.add_argument("--jar", default=None)
        parser.add_argument("--java", default=None)
        parser.add_argument("--reason", default=None)
        parser.add_argument("--fuzz-seed", type=int, action="append", default=[])
        parser.add_argument("--fuzz-cells", type=int, default=64)
        parser.add_argument("--fuzz-style", choices=sorted(seeded.PALETTES) + ["bud", "slime3d", "timing"],
                            default="mixed")
    return command, parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    command, args = parse_args(argv)
    if command == "verify":
        return verify(args)
    if command == "replay":
        return replay(args)
    return record(args, rerecord=command == "re-record")


if __name__ == "__main__":
    raise SystemExit(main())
