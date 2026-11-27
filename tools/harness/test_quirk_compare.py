"""Comparison-shape contracts for the quirk/farm parity gate.

A selected comparison must never pass when one receipt has fewer or more cells or
trace observations than the other: zipping unequal lists would silently ignore the
tail. These tests are synthetic comparator fixtures, not vanilla gameplay proof.

Run with: python3 -m unittest tools.harness.test_quirk_compare
"""

from __future__ import annotations

import json
import unittest
from pathlib import Path

from .quirk_parity import __main__ as qmain
from .quirk_parity import compare
from .quirk_parity.__main__ import rate_group_verdict


def cell(blocks: dict[str, str] | None = None) -> dict:
    return {"blocks": dict(blocks or {}), "entities": [], "containers": {}}


CELL_SPEC = {"blocks": "exact", "entities": "counts", "containers": "exact"}
STONE = {"0,0,0": "minecraft:stone"}
DIRT = {"0,0,0": "minecraft:dirt"}


def snapshot(count: int) -> dict:
    return {"cells": [cell() for _ in range(count)]}


def scenario(compare_spec: dict, phases: list[str] | None = None, trace: bool = False) -> dict:
    spec = dict(compare_spec)
    if phases is not None:
        spec["phases"] = phases
    out = {"compare": spec}
    if trace:
        out["trace"] = True
    return out


def trace_scenario(phases: list[str] | None = None) -> dict:
    return scenario(CELL_SPEC, phases, trace=True)


def phase(name: str, blocks: dict[str, str] | None = None, trace: list[dict] | None = None) -> dict:
    entry = {"name": name, "snapshot": cell(blocks)}
    if trace is not None:
        entry["trace"] = trace
    return entry


class CellSnapshotTests(unittest.TestCase):
    def test_equal_cell_lists_still_match(self):
        self.assertEqual(compare.compare_snapshot(CELL_SPEC, snapshot(3), snapshot(3)), {})

    def test_truncated_cell_list_is_a_diff(self):
        self.assertEqual(
            compare.compare_snapshot(CELL_SPEC, snapshot(3), snapshot(2)),
            {"cells": {"error": "cell count differs: vanilla 3, Krasnokamen 2"}},
        )

    def test_extra_cell_is_a_diff(self):
        self.assertEqual(
            compare.compare_snapshot(CELL_SPEC, snapshot(2), snapshot(3)),
            {"cells": {"error": "cell count differs: vanilla 2, Krasnokamen 3"}},
        )

    def test_one_sided_cell_lists_are_a_diff(self):
        union = {"blocks": STONE, "entities": [], "containers": {}}
        self.assertEqual(
            compare.compare_snapshot(CELL_SPEC, snapshot(2), union),
            {"cells": {"error": "per-cell snapshot missing from Krasnokamen"}},
        )
        self.assertEqual(
            compare.compare_snapshot(CELL_SPEC, union, snapshot(2)),
            {"cells": {"error": "per-cell snapshot missing from vanilla"}},
        )

    def test_different_content_in_matched_cells_is_reported(self):
        vanilla = {"cells": [cell(), cell(STONE)]}
        krasnokamen = {"cells": [cell(), cell(DIRT)]}
        self.assertEqual(
            compare.compare_snapshot(CELL_SPEC, vanilla, krasnokamen),
            {"cells": {"1": {"blocks": {"count": 1, "first": [
                {"pos": "0,0,0", "vanilla": "minecraft:stone", "krasnokamen": "minecraft:dirt"}]}}}},
        )


