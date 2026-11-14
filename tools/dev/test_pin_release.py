"""Release-pin target/kind and pooled-copy regressions; no Cargo builds."""
from __future__ import annotations

import argparse
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

    def test_slot_is_the_pool_child_for_host_and_cross_artifacts(self):
        with patch.object(pin.w, "TARGETS", self.targets):
            for artifact in (self.slot / "release/solaris-v2",
                             self.slot / WINDOWS / "release/solaris-client.exe"):
                with self.subTest(artifact=artifact):
                    self.assertEqual(pin.artifact_slot(artifact), self.slot)
            for artifact in (self.root / "elsewhere/release/client", self.targets / "not-a-slot/release/client"):
                with self.assertRaises(pin.w.ToolError):
                    pin.artifact_slot(artifact)

    def test_kind_and_target_are_required_for_reuse(self):
        directory = self.root / "pin"
        directory.mkdir()
        binary = directory / "solaris-client.exe"
        binary.write_bytes(b"test pin")
        manifest = {"repo": "client", "kind": "client", "target": WINDOWS,
                    "binary": str(binary), "binary_sha256": pin.w.sha256_file(binary)}
        (directory / "manifest.json").write_text(json.dumps(manifest))
        self.assertEqual(pin.existing_pin(directory, "client", WINDOWS, False), binary)
        with self.assertRaises(pin.w.ToolError):
            pin.existing_pin(directory, "client", LINUX, False)
        with self.assertRaises(pin.w.ToolError):  # a pin without the field predates incremental builds
            pin.existing_pin(directory, "client", WINDOWS, True)
        manifest.update(repo="core", kind="server")
        (directory / "manifest.json").write_text(json.dumps(manifest))
        with self.assertRaises(pin.w.ToolError):
            pin.existing_pin(directory, "bots", WINDOWS, False)
        manifest.update(repo="client", kind="client", incremental=True)
        (directory / "manifest.json").write_text(json.dumps(manifest))
        self.assertEqual(pin.existing_pin(directory, "client", WINDOWS, True), binary)
        with self.assertRaises(pin.w.ToolError):
            pin.existing_pin(directory, "client", WINDOWS, False)

    def test_target_is_explicit_and_inherited_target_is_removed(self):
        artifact = self.slot / WINDOWS / "release/solaris-client.exe"
        artifact.parent.mkdir(parents=True)
        artifact.touch()
        message = {"reason": "compiler-artifact", "target": {"name": "solaris-client", "kind": ["bin"]},
                   "executable": str(artifact)}

        class Process:
            stdout = [json.dumps(message) + "\n"]
            returncode = 0
            def __enter__(self): return self
            def __exit__(self, *args): return False

        with patch.dict(os.environ, {"CARGO_BUILD_TARGET": "wrong-target", "CARGO_TARGET_DIR": "/wrong"}), \
             patch.object(pin.subprocess, "Popen", return_value=Process()) as launched:
            command, found = pin.build(self.root, "client", WINDOWS, True, target=self.slot)
        self.assertEqual(found, artifact)
        self.assertEqual(command[command.index("--target") + 1], WINDOWS)
        self.assertIn(f"CARGO_TARGET_DIR={self.slot}", command)
        self.assertNotIn("CARGO_BUILD_TARGET", launched.call_args.kwargs["env"])
        self.assertNotIn("CARGO_TARGET_DIR", launched.call_args.kwargs["env"])
        self.assertIn("--locked", command)
        self.assertIn("--release", command)
        self.assertIn("CARGO_INCREMENTAL=1", command)
        self.assertNotIn("RUSTC_WRAPPER=", command)  # the runner selects the wrapper
        with patch.object(pin.subprocess, "Popen", return_value=Process()):
            command, _ = pin.build(self.root, "client", WINDOWS, False, target=self.slot)
        self.assertIn("CARGO_INCREMENTAL=0", command)

    def test_host_selection_uses_rustc_not_cargo_build_target(self):
        with patch.dict(os.environ, {"CARGO_BUILD_TARGET": WINDOWS}), \
             patch.object(pin.subprocess, "check_output", return_value=f"rustc test\nhost: {LINUX}\n"):
            self.assertEqual(pin.host_target(self.root), LINUX)

    def test_target_paths_are_not_pin_directory_names(self):
        self.assertEqual(pin.target_triple(WINDOWS), WINDOWS)
        for value in ("../target.json", "/tmp/target.json", "a/b-c", "a\\b-c", "bad target"):
            with self.subTest(value=value), self.assertRaises(argparse.ArgumentTypeError):
                pin.target_triple(value)

    def test_cli_namespaces_targets_and_kinds_and_keeps_in_use_mark(self):
        pins = self.root / "pins"
        locks = self.root / "locks"
        locks.mkdir()
        tree = self.root / "Tree"
        tree.mkdir()
        (self.slot / ".solaris-tree").write_text(str(tree))
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
            self.assertEqual("inputs_sha256" in manifest, kind != "client")  # the client's pin is unchanged
            self.assertEqual("build_tree" in manifest, kind != "client")
            self.assertEqual(final.read_bytes(), artifact.read_bytes())
        self.assertEqual(len(set(paths)), 5)

    def test_foreign_config_target_is_not_labeled_native(self):
        artifact = self.slot / WINDOWS / "release/solaris-client.exe"
        artifact.parent.mkdir(parents=True)
        artifact.touch()
        message = {"reason": "compiler-artifact", "target": {"name": "solaris-client", "kind": ["bin"]},
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

    def test_key_changes_for_toolchain_target_kind_incremental_and_build_env(self):
        base = self.key()
        self.assertNotEqual(self.key(target="x86_64-pc-windows-gnu"), base)
        self.assertNotEqual(self.key(kind="bots"), base)
        self.assertNotEqual(self.key(incremental=False), base)
        with patch.dict(os.environ, {"RUSTFLAGS": "-C target-cpu=native"}):
            self.assertNotEqual(self.key(), base)
        with patch.dict(os.environ, {"CARGO_BUILD_JOBS": "3", "CARGO_TARGET_DIR": "/x", "HOME_X": "1"}):
            self.assertEqual(self.key(), base)
        self.rustc = "rustc 1.99.1\nhost: x86_64-unknown-linux-gnu\n"
        self.assertNotEqual(self.key(), base)


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
        binary = directory / pin.KINDS[kind]["bin"]
        binary.write_bytes(b"binary " + name.encode())
        binary.chmod(0o555)
        manifest = {"repo": "core", "kind": kind, "target": LINUX, "incremental": True, "head": "1" * 40,
                    "diff_sha256": "2" * 64, "binary": str(binary), "binary_sha256": pin.w.sha256_file(binary),
                    "command": "mem-run cargo build", "inputs_sha256": "K", "source": "/old", "build_tree": "/old"}
        manifest.update(overrides)
        (directory / "manifest.json").write_text(json.dumps(manifest))
        return directory, binary

    def reuse(self, kind="server", incremental=True, inputs="K"):
        return pin.reuse_pin(self.pins / f"new-{kind}", self.top, kind, LINUX, incremental, inputs, self.before)

    def test_reuse_links_the_binary_and_writes_this_trees_manifest(self):
        directory, binary = self.source_pin()
        reused = self.reuse()
        self.assertEqual(reused, self.pins / "new-server" / "solaris-v2")
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

    def test_reuse_falls_back_to_a_copy_when_linking_fails(self):
        _, binary = self.source_pin()
        with patch.object(pin.os, "link", side_effect=OSError("cross-device")):
            reused = self.reuse()
        self.assertNotEqual(reused.stat().st_ino, binary.stat().st_ino)
        self.assertEqual(reused.read_bytes(), binary.read_bytes())

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

    def test_a_damaged_candidate_does_not_hide_a_valid_one(self):
        _, bad = self.source_pin("a-damaged")
        bad.chmod(0o755)
        bad.write_bytes(b"tampered")
        self.source_pin("b-good")
        self.assertEqual(json.loads((self.reuse().parent / "manifest.json").read_text())["reused_from"], "b-good")

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

    def test_existing_pin_with_other_build_inputs_is_refused_not_returned(self):
        directory, binary = self.source_pin(inputs_sha256="K")
        self.assertEqual(pin.existing_pin(directory, "server", LINUX, True, "K"), binary)
        self.assertEqual(pin.existing_pin(directory, "server", LINUX, True, None), binary)
        with self.assertRaisesRegex(pin.w.ToolError, "other build inputs"):
            pin.existing_pin(directory, "server", LINUX, True, "changed")
        (directory / "manifest.json").write_text(json.dumps(
            {**json.loads((directory / "manifest.json").read_text()), "inputs_sha256": None}))
        self.assertEqual(pin.existing_pin(directory, "server", LINUX, True, "changed"), binary, "legacy pin")

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
        self.assertEqual(output.getvalue().strip(), f"SOLARIS_SERVER_BINARY={new / 'solaris-v2'}")
        self.assertEqual(json.loads((new / "manifest.json").read_text())["reused_from"], directory.name)


if __name__ == "__main__":
    unittest.main()
