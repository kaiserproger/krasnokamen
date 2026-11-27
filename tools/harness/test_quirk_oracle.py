"""Record/replay corpus, wildcard, known-diff and promotion-guard contracts of the vanilla oracle.

Each guard has a positive case and a negative control: an edited or removed recorded case
fails replay, a fix that drops tests or assertions or adds `#[ignore]` is refused. These are
synthetic fixtures for the harness itself, not vanilla gameplay evidence.

Run with: python3 -m unittest tools.harness.test_quirk_oracle
"""
from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from .quirk_parity import __main__ as oracle
from .quirk_parity import compare, guard
from .quirk_parity.corpus import Corpus, CorpusError, scenario_sha256

SCENARIO = {"id": "case-a", "family": "blocks", "box": [0, 0, 0, 1, 1, 1], "setup": ["setblock 0 0 0 stone"],
            "phases": [{"name": "p", "commands": [], "ticks": 1}],
            "compare": {"blocks": "exact", "entities": "counts", "containers": "exact"}}


def run(blocks: dict, entities: list | None = None) -> dict:
    snapshot = {"blocks": dict(blocks), "entities": list(entities or []), "containers": {}}
    return {"id": "case-a", "server": "vanilla", "command_log": [],
            "phases": [{"name": "setup", "snapshot": snapshot}, {"name": "p", "snapshot": snapshot}]}


def doc(case: str = "case-a", version: int = 1, scenario: dict | None = None, wildcards=()) -> dict:
    scenario = dict(scenario or SCENARIO, id=case)
    return {"format": 1, "case": case, "version": version, "scenario": scenario,
            "scenario_sha256": scenario_sha256(scenario), "recordings": [run({"0,0,0": "minecraft:stone"})] * 2,
            "wildcards": list(wildcards), "recorded_at": "2026-11-27T00:00:00+00:00"}


class CorpusTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.lock = self.tmp / "corpus-lock.jsonl"
        self.corpus = Corpus(self.tmp / "corpus", "26.3", lock=self.lock)

    def test_recorded_cases_verify_and_load(self):
        first = self.corpus.append(doc("case-a"), op="record")
        self.corpus.append(doc("case-b"), op="record")
        state = self.corpus.verify()
        self.assertEqual(state["problems"], [])
        self.assertEqual(sorted(state["latest"]), ["case-a", "case-b"])
        self.assertEqual(self.corpus.load(first)["case"], "case-a")
        self.assertEqual(len(self.lock.read_text().splitlines()), 2)

    def test_edited_case_fails_verification(self):
        entry = self.corpus.append(doc(), op="record")
        path = self.corpus.dir / entry["file"]
        edited = json.loads(path.read_text())
        edited["recordings"][0]["phases"][1]["snapshot"]["blocks"]["0,0,0"] = "minecraft:dirt"
        path.write_text(json.dumps(edited, indent=1, sort_keys=True) + "\n")
        problems = self.corpus.verify()["problems"]
        self.assertTrue(any("case-a v1 edited" in p for p in problems), problems)
        with self.assertRaises(CorpusError):
            self.corpus.load(entry)

    def test_edited_case_with_rewritten_manifest_hash_still_fails(self):
        entry = self.corpus.append(doc(), op="record")
        path = self.corpus.dir / entry["file"]
        path.write_text(path.read_text().replace("minecraft:stone", "minecraft:dirt"))
        line = json.loads(self.corpus.manifest.read_text())
        line["sha256"] = __import__("hashlib").sha256(path.read_bytes()).hexdigest()
        self.corpus.manifest.write_text(json.dumps(line, sort_keys=True, separators=(",", ":")) + "\n")
        problems = self.corpus.verify()["problems"]
        self.assertTrue(any("locked case case-a v1 edited" in p for p in problems), problems)

    def test_removed_case_fails_verification(self):
        entry = self.corpus.append(doc(), op="record")
        (self.corpus.dir / entry["file"]).unlink()
        problems = self.corpus.verify()["problems"]
        self.assertTrue(any("case-a v1 removed" in p for p in problems), problems)

    def test_removed_manifest_line_fails_verification(self):
        self.corpus.append(doc("case-a"), op="record")
        self.corpus.append(doc("case-b"), op="record")
        lines = self.corpus.manifest.read_text().splitlines()
        self.corpus.manifest.write_text(lines[1] + "\n")
        problems = self.corpus.verify()["problems"]
        self.assertTrue(any("hash chain broken" in p for p in problems), problems)
        self.assertTrue(any("locked case case-a v1 removed" in p for p in problems), problems)

    def test_rerecord_keeps_the_old_version_and_a_dropped_old_lock_line_fails(self):
        self.corpus.append(doc(), op="record")
        kept = self.corpus.keep_manifest("before-v2-case-a")
        self.corpus.append(dict(doc(version=2), supersedes={"version": 1}), op="re-record", reason="setup fixed")
        state = self.corpus.verify()
        self.assertEqual(state["problems"], [])
        self.assertEqual(state["latest"]["case-a"]["version"], 2)
        self.assertTrue((self.corpus.dir / "cases/case-a/v001.json").is_file())
        self.assertEqual(len(kept.read_text().splitlines()), 1)
        self.lock.write_text(self.lock.read_text().splitlines()[1] + "\n")
        problems = self.corpus.verify()["problems"]
        self.assertTrue(any("older version's lock line was removed" in p for p in problems), problems)

    def test_append_refuses_a_tampered_corpus_and_never_overwrites(self):
        entry = self.corpus.append(doc(), op="record")
        with self.assertRaises(FileExistsError):
            self.corpus.append(doc(), op="record")
        (self.corpus.dir / entry["file"]).write_text("{}")
        with self.assertRaises(CorpusError):
            self.corpus.append(doc("case-b"), op="record")


class ReplayTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.lock = self.tmp / "corpus-lock.jsonl"
        self.scenarios = self.tmp / "scenarios"
        self.scenarios.mkdir()
        (self.scenarios / "case-a.json").write_text(json.dumps(dict(SCENARIO, prelude=False)))
        self.patches = [mock.patch("tools.harness.quirk_parity.corpus.LOCK", self.lock),
                        mock.patch.object(oracle, "LOCK", self.lock),
                        mock.patch.object(oracle, "SCENARIO_DIR", self.scenarios),
                        mock.patch.object(oracle, "KNOWN_DIFFS", self.tmp / "known-diffs.json")]
        for patch in self.patches:
            patch.start()
        self.addCleanup(lambda: [patch.stop() for patch in self.patches])
        self.corpus = Corpus(self.tmp / "corpus", "26.3", lock=self.lock)
        case = oracle.resolved(json.loads((self.scenarios / "case-a.json").read_text()))
        self.entry = self.corpus.append(dict(doc(scenario=case)), op="record")

    def replay(self, krasnokamen: dict) -> tuple[int, dict, mock.Mock]:
        args = SimpleNamespace(out=self.tmp / "out", corpus=str(self.tmp / "corpus"), mc_version="26.3",
                               scenario=[], family=[], restart_fault=None)
        with mock.patch.object(oracle, "run_krasnokamen", return_value=krasnokamen) as runner:
            code = oracle.replay(args)
        return code, json.loads((self.tmp / "out" / "receipt.json").read_text()), runner

    def test_matching_replay_passes(self):
        code, receipt, _ = self.replay({"case-a": dict(run({"0,0,0": "minecraft:stone"}), server="krasnokamen")})
        self.assertEqual((code, receipt["status"]), (0, "pass"), receipt)
        self.assertEqual(receipt["integrity"]["problems"], [])

    def record_with_jar(self, version: str, bundler: bool = True) -> tuple[int, dict, mock.Mock]:
        jar = self.tmp / f"server-{version}.jar"
        with zipfile.ZipFile(jar, "w") as archive:
            archive.writestr("version.json", json.dumps({"id": version}))
            if bundler:
                archive.writestr("META-INF/versions.list", "x\t26.1.2\t26.1.2/server-26.1.2.jar\n")
        (self.scenarios / "case-b.json").write_text(json.dumps(dict(SCENARIO, id="case-b")))
        args = SimpleNamespace(out=self.tmp / "rec", corpus=str(self.tmp / "corpus"), mc_version="26.3",
                               scenario=["case-b"], family=[], jar=str(jar), java=None, reason=None,
                               fuzz_seed=[], fuzz_cells=64, fuzz_style="mixed")
        with mock.patch.object(oracle, "run_vanilla") as vanilla, \
                mock.patch.object(oracle, "resolve_java", return_value="/bin/true"), \
                mock.patch.object(oracle, "java_version", return_value="test"):
            code = oracle.record(args)
        return code, json.loads((self.tmp / "rec" / "receipt.json").read_text()), vanilla

    def test_recording_with_another_release_jar_is_refused_and_changes_nothing(self):
        before = (self.lock.read_text(), self.corpus.manifest.read_text())
        for version, bundler, message in (("26.1.2", True, "is vanilla '26.1.2', not 26.3"),
                                          ("26.3", False, "inner game jar")):
            with self.subTest(version=version, bundler=bundler):
                code, receipt, vanilla = self.record_with_jar(version, bundler)
                self.assertEqual((code, receipt["status"]), (1, "fail"))
                self.assertIn(message, receipt["error"])
                vanilla.assert_not_called()
                self.assertEqual((self.lock.read_text(), self.corpus.manifest.read_text()), before)

    def test_restart_phase_krasnokamen_did_not_restart_fails(self):
        scenario = dict(SCENARIO, phases=[{"name": "p", "restart": True, "commands": [], "ticks": 1}])
        vanilla = run({"0,0,0": "minecraft:stone"})
        self.assertEqual(oracle.verdict(scenario, vanilla, vanilla)["status"], "fail")
        restarted = dict(vanilla, phases=[vanilla["phases"][0], dict(vanilla["phases"][1], restarted=True)])
        self.assertEqual(oracle.verdict(scenario, vanilla, restarted)["status"], "pass")
        failed = dict(restarted, error="native store refused: missing native/state.redb")
        self.assertEqual(oracle.verdict(scenario, vanilla, failed)["status"], "fail")

    def test_edited_case_fails_replay_without_running_krasnokamen(self):
        path = self.corpus.dir / self.entry["file"]
        path.write_text(path.read_text().replace("minecraft:stone", "minecraft:dirt"))
        code, receipt, runner = self.replay({})
        self.assertEqual((code, receipt["status"]), (1, "fail"))
        self.assertIn("corpus integrity", receipt["error"])
        runner.assert_not_called()

    def test_removed_case_fails_replay(self):
        (self.corpus.dir / self.entry["file"]).unlink()
        code, receipt, runner = self.replay({})
        self.assertEqual(code, 1)
        self.assertTrue(any("removed" in p for p in receipt["integrity"]["problems"]))
        runner.assert_not_called()

    def test_changed_scenario_needs_a_rerecord(self):
        (self.scenarios / "case-a.json").write_text(json.dumps(dict(SCENARIO, prelude=False, compare={
            "blocks": "exact", "entities": "counts", "entity_tolerance": 5})))
        code, receipt, _ = self.replay({})
        self.assertEqual(code, 1)
        self.assertIn("re-record", receipt["scenarios"][0]["reasons"][0])

    def test_unrecorded_scenario_fails(self):
        (self.scenarios / "case-b.json").write_text(json.dumps(dict(SCENARIO, id="case-b")))
        code, receipt, _ = self.replay({"case-a": run({"0,0,0": "minecraft:stone"})})
        self.assertEqual(code, 1)
        self.assertEqual({row["id"]: row["status"] for row in receipt["scenarios"]},
                         {"case-a": "pass", "case-b": "fail"})

    def test_divergence_is_grouped_under_a_stable_mismatch_id(self):
        code, receipt, _ = self.replay({"case-a": run({"0,0,0": "minecraft:dirt"})})
        self.assertEqual(code, 1)
        self.assertEqual(receipt["mismatches"], [{
            "case": "case-a", "mismatch_id": compare.mismatch_id("case-a", "setup/blocks/0,0,0"),
            "path": "setup/blocks/0,0,0", "vanilla": "minecraft:stone", "krasnokamen": "minecraft:dirt",
            "rows": 2, "status": "new"}])


