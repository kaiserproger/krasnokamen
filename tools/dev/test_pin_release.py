"""Release-pin target/kind and pooled-copy regressions; no Cargo builds."""
from __future__ import annotations

from contextlib import ExitStack, redirect_stdout
import importlib.machinery
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
loader = importlib.machinery.SourceFileLoader("pin_release", str(HERE / "pin-release"))
spec = importlib.util.spec_from_loader(loader.name, loader)
pin = importlib.util.module_from_spec(spec)
sys.modules[loader.name] = pin
loader.exec_module(pin)

LINUX = "x86_64-unknown-linux-gnu"
WINDOWS = "x86_64-pc-windows-gnu"


class PinReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.targets = self.root / "targets"
        self.slot = self.targets / "slot-a"
        self.slot.mkdir(parents=True)

    def test_kind_and_target_are_required_for_reuse(self):
        directory = self.root / "pin"
        directory.mkdir()
        binary = directory / "krasnokamen-client.exe"
        binary.write_bytes(b"test pin")
        manifest = {"repo": "client", "kind": "client", "target": WINDOWS, "inputs_sha256": "C", "env_deps": {},
                    "binary": str(binary), "binary_sha256": pin.w.sha256_file(binary)}
        (directory / "manifest.json").write_text(json.dumps(manifest))
        self.assertEqual(pin.existing_pin(directory, "client", WINDOWS, False, "C"), binary)
        with self.assertRaises(pin.w.ToolError):
            pin.existing_pin(directory, "client", LINUX, False, "C")
        with self.assertRaises(pin.w.ToolError):  # a pin without the field predates incremental builds
            pin.existing_pin(directory, "client", WINDOWS, True, "C")
        manifest.update(repo="core", kind="server")
        (directory / "manifest.json").write_text(json.dumps(manifest))
        with self.assertRaises(pin.w.ToolError):
            pin.existing_pin(directory, "bots", WINDOWS, False)
        manifest.update(repo="client", kind="client", incremental=True)
        (directory / "manifest.json").write_text(json.dumps(manifest))
        self.assertEqual(pin.existing_pin(directory, "client", WINDOWS, True, "C"), binary)
        with self.assertRaises(pin.w.ToolError):
            pin.existing_pin(directory, "client", WINDOWS, False, "C")

    def test_a_pin_of_another_working_state_is_refused_not_reused(self):
        directory = self.root / "pin"
        directory.mkdir()
        binary = directory / "krasnokamen"
        binary.write_bytes(b"another tree's server")
        before = {"head": "9" * 40, "diff_sha256": "8" * 64, "dirty": False, "untracked_files": 0}
        manifest = {"repo": "core", "kind": "server", "target": LINUX, "incremental": True,
                    "head": "1" * 40, "diff_sha256": "2" * 64, "binary": str(binary),
                    "binary_sha256": pin.w.sha256_file(binary), "inputs_sha256": "K", "env_deps": {}}
        (directory / "manifest.json").write_text(json.dumps(manifest))
        # the pre-fix cross-tree reuse wrote exactly this: another HEAD under this tree's pin name
        with self.assertRaisesRegex(pin.w.ToolError, "another tree's binary is never reused"):
            pin.existing_pin(directory, "server", LINUX, True, "K", before)
        manifest.update(head=before["head"], diff_sha256=before["diff_sha256"])
        (directory / "manifest.json").write_text(json.dumps(manifest))
        self.assertEqual(pin.existing_pin(directory, "server", LINUX, True, "K", before), binary)

    def test_a_client_pin_fails_closed_without_inputs_or_with_other_codegen_parameters(self):
        directory = self.root / "pin"
        directory.mkdir()
        binary = directory / "krasnokamen-client"
        binary.write_bytes(b"client")
        manifest = {"repo": "client", "kind": "client", "target": LINUX, "incremental": False,
                    "binary": str(binary), "binary_sha256": pin.w.sha256_file(binary)}  # no inputs, no env_deps
        (directory / "manifest.json").write_text(json.dumps(manifest))
        with self.assertRaisesRegex(pin.w.ToolError, "records no build inputs"):
            pin.existing_pin(directory, "client", LINUX, False, "C")
        manifest["inputs_sha256"] = "C"
        (directory / "manifest.json").write_text(json.dumps(manifest))
        with self.assertRaisesRegex(pin.w.ToolError, "records no build-script environment"):
            pin.existing_pin(directory, "client", LINUX, False, "C")
        manifest["env_deps"] = {}
        (directory / "manifest.json").write_text(json.dumps(manifest))
        self.assertEqual(pin.existing_pin(directory, "client", LINUX, False, "C"), binary)
        # another codegen environment yields another input digest: never this binary
        with self.assertRaisesRegex(pin.w.ToolError, "other build inputs"):
            pin.existing_pin(directory, "client", LINUX, False, "C-with-force-frame-pointers")

    def test_a_relabelled_pin_is_refused_and_never_registered(self):
        # the installed 04af6cc1f could relabel another tree's binary with this tree's provenance
        pins = self.root / "pins"
        pins.mkdir()
        source = pins / "3497c143-old"
        source.mkdir()
        foreign = source / "krasnokamen"
        foreign.write_bytes(b"other tree's server")
        (source / "manifest.json").write_text(json.dumps({
            "repo": "core", "kind": "server", "target": LINUX, "incremental": True,
            "head": "1" * 40, "diff_sha256": "2" * 64, "binary": str(foreign),
            "binary_sha256": pin.w.sha256_file(foreign), "inputs_sha256": "K"}))
        relabelled = pins / "fa637b84c392-server-x86_64-unknown-linux-gnu-incr"
        relabelled.mkdir()
        copied = relabelled / "krasnokamen"
        shutil.copyfile(foreign, copied)
        before = {"head": "b" * 40, "diff_sha256": "c" * 64, "dirty": False, "untracked_files": 0}
        (relabelled / "manifest.json").write_text(json.dumps({
            "repo": "core", "kind": "server", "target": LINUX, "incremental": True,
            "head": before["head"], "diff_sha256": before["diff_sha256"], "binary": str(copied),
            "binary_sha256": pin.w.sha256_file(copied), "inputs_sha256": "K", "reused_from": source.name}))
        with self.assertRaisesRegex(pin.w.ToolError, "relabelled pin"):
            pin.existing_pin(relabelled, "server", LINUX, True, "K", before)
        # a source that is missing or damaged is equally refused
        shutil.rmtree(source)
        with self.assertRaisesRegex(pin.w.ToolError, "missing or unreadable"):
            pin.existing_pin(relabelled, "server", LINUX, True, "K", before)

    def test_cli_namespaces_targets_and_kinds_and_keeps_in_use_mark(self):
        pins = self.root / "pins"
        locks = self.root / "locks"
        locks.mkdir()
        tree = self.root / "Tree"
        tree.mkdir()
        (self.slot / ".krasnokamen-tree").write_text(str(tree))
        host_build = self.slot / "release/build/zstd-sys-0/output"  # a build script of the host profile dir
        host_build.parent.mkdir(parents=True)
        host_build.write_text("cargo:rerun-if-env-changed=ZSTD_SYS_USE_PKG_CONFIG\n")
        provenance = {"id": "123456789abc-dirty123", "head": "1" * 40, "tree": "2" * 40,
                      "diff_sha256": "3" * 64, "dirty": True, "untracked_files": 0}
        paths = []
        cases = (("client", WINDOWS, True), ("client", LINUX, True), ("bots", LINUX, True),
                 ("server", LINUX, True), ("server", LINUX, False))
        for kind, target, incremental in cases:
            name = pin.KINDS[kind]["bin"] + (".exe" if target == WINDOWS else "")
            artifact = self.slot / target / "release" / name
            artifact.parent.mkdir(parents=True, exist_ok=True)
            artifact.write_bytes((kind + target).encode())
            flags = ["--client"] if kind == "client" else ["--bots"] if kind == "bots" else []
            flags += [] if incremental else ["--no-incremental"]
            output = io.StringIO()
            with ExitStack() as stack:
                stack.enter_context(patch.object(pin.w, "PINS", pins))
                stack.enter_context(patch.object(pin.w, "TARGETS", self.targets))
                stack.enter_context(patch.object(pin, "SLOT_LOCKS", locks))
                stack.enter_context(patch.object(pin.w, "check_host"))
                stack.enter_context(patch.object(pin.w, "git", return_value=str(tree)))
                stack.enter_context(patch.object(pin, "check_tree"))
                stack.enter_context(patch.object(pin, "provenance", return_value=provenance))
                build = stack.enter_context(patch.object(pin, "build", return_value=(["test-build", target], artifact)))
                mark = stack.enter_context(patch.object(pin.w, "in_use_add"))
                stack.enter_context(patch.object(sys, "argv", ["pin-release", str(tree), *flags, "--target", target]))
                stack.enter_context(patch.dict(os.environ))
                os.environ.pop("ZSTD_SYS_USE_PKG_CONFIG", None)
                with redirect_stdout(output):
                    pin.main()
                    pin.main()
                self.assertEqual(build.call_count, 2, "reuse must not rebuild; initial copy does a locked second build")
                self.assertEqual([call.args[3] for call in build.call_args_list], [incremental] * 2)
                directory = pins / f"{provenance['id']}-{kind}-{target}{'-incr' if incremental else ''}"
                final = directory / name
                mark.assert_called_with(directory, tree.name, "pin")
            paths.append(final)
            self.assertEqual(output.getvalue().splitlines(), [f"{pin.KINDS[kind]['env']}={final}"] * 2)
            manifest = json.loads((directory / "manifest.json").read_text())
            self.assertEqual((manifest["kind"], manifest["target"], manifest["incremental"]), (kind, target, incremental))
            # every kind records its input list, its build tree and the env deps of its build
            self.assertIn("inputs_sha256", manifest)
            self.assertEqual(manifest["build_tree"], str(tree))
            self.assertEqual(manifest.get("env_deps"), {"ZSTD_SYS_USE_PKG_CONFIG": None})
            self.assertEqual(final.read_bytes(), artifact.read_bytes())
        self.assertEqual(len(set(paths)), 5)

    def test_foreign_config_target_is_not_labeled_native(self):
        artifact = self.slot / WINDOWS / "release/krasnokamen-client.exe"
        artifact.parent.mkdir(parents=True)
        artifact.touch()
        message = {"reason": "compiler-artifact", "target": {"name": "krasnokamen-client", "kind": ["bin"]},
                   "executable": str(artifact)}

        class Process:
            stdout = [json.dumps(message) + "\n"]
            returncode = 0
            def __enter__(self): return self
            def __exit__(self, *args): return False

        with patch.object(pin.w, "TARGETS", self.targets), \
             patch.object(pin, "host_target", return_value=LINUX), \
             patch.object(pin.subprocess, "Popen", return_value=Process()):
            with self.assertRaisesRegex(pin.w.ToolError, "not a native"):
                pin.build(self.root, "client", None, True, target=self.slot)


