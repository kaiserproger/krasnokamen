"""Record/replay corpus, whole-recording parity, waivers and promotion-guard contracts.

Each guard has a positive case and a negative control: an edited or removed recorded case
fails replay, and a fix that drops or disables tests is refused. These are
synthetic fixtures for the harness itself, not vanilla gameplay evidence.

Run with: python3 -m unittest tools.harness.test_quirk_oracle
"""
from __future__ import annotations

import json
import shutil
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from . import ledger, profiles
from .quirk_parity import __main__ as oracle
from .quirk_parity import compare, guard
from .quirk_parity.corpus import Corpus, CorpusError, committed_lock, read_lock, scenario_sha256, trusted_lock

SCENARIO = {"id": "case-a", "family": "blocks", "box": [0, 0, 0, 1, 1, 1], "setup": ["setblock 0 0 0 stone"],
            "phases": [{"name": "p", "commands": [], "ticks": 1}],
            "compare": {"blocks": "exact", "entities": "counts", "containers": "exact"}}


def run(blocks: dict, entities: list | None = None) -> dict:
    snapshot = {"blocks": dict(blocks), "entities": list(entities or []), "containers": {}}
    return {"id": "case-a", "server": "vanilla", "command_log": [],
            "phases": [{"name": "setup", "snapshot": snapshot}, {"name": "p", "snapshot": snapshot}]}