class WildcardAndKnownDiffTests(unittest.TestCase):
    def test_paths_differing_between_recordings_become_wildcards(self):
        a = run({"0,0,0": "minecraft:stone", "1,0,0": "minecraft:gravel"})
        b = run({"0,0,0": "minecraft:stone", "1,0,0": "minecraft:flint_block"})
        problems, wildcards = oracle.recording_problems(SCENARIO, [a, b])
        self.assertEqual((problems, wildcards), ([], ["p/blocks/1,0,0", "setup/blocks/1,0,0"]))
        krasnokamen = run({"0,0,0": "minecraft:stone", "1,0,0": "minecraft:sand"})
        self.assertEqual(oracle.verdict(SCENARIO, a, krasnokamen, wildcards=frozenset(wildcards))["status"], "pass")
        self.assertEqual(oracle.verdict(SCENARIO, a, krasnokamen)["status"], "fail")

    def test_recordings_differing_in_shape_are_refused(self):
        a = run({})
        b = dict(run({}), phases=run({})["phases"][:1])
        problems, _ = oracle.recording_problems(SCENARIO, [a, b])
        self.assertTrue(any("differ in shape" in p for p in problems), problems)

    def test_known_mismatch_reports_known_divergence_and_fails_once_gone(self):
        vanilla, krasnokamen = run({"0,0,0": "minecraft:stone"}), run({"0,0,0": "minecraft:dirt"})
        known = {"reason": "documented", "mismatches": [compare.mismatch_id("case-a", "setup/blocks/0,0,0")]}
        self.assertEqual(oracle.verdict(SCENARIO, vanilla, krasnokamen, known=known)["status"], "known-divergence")
        stale = oracle.verdict(SCENARIO, vanilla, vanilla, known=known)
        self.assertEqual(stale["status"], "fail")
        self.assertIn("no longer differs", stale["reasons"][0])
        other = {"reason": "documented", "mismatches": ["000000000000"]}
        self.assertEqual(oracle.verdict(SCENARIO, vanilla, krasnokamen, known=other)["status"], "fail")

    def test_mismatch_groups_follow_the_first_differing_path_per_cell(self):
        rows = [("p/trace/1/cells/0/blocks/0,0,0", "a", "b"), ("p/cells/0/blocks/0,0,0", "a", "b"),
                ("p/cells/1/entities/minecraft:tnt", 1, 0)]
        groups = compare.mismatch_groups("fuzz", rows)
        self.assertEqual([(g["path"], g["rows"]) for g in groups],
                         [("p/trace/1/cells/0/blocks/0,0,0", 2), ("p/cells/1/entities/minecraft:tnt", 1)])
        self.assertEqual(groups[0]["mismatch_id"], compare.mismatch_id("fuzz", "p/trace/1/cells/0/blocks/0,0,0"))

    def test_entity_fields_compare_health_and_fuse(self):
        spec = {"blocks": "exact", "entities": "counts",
                "entity_fields": {"types": ["minecraft:pig"], "fields": ["health"], "epsilon": 0.01}}
        pig = {"type": "minecraft:pig", "health": 3.0}
        same = compare.compare_cell(spec, {"blocks": {}, "entities": [pig]}, {"blocks": {}, "entities": [dict(pig)]})
        hurt = compare.compare_cell(spec, {"blocks": {}, "entities": [pig]},
                                    {"blocks": {}, "entities": [dict(pig, health=10.0)]})
        self.assertEqual(same, {})
        self.assertEqual(hurt["entity_fields"]["minecraft:pig"], {"vanilla": [(3.0,)], "krasnokamen": [(10.0,)]})


