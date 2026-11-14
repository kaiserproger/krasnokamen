"""Tests for ``python3 -m tools.harness plan``: one per rule class, plus union and drift checks."""

from __future__ import annotations

import ast
import contextlib
import io
import json
import re
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from . import __main__ as cli
from . import plan, profiles, scenarios

REPO = Path(__file__).resolve().parents[2]
PY = "python3 -m tools.harness run"


def build(paths: list[str], root: Path = REPO, base: str = "HEAD") -> dict:
    return plan.build_plan(root, base, paths)


def gates(result: dict) -> dict[str, dict]:
    return {entry["gate"]: entry for entry in result["entries"]}


def git_tree(files: dict[str, str]) -> tempfile.TemporaryDirectory:
    """A throwaway core-shaped git tree with `files` committed."""
    tmp = tempfile.TemporaryDirectory()
    root = Path(tmp.name)
    (root / "tools" / "harness").mkdir(parents=True)
    (root / "tools" / "harness" / "__main__.py").write_text("", encoding="utf-8")
    write_tree(root, files)
    for args in (["init", "-q"], ["add", "-A"],
                 ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "base"]):
        subprocess.run(["git", "-C", str(root), *args], check=True, capture_output=True)
    return tmp


def write_tree(root: Path, files: dict[str, str]) -> None:
    for rel, text in files.items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")


def ledger(rows: dict[str, tuple[str, str]]) -> str:
    """rows: id -> (status, extra toml lines)."""
    return "".join(
        f'[[bug]]\nid = "{row_id}"\ntitle = "t"\narea = "gameplay"\nseverity = "major"\n'
        f'status = "{status}"\nsource = "s"\nrepro = "cargo"\nregression = []\n{extra}\n'
        for row_id, (status, extra) in rows.items())


class RustTests(unittest.TestCase):
    def test_rust_change_gets_l0_for_the_crate_l1_and_l2(self):
        result = build(["crates/mc-core/src/beacon.rs", "crates/mc-edge/src/backup_tests.rs"])
        found = gates(result)
        l0 = found[f"{PY} test -p mc-core --lib beacon"]
        self.assertEqual((l0["runs"], l0["negative_control"], l0["reviewer"]), (1, True, []))
        self.assertIn(f"{PY} test -p mc-edge --lib backup_tests", found)
        self.assertFalse(found[f"{PY} test -p mc-edge --lib backup_tests"]["negative_control"])
        self.assertEqual(found[f"{PY} fmt"]["when"], "slice")
        self.assertEqual(found[f"{PY} code-health"]["when"], "slice")
        self.assertEqual(found[f"{PY} correctness"]["when"], "commit")

    def test_filters_are_minimal_and_manifests_widen_the_scope(self):
        found = gates(build(["crates/mc-core/src/core.rs", "crates/mc-core/src/core/journal.rs",
                             "crates/mc-core/src/tick/hydration.rs", "crates/mc-core/src/tick/mod.rs"]))
        self.assertIn(f"{PY} test -p mc-core --lib core tick", found)
        found = gates(build(["crates/mc-core/Cargo.toml", "crates/mc-core/src/beacon.rs"]))
        self.assertIn(f"{PY} test -p mc-core", found)
        self.assertNotIn(f"{PY} test -p mc-core --lib beacon", found)
        found = gates(build(["crates/mc-edge/tests/handshake.rs", "Cargo.lock"]))
        self.assertIn(f"{PY} test -p mc-edge --test handshake", found)
        self.assertIn(f"{PY} test", found)


class PersistenceTests(unittest.TestCase):
    def test_persistence_path_needs_restart_tests_and_the_reviewer(self):
        result = build(["crates/mc-core/src/core/journal.rs"])
        entry = gates(result)[f"{PY} test -p mc-core --lib restart recovery checkpoint journal reopen"]
        self.assertEqual(entry["reviewer"], ["persistence"])
        self.assertIn("docs/architecture/persistence.md", entry["why"][0])
        self.assertEqual([r["kind"] for r in result["reviewers"]], ["persistence"])

    def test_test_only_persistence_slice_has_no_reviewer(self):
        result = build(["crates/mc-core/src/chunk_loader_tests.rs"])
        self.assertIn(f"{PY} test -p mc-core --lib restart recovery checkpoint journal reopen", gates(result))
        self.assertEqual(result["reviewers"], [])

    def test_world_storage_and_anvil_are_persistence(self):
        found = gates(build(["crates/mc-world/src/anvil/chunk_nbt.rs", "crates/mc-world/src/native/mod.rs"]))
        self.assertIn(f"{PY} test -p mc-world --lib restart recovery checkpoint journal reopen", found)