def doc(case: str = "case-a", version: int = 1, scenario: dict | None = None) -> dict:
    scenario = dict(scenario or SCENARIO, id=case)
    return {"format": 1, "case": case, "version": version, "scenario": scenario,
            "scenario_sha256": scenario_sha256(scenario), "recordings": [run({"0,0,0": "minecraft:stone"})] * 2,
            "recorded_at": "2026-11-27T00:00:00+00:00"}


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

    def rerecorded(self) -> list[dict]:
        """case-a v1 and a re-recorded v2; returns the lock rows."""
        self.corpus.append(doc(), op="record")
        self.corpus.append(dict(doc(version=2), supersedes={"version": 1}), op="re-record", reason="setup fixed")
        return read_lock("26.3", self.lock)

    def test_dropping_a_newer_version_line_rolls_back_only_without_the_trusted_baseline(self):
        trusted = self.rerecorded()
        self.lock.write_text(self.lock.read_text().splitlines()[0] + "\n")
        # negative control: the tree's own lock alone accepts the rollback to v1
        self.assertEqual(self.corpus.verify()["problems"], [])
        state = self.corpus.verify(trusted=trusted)
        self.assertTrue(any("case-a v2 on the trusted baseline is gone" in p for p in state["problems"]),
                        state["problems"])

    def test_a_newer_version_needs_a_rerecord_reason_without_any_baseline(self):
        self.corpus.append(doc(), op="record")
        self.corpus.append(doc(version=2), op="record")  # a fix appending v2 of an existing case
        problems = self.corpus.verify()["problems"]
        self.assertTrue(any("case-a v2 was not made by `re-record --reason`" in p for p in problems), problems)

    def test_rerecords_and_added_cases_since_the_trusted_baseline_are_listed(self):
        self.corpus.append(doc(), op="record")
        trusted = read_lock("26.3", self.lock)
        self.corpus.append(dict(doc(version=2), supersedes={"version": 1}), op="re-record", reason="setup fixed")
        self.corpus.append(doc("case-b"), op="record")
        state = self.corpus.verify(trusted=trusted)
        self.assertEqual(state["problems"], [])
        self.assertEqual(state["re_recorded"], [{"case": "case-a", "from": 1, "to": 2, "reasons": ["setup fixed"]}])
        self.assertEqual(state["added"], ["case-b"])

    def repo(self):
        """A Git repository holding no lock yet: (repo, lock path, git)."""
        repo = self.tmp / "repo"
        lock = repo / "tools" / "corpus-lock.jsonl"
        lock.parent.mkdir(parents=True)
        git = lambda *args: subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *args],
                                           cwd=repo, check=True, capture_output=True)
        git("init", "-q", "-b", "main")
        (repo / "README").write_text("x\n")
        git("add", "README")
        git("commit", "-q", "-m", "no lock yet")
        return repo, lock, git

    def test_trusted_lock_reads_the_ref_and_fails_closed_on_an_unknown_ref(self):
        repo, lock, git = self.repo()
        self.assertEqual(trusted_lock("26.3", "main", repo, lock), [])
        self.corpus.append(doc(), op="record")
        lock.write_text(self.lock.read_text())
        git("add", "tools/corpus-lock.jsonl")
        git("commit", "-q", "-m", "lock")
        self.assertEqual(trusted_lock("26.3", "main", repo, lock), read_lock("26.3", self.lock))
        with self.assertRaises(CorpusError):
            trusted_lock("26.3", "no-such-ref", repo, lock)

    def test_a_lock_line_any_ancestor_committed_must_stay_even_after_a_commit_drops_it(self):
        """D131 P1-3: no trusted ref needed, and a committed rollback is still caught once it is main."""
        repo, lock, git = self.repo()
        self.assertEqual(committed_lock("26.3", repo, lock), [])
        rows = self.rerecorded()
        lock.write_text(self.lock.read_text())
        git("add", "-A")
        git("commit", "-q", "-m", "v1 and v2")
        lock.write_text(self.lock.read_text().splitlines()[0] + "\n")  # roll case-a back to v1
        git("commit", "-q", "-am", "drop v2")  # HEAD is main: comparing with main sees nothing
        self.lock.write_text(lock.read_text())
        self.assertEqual(self.corpus.verify(trusted=trusted_lock("26.3", "main", repo, lock))["problems"], [])
        self.assertEqual(sorted(map(json.dumps, committed_lock("26.3", repo, lock))), sorted(map(json.dumps, rows)))
        problems = self.corpus.verify(committed=committed_lock("26.3", repo, lock))["problems"]
        self.assertTrue(any("case-a v2 in the lock's Git history is gone" in p for p in problems), problems)

    def test_shallow_checkout_refuses_lock_verification(self):
        repo, lock, commit = self.repo()
        self.corpus.append(doc(), op="record")
        lock.write_text(self.lock.read_text())
        commit("add", "-A")
        commit("commit", "-qm", "lock")
        shallow = self.tmp / "shallow"
        subprocess.run(["git", "clone", "-q", "--depth=1", repo.as_uri(), str(shallow)], check=True)
        with self.assertRaisesRegex(CorpusError, "cannot verify corpus lock history in a shallow checkout"):
            committed_lock("26.3", shallow, shallow / lock.relative_to(repo))


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
                        mock.patch.object(oracle, "KNOWN_DIFFS", self.tmp / "known-diffs.json"),
                        mock.patch.object(oracle, "trusted_lock", return_value=[]),
                        mock.patch.object(oracle, "committed_lock", return_value=[])]
        for patch in self.patches:
            patch.start()
        self.addCleanup(lambda: [patch.stop() for patch in self.patches])
        self.corpus = Corpus(self.tmp / "corpus", "26.3", lock=self.lock)
        case = oracle.resolved(json.loads((self.scenarios / "case-a.json").read_text()))
        self.entry = self.corpus.append(dict(doc(scenario=case)), op="record")

    def replay(self, krasnokamen: dict) -> tuple[int, dict, mock.Mock]:
        args = SimpleNamespace(out=self.tmp / "out", corpus=str(self.tmp / "corpus"), mc_version="26.3",
                               scenario=[], family=[], restart_fault=None, trusted_ref="main")
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
                               fuzz_seed=[], fuzz_cells=64, fuzz_style="mixed", trusted_ref="main")
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
        self.assertEqual(oracle.verdict(scenario, [vanilla], vanilla)["status"], "fail")
        restarted = dict(vanilla, phases=[vanilla["phases"][0], dict(vanilla["phases"][1], restarted=True)])
        self.assertEqual(oracle.verdict(scenario, [vanilla], restarted)["status"], "pass")
        failed = dict(restarted, error="native store refused: missing native/state.redb")
        self.assertEqual(oracle.verdict(scenario, [vanilla], failed)["status"], "fail")

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


