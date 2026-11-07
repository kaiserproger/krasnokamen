"""Tests for tools/dev/slot-select.sh (mem-run's target-dir ownership): python3 tools/dev/test_slot_select.py

Each command is a real bash process that sources slot-select.sh, runs select_slot and,
on success, keeps its lock fds open until the test closes its stdin. Trees are plain
directories (the cwd), so two different trees can contend for one slot.
"""

from __future__ import annotations

import fcntl
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SELECT = HERE / "slot-select.sh"


class SlotSelectTests(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        base = Path(self._tmp.name).resolve()
        self.locks, self.targets = base / "locks", base / "targets"
        self.tree_a, self.tree_b = base / "treeA", base / "treeB"
        for d in (self.locks, self.targets, self.tree_a, self.tree_b):
            d.mkdir()
        self._held: set[int] = set()

    def slot(self, letter: str, tree: Path | None = None) -> Path:
        path = self.targets / f"slot-{letter}"
        path.mkdir(exist_ok=True)
        if tree is not None:
            (path / ".solaris-tree").write_text(f"{tree}\n")
        return path

    def marker(self, letter: str) -> str:
        return (self.targets / f"slot-{letter}" / ".solaris-tree").read_text().strip()

    def start(self, tree: Path, explicit: Path | None = None) -> subprocess.Popen:
        arg = f"CARGO_TARGET_DIR={explicit}" if explicit else ""
        script = (f"set -euo pipefail; dir={self.locks}; SOLARIS_SLOT_TARGETS={self.targets}; SOLARIS_SLOT_MIN_FREE_GB=0; "
                  f"source {SELECT}; select_slot {arg}; echo \"target=$target\"; exec cat")
        proc = subprocess.Popen(["bash", "-c", script], cwd=tree, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, text=True)
        self.addCleanup(self.stop, proc)
        return proc

    def stop(self, proc: subprocess.Popen) -> None:
        if proc.stdin and not proc.stdin.closed:
            proc.stdin.close()
        proc.wait(timeout=30)
        for stream in (proc.stdout, proc.stderr):
            if stream:
                stream.close()

    def running(self, tree: Path, explicit: Path | None = None) -> tuple[subprocess.Popen, str]:
        """Start a command and return once select_slot succeeded (it holds its locks)."""
        proc = self.start(tree, explicit)
        line = proc.stdout.readline().strip()
        self.assertTrue(line.startswith("target="), f"refused: {proc.stderr.read() if proc.poll() is not None else line}")
        return proc, line.removeprefix("target=")

    def refused(self, tree: Path, explicit: Path) -> str:
        proc = self.start(tree, explicit)
        proc.stdin.close()
        self.assertEqual(proc.wait(timeout=30), 75)
        return proc.stderr.read()

    def hold(self, path: Path, op: int) -> int:
        fd = os.open(path, os.O_WRONLY | os.O_CREAT)
        try:
            fcntl.flock(fd, op | fcntl.LOCK_NB)
        except BlockingIOError:
            os.close(fd)
            raise
        self._held.add(fd)
        self.addCleanup(self.release, fd)
        return fd

    def release(self, fd: int) -> None:
        if fd in self._held:
            self._held.discard(fd)
            os.close(fd)

    def test_explicit_target_of_another_tree_is_refused_while_the_owner_runs(self):
        slot = self.slot("a")
        self.running(self.tree_a, slot)
        err = self.refused(self.tree_b, slot)
        self.assertIn("in use by another tree", err)
        self.assertEqual(self.marker("a"), str(self.tree_a))

    def test_same_tree_shares_a_held_slot_as_pin_release_does(self):
        slot = self.slot("a", self.tree_a)
        self.hold(self.locks / "a", fcntl.LOCK_EX)  # pin-release holds the pool lock around its build
        _, target = self.running(self.tree_a, slot)
        self.assertEqual(target, str(slot))
        with self.assertRaises(BlockingIOError):  # the sharer holds the use lock for slot-gc
            self.hold(self.locks / "a.use", fcntl.LOCK_EX)

    def test_explicit_target_takes_a_free_slot_and_re_marks_it(self):
        slot = self.slot("a", self.tree_b)
        self.running(self.tree_a, slot)
        self.assertEqual(self.marker("a"), str(self.tree_a))

    def test_target_nested_inside_a_slot_is_refused(self):
        slot = self.slot("a", self.tree_b)
        err = self.refused(self.tree_a, slot / "subdir")
        self.assertIn("inside pooled slot-a/subdir", err)
        self.assertEqual(self.marker("a"), str(self.tree_b))

    def test_picked_slot_skips_a_slot_a_same_tree_sharer_still_uses(self):
        slot_a = self.slot("a")
        owner, _ = self.running(self.tree_a, slot_a)
        self.running(self.tree_a, slot_a)  # a second command of tree A shares it
        self.stop(owner)  # the owner exits; the sharer keeps running
        _, picked = self.running(self.tree_b)
        self.assertEqual(picked, str(self.targets / "slot-b"))
        self.assertEqual(self.marker("a"), str(self.tree_a))

    def test_another_tree_cannot_take_over_while_a_same_tree_sharer_runs(self):
        slot_a = self.slot("a")
        owner, _ = self.running(self.tree_a, slot_a)
        self.running(self.tree_a, slot_a)
        self.stop(owner)
        self.assertIn("in use by another tree", self.refused(self.tree_b, slot_a))
        self.assertEqual(self.marker("a"), str(self.tree_a))

    def test_concurrent_switch_by_another_tree_is_seen_before_sharing(self):
        slot = self.slot("a", self.tree_a)  # still marked for A, but B is taking it over right now
        pool = self.hold(self.locks / "a", fcntl.LOCK_EX)
        use = self.hold(self.locks / "a.use", fcntl.LOCK_EX)
        proc = self.start(self.tree_a, slot)
        self.assertIn("being re-marked or cleaned", proc.stderr.readline())  # A waits instead of trusting the old mark
        (slot / ".solaris-tree").write_text(f"{self.tree_b}\n")  # B finishes its switch
        self.release(use)
        proc.stdin.close()
        self.assertEqual(proc.wait(timeout=30), 75)
        self.assertEqual(self.marker("a"), str(self.tree_b))
        self.release(pool)

    def test_slot_gc_pass_is_waited_for_then_the_slot_is_used(self):
        slot = self.slot("a", self.tree_a)
        pool = self.hold(self.locks / "a", fcntl.LOCK_EX)  # slot-gc holds both while it cleans
        use = self.hold(self.locks / "a.use", fcntl.LOCK_EX)
        proc = self.start(self.tree_a, slot)
        self.assertIn("being re-marked or cleaned", proc.stderr.readline())
        self.release(use)
        self.release(pool)
        self.assertEqual(proc.stdout.readline().strip(), f"target={slot}")

    def test_picked_slot_is_owned_exclusively(self):
        _, first = self.running(self.tree_a)
        _, second = self.running(self.tree_b)
        self.assertNotEqual(first, second)
        self.assertEqual(self.marker(first[-1]), str(self.tree_a))
        self.assertEqual(self.marker(second[-1]), str(self.tree_b))



if __name__ == "__main__":
    unittest.main()