class ProtocolTests(unittest.TestCase):
    def test_protocol_needs_tests_a_real_client_gate_and_the_reviewer(self):
        result = build(["crates/mc-protocol/src/packets/play.rs", "crates/mc-edge/src/translation/mod.rs"])
        found = gates(result)
        self.assertEqual(found[f"{PY} test -p mc-protocol"]["reviewer"], ["protocol"])
        self.assertIn(f"{PY} test -p mc-edge --lib wire decode translation", found)
        unmapped = [g for g in found if g.startswith("no mapped gate: choose from core-client")]
        self.assertEqual(len(unmapped), 1)
        self.assertEqual(found[unmapped[0]]["reviewer"], ["protocol"])


class CustodyTests(unittest.TestCase):
    def test_container_release_in_merge_is_custody(self):
        result = build(["crates/mc-core/src/tick/merge.rs"])
        found = gates(result)
        self.assertEqual(found[f"{PY} test -p mc-core --lib conservation custody"]["reviewer"], ["custody"])
        self.assertIn(f"{PY} inventory --client native --windowed", found)
        self.assertEqual([r["kind"] for r in result["reviewers"]], ["custody"])

    def test_item_entities_and_menus_are_custody(self):
        for path in ("crates/mc-core/src/item_motion.rs", "crates/mc-core/src/mounts/menus.rs",
                     "crates/mc-core/src/inventory/click.rs", "crates/mc-core/src/tick/containers.rs"):
            with self.subTest(path=path):
                self.assertEqual([r["kind"] for r in build([path])["reviewers"]], ["custody"])


class GameplayTests(unittest.TestCase):
    def test_settlement_change_maps_to_both_settlement_gates_variable(self):
        found = gates(build(["crates/mc-core/src/settlements/construction.rs"]))
        for profile in ("civilization-seed-qa", "village-autonomy-qa"):
            entry = found[f"{PY} {profile} --client native --windowed"]
            self.assertEqual(entry["runs"], 3)
            self.assertTrue(any("variable" in note for note in entry["notes"]))

    def test_unmapped_gameplay_prints_the_choices(self):
        found = gates(build(["crates/mc-core/src/beacon.rs"]))
        unmapped = [e for g, e in found.items() if g.startswith("no mapped gate: choose from admin-commands")]
        self.assertEqual(len(unmapped), 1)
        self.assertEqual(unmapped[0]["paths"], ["crates/mc-core/src/beacon.rs"])

    def test_tests_and_persistence_only_files_ask_for_no_real_client_gate(self):
        found = gates(build(["crates/mc-core/src/beacon_tests.rs", "crates/mc-core/src/core/journal.rs"]))
        self.assertFalse([g for g in found if g.startswith("no mapped gate")])

    def test_every_area_gate_exists(self):
        for area in plan.AREAS:
            for gate in area.gates:
                kind, _, name = gate.partition(":")
                with self.subTest(area=area.id, gate=gate):
                    if kind == "profile":
                        self.assertIn(name, profiles.PROFILES)
                    else:
                        self.assertIn(name, scenarios.SCENARIOS)