class RecordingAndKnownDiffTests(unittest.TestCase):
    def test_case_matches_either_recording_but_no_third_value(self):
        a = run({"0,0,0": "minecraft:stone", "1,0,0": "minecraft:gravel"})
        b = run({"0,0,0": "minecraft:stone", "1,0,0": "minecraft:flint_block"})
        self.assertEqual(oracle.recording_problems(SCENARIO, [a, b]), [])
        for value, status in (("minecraft:gravel", "pass"), ("minecraft:flint_block", "pass"),
                              ("minecraft:sand", "fail")):
            krasnokamen = run({"0,0,0": "minecraft:stone", "1,0,0": value})
            self.assertEqual(oracle.verdict(SCENARIO, [a, b], krasnokamen)["status"], status, value)
        flint = run({"0,0,0": "minecraft:stone", "1,0,0": "minecraft:flint_block"})
        self.assertEqual(oracle.verdict(SCENARIO, [a], flint)["status"], "fail")

    def test_positions_and_fields_match_the_same_recording(self):
        spec = {"blocks": "exact", "entities": "counts",
                "entity_positions": {"types": ["minecraft:pig"], "epsilon": 0.05},
                "entity_fields": {"types": ["minecraft:pig"], "fields": ["health", "fuse"], "epsilon": 0.01}}
        scenario = dict(SCENARIO, compare=spec)

        def pigs(second_x: float, second_health: float, first_x: float = 0.5, first_health: float = 10.0):
            return run({}, [{"type": "minecraft:pig", "pos": [first_x, 1.0, 0.5], "health": first_health},
                            {"type": "minecraft:pig", "pos": [second_x, 1.0, 0.5], "health": second_health}])

        recordings = [pigs(3.0, 6.0), pigs(4.0, 8.0)]
        self.assertEqual(oracle.recording_problems(scenario, recordings), [])
        self.assertEqual(oracle.verdict(scenario, recordings, recordings[1])["status"], "pass")
        for label, krasnokamen in (("unrecorded position", pigs(5.0, 8.0)), ("unrecorded health", pigs(4.0, 7.0)),
                                   ("other pig moved", pigs(4.0, 8.0, first_x=1.5)),
                                   ("other pig hurt", pigs(4.0, 8.0, first_health=9.0)),
                                   ("position of recording 2, health of recording 1", pigs(4.0, 6.0))):
            self.assertEqual(oracle.verdict(scenario, recordings, krasnokamen)["status"], "fail", label)

    def test_block_values_of_the_two_recordings_never_mix(self):
        """Vanilla showed the block at one of two cells; a world holding it at neither loses it."""
        first, second = run({"0,0,0": "minecraft:sand"}), run({"1,0,0": "minecraft:sand"})
        self.assertEqual(oracle.recording_problems(SCENARIO, [first, second]), [])
        for label, krasnokamen, status in (("recording 1", first, "pass"), ("recording 2", second, "pass"),
                                           ("air at both cells", run({}), "fail"),
                                           ("sand at both cells", run({"0,0,0": "minecraft:sand",
                                                                      "1,0,0": "minecraft:sand"}), "fail")):
            self.assertEqual(oracle.verdict(SCENARIO, [first, second], krasnokamen)["status"], status, label)

    def test_tolerance_is_measured_against_the_same_recording_as_the_blocks(self):
        spec = dict(SCENARIO["compare"], entity_positions={"types": ["minecraft:pig"], "epsilon": 0.05})
        scenario = dict(SCENARIO, compare=spec)

        def world(block, x):
            return run({"0,0,0": block}, [{"type": "minecraft:pig", "pos": [x, 0, 0]}])

        recordings = [world("minecraft:stone", 0), world("minecraft:gravel", 0.04)]
        self.assertEqual(oracle.verdict(scenario, recordings, world("minecraft:gravel", 0.08))["status"], "pass")
        self.assertEqual(oracle.verdict(scenario, recordings, world("minecraft:gravel", -0.04))["status"], "fail")

    def test_recordings_differing_in_shape_are_refused(self):
        a = run({})
        b = dict(run({}), phases=run({})["phases"][:1])
        problems = oracle.recording_problems(SCENARIO, [a, b])
        self.assertTrue(any("differ in shape" in p for p in problems), problems)

    def test_known_difference_waives_only_exact_rows_including_later_rows(self):
        def world(first, late="minecraft:stone"):
            cells = [{"blocks": {"0,0,0": first, "1,0,0": late}, "entities": [], "containers": {}}]
            return dict(run({}), phases=[{"name": "p", "snapshot": {"cells": cells}}])

        vanilla = world("minecraft:stone")
        path = "p/cells/0/blocks/0,0,0"
        known = {"reason": "documented", "rows": [[path, "minecraft:stone", "minecraft:dirt"]]}
        for observed, status in ((world("minecraft:dirt"), "known-divergence"),
                                 (world("minecraft:bedrock"), "fail"),
                                 (world("minecraft:dirt", "minecraft:gravel"), "fail")):
            with self.subTest(observed=observed):
                result = oracle.verdict(SCENARIO, [vanilla, vanilla], observed, known=known)
                self.assertEqual(result["status"], status)
                self.assertEqual(len(result["mismatches"]), 1)
                self.assertEqual(result["mismatches"][0]["path"], path)
                self.assertEqual(result["mismatches"][0]["status"], "new" if status == "fail" else "known")
        stale = oracle.verdict(SCENARIO, [vanilla, vanilla], vanilla, known=known)
        self.assertEqual(stale["status"], "fail")
        self.assertIn("no longer differs", stale["reasons"][0])

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