class TraceScenarioTests(unittest.TestCase):
    def test_equal_traces_still_match(self):
        vanilla = {"phases": [phase("run", trace=[cell(), cell(STONE)])]}
        krasnokamen = {"phases": [phase("run", trace=[cell(), cell(STONE)])]}
        self.assertEqual(compare.compare_scenario(trace_scenario(), vanilla, krasnokamen),
                         {"phases": {}, "match": True})

    def test_truncated_trace_is_a_diff(self):
        vanilla = {"phases": [phase("run", trace=[cell(), cell(STONE)])]}
        krasnokamen = {"phases": [phase("run", trace=[cell()])]}
        result = compare.compare_scenario(trace_scenario(), vanilla, krasnokamen)
        self.assertFalse(result["match"])
        self.assertEqual(result["phases"]["run"],
                         {"trace": {"error": "trace length differs: vanilla 2, Krasnokamen 1"}})

    def test_extra_trace_observation_is_a_diff(self):
        vanilla = {"phases": [phase("run", trace=[cell()])]}
        krasnokamen = {"phases": [phase("run", trace=[cell(), cell()])]}
        result = compare.compare_scenario(trace_scenario(), vanilla, krasnokamen)
        self.assertFalse(result["match"])
        self.assertEqual(result["phases"]["run"],
                         {"trace": {"error": "trace length differs: vanilla 1, Krasnokamen 2"}})

    def test_trace_missing_from_one_side_is_a_diff(self):
        vanilla = {"phases": [phase("run", trace=[cell()])]}
        krasnokamen = {"phases": [phase("run")]}
        result = compare.compare_scenario(trace_scenario(), vanilla, krasnokamen)
        self.assertFalse(result["match"])
        self.assertEqual(result["phases"]["run"],
                         {"trace": {"error": "trace missing from Krasnokamen phase receipt"}})

        vanilla = {"phases": [phase("run")]}
        krasnokamen = {"phases": [phase("run", trace=[cell()])]}
        result = compare.compare_scenario(trace_scenario(), vanilla, krasnokamen)
        self.assertFalse(result["match"])
        self.assertEqual(result["phases"]["run"],
                         {"trace": {"error": "trace missing from vanilla phase receipt"}})

    def test_different_content_in_matched_trace_ticks_is_reported(self):
        vanilla = {"phases": [phase("run", trace=[cell(), cell(STONE)])]}
        krasnokamen = {"phases": [phase("run", trace=[cell(), cell(DIRT)])]}
        result = compare.compare_scenario(trace_scenario(), vanilla, krasnokamen)
        self.assertFalse(result["match"])
        self.assertEqual(result["phases"]["run"]["trace"],
                         {"2": {"blocks": {"count": 1, "first": [
                             {"pos": "0,0,0", "vanilla": "minecraft:stone", "krasnokamen": "minecraft:dirt"}]}}})

    def test_setup_phase_without_trace_on_both_sides_matches(self):
        # Both drivers record no trace for the initial setup phase even when the
        # scenario enables traces; only trace-bearing phases are compared.
        vanilla = {"phases": [phase("setup"), phase("run", trace=[cell()])]}
        krasnokamen = {"phases": [phase("setup"), phase("run", trace=[cell()])]}
        self.assertEqual(compare.compare_scenario(trace_scenario(), vanilla, krasnokamen),
                         {"phases": {}, "match": True})

    def test_setup_only_receipts_with_root_trace_match(self):
        vanilla = {"phases": [phase("setup")]}
        krasnokamen = {"phases": [phase("setup")]}
        self.assertEqual(compare.compare_scenario(trace_scenario(), vanilla, krasnokamen),
                         {"phases": {}, "match": True})

    def test_declared_phase_missing_trace_on_both_sides_is_a_diff(self):
        # Both drivers record a trace for every declared phase when the root
        # switch is on, even with zero ticks; only the initial setup omits it.
        vanilla = {"phases": [phase("setup"), phase("run")]}
        krasnokamen = {"phases": [phase("setup"), phase("run")]}
        result = compare.compare_scenario(trace_scenario(), vanilla, krasnokamen)
        self.assertFalse(result["match"])
        self.assertEqual(result["phases"]["run"],
                         {"trace": {"error": "trace missing from both phase receipts"}})

    def test_phase_filter_still_skips_unselected_phases(self):
        vanilla = {"phases": [phase("armed", trace=[cell()]), phase("fired", trace=[cell(), cell()])]}
        krasnokamen = {"phases": [phase("armed", trace=[cell()]), phase("fired", trace=[cell()])]}
        selected = compare.compare_scenario(trace_scenario(["armed"]), vanilla, krasnokamen)
        self.assertEqual(selected, {"phases": {}, "match": True})
        unfiltered = compare.compare_scenario(trace_scenario(), vanilla, krasnokamen)
        self.assertFalse(unfiltered["match"])

    def test_trace_opt_in_is_the_scenario_root_key(self):
        # docs/QUIRK_PARITY.md and both drivers read the scenario-root "trace";
        # a trace key inside "compare" is not the switch and must not enable it.
        misnested = {"compare": dict(CELL_SPEC, trace=True)}
        vanilla = {"phases": [phase("run", trace=[cell()])]}
        krasnokamen = {"phases": [phase("run", trace=[cell(), cell()])]}
        self.assertEqual(compare.compare_scenario(misnested, vanilla, krasnokamen),
                         {"phases": {}, "match": True})

        rooted = trace_scenario()
        self.assertFalse(compare.compare_scenario(rooted, vanilla, krasnokamen)["match"])

    def test_trace_absent_or_false_at_root_is_not_compared(self):
        vanilla = {"phases": [phase("run", trace=[cell()])]}
        krasnokamen = {"phases": [phase("run", trace=[cell(), cell()])]}
        for disabled in ({}, {"trace": False}):
            self.assertEqual(compare.compare_scenario(dict({"compare": dict(CELL_SPEC)}, **disabled),
                                                      vanilla, krasnokamen),
                             {"phases": {}, "match": True})

    def test_missing_selected_phase_still_reported(self):
        vanilla = {"phases": [phase("armed")]}
        krasnokamen = {"phases": []}
        result = compare.compare_scenario(scenario(CELL_SPEC), vanilla, krasnokamen)
        self.assertEqual(result["phases"]["armed"], {"error": "phase missing from Krasnokamen receipt"})