class ChangedGateTests(unittest.TestCase):
    def test_changed_backend_needs_negative_control_and_three_runs_when_variable(self):
        result = build(["tools/harness/backends/civilization_seed_qa.py"])
        entry = gates(result)[f"{PY} civilization-seed-qa --client native --windowed"]
        self.assertEqual((entry["runs"], entry["negative_control"]), (3, True))
        self.assertTrue(any("real client" in note for note in entry["notes"]))

    def test_nonvariable_gate_needs_one_run_with_control(self):
        entry = gates(build(["tools/harness/backends/dialogs.py"]))[f"{PY} dialogs --client native --windowed"]
        self.assertEqual((entry["runs"], entry["negative_control"]), (1, True))

    def test_shared_backend_with_a_scenario_runs_the_scenario(self):
        found = gates(build(["tools/harness/backends/native_craft_drag_paced.py"]))
        self.assertIn(f"{PY} scenario native-v2-craft-drag-paced --client native --windowed", found)
        shared = gates(build(["tools/harness/backends/native_craft_drag.py"]))
        self.assertIn(f"{PY} entities --client native --windowed", shared)

    def test_unmapped_backend_prints_the_choices(self):
        found = gates(build(["tools/harness/backends/not_a_gate_engine.py"]))
        entry = next(e for g, e in found.items() if g.startswith("no mapped gate"))
        self.assertTrue(entry["negative_control"])

    def test_only_added_and_changed_scenarios_are_gates(self):
        entry_a = ('"a": _entry(id="a", profile="dialogs", summary="x", sources=("x.py",)),\n'
                   '    "b": _entry(id="b", profile="entities", summary="y", sources=("y.py",)),\n')
        template = "SCENARIOS = {\n    %s}\n"
        with git_tree({"tools/harness/scenarios.py": template % entry_a}) as tmp:
            root = Path(tmp)
            changed = entry_a.replace('summary="y"', 'summary="z"') + (
                '    "c": _entry(id="c", profile="dialogs", summary="new", sources=()),\n')
            write_tree(root, {"tools/harness/scenarios.py": template % changed})
            found = gates(build(["tools/harness/scenarios.py"], root))
        self.assertIn(f"{PY} scenario b --client native --windowed", found)
        self.assertIn(f"{PY} scenario c --client native --windowed", found)
        self.assertNotIn(f"{PY} scenario a --client native --windowed", found)
        self.assertTrue(found[f"{PY} scenario c --client native --windowed"]["negative_control"])

    def test_changed_profile_is_a_gate(self):
        text = ('def run_x(ctx):\n    return {}\n'
                'PROFILES = {"dialogs": {"scope": "real-client", "description": "d", "run": run_x}}\n')
        with git_tree({"tools/harness/profiles.py": text}) as tmp:
            root = Path(tmp)
            write_tree(root, {"tools/harness/profiles.py": text.replace("return {}", "return {'a': 1}")})
            found = gates(build(["tools/harness/profiles.py"], root))
        self.assertIn(f"{PY} dialogs --client native --windowed", found)

    def test_harness_python_gets_module_tests_and_the_suite(self):
        found = gates(build(["tools/harness/plan.py"]))
        self.assertIn("python3 -m unittest tools.harness.test_plan", found)
        self.assertIn(plan.HARNESS_SUITE, found)


class LedgerTests(unittest.TestCase):
    def ledger_plan(self, before: dict, after: dict) -> dict:
        with git_tree({"docs/bugs/ledger.toml": ledger(before)}) as tmp:
            root = Path(tmp)
            write_tree(root, {"docs/bugs/ledger.toml": ledger(after)})
            return build(["docs/bugs/ledger.toml"], root)

    def test_edit_without_status_change_runs_regressions_only(self):
        result = self.ledger_plan({"A-ONE": ("open", "")}, {"A-ONE": ("open", "")})
        self.assertEqual(list(gates(result)), [f"{PY} regressions"])
        self.assertEqual(result["reviewers"], [])

    def test_batch_of_five_promotions_needs_the_reviewer(self):
        ids = [f"A-{n}" for n in range(5)]
        promoted = 'fix_commit = "abc"\npromotion = "docs/bugs/promotions/x.json"\n'
        result = self.ledger_plan({i: ("open", "") for i in ids}, {i: ("fixed", promoted) for i in ids})
        found = gates(result)
        self.assertEqual(len([g for g in found if "ledger promote" in g]), 5)
        self.assertTrue(all(found[g]["reviewer"] == ["ledger-batch"] for g in found if "ledger promote" in g))
        self.assertEqual([r["kind"] for r in result["reviewers"]], ["ledger-batch"])

    def test_four_promotions_need_no_reviewer(self):
        ids = [f"A-{n}" for n in range(4)]
        promoted = 'fix_commit = "abc"\npromotion = "docs/bugs/promotions/x.json"\n'
        result = self.ledger_plan({i: ("open", "") for i in ids}, {i: ("fixed", promoted) for i in ids})
        self.assertEqual(result["reviewers"], [])

    def test_hand_set_fixed_is_a_warning(self):
        result = self.ledger_plan({"A-ONE": ("open", "")}, {"A-ONE": ("fixed", "")})
        self.assertTrue(any("A-ONE" in w and "ledger promote" in w for w in result["warnings"]))

    def test_client_row_promotion_adds_the_client_regressions(self):
        extra = 'fix_commit = "abc"\npromotion = "p"\n'
        before = ledger({"A-ONE": ("open", "")})
        after = ledger({"A-ONE": ("fixed", extra)}).replace('repro = "cargo"', 'repro = "client"')
        with git_tree({"docs/bugs/ledger.toml": before}) as tmp:
            root = Path(tmp)
            write_tree(root, {"docs/bugs/ledger.toml": after})
            self.assertIn(f"{PY} regressions-client", gates(build(["docs/bugs/ledger.toml"], root)))


