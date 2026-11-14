"""Tests for the release-build policy of the owner host.

python3 tools/dev/test_mem_run_release_policy.py

One release binary per integrated commit comes from tools/dev/pin-release; a plain
`cargo build|test|run --release` (or `--profile release*`) is refused unless
pin-release, the harness/ledger tooling or Main's explicit override asks for it.
The decision lives in tools/dev/release-build-policy.sh, which mem-run.sh sources
before it takes a slot or a build token: the cases below call that file directly
(so no cargo runs, no token is taken) plus one mem-run integration refusal.
"""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
POLICY = HERE / "release-build-policy.sh"
SCRIPT = HERE / "mem-run.sh"
PARENT = """#!/usr/bin/python3
import subprocess
import sys

sys.exit(subprocess.run(sys.argv[1:]).returncode)
"""


class ReleasePolicy(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.env = {key: value for key, value in os.environ.items() if not key.startswith("KRASNOKAMEN_")}
        self.env["TMPDIR"] = str(self.root)

    def guard(self, *argv: str, env: dict[str, str] | None = None, parent: list[str] | None = None):
        """Run release_build_guard(argv) from release-build-policy.sh in a subshell."""
        command = ["bash", "-c", 'source "$1"; shift; release_build_guard "$@"', "_", str(POLICY), *argv]
        if parent is not None:
            script = self.root / "tools" / "harness" / "parent.py"
            script.parent.mkdir(parents=True, exist_ok=True)
            script.write_text(PARENT)
            command = ["python3", str(script), *command]
        return subprocess.run(command, env=env or self.env, capture_output=True, text=True, timeout=120)

    def assert_refused(self, result) -> None:
        self.assertEqual(result.returncode, 75, result.stderr)
        self.assertIn("refusing a release build", result.stderr)
        self.assertIn("pin-release", result.stderr)

    def test_allows_a_debug_build(self) -> None:
        result = self.guard("cargo", "build", "-p", "mc-core")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")

    def test_allows_other_commands(self) -> None:
        self.assertEqual(self.guard("python3", "-c", "print('ok')").returncode, 0)
        self.assertEqual(self.guard("cargo", "clippy", "--release").returncode, 0)
        self.assertEqual(self.guard("cargo", "fmt").returncode, 0)
        self.assertEqual(self.guard("cargo", "nextest", "run").returncode, 0)

    def test_refuses_a_release_build(self) -> None:
        self.assert_refused(self.guard("cargo", "build", "--release"))

    def test_refuses_a_release_profile(self) -> None:
        self.assert_refused(self.guard("cargo", "build", "--profile", "release-lto"))
        self.assert_refused(self.guard("cargo", "build", "--profile=release-fast"))

    def test_refuses_release_test_and_run(self) -> None:
        self.assert_refused(self.guard("cargo", "test", "--release", "-p", "mc-core", "--lib"))
        self.assert_refused(self.guard("cargo", "run", "--release", "--bin", "krasnokamen"))

    def test_refuses_after_env_assignments(self) -> None:
        self.assert_refused(self.guard("env", "RUSTFLAGS=-Dwarnings", "cargo", "build", "--release"))
        self.assert_refused(self.guard("env", "-u", "RUSTC_WRAPPER", "cargo", "test", "--release"))

    def test_allows_the_pin_release_marker(self) -> None:
        env = {**self.env, "KRASNOKAMEN_PIN_RELEASE": "1"}
        self.assertEqual(self.guard("cargo", "build", "--release", env=env).returncode, 0)

    def test_allows_main_override(self) -> None:
        env = {**self.env, "KRASNOKAMEN_RELEASE_BUILD_ALLOW": "1"}
        self.assertEqual(self.guard("cargo", "build", "--release", env=env).returncode, 0)

    def test_allows_a_harness_tooling_caller(self) -> None:
        result = self.guard("cargo", "build", "--release", parent=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_mem_run_refuses_before_taking_a_slot(self) -> None:
        result = subprocess.run([str(SCRIPT), "-m", "1G", "cargo", "build", "--release"],
                                env=self.env, capture_output=True, text=True, timeout=120)
        self.assertEqual(result.returncode, 75, result.stderr)
        self.assertIn("refusing a release build", result.stderr)
        self.assertNotIn("mem-run: scope", result.stderr, "a refused release build took a slot or a token")


if __name__ == "__main__":
    unittest.main()
