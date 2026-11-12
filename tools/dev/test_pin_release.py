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
            for artifact in (self.slot / "release/krasnokamen",
                             self.slot / WINDOWS / "release/krasnokamen-client.exe"):
                with self.subTest(artifact=artifact):
                    self.assertEqual(pin.artifact_slot(artifact), self.slot)
            for artifact in (self.root / "elsewhere/release/client", self.targets / "not-a-slot/release/client"):
                with self.assertRaises(pin.w.ToolError):
                    pin.artifact_slot(artifact)

    def test_kind_and_target_are_required_for_reuse(self):
        directory = self.root / "pin"
        directory.mkdir()
        binary = directory / "krasnokamen-client.exe"
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
            self.assertEqual("inputs_sha256" in manifest, kind != "client")  # the client's pin is unchanged
            self.assertEqual("build_tree" in manifest, kind != "client")
            self.assertEqual(manifest.get("env_deps"), None if kind == "client" else {"ZSTD_SYS_USE_PKG_CONFIG": None})
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
        binary = directory / pin.KINDS[kind]["bin"]
        binary.write_bytes(b"binary " + name.encode())
        binary.chmod(0o555)
        manifest = {"repo": "core", "kind": kind, "target": LINUX, "incremental": True, "head": "1" * 40,
                    "diff_sha256": "2" * 64, "binary": str(binary), "binary_sha256": pin.w.sha256_file(binary),
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
        self.assertEqual(pin.existing_pin(directory, "server", LINUX, True, "K"), binary, "pin from before the map")


if __name__ == "__main__":
    unittest.main()