class InnovationTests(unittest.TestCase):
    MANIFEST = ('[package]\nname = "x"\n\n[dependencies]\n'
                'serde = "1.0.100"\nmc-nbt = { path = "../mc-nbt" }\n')

    def innovation(self, files_before: dict[str, str], files_after: dict[str, str], paths: list[str]) -> dict:
        with git_tree(files_before) as tmp:
            root = Path(tmp)
            write_tree(root, files_after)
            return build(paths, root)

    def records(self, result: dict) -> list[str]:
        return [g for g in gates(result) if g.startswith("decision record required")]

    def test_added_dependency_needs_a_decision_record_without_an_extra_reviewer(self):
        path = "crates/x/Cargo.toml"
        result = self.innovation({path: self.MANIFEST},
                                 {path: self.MANIFEST + 'left-pad = "0.1"\nmc-other = { path = "../o" }\n'}, [path])
        (record,) = self.records(result)
        self.assertIn("left-pad", record)
        self.assertEqual(gates(result)[record]["reviewer"], [])
        self.assertEqual(result["reviewers"], [])

    def test_version_bump_removal_and_workspace_reuse_are_not_innovation(self):
        path = "crates/x/Cargo.toml"
        root_manifest = '[workspace]\n[workspace.dependencies]\ntokio = "1"\n'
        bumped = self.MANIFEST.replace("1.0.100", "1.0.200") + 'tokio.workspace = true\n'
        result = self.innovation({path: self.MANIFEST, "Cargo.toml": root_manifest},
                                 {path: bumped}, [path])
        self.assertEqual(self.records(result), [])
        self.assertEqual(result["reviewers"], [])
        removed = self.innovation({path: self.MANIFEST}, {path: '[package]\nname = "x"\n'}, [path])
        self.assertEqual(self.records(removed), [])

    def test_new_workspace_dependency_is_innovation(self):
        before = '[workspace]\n[workspace.dependencies]\ntokio = "1"\n'
        result = self.innovation({"Cargo.toml": before},
                                 {"Cargo.toml": before + 'axum = "0.7"\n'}, ["Cargo.toml"])
        self.assertTrue(self.records(result)[0].endswith("new dependency axum"))

    def test_toolchain_and_cargo_config_change_is_innovation(self):
        files = {"rust-toolchain.toml": '[toolchain]\nchannel = "1.95.0"\n', ".cargo/config.toml": "a = 1\n"}
        result = self.innovation(files, {"rust-toolchain.toml": '[toolchain]\nchannel = "nightly"\n'},
                                 ["rust-toolchain.toml"])
        (record,) = self.records(result)
        self.assertIn("rust-toolchain.toml", record)
        self.assertEqual(gates(result)[record]["reviewer"], [])
        unchanged = self.innovation(files, {}, ["rust-toolchain.toml", ".cargo/config.toml"])
        self.assertEqual(self.records(unchanged), [])

    def test_new_prerequisite_token_is_innovation_but_reuse_is_not(self):
        text = ('_NONE = ((), (), (), ())\n_X = (("xvfb",), (), (), ())\n'
                'PROFILE_PREREQUISITES = {"a": _NONE, "b": _X}\n')
        path = "tools/harness/profiles.py"
        reuse = text.replace('"a": _NONE', '"a": _X')
        self.assertEqual(self.records(self.innovation({path: text}, {path: reuse}, [path])), [])
        new = text.replace('"a": _NONE', '"a": (("blender",), (), (), ())')
        (record,) = self.records(self.innovation({path: text}, {path: new}, [path]))
        self.assertIn("new required prerequisite blender (profile a)", record)