class PhaseSymmetryTests(unittest.TestCase):
    def test_extra_krasnokamen_phase_is_a_diff(self):
        vanilla = {"phases": [phase("run")]}
        krasnokamen = {"phases": [phase("run"), phase("ghost")]}
        self.assertEqual(
            compare.compare_scenario(scenario(CELL_SPEC), vanilla, krasnokamen),
            {"phases": {"ghost": {"error": "phase not present in vanilla receipt"}}, "match": False},
        )

    def test_extra_krasnokamen_phase_outside_the_phase_filter_is_ignored(self):
        vanilla = {"phases": [phase("run")]}
        krasnokamen = {"phases": [phase("run"), phase("ghost")]}
        self.assertEqual(
            compare.compare_scenario(scenario(CELL_SPEC, phases=["run"]), vanilla, krasnokamen),
            {"phases": {}, "match": True},
        )

    def test_extra_selected_krasnokamen_phase_is_a_diff(self):
        vanilla = {"phases": [phase("run")]}
        krasnokamen = {"phases": [phase("run"), phase("ghost")]}
        self.assertFalse(
            compare.compare_scenario(scenario(CELL_SPEC, phases=["run", "ghost"]), vanilla, krasnokamen)["match"])


class FuzzVerdictShapeTests(unittest.TestCase):
    def fuzz_scenario(self, compare_spec: dict, trace: bool = False) -> dict:
        out = {
            "id": "fuzz-shape",
            "fuzz": {"seed": 1, "style": "bud", "cells": [{"index": 0, "blocks": []}]},
            "compare": dict(compare_spec),
        }
        if trace:
            out["trace"] = True
        return out

    def test_cell_count_mismatch_fails_the_fuzz_gate(self):
        from .quirk_parity import __main__ as qmain

        fuzz_scenario = self.fuzz_scenario(CELL_SPEC)
        vanilla = {"phases": [{"name": "setup", "snapshot": snapshot(1)},
                              {"name": "repower", "snapshot": snapshot(2)}]}
        krasnokamen = {"phases": [{"name": "setup", "snapshot": snapshot(1)},
                                  {"name": "repower", "snapshot": snapshot(1)}]}
        entry: dict = {"id": "fuzz-shape", "status": "pass"}
        verdict = qmain.fuzz_verdict(fuzz_scenario, [vanilla], krasnokamen, entry, [])
        self.assertEqual(verdict["status"], "fail")
        self.assertIn("comparison shape differs", " ".join(verdict["reasons"]))
        self.assertEqual(verdict["diff_cells"], {})

    def test_trace_error_fails_the_fuzz_gate(self):
        from .quirk_parity import __main__ as qmain

        fuzz_scenario = self.fuzz_scenario(CELL_SPEC, trace=True)
        vanilla = {"phases": [{"name": "setup", "snapshot": snapshot(1)},
                              {"name": "repower", "snapshot": snapshot(1), "trace": [cell()]}]}
        krasnokamen = {"phases": [{"name": "setup", "snapshot": snapshot(1)},
                                  {"name": "repower", "snapshot": snapshot(1)}]}
        verdict = qmain.fuzz_verdict(fuzz_scenario, [vanilla], krasnokamen, {"status": "pass"}, [])
        self.assertEqual(verdict["status"], "fail")
        self.assertIn("trace missing from Krasnokamen phase receipt", " ".join(verdict["reasons"]))
        self.assertEqual(verdict["diff_cells"], {})

    def test_declared_phase_missing_trace_on_both_sides_fails_the_fuzz_gate(self):
        from .quirk_parity import __main__ as qmain

        fuzz_scenario = self.fuzz_scenario(CELL_SPEC, trace=True)
        vanilla = {"phases": [{"name": "setup", "snapshot": snapshot(1)},
                              {"name": "repower", "snapshot": snapshot(1)}]}
        krasnokamen = {"phases": [{"name": "setup", "snapshot": snapshot(1)},
                                  {"name": "repower", "snapshot": snapshot(1)}]}
        verdict = qmain.fuzz_verdict(fuzz_scenario, [vanilla], krasnokamen, {"status": "pass"}, [])
        self.assertEqual(verdict["status"], "fail")
        self.assertIn("trace missing from both phase receipts", " ".join(verdict["reasons"]))
        self.assertEqual(verdict["diff_cells"], {})

    def test_extra_krasnokamen_phase_fails_the_fuzz_gate(self):
        from .quirk_parity import __main__ as qmain

        fuzz_scenario = self.fuzz_scenario(CELL_SPEC)
        vanilla = {"phases": [{"name": "setup", "snapshot": snapshot(1)}]}
        krasnokamen = {"phases": [{"name": "setup", "snapshot": snapshot(1)},
                                  {"name": "ghost", "snapshot": snapshot(1)}]}
        verdict = qmain.fuzz_verdict(fuzz_scenario, [vanilla], krasnokamen, {"status": "pass"}, [])
        self.assertEqual(verdict["status"], "fail")
        self.assertIn("phase not present in vanilla receipt", " ".join(verdict["reasons"]))
        self.assertEqual(verdict["diff_cells"], {})


