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
        self.assertIn("RUSTC_WRAPPER=", command)
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


if __name__ == "__main__":
    unittest.main()