class MarkdownTests(unittest.TestCase):
    def test_markdown_only_change_needs_doclinks_only(self):
        result = build(["docs/architecture/persistence.md", "AGENTS.md"])
        self.assertEqual(list(gates(result)), ["python3 tools/doclinks.py"])
        self.assertEqual(result["reviewers"], [])
        self.assertEqual(result["manual_classification"], [])


class SiblingTests(unittest.TestCase):
    def test_client_path_gets_strict_clippy_tests_and_a_core_real_client_gate(self):
        found = gates(build(["../solaris-client/src/ui/hud.rs"]))
        self.assertIn("cargo clippy --workspace --all-targets --locked -- -D warnings", found)
        self.assertEqual(found["cargo clippy --workspace --all-targets --locked -- -D warnings"]["when"], "integrate")
        self.assertIn("cargo test --workspace --locked", found)
        unmapped = next(e for g, e in found.items() if g.startswith("no mapped gate"))
        self.assertEqual(unmapped["cwd"], "../solaris")

    def test_loader_and_plugin_paths(self):
        found = gates(build(["../solaris-loader/loader-core/src/main/java/A.java",
                             "../solaris-default-plugins/krasnokamen-towns/plugin.toml"]))
        self.assertIn("./gradlew :loader-core:test :loader-fabric:test :loader-neoforge:test :loader-forge:test", found)
        self.assertIn(f"{PY} loader-live --platform fabric", found)
        self.assertIn("python3 tools/build_standard_pack.py --core-root ../solaris --check", found)
        self.assertIn(f"{PY} standard-pack --real-client", found)

    def test_sibling_paths_do_not_trigger_core_rules(self):
        found = gates(build(["../solaris-client/src/ui/hud.rs"]))
        self.assertNotIn(f"{PY} correctness", found)

    def test_a_sibling_checkout_classifies_its_own_diff(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "loader-core").mkdir()
            (root / "settings.gradle.kts").write_text("", encoding="utf-8")
            result = build(["loader-core/src/A.java"], root)
        self.assertEqual(result["repo"], "loader")
        self.assertIn("./gradlew :loader-core:test :loader-fabric:test :loader-neoforge:test :loader-forge:test",
                      gates(result))


class ManualClassificationTests(unittest.TestCase):
    def test_unmatched_path_is_never_silent(self):
        result = build(["Justfile"])
        self.assertEqual([m["path"] for m in result["manual_classification"]], ["Justfile"])
        self.assertIn("manual classification required: Justfile", plan.render(result))

    def test_two_domain_path_is_never_silent(self):
        path = "crates/mc-core/src/core_checkpoint_custody_tests.rs"
        result = build([path])
        (item,) = result["manual_classification"]
        self.assertEqual((item["path"], item["domains"]), (path, ["persistence", "custody"]))
        self.assertIn(f"manual classification required: {path}", plan.render(result))
        # The union of both domains' rules is still printed.
        found = gates(result)
        self.assertIn(f"{PY} test -p mc-core --lib conservation custody", found)
        self.assertIn(f"{PY} test -p mc-core --lib restart recovery checkpoint journal reopen", found)

    def test_single_domain_paths_are_classified(self):
        paths = ["crates/mc-core/src/beacon.rs", "crates/mc-core/src/core/journal.rs",
                 "crates/mc-core/src/tick/merge.rs", "tools/harness/backends/dialogs.py"]
        self.assertEqual(build(paths)["manual_classification"], [])