def git(cwd, *args):
    subprocess.run(["git", "-C", str(cwd), "-c", "user.name=t", "-c", "user.email=t@t", *args],
                   check=True, capture_output=True)


class InputKeyTests(unittest.TestCase):
    """inputs_sha256 over a real throwaway repository; rustc is stubbed."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.top = Path(self.temp.name) / "tree"
        for name, text in {
            "crates/a/src/lib.rs": "fn a() {}\n", "Cargo.toml": "[workspace]\n", "Cargo.lock": "# lock\n",
            "rust-toolchain.toml": "[toolchain]\n", "sdk/rust/x.rs": "x\n", ".cargo/config.toml": "[build]\n",
            "tools/extract-block-light/L.java": "class L {}\n", "tools/VanillaWorldgenOracle.java": "class O {}\n",
            "tools/dev/pin-release": "# not an input\n", "docs/a.md": "docs\n",
        }.items():
            self.write(name, text)
        subprocess.run(["git", "init", "-q", str(self.top)], check=True)
        git(self.top, "add", "-A")
        git(self.top, "commit", "-qm", "base")
        self.rustc = "rustc 1.99.0\nhost: x86_64-unknown-linux-gnu\n"

    def write(self, name, text):
        path = self.top / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def key(self, kind="server", target=LINUX, incremental=True):
        tree, _ = pin.w.working_state(self.top)
        with patch.object(pin.subprocess, "check_output", return_value=self.rustc):
            return pin.inputs_sha256(self.top, tree, kind, target, incremental)

    def test_key_ignores_changes_outside_the_inputs(self):
        base = self.key()
        self.write("docs/a.md", "edited\n")
        self.write("tools/dev/pin-release", "# edited\n")
        self.write("tools/new-file.txt", "untracked outside\n")
        self.assertEqual(self.key(), base)
        git(self.top, "add", "-A")
        git(self.top, "commit", "-qm", "docs and tools only")
        self.assertEqual(self.key(), base, "a commit that touches nothing in the inputs keeps the key")

    def test_key_changes_for_tracked_dirty_and_untracked_changes_inside_the_inputs(self):
        base = self.key()
        seen = {base}
        for name, text in (("crates/a/src/lib.rs", "fn a() { 1; }\n"), ("Cargo.lock", "# other\n"),
                           ("sdk/rust/x.rs", "y\n"), ("tools/extract-block-light/L.java", "class M {}\n"),
                           ("tools/VanillaWorldgenOracle.java", "class P {}\n"), (".cargo/config.toml", "[env]\n"),
                           ("crates/a/src/new.rs", "untracked\n"), ("rust-toolchain.toml", "[x]\n")):
            with self.subTest(name=name):
                old = (self.top / name).read_text() if (self.top / name).exists() else None
                self.write(name, text)
                key = self.key()
                self.assertNotIn(key, seen)
                seen.add(key)
                if old is None:
                    (self.top / name).unlink()
                else:
                    self.write(name, old)
                self.assertEqual(self.key(), base)

    def test_key_changes_for_cargo_config_outside_the_tree(self):
        base = self.key()
        parent_config = Path(self.temp.name) / ".cargo" / "config.toml"
        parent_config.parent.mkdir()
        parent_config.write_text("[profile.release]\nopt-level = 0\n")
        self.assertNotEqual(self.key(), base, "a parent directory's Cargo config changes the build")
        parent_config.unlink()
        self.assertEqual(self.key(), base)
        home = Path(self.temp.name) / "cargo-home"
        home.mkdir()
        with patch.dict(os.environ, {"CARGO_HOME": str(home)}):
            empty_home = self.key()
            (home / "config.toml").write_text("[profile.release]\nopt-level = 0\n")
            self.assertNotEqual(self.key(), empty_home, "CARGO_HOME config changes the build")

    def test_identical_trees_at_different_depths_share_a_key(self):
        nested = Path(self.temp.name) / "wt" / "tree2"
        shutil.copytree(self.top, nested)
        tree, _ = pin.w.working_state(nested)
        with patch.object(pin.subprocess, "check_output", return_value=self.rustc):
            nested_key = pin.inputs_sha256(nested, tree, "server", LINUX, True)
        self.assertEqual(nested_key, self.key(), "main and wt/<name> with the same inputs reuse each other")
        (nested.parent / ".cargo").mkdir()
        (nested.parent / ".cargo" / "config.toml").write_text("[profile.release]\nopt-level = 0\n")
        with patch.object(pin.subprocess, "check_output", return_value=self.rustc):
            self.assertNotEqual(pin.inputs_sha256(nested, tree, "server", LINUX, True), self.key())

    def test_same_config_content_at_different_paths_is_not_one_key(self):
        keys = []
        for side in ("x", "y"):
            root = Path(self.temp.name) / side
            shutil.copytree(self.top, root / "tree")
            (root / ".cargo").mkdir()
            (root / ".cargo" / "config.toml").write_text('[env]\nCC = { value = "bin/cc", relative = true }\n')
            tree, _ = pin.w.working_state(root / "tree")
            with patch.object(pin.subprocess, "check_output", return_value=self.rustc):
                keys.append(pin.inputs_sha256(root / "tree", tree, "server", LINUX, True))
        self.assertNotEqual(keys[0], keys[1], "relative config values resolve from the config's own directory")


class ReuseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.pins = self.root / "pins"
        self.pins.mkdir()
        self.top = self.root / "NewTree"
        self.top.mkdir()
        patcher = patch.object(pin.w, "PINS", self.pins)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.before = {"head": "9" * 40, "diff_sha256": "8" * 64, "dirty": False, "untracked_files": 0}

    def source_pin(self, name="old-server-x86_64-unknown-linux-gnu-incr", kind="server", **overrides):
        directory = self.pins / name
        directory.mkdir()
        binary = directory / (pin.KINDS[kind]["bin"] or "mc_core-0123456789abcdef")
        binary.write_bytes(b"binary " + name.encode())
        binary.chmod(0o555)
        manifest = {"repo": "core", "kind": kind, "target": LINUX, "incremental": True,
                    "head": self.before["head"], "diff_sha256": self.before["diff_sha256"],
                    "binary": str(binary), "binary_sha256": pin.w.sha256_file(binary),
                    "command": "mem-run cargo build", "inputs_sha256": "K", "source": "/old", "build_tree": "/old",
                    "env_deps": {}}
        manifest.update(overrides)
        (directory / "manifest.json").write_text(json.dumps(manifest))
        return directory, binary

    def reuse(self, kind="server", incremental=True, inputs="K"):
        return pin.reuse_pin(self.pins / f"new-{kind}", self.top, kind, LINUX, incremental, inputs, self.before)

    def test_reuse_links_the_binary_and_writes_this_trees_manifest(self):
        directory, binary = self.source_pin()
        reused = self.reuse()
        self.assertEqual(reused, self.pins / "new-server" / "krasnokamen")
        self.assertEqual(reused.stat().st_ino, binary.stat().st_ino, "hard link, not a copy")
        manifest = json.loads((self.pins / "new-server" / "manifest.json").read_text())
        self.assertEqual((manifest["head"], manifest["diff_sha256"], manifest["source"]),
                         (self.before["head"], self.before["diff_sha256"], str(self.top)))
        self.assertEqual((manifest["inputs_sha256"], manifest["reused_from"], manifest["incremental"]),
                         ("K", directory.name, True))
        self.assertEqual(manifest["binary"], str(reused))
        self.assertEqual(manifest["binary_sha256"], pin.w.sha256_file(binary))
        self.assertEqual(manifest["build_tree"], "/old")
        self.assertEqual(pin.existing_pin(self.pins / "new-server", "server", LINUX, True), reused)
        self.assertEqual([p.name for p in self.pins.iterdir() if p.name.startswith(".tmp")], [])

    def test_damaged_or_mismatched_source_pins_are_not_reused(self):
        directory, binary = self.source_pin()
        binary.chmod(0o755)
        binary.write_bytes(b"tampered")
        self.assertIsNone(self.reuse())
        self.assertFalse((self.pins / "new-server").exists())
        binary.unlink()
        self.assertIsNone(self.reuse(), "missing binary")
        shutil.rmtree(directory)
        self.source_pin(inputs_sha256="other")
        self.assertIsNone(self.reuse())
        self.assertIsNone(self.reuse(incremental=False))
        self.assertIsNone(self.reuse(inputs="K2"))

    def test_pins_without_the_inputs_field_are_never_reused(self):
        self.source_pin(inputs_sha256=None)
        self.assertIsNone(self.reuse())

    def test_bots_are_reused_only_by_the_tree_that_built_them(self):
        # the binary reads <build tree>/data/vanilla/reports at run time, so another tree's copy would
        # break when that tree is removed
        self.source_pin("old-bots", kind="bots", build_tree=str(self.root / "OldTree"))
        self.assertIsNone(self.reuse("bots"))
        self.assertFalse((self.pins / "new-bots").exists())
        self.source_pin("own-bots", kind="bots", build_tree=str(self.top))
        reused = self.reuse("bots")
        self.assertEqual(json.loads((reused.parent / "manifest.json").read_text())["reused_from"], "own-bots")
        self.assertEqual(json.loads((reused.parent / "manifest.json").read_text())["build_tree"], str(self.top))

    def test_another_worktree_of_the_same_tree_reuses_the_pin(self):
        # the same HEAD plus uncommitted state is the same tree, wherever the worktree lives
        _, binary = self.source_pin()
        other = self.root / "OtherWorktree"
        other.mkdir()
        reused = pin.reuse_pin(self.pins / "new-other", other, "server", LINUX, True, "K", self.before)
        self.assertEqual(reused.stat().st_ino, binary.stat().st_ino)
        manifest = json.loads((self.pins / "new-other" / "manifest.json").read_text())
        self.assertEqual((manifest["source"], manifest["head"], manifest["reused_from"]),
                         (str(other), self.before["head"], "old-server-x86_64-unknown-linux-gnu-incr"))

    def test_a_shared_pin_is_never_relabelled_as_another_tree(self):
        self.source_pin()
        for before in ({**self.before, "head": "a" * 40}, {**self.before, "diff_sha256": "7" * 64}):
            with self.subTest(before=before):
                self.assertIsNone(pin.reuse_pin(self.pins / "new-other", self.top, "server", LINUX, True, "K", before))
                self.assertFalse((self.pins / "new-other").exists())

    def test_core_tests_are_reused_only_by_the_tree_that_built_them(self):
        # the release mc-core test executable bakes its CARGO_MANIFEST_DIR (world and data) paths
        self.source_pin("old-tests", kind="core-tests", build_tree=str(self.root / "OldTree"))
        self.assertIsNone(self.reuse("core-tests"))
        self.assertFalse((self.pins / "new-core-tests").exists(), "another tree gets its own pin, never this one")
        _, binary = self.source_pin("own-tests", kind="core-tests", build_tree=str(self.top))
        reused = self.reuse("core-tests")
        self.assertEqual(reused, self.pins / "new-core-tests" / binary.name)
        manifest = json.loads((self.pins / "new-core-tests" / "manifest.json").read_text())
        self.assertEqual((manifest["kind"], manifest["reused_from"], manifest["build_tree"]),
                         ("core-tests", "own-tests", str(self.top)))
        self.assertEqual(reused.stat().st_ino, binary.stat().st_ino, "hard link, not a copy")

    def test_core_tests_pin_names_its_env_and_refuses_a_damaged_binary(self):
        directory, binary = self.source_pin("tests", kind="core-tests", build_tree=str(self.top))
        self.assertEqual(pin.existing_pin(directory, "core-tests", LINUX, True), binary)
        self.assertEqual(pin.KINDS["core-tests"]["env"], "KRASNOKAMEN_CORE_TESTS_BINARY")
        binary.chmod(0o755)
        binary.write_bytes(b"tampered")
        with self.assertRaisesRegex(pin.w.ToolError, "damaged"):
            pin.existing_pin(directory, "core-tests", LINUX, True)

    def test_core_tests_build_runs_no_tests_and_takes_the_test_executable(self):
        slot = self.root / "targets" / "slot-a" / "release"
        artifact = slot / "mc_core-0123456789abcdef"
        artifact.parent.mkdir(parents=True)
        artifact.write_bytes(b"test executable")
        messages = [
            {"reason": "compiler-artifact", "target": {"name": "mc_core", "kind": ["lib"]},
             "profile": {"test": False}, "executable": str(slot / "libmc_core.rlib")},
            {"reason": "compiler-artifact", "target": {"name": "mc_core", "kind": ["lib"]},
             "profile": {"test": True}, "executable": str(artifact)},
        ]
        commands = []

        class Fake:
            returncode = 0
            def __init__(self, cmd, **_):
                commands.append(cmd)
                self.stdout = iter(json.dumps(message) + "\n" for message in messages)
            def __enter__(self):
                return self
            def __exit__(self, *_):
                return False

        with patch.object(pin.subprocess, "Popen", Fake), \
                patch.object(pin, "artifact_slot", return_value=self.root / "targets" / "slot-a"):
            cmd, built = pin.build(self.top, "core-tests", None, False)
        self.assertEqual(built, artifact)
        self.assertEqual(cmd[cmd.index("test"):cmd.index("test") + 6],
                         ["test", "--release", "--locked", "--no-run", "-p", "mc-core"])
        self.assertIn("--lib", cmd)
        self.assertNotIn("--bin", cmd)
        # the mc-core lib test build OOM-killed a 6G pin (exit 143): core-tests gets 12G
        self.assertEqual(cmd[cmd.index("-m") + 1], "12G")
        self.assertIn("--message-format=json-render-diagnostics", cmd)

    def test_existing_pin_with_other_or_missing_build_inputs_is_refused_not_returned(self):
        directory, binary = self.source_pin(inputs_sha256="K")
        self.assertEqual(pin.existing_pin(directory, "server", LINUX, True, "K"), binary)
        self.assertEqual(pin.existing_pin(directory, "server", LINUX, True, None), binary)
        with self.assertRaisesRegex(pin.w.ToolError, "other build inputs"):
            pin.existing_pin(directory, "server", LINUX, True, "changed")
        recorded = json.loads((directory / "manifest.json").read_text())
        (directory / "manifest.json").write_text(json.dumps({**recorded, "inputs_sha256": None}))
        with self.assertRaisesRegex(pin.w.ToolError, "records no build inputs"):
            pin.existing_pin(directory, "server", LINUX, True, "K")

    def test_cli_refuses_a_same_name_pin_after_rustflags_change_and_builds_nothing(self):
        top = self.root / "Tree"
        top.mkdir()
        provenance = {"id": "999999999999", "head": "9" * 40, "tree": "2" * 40, "diff_sha256": "8" * 64,
                      "dirty": False, "untracked_files": 0}
        name = f"{provenance['id']}-server-{LINUX}-incr"
        directory, _ = self.source_pin(name, inputs_sha256="from-old-flags")
        with ExitStack() as stack:
            stack.enter_context(patch.object(pin.w, "check_host"))
            stack.enter_context(patch.object(pin.w, "git", return_value=str(top)))
            stack.enter_context(patch.object(pin, "check_tree"))
            stack.enter_context(patch.object(pin, "host_target", return_value=LINUX))
            stack.enter_context(patch.object(pin, "provenance", return_value=provenance))
            stack.enter_context(patch.object(pin.subprocess, "check_output", return_value="rustc 1.99.0\n"))
            stack.enter_context(patch.object(pin.w, "in_use_add"))
            stack.enter_context(patch.object(sys, "argv", ["pin-release", str(top)]))
            build = stack.enter_context(patch.object(pin, "build"))
            with patch.dict(os.environ, {"RUSTFLAGS": "-C target-cpu=native"}):
                with self.assertRaisesRegex(pin.w.ToolError, "other build inputs"):
                    pin.main()
            build.assert_not_called()

    def test_cli_reuses_instead_of_building_and_new_builds_record_the_inputs(self):
        directory, binary = self.source_pin()
        provenance = {"id": "999999999999", "head": "9" * 40, "tree": "2" * 40, "diff_sha256": "8" * 64,
                      "dirty": False, "untracked_files": 0}
        output = io.StringIO()
        with ExitStack() as stack:
            stack.enter_context(patch.object(pin.w, "check_host"))
            stack.enter_context(patch.object(pin.w, "git", return_value=str(self.top)))
            stack.enter_context(patch.object(pin, "check_tree"))
            stack.enter_context(patch.object(pin, "host_target", return_value=LINUX))
            stack.enter_context(patch.object(pin, "provenance", return_value=provenance))
            stack.enter_context(patch.object(pin, "inputs_sha256", return_value="K"))
            build = stack.enter_context(patch.object(pin, "build"))
            mark = stack.enter_context(patch.object(pin.w, "in_use_add"))
            stack.enter_context(patch.object(sys, "argv", ["pin-release", str(self.top)]))
            with redirect_stdout(output):
                pin.main()
        build.assert_not_called()
        new = self.pins / f"{provenance['id']}-server-{LINUX}-incr"
        mark.assert_called_with(new, self.top.name, "pin")
        self.assertEqual(output.getvalue().strip(), f"KRASNOKAMEN_SERVER_BINARY={new / 'krasnokamen'}")
        self.assertEqual(json.loads((new / "manifest.json").read_text())["reused_from"], directory.name)


class EnvDepTests(unittest.TestCase):
    """Variables cargo tracks for a build (build scripts' rerun-if-env-changed, dep-info env-dep) gate reuse."""
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.slot = self.root / "targets" / "slot-a"
        self.release = self.slot / "release"
        (self.release / "build/zstd-sys-1/output").parent.mkdir(parents=True)
        (self.release / "build/zstd-sys-1/output").write_text(
            "cargo:rerun-if-changed=zstd\ncargo:rerun-if-env-changed=ZSTD_SYS_USE_PKG_CONFIG\n"
            "cargo::rerun-if-env-changed=PKG_CONFIG_PATH\ncargo:rustc-link-lib=static=zstd\n")
        (self.release / "deps").mkdir()
        (self.release / "deps/mc_edge-1.d").write_text(
            "/x/lib.rs: /x/lib.rs\n\n# env-dep:GIT_REV=abc\n# env-dep:CARGO_PKG_VERSION\n")
        self.artifact = self.release / "krasnokamen"
        self.artifact.write_bytes(b"bin")
        (self.release / "krasnokamen.d").write_text("krasnokamen: /x/main.rs\n# env-dep:BINARY_ONLY=1\n")
        self.pins = self.root / "pins"
        self.pins.mkdir()
        self.top = self.root / "Tree"
        self.top.mkdir()
        patcher = patch.object(pin.w, "PINS", self.pins)
        patcher.start()
        self.addCleanup(patcher.stop)
        env = patch.dict(os.environ)
        env.start()
        self.addCleanup(env.stop)
        for name in ("ZSTD_SYS_USE_PKG_CONFIG", "PKG_CONFIG_PATH", "GIT_REV", "BINARY_ONLY"):
            os.environ.pop(name, None)
        self.before = {"head": "9" * 40, "diff_sha256": "8" * 64, "dirty": False, "untracked_files": 0}

    def recorded(self):
        return pin.build_env_deps(pin.build_env_names(self.artifact, self.slot), pin.build_env(True))

    def source_pin(self, **overrides):
        directory = self.pins / "old-server"
        directory.mkdir(exist_ok=True)
        binary = directory / "krasnokamen"
        binary.write_bytes(b"old binary")
        manifest = {"repo": "core", "kind": "server", "target": LINUX, "incremental": True,
                    "head": self.before["head"], "diff_sha256": self.before["diff_sha256"],
                    "binary": str(binary), "binary_sha256": pin.w.sha256_file(binary), "command": "c",
                    "inputs_sha256": "K", "build_tree": "/old", "env_deps": self.recorded()}
        manifest.update(overrides)
        (directory / "manifest.json").write_text(json.dumps(manifest))
        return directory

    def reuse(self):
        new = self.pins / "new-server"
        shutil.rmtree(new, ignore_errors=True)
        return pin.reuse_pin(new, self.top, "server", LINUX, True, "K", self.before)

    def test_names_come_from_build_script_output_and_dep_info(self):
        self.assertEqual(
            self.recorded(),
            {"BINARY_ONLY": None, "CARGO_PKG_VERSION": None, "GIT_REV": None, "PKG_CONFIG_PATH": None,
             "ZSTD_SYS_USE_PKG_CONFIG": None})
        os.environ["ZSTD_SYS_USE_PKG_CONFIG"] = "1"
        os.environ["GIT_REV"] = ""
        self.assertEqual((self.recorded()["ZSTD_SYS_USE_PKG_CONFIG"], self.recorded()["GIT_REV"]), ("1", ""))
        self.assertEqual(list(self.recorded()), sorted(self.recorded()))

    def test_cross_target_build_scripts_of_the_host_profile_are_read(self):
        artifact = self.slot / LINUX / "release" / "krasnokamen"
        artifact.parent.mkdir(parents=True)
        artifact.touch()
        self.assertIn("ZSTD_SYS_USE_PKG_CONFIG", pin.build_env_names(artifact, self.slot))

    def test_reuse_flips_with_a_build_script_variable(self):
        self.source_pin()
        self.assertIsNotNone(self.reuse())
        os.environ["ZSTD_SYS_USE_PKG_CONFIG"] = "1"
        self.assertIsNone(self.reuse(), "set now, unset when built")
        self.assertFalse((self.pins / "new-server").exists())
        self.source_pin(env_deps=self.recorded())  # a pin built with it set
        self.assertIsNotNone(self.reuse())
        os.environ["ZSTD_SYS_USE_PKG_CONFIG"] = "0"
        self.assertIsNone(self.reuse(), "another value")
        del os.environ["ZSTD_SYS_USE_PKG_CONFIG"]
        self.assertIsNone(self.reuse(), "unset now, set when built")

    def test_reuse_flips_with_a_dep_info_variable(self):
        os.environ["GIT_REV"] = "abc"
        self.source_pin()
        self.assertIsNotNone(self.reuse())
        os.environ["GIT_REV"] = "def"
        self.assertIsNone(self.reuse())

    def test_a_reused_manifest_carries_the_map(self):
        os.environ["GIT_REV"] = "abc"
        self.source_pin()
        manifest = json.loads((self.reuse().parent / "manifest.json").read_text())
        self.assertEqual(manifest["env_deps"]["GIT_REV"], "abc")

    def test_a_manifest_without_the_map_is_not_reused(self):
        directory = self.source_pin()
        for broken in (None, [], "ZSTD_SYS_USE_PKG_CONFIG"):
            with self.subTest(env_deps=broken):
                manifest = json.loads((directory / "manifest.json").read_text())
                manifest.pop("env_deps", None)
                if broken is not None:
                    manifest["env_deps"] = broken
                (directory / "manifest.json").write_text(json.dumps(manifest))
                self.assertIsNone(self.reuse())
        self.source_pin(env_deps={})
        self.assertIsNotNone(self.reuse(), "an explicit empty map is a map")

    def test_the_same_named_pin_is_refused_after_a_tracked_variable_changes(self):
        directory = self.source_pin()
        binary = directory / "krasnokamen"
        self.assertEqual(pin.existing_pin(directory, "server", LINUX, True, "K"), binary)
        os.environ["ZSTD_SYS_USE_PKG_CONFIG"] = "1"
        with self.assertRaisesRegex(pin.w.ToolError, "tracks"):
            pin.existing_pin(directory, "server", LINUX, True, "K")
        del os.environ["ZSTD_SYS_USE_PKG_CONFIG"]
        manifest = json.loads((directory / "manifest.json").read_text())
        manifest.pop("env_deps")
        (directory / "manifest.json").write_text(json.dumps(manifest))
        os.environ["ZSTD_SYS_USE_PKG_CONFIG"] = "1"
        with self.assertRaisesRegex(pin.w.ToolError, "records no build-script environment"):
            pin.existing_pin(directory, "server", LINUX, True, "K")


LEDGER_PROBE = (
    "import json, sys\n"
    "from pathlib import Path\n"
    "from tools.harness.ledger import _server_built_from, tree_fingerprint\n"
    "root = Path(sys.argv[1])\n"
    "print(_server_built_from(root, json.dumps({'sha256': sys.argv[2]}), tree_fingerprint(root)))\n"
)


class LedgerRegistrationTests(unittest.TestCase):
    """A pin is registered where tools/harness/ledger.py looks, and only for its own tree."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = {**os.environ, "GIT_AUTHOR_NAME": "pin-test", "GIT_AUTHOR_EMAIL": "pin-test@example.invalid",
                    "GIT_COMMITTER_NAME": "pin-test", "GIT_COMMITTER_EMAIL": "pin-test@example.invalid"}

    def tree(self, name, content):
        tree = self.root / name
        (tree / "tools").mkdir(parents=True)
        shutil.copytree(HERE.parent / "harness", tree / "tools/harness")
        # ledger.py imports tools.dev.wtlib (via regressions) for the pins root.
        (tree / "tools/dev").mkdir()
        shutil.copy2(HERE / "wtlib.py", tree / "tools/dev/wtlib.py")
        (tree / "README").write_text(content)
        # Importing ledger.py writes __pycache__; untracked, it would make every fixture tree dirty.
        (tree / ".gitignore").write_text("__pycache__/\n")
        subprocess.run(["git", "init", "-q", str(tree)], check=True, env=self.env)
        subprocess.run(["git", "-C", str(tree), "add", "."], check=True, env=self.env)
        subprocess.run(["git", "-C", str(tree), "commit", "-qm", content], check=True, env=self.env)
        return tree

    def binary(self, name="krasnokamen"):
        binary = self.root / name
        binary.write_bytes(b"release server " + name.encode())
        return binary

    def probe(self, tree, sha):
        return subprocess.run([sys.executable, "-c", LEDGER_PROBE, str(tree), sha],
                              cwd=tree, capture_output=True, text=True, env=self.env)

    def test_a_server_pin_is_registered_for_its_own_tree_only(self):
        candidate = self.tree("candidate", "candidate")
        binary = self.binary()
        pin.ledger_register(candidate, binary)
        sha = pin.w.sha256_file(binary)
        registry = candidate / ".analysis/ledger-builds"
        fingerprint = pin.ledger_fingerprint(candidate)
        # One file per (binary, tree) pair: a python-only fix cites the same server binary for the
        # candidate and its reverted control tree, so a per-binary name would drop one of them.
        recorded = json.loads((registry / f"{sha}-{fingerprint['sha256']}.json").read_text())
        self.assertEqual(recorded, {"sha256": sha, "tree": {"head": fingerprint["head"], "sha256": fingerprint["sha256"]}})
        self.assertEqual(sorted(path.name for path in registry.glob("*.json")), [f"{sha}-{fingerprint['sha256']}.json"])
        accepted = self.probe(candidate, sha)
        self.assertEqual(accepted.returncode, 0, accepted.stderr)
        self.assertEqual(accepted.stdout.strip(), sha)

        control = self.tree("control", "a different tree")
        shutil.copytree(registry, control / ".analysis/ledger-builds")
        refused = self.probe(control, sha)
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("different tree", refused.stderr)

    def test_an_untracked_file_marks_the_tree_dirty_and_changes_its_fingerprint(self):
        candidate = self.tree("candidate", "candidate")
        clean = pin.ledger_fingerprint(candidate)
        self.assertFalse(clean["dirty"])
        (candidate / "scratch.txt").write_text("one")
        first = pin.ledger_fingerprint(candidate)
        self.assertTrue(first["dirty"])
        self.assertEqual(first["head"], clean["head"])
        self.assertNotEqual(first["sha256"], clean["sha256"])
        (candidate / "scratch.txt").write_text("two")
        self.assertNotEqual(pin.ledger_fingerprint(candidate)["sha256"], first["sha256"])

    def test_a_test_executable_is_never_registered_as_the_server_evidence(self):
        # ledger-builds is the server binary registry: core-tests must never be certified there
        self.assertTrue(pin.registers_for_ledger("server"))
        for kind in ("core-tests", "bots", "client"):
            self.assertFalse(pin.registers_for_ledger(kind), kind)
        candidate = self.tree("candidate", "candidate")
        binary = self.binary("mc_core-0123456789abcdef")
        # ledger_register itself is kind-agnostic; main() calls it for server pins only, and the
        # registry has no entry for a test executable, so a row cannot cite one as its server.
        registry = candidate / ".analysis" / "ledger-builds"
        self.assertFalse(registry.exists())
        refused = self.probe(candidate, pin.w.sha256_file(binary))
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("no server binary built from the candidate", refused.stderr)


if __name__ == "__main__":
    unittest.main()