# qc-piston-bud block values transcribed from the sole authorized real 26.1.2
# vanilla observation,
# .analysis/validation/20261123T082926-quirk-parity-etrrji0s/quirk-parity/vanilla-results.json
# (exactly the positions the scenario's vanilla_expect asserts, with that
# receipt's block strings; entity lists and containers are its empty ones).
RETAINED_QC_BLOCKS = {
    "setup": {
        "2,64,2": "minecraft:piston[extended=false,facing=east]",
        "6,64,2": "minecraft:sticky_piston[extended=false,facing=east]",
        "7,64,2": "minecraft:cobblestone",
        "10,64,2": "minecraft:piston[extended=false,facing=east]",
    },
    "idle": {
        "2,64,2": "minecraft:piston[extended=false,facing=east]",
        "6,64,2": "minecraft:sticky_piston[extended=false,facing=east]",
        "7,64,2": "minecraft:cobblestone",
        "10,64,2": "minecraft:piston[extended=false,facing=east]",
    },
    "bud": {
        "2,64,2": "minecraft:piston[extended=true,facing=east]",
        "3,64,2": "minecraft:piston_head[facing=east,short=false,type=normal]",
        "6,64,2": "minecraft:sticky_piston[extended=true,facing=east]",
        "7,64,2": "minecraft:piston_head[facing=east,short=false,type=sticky]",
        "8,64,2": "minecraft:cobblestone",
        "10,64,2": "minecraft:piston[extended=false,facing=east]",
    },
    "retract-on-removal": {
        "2,64,2": "minecraft:piston[extended=false,facing=east]",
        "6,64,2": "minecraft:sticky_piston[extended=true,facing=east]",
        "7,64,2": "minecraft:piston_head[facing=east,short=false,type=sticky]",
        "8,64,2": "minecraft:cobblestone",
        "10,64,2": "minecraft:piston[extended=false,facing=east]",
    },
}
QC_SCENARIO = Path(__file__).with_name("quirk_parity") / "scenarios" / "qc-piston-bud.json"


def qc_scenario() -> dict:
    return json.loads(QC_SCENARIO.read_text())


def retained_qc_observation() -> dict:
    return {
        "id": "qc-piston-bud",
        "server": "vanilla",
        "phases": [
            {"name": name, "snapshot": {"blocks": dict(blocks), "entities": [], "containers": {}}}
            for name, blocks in RETAINED_QC_BLOCKS.items()
        ],
        "command_log": [],
        "wall_ms": 6495,
    }


def retained_blocks(observation: dict, name: str) -> dict:
    return next(phase["snapshot"]["blocks"] for phase in observation["phases"] if phase["name"] == name)