class UnionTests(unittest.TestCase):
    def test_mixed_change_unions_dedupes_and_takes_the_strictest_runs(self):
        result = build([
            "crates/mc-core/src/settlements/construction.rs",    # area gate, 3 runs (variable)
            "tools/harness/backends/village_autonomy_qa.py",     # changed gate: same profile + control
            "crates/mc-core/src/core/journal.rs",                # persistence
            "crates/mc-core/src/tick/merge.rs",                  # custody
            "docs/settlements/construction.md",                  # markdown
        ])
        all_gates = [e["gate"] for e in result["entries"]]
        self.assertEqual(len(all_gates), len(set(all_gates)))
        entry = gates(result)[f"{PY} village-autonomy-qa --client native --windowed"]
        self.assertEqual((entry["runs"], entry["negative_control"]), (3, True))
        self.assertEqual(len(entry["why"]), 2)
        self.assertEqual(sorted(r["kind"] for r in result["reviewers"]), ["custody", "persistence"])
        self.assertIn("python3 tools/doclinks.py", all_gates)
        self.assertEqual(all_gates.count(f"{PY} correctness"), 1)

    def test_plan_json_roundtrips_through_the_cli(self):
        out = io.StringIO()
        with mock.patch.object(plan, "git_root", return_value=REPO), contextlib.redirect_stdout(out):
            code = cli.main(["plan", "--paths", "docs/STATUS.md", "--json"])
        self.assertEqual(code, 0)
        data = json.loads(out.getvalue())
        self.assertEqual([e["gate"] for e in data["entries"]], ["python3 tools/doclinks.py"])
        for key in ("gate", "why", "runs", "negative_control", "reviewer"):
            self.assertIn(key, data["entries"][0])


class TableDriftTests(unittest.TestCase):
    """The data tables must agree with the catalogs they name."""

    def test_native_windowed_profiles_are_enabled_for_native(self):
        source = (REPO / "tools/harness/__main__.py").read_text(encoding="utf-8")
        match = re.search(r'client == "native" and profile not in \{([^}]*)\}', source)
        self.assertIsNotNone(match)
        allowed = set(re.findall(r'"([^"]+)"', match.group(1)))
        self.assertEqual(plan.NATIVE_WINDOWED - allowed, set())

    def test_profile_names_in_tables_exist(self):
        names = set(profiles.PROFILES)
        used = set(plan.VARIABLE_PROFILES) | plan.NATIVE_WINDOWED
        for targets in plan.BACKEND_PROFILES.values():
            used |= set(targets)
        self.assertEqual(used - names, set())

    def test_backend_engines_are_reachable_from_their_profiles(self):
        """Each BACKEND_PROFILES row: the profile's run function imports the engine, directly or transitively."""
        backends = REPO / "tools/harness/backends"

        def imports(tree: ast.AST) -> set[str]:
            found = set()
            for node in ast.walk(tree):
                if isinstance(node, ast.ImportFrom):
                    module = node.module or ""
                    if module.endswith("backends") or module == "":
                        found |= {alias.name for alias in node.names}
                    elif node.level:
                        found.add(module.split(".")[0])  # `from .x import y`
                    elif "backends." in module:
                        found.add(module.rsplit(".", 1)[-1])
            return found

        def closure(start: set[str]) -> set[str]:
            seen, queue = set(), list(start)
            while queue:
                name = queue.pop()
                if name in seen:
                    continue
                seen.add(name)
                path = backends / f"{name}.py"
                if path.is_file():
                    queue.extend(imports(ast.parse(path.read_text(encoding="utf-8"))))
            return seen

        tree = ast.parse((REPO / "tools/harness/profiles.py").read_text(encoding="utf-8"))
        functions = {n.name: n for n in tree.body if isinstance(n, ast.FunctionDef)}
        run_of: dict[str, str] = {}
        for node in tree.body:
            if isinstance(node, ast.AnnAssign) and getattr(node.target, "id", "") == "PROFILES":
                for key, value in zip(node.value.keys, node.value.values):
                    spec = {k.value: v for k, v in zip(value.keys, value.values)}
                    run_of[key.value] = spec["run"].id
        for stem, targets in plan.BACKEND_PROFILES.items():
            for profile in targets:
                with self.subTest(engine=stem, profile=profile):
                    reachable = closure(imports(functions[run_of[profile]]))
                    self.assertIn(stem, reachable)


if __name__ == "__main__":
    unittest.main()
