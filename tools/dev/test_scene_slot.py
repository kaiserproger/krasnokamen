"""Tests for scene-slot.sh: python3 tools/dev/test_scene_slot.py"""

from __future__ import annotations

import fcntl
import os
import select
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "scene-slot.sh"
# Each slot holds exactly one of these after the command starts (or none).
PROBE = (
    "import fcntl, os, sys\n"
    "held = []\n"
    "for slot in 'abc':\n"
    "    fd = os.open('%s/solaris-scene-slot-%s.lock' % (sys.argv[1], slot), os.O_WRONLY | os.O_CREAT)\n"
    "    try:\n"
    "        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)\n"
    "    except BlockingIOError:\n"
    "        held.append(slot)\n"
    "    os.close(fd)\n"
    "print(''.join(held))\n"
)


class SceneSlot(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.dir = tmp.name
        self.env = {**os.environ, "SOLARIS_SCENE_SLOT_DIR": self.dir}

    def lock_path(self, slot: str) -> str:
        return f"{self.dir}/solaris-scene-slot-{slot}.lock"

    def hold(self, slot: str) -> int:
        fd = os.open(self.lock_path(slot), os.O_WRONLY | os.O_CREAT, 0o666)
        fcntl.flock(fd, fcntl.LOCK_EX)
        return fd

    def held_by_other(self, slot: str) -> bool:
        fd = os.open(self.lock_path(slot), os.O_WRONLY | os.O_CREAT, 0o666)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return False
        except BlockingIOError:
            return True
        finally:
            os.close(fd)

    def wait_blocked_on(self, slot: str, deadline: float) -> None:
        """Return once /proc/locks shows a waiter blocked on this slot's lock file."""
        st = os.stat(self.lock_path(slot))
        key = f"{os.major(st.st_dev):02x}:{os.minor(st.st_dev):02x}:{st.st_ino}"
        while time.monotonic() < deadline:
            with open("/proc/locks") as locks:
                if any("->" in line and key in line.split() for line in locks):
                    return
            os.sched_yield()
        self.fail(f"no waiter blocked on slot {slot}")

    def test_waiter_takes_whichever_slot_frees_first(self) -> None:
        held = {slot: self.hold(slot) for slot in "abc"}
        try:
            waiter = subprocess.Popen(
                [str(SCRIPT), "sh", "-c", "echo run; read _"],
                env=self.env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
            )
            self.addCleanup(waiter.kill)
            deadline = time.monotonic() + 30
            # The waiter has given up on non-blocking attempts once it waits on slot a.
            self.wait_blocked_on("a", deadline)
            os.close(held.pop("b"))
            ready, _, _ = select.select([waiter.stdout], [], [], max(0.0, deadline - time.monotonic()))
            self.assertTrue(ready, "waiter did not start when slot b freed while a stayed held")
            self.assertEqual(waiter.stdout.readline().strip(), "run")
            self.assertTrue(self.held_by_other("b"), "the running command holds slot b")
            waiter.stdin.close()
            waiter.wait(timeout=30)
            self.assertFalse(self.held_by_other("b"), "slot b is free after the command exits")
        finally:
            for fd in held.values():
                os.close(fd)

    def test_lock_survives_a_command_that_reopens_low_fds(self) -> None:
        # mem-run.sh does `exec 3>...` and `3>&-`; the slot must stay held through it.
        probe = Path(self.dir) / "probe.py"
        probe.write_text(PROBE)
        command = f"exec 3>/dev/null 4>/dev/null 5>/dev/null 6>/dev/null 7>/dev/null 8>/dev/null; " \
                  f"exec {sys.executable} {probe} {self.dir}"
        out = subprocess.run([str(SCRIPT), "sh", "-c", command], env=self.env,
                             capture_output=True, text=True, timeout=60, check=True)
        self.assertEqual(len(out.stdout.strip()), 1, f"exactly one slot held, got {out.stdout!r}")


if __name__ == "__main__":
    unittest.main()
