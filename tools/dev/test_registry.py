"""Tests for the in-use registry, wt-done's refusal and main-guard: python3 tools/dev/test_registry.py"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import wtlib as w  # noqa: E402
from test_slot_gc import load_tool  # noqa: E402

wt_done = load_tool("wt-done")
main_guard = load_tool("main-guard")


class RegistryCase(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.base = Path(self._tmp.name).resolve()
        self.registry = self.base / "receipts" / ".in-use.jsonl"
        self.wt = self.base / "wt" / "Alpha"
        (self.wt / "target" / "x").mkdir(parents=True)


class WtDoneGuardTests(RegistryCase):
    def test_refuses_when_another_owner_registered_the_worktree(self):
        w.in_use_add(self.wt, "Alpha", "worktree", self.registry)  # wt-new's own mark never blocks
        self.assertEqual(wt_done.check_in_use("Alpha", self.wt, None, self.registry), [])
        w.in_use_add(self.wt / "target" / "x", "QABot", "receipt_binary", self.registry)
        with self.assertRaises(w.ToolError) as ctx:
            wt_done.check_in_use("Alpha", self.wt, None, self.registry)
        self.assertIn("QABot", str(ctx.exception))
        self.assertIn("receipt_binary", str(ctx.exception))
        self.assertNotIn("force\"", self.registry.read_text())  # a refusal logs nothing

    def test_force_needs_the_reason_and_logs_it(self):
        w.in_use_add(self.wt, "QABot", "client_root", self.registry)
        err = io.StringIO()
        sys.stderr, saved = err, sys.stderr
        try:
            others = wt_done.check_in_use("Alpha", self.wt, "owner confirmed the run is dead", self.registry)
        finally:
            sys.stderr = saved
        self.assertEqual([m["owner"] for m in others], ["QABot"])
        forced = [json.loads(line) for line in self.registry.read_text().splitlines() if '"force"' in line]
        self.assertEqual(len(forced), 1)
        self.assertEqual(forced[0]["reason"], "owner confirmed the run is dead")
        self.assertEqual(forced[0]["owners"], ["QABot"])
        self.assertIn("reason logged", err.getvalue())

    def test_release_drops_every_mark_on_the_removed_worktree(self):
        w.in_use_add(self.wt, "Alpha", "worktree", self.registry)
        w.in_use_add(self.wt / "target" / "x", "QABot", "receipt_binary", self.registry)
        w.in_use_add(self.base / "wt" / "Elsewhere", "Other", "worktree", self.registry)
        self.assertEqual(wt_done.release_marks(self.wt, self.registry), 2)
        self.assertEqual([m["owner"] for m in w.in_use_marks(self.registry)], ["Other"])

    def test_blank_force_reason_is_rejected_before_anything_else(self):
        proc = subprocess.run([sys.executable, str(HERE / "wt-done"), "Nope", "--force", " "], capture_output=True, text=True)
        self.assertEqual(proc.returncode, 1)
        self.assertIn("--force needs a reason", proc.stderr)


class MainGuardTests(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.repo = Path(self._tmp.name).resolve() / "main"
        for rel in ("crates/a/src", "sdk/rust", "tools/harness/backends", "docs"):
            (self.repo / rel).mkdir(parents=True)
        (self.repo / "crates/a/src/lib.rs").write_text("one\n")
        (self.repo / "crates/a/src/gone.rs").write_text("bye\n")
        (self.repo / "docs/x.md").write_text("doc\n")
        self.git("init", "-q")
        self.git("add", "-A")
        self.git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "base")

    def git(self, *args):
        subprocess.run(["git", "-C", str(self.repo), *args], check=True, capture_output=True)

    def test_new_changed_and_gone_files_against_the_baseline(self):
        (self.repo / "crates/a/src/lib.rs").write_text("two\n")  # dirty at baseline
        (self.repo / "sdk/rust/pre.rs").write_text("pre\n")  # untracked at baseline
        (self.repo / "docs/x.md").write_text("outside the guarded paths\n")
        baseline = main_guard.dirty_state(self.repo)
        self.assertEqual(set(baseline), {"crates/a/src/lib.rs", "sdk/rust/pre.rs"})
        self.assertEqual(main_guard.compare(baseline, main_guard.dirty_state(self.repo)), ([], [], []))

        (self.repo / "crates/a/src/lib.rs").write_text("three\n")  # changed again after the baseline
        (self.repo / "tools/harness/backends/new.py").write_text("n\n")  # new untracked
        (self.repo / "crates/a/src/gone.rs").unlink()  # new deletion
        (self.repo / "sdk/rust/pre.rs").unlink()  # was dirty, now clean
        (self.repo / "docs/y.md").write_text("y\n")  # outside: ignored
        new, changed, gone = main_guard.compare(baseline, main_guard.dirty_state(self.repo))
        self.assertEqual(new, ["crates/a/src/gone.rs", "tools/harness/backends/new.py"])
        self.assertEqual(changed, ["crates/a/src/lib.rs"])
        self.assertEqual(gone, ["sdk/rust/pre.rs"])

    def test_does_not_rewrite_the_index(self):
        index = self.repo / ".git" / "index"
        (self.repo / "crates/a/src/lib.rs").write_text("touched\n")
        os.utime(index, (1, 1))
        main_guard.dirty_state(self.repo)
        self.assertEqual(index.stat().st_mtime, 1)


if __name__ == "__main__":
    unittest.main()