LIB = "#[cfg(test)]\nmod a_tests;\n#[cfg(test)]\nmod b_tests;\n"
A_TESTS = "#[test]\nfn one() {\n    assert_eq!(1, 1);\n    assert_eq!(2, 2);\n}\n\n#[test]\nfn two() {}\n"
B_TESTS = "#[test]\nfn three() {}\n"
LOCK_A = '{"case":"a","mc":"26.3","scenario_sha256":"s","sha256":"h1","version":1}'


class PromotionGuardTests(unittest.TestCase):
    """The guard on a tiny Cargo workspace: tests by identity (real `cargo nextest list`), corpus lock lines.

    Every refusal here is that guard path's negative control; the passing cases show a fix that keeps
    every test is not refused for merging assertions or adding tests."""

    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root.parent / f"{self.root.name}-artifacts", True)
        self.addCleanup(shutil.rmtree, self.root, True)
        git(self.root, "init", "-q")
        git(self.root, "config", "user.email", "t@example.invalid")
        git(self.root, "config", "user.name", "t")
        self.write({"Cargo.toml": '[workspace]\nmembers = ["crates/demo"]\nresolver = "2"\n',
                    "crates/demo/Cargo.toml": '[package]\nname = "demo"\nversion = "0.1.0"\nedition = "2021"\n',
                    "crates/demo/src/lib.rs": LIB, "crates/demo/src/a_tests.rs": A_TESTS,
                    "crates/demo/src/b_tests.rs": B_TESTS, ".gitignore": "target/\n", guard.LOCK_REL: LOCK_A + "\n"})
        self.base = self.commit("base")
        artifacts = self.root.parent / f"{self.root.name}-artifacts"
        artifacts.mkdir()
        self.ctx = profiles.ProfileContext(artifacts)

    def write(self, files: dict[str, str]) -> None:
        for rel, text in files.items():
            path = self.root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)

    def commit(self, message: str) -> str:
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", message)
        return git(self.root, "rev-parse", "HEAD")

    def fix(self, files: dict[str, str]) -> str:
        """Commit `files` as the fix and return its diff (the candidate is the repository itself)."""
        self.write(files)
        fix = self.commit("fix")
        return git(self.root, "diff", "--binary", f"{fix}^1", fix) + "\n"

    def real(self, tree: Path, packages: list[str]) -> dict[str, bool]:
        return ledger._test_listing(self.ctx, tree, packages)

    def test_a_dropped_mod_declaration_is_refused(self):
        diff = self.fix({"crates/demo/src/lib.rs": "#[cfg(test)]\nmod a_tests;\n"})  # b_tests.rs stays
        with self.assertRaisesRegex(guard.GuardRefused, r"drops 1 test.*demo::b_tests::three"):
            guard.promotion_guard(self.root, diff, self.real, patch=False)

    def test_a_deleted_test_replaced_by_another_is_refused(self):
        diff = self.fix({"crates/demo/src/a_tests.rs": A_TESTS.replace("fn two()", "fn four()")})
        with self.assertRaisesRegex(guard.GuardRefused, r"drops 1 test.*test identity changed: demo::a_tests::two -> "
                                                        "missing; land the move as its own commit"):
            guard.promotion_guard(self.root, diff, self.real, patch=False)

    def test_a_new_ignore_is_refused(self):
        diff = self.fix({"crates/demo/src/b_tests.rs": "#[test]\n#[ignore]\nfn three() {}\n"})
        with self.assertRaisesRegex(guard.GuardRefused, r"ignores 1 test.*demo::b_tests::three"):
            guard.promotion_guard(self.root, diff, self.real, patch=False)

    def test_merged_assertions_and_an_added_test_pass(self):
        merged = A_TESTS.replace("    assert_eq!(1, 1);\n    assert_eq!(2, 2);\n", "    assert_eq!((1, 2), (1, 2));\n")
        diff = self.fix({"crates/demo/src/a_tests.rs": merged + "\n#[test]\nfn five() {}\n"})
        record = guard.promotion_guard(self.root, diff, self.real, patch=False)
        self.assertEqual((record["verdict"], record["packages"]), ("pass", ["demo"]))
        self.assertEqual((record["tests"]["baseline"], record["tests"]["after"], record["tests"]["added"]),
                         (3, 4, ["demo::a_tests::five"]))

    @staticmethod
    def fake(tree: Path, packages: list[str]) -> dict[str, bool]:
        """Names of the `fn` items in the package's test files: enough to see which tree was measured."""
        names = {}
        for path in sorted((tree / "crates/demo/src").glob("*_tests.rs")):
            for line in path.read_text().splitlines():
                if line.startswith("fn "):
                    names[f"demo::{path.stem}::{line[3:].split('(')[0]}"] = False
        return names

    def test_a_removed_lock_line_is_refused(self):
        diff = self.fix({guard.LOCK_REL: ""})
        with self.assertRaisesRegex(guard.GuardRefused, "removes 1 recorded-corpus lock line"):
            guard.promotion_guard(self.root, diff, self.fake, patch=False)

    def test_a_new_version_of_a_recorded_case_is_refused_and_a_new_case_is_not(self):
        v2 = LOCK_A.replace('"h1","version":1', '"h2","version":2')
        diff = self.fix({guard.LOCK_REL: f"{LOCK_A}\n{v2}\n"})
        with self.assertRaisesRegex(guard.GuardRefused, r"replayed version of recorded case\(s\) \['a v2'\]"):
            guard.promotion_guard(self.root, diff, self.fake, patch=False)
        git(self.root, "checkout", "-q", "--", ".")
        git(self.root, "reset", "-q", "--hard", self.base)
        other = LOCK_A.replace('"case":"a"', '"case":"b"')
        diff = self.fix({guard.LOCK_REL: f"{LOCK_A}\n{other}\n"})
        record = guard.promotion_guard(self.root, diff, self.fake, patch=False)
        self.assertEqual(record["lock"]["added_cases"], ["b"])

    def test_a_patch_whose_test_hunks_are_not_in_the_tree_is_refused(self):
        self.write({"crates/demo/src/b_tests.rs": ""})  # the uncommitted fix deletes `three`
        patch = git(self.root, "diff", "--binary") + "\n"
        claimed = patch + ("diff --git a/crates/demo/src/c_tests.rs b/crates/demo/src/c_tests.rs\nnew file mode 100644\n"
                           "--- /dev/null\n+++ b/crates/demo/src/c_tests.rs\n@@ -0,0 +1 @@\n+fn ghost() {}\n")
        with self.assertRaisesRegex(guard.GuardRefused, "does not reverse as a whole"):
            guard.promotion_guard(self.root, claimed, self.fake, patch=True)
        with self.assertRaisesRegex(guard.GuardRefused, r"drops 1 test.*demo::b_tests::three"):
            guard.promotion_guard(self.root, patch, self.fake, patch=True)

    def test_a_fix_that_no_longer_reverses_on_the_candidate_is_refused(self):
        """No historical tree stands in for the candidate: a later commit editing the fix's lines refuses."""
        diff = self.fix({"crates/demo/src/b_tests.rs": ""})
        self.write({"crates/demo/src/b_tests.rs": "fn later() {}\n"})
        self.commit("later")
        with self.assertRaisesRegex(guard.GuardRefused, "fix no longer reverses on the candidate; use patch: mode"):
            guard.promotion_guard(self.root, diff, self.fake, patch=False)

    def test_a_nextest_filter_outside_crates_lists_the_whole_workspace(self):
        """A fix in `demo` that also filters out package `other`'s test through .config/nextest.toml
        (nextest 0.9.143 leaves a default-filtered test out of `list` altogether)."""
        self.write({"Cargo.toml": '[workspace]\nmembers = ["crates/demo", "crates/other"]\nresolver = "2"\n',
                    "crates/other/Cargo.toml": '[package]\nname = "other"\nversion = "0.1.0"\nedition = "2021"\n',
                    "crates/other/src/lib.rs": "#[cfg(test)]\nmod tests {\n    #[test]\n    fn kept() {}\n}\n"})
        self.commit("second package")
        diff = self.fix({".config/nextest.toml": '[profile.default]\ndefault-filter = "not package(other)"\n',
                         "crates/demo/src/a_tests.rs": A_TESTS + "\n#[test]\nfn five() {}\n"})
        listed = []

        def lister(tree: Path, packages: list[str] | None) -> dict[str, bool]:
            listed.append(packages)
            return self.real(tree, packages)

        with self.assertRaisesRegex(guard.GuardRefused, r"drops 1 test.*other::tests::kept -> missing"):
            guard.promotion_guard(self.root, diff, lister, patch=False)
        self.assertEqual(listed, [None, None])

    def test_a_ci_only_nextest_filter_cannot_hide_a_test(self):
        self.write({".config/nextest.toml": "[profile.ci]\nretries = 0\n"})
        self.commit("CI profile")
        diff = self.fix({".config/nextest.toml": '[profile.ci]\ndefault-filter = "not test(=b_tests::three)"\n'})
        with self.assertRaisesRegex(guard.GuardRefused, r"(drops|ignores) 1 test.*ci/demo::b_tests::three"):
            guard.promotion_guard(self.root, diff, self.real, patch=False)


if __name__ == "__main__":
    unittest.main()