class QcPistonBudOracleTests(unittest.TestCase):
    """The real qc-piston-bud scenario must keep proving the retained vanilla quirk.

    The fixture is the retained observation; the mutation tests are its negative
    controls, so dropping an assertion from the scenario (or a comparator regression
    that stops honouring one) turns them red instead of leaving a vacuous pass.
    """

    def test_retained_observation_satisfies_the_scenario_self_check(self):
        self.assertEqual(compare.check_vanilla(qc_scenario(), retained_qc_observation()), [])

    def test_bud_sticky_piston_state_is_enforced(self):
        observation = retained_qc_observation()
        retained_blocks(observation, "bud")["6,64,2"] = "minecraft:sticky_piston[extended=false,facing=east]"
        failures = compare.check_vanilla(qc_scenario(), observation)
        self.assertTrue(any("6,64,2" in failure for failure in failures), failures)

    def test_bud_normal_piston_head_is_enforced(self):
        observation = retained_qc_observation()
        retained_blocks(observation, "bud")["3,64,2"] = "minecraft:piston_head[facing=east,short=false,type=sticky]"
        failures = compare.check_vanilla(qc_scenario(), observation)
        self.assertTrue(any("3,64,2" in failure for failure in failures), failures)

    def test_unextended_idle_sticky_piston_is_enforced(self):
        observation = retained_qc_observation()
        retained_blocks(observation, "idle")["6,64,2"] = "minecraft:sticky_piston[extended=true,facing=east]"
        failures = compare.check_vanilla(qc_scenario(), observation)
        self.assertTrue(any("6,64,2" in failure for failure in failures), failures)

    def test_bud_push_is_enforced(self):
        observation = retained_qc_observation()
        bud = retained_blocks(observation, "bud")
        del bud["8,64,2"]
        bud["7,64,2"] = "minecraft:cobblestone"
        failures = compare.check_vanilla(qc_scenario(), observation)
        self.assertTrue(any("7,64,2" in failure for failure in failures), failures)
        self.assertTrue(any("8,64,2" in failure for failure in failures), failures)

    def test_retraction_is_enforced(self):
        observation = retained_qc_observation()
        retained_blocks(observation, "retract-on-removal")["2,64,2"] = (
            "minecraft:piston[extended=true,facing=east]")
        failures = compare.check_vanilla(qc_scenario(), observation)
        self.assertTrue(any("2,64,2" in failure for failure in failures), failures)

    def test_missing_phase_fails_closed(self):
        observation = retained_qc_observation()
        observation["phases"] = [phase for phase in observation["phases"] if phase["name"] != "bud"]
        failures = compare.check_vanilla(qc_scenario(), observation)
        self.assertTrue(any("no such phase" in failure for failure in failures), failures)

    def test_missing_block_position_fails_closed(self):
        observation = retained_qc_observation()
        del retained_blocks(observation, "idle")["7,64,2"]
        failures = compare.check_vanilla(qc_scenario(), observation)
        self.assertTrue(any("block 7,64,2" in failure and "None" in failure for failure in failures), failures)


class FarmOracleTests(unittest.TestCase):
    def test_every_seed_must_pass_vanilla_population_check_even_when_rates_match(self):
        farm = json.loads(
            (Path(__file__).with_name("quirk_parity") / "scenarios" / "farm-chicken-eggs.json").read_text()
        )
        variants = [dict(farm, id=f"farm-chicken-eggs-{seed}", seed=seed) for seed in (11, 12)]

        def run(chickens):
            def observation(eggs):
                return {"blocks": {}, "entities": [{"type": "minecraft:chicken"}] * chickens,
                        "containers": {"hopper": {"minecraft:egg": eggs}}}
            return {"command_log": [], "phases": [
                {"name": "warmup", "snapshot": observation(0)},
                {"name": "run4", "snapshot": observation(20)},
            ]}

        good = {variant["id"]: [run(16), run(16)] for variant in variants}
        bad = dict(good, **{variants[1]["id"]: [run(16), run(1)]})
        server = {variant["id"]: run(16) for variant in variants}
        self.assertEqual(rate_group_verdict("farm-chicken-eggs", variants, good, server)["status"], "pass")
        result = rate_group_verdict("farm-chicken-eggs", variants, bad, server)
        self.assertEqual(result["status"], "fail")
        self.assertIn("minecraft:chicken", result["vanilla_selfcheck_failures"][variants[1]["id"]][0])
        # The same population check refuses the recording itself.
        problems = qmain.recording_problems(variants[1], bad[variants[1]["id"]])
        self.assertTrue(any("recording 2" in problem and "minecraft:chicken" in problem for problem in problems))


if __name__ == "__main__":
    unittest.main()