def git(root: Path, *args: str) -> str:
    return subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, text=True).stdout.strip()


class PromotionGuardTests(unittest.TestCase):
    BASE = "#[test]\nfn a() {\n    assert_eq!(1, 1);\n    assert!(true);\n}\n"

    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        git(self.root, "init", "-q")
        git(self.root, "config", "user.email", "t@example.invalid")
        git(self.root, "config", "user.name", "t")
        self.src = self.root / "crates/mc-core/src/lib_tests.rs"
        self.src.parent.mkdir(parents=True)
        self.lock = self.root / guard.LOCK_REL
        self.lock.parent.mkdir(parents=True)
        self.lock.write_text('{"case":"a"}\n')
        self.commit(self.BASE, "base")

    def commit(self, text: str, message: str) -> str:
        self.src.write_text(text)
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", message)
        return git(self.root, "rev-parse", "HEAD")

    def test_fix_that_adds_a_test_passes_and_records_the_baseline(self):
        fix = self.commit(self.BASE + "#[test]\nfn b() {\n    assert!(true);\n}\n", "fix")
        record = guard.promotion_guard(self.root, fix, None)
        self.assertEqual(record["verdict"], "pass")
        self.assertEqual((record["baseline"]["tests"], record["after"]["tests"]), (1, 2))
        self.assertEqual(record["delta"]["total"], {"tests": 1, "asserts": 1, "ignores": 0})
        self.assertEqual(record["delta"]["crates"]["mc-core"]["tests"], 1)

    def test_fix_that_drops_a_test_is_refused(self):
        fix = self.commit("fn a() {\n    assert_eq!(1, 1);\n    assert!(true);\n}\n", "fix")
        with self.assertRaisesRegex(guard.GuardRefused, "removes 1 test"):
            guard.promotion_guard(self.root, fix, None)

    def test_fix_that_drops_an_assertion_is_refused(self):
        fix = self.commit("#[test]\nfn a() {\n    assert_eq!(1, 1);\n}\n", "fix")
        with self.assertRaisesRegex(guard.GuardRefused, "removes 1 assertion"):
            guard.promotion_guard(self.root, fix, None)

    def test_fix_that_ignores_a_test_is_refused(self):
        fix = self.commit(self.BASE.replace("#[test]\n", "#[test]\n#[ignore = \"flaky\"]\n"), "fix")
        with self.assertRaisesRegex(guard.GuardRefused, r"adds 1 #\[ignore\]"):
            guard.promotion_guard(self.root, fix, None)

    def test_fix_that_removes_a_corpus_lock_line_is_refused(self):
        self.lock.write_text("")
        fix = self.commit(self.BASE, "fix")
        with self.assertRaisesRegex(guard.GuardRefused, "corpus lock line"):
            guard.promotion_guard(self.root, fix, None)

    def test_patch_fix_that_drops_a_test_is_refused(self):
        patch = ("diff --git a/crates/mc-core/src/lib_tests.rs b/crates/mc-core/src/lib_tests.rs\n"
                 "--- a/crates/mc-core/src/lib_tests.rs\n+++ b/crates/mc-core/src/lib_tests.rs\n"
                 "@@ -1,2 +1,1 @@\n-#[test]\n fn a() {\n")
        with self.assertRaisesRegex(guard.GuardRefused, "removes 1 test"):
            guard.promotion_guard(self.root, "patch:x", patch)

    def test_moved_tests_and_commented_asserts_do_not_count(self):
        moved = self.root / "crates/mc-new/src/lib_tests.rs"
        moved.parent.mkdir(parents=True)
        moved.write_text(self.BASE)
        fix = self.commit("// assert!(old);\n", "move")
        record = guard.promotion_guard(self.root, fix, None)
        self.assertEqual(record["delta"]["total"], {"tests": 0, "asserts": 0, "ignores": 0})


if __name__ == "__main__":
    unittest.main()
