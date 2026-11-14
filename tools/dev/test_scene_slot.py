"""Tests for scene-slot.sh: python3 tools/dev/test_scene_slot.py"""

from __future__ import annotations

import fcntl
import os
import select
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "scene-slot.sh"
# Each slot holds exactly one of these after the command starts (or none).
PROBE = (
    "import fcntl, os, sys\n"
    "held = []\n"
    "for slot in 'abcdef':\n"
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
        self.env = {**os.environ, "KRASNOKAMEN_SCENE_SLOT_DIR": self.dir}

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

    def test_waiter_takes_whichever_slot_frees_first(self) -> None:
        held = {slot: self.hold(slot) for slot in "abcdef"}
        ready_r, ready_w = os.pipe()
        try:
            waiter = subprocess.Popen(
                [str(SCRIPT), "sh", "-c", "echo run; read _"],
                env={**self.env, "KRASNOKAMEN_SCENE_SLOT_READY_FD": str(ready_w)}, pass_fds=(ready_w,),
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
            )
            self.addCleanup(waiter.kill)
            os.close(ready_w)
            # The script writes once its slot waiters are queued; 30 s only bounds a failure.
            queued, _, _ = select.select([ready_r], [], [], 30)
            self.assertTrue(queued and os.read(ready_r, 1) == b"w", "waiter never reported it is queued")
            os.close(held.pop("b"))
            started, _, _ = select.select([waiter.stdout], [], [], 30)
            self.assertTrue(started, "waiter did not start when slot b freed while a stayed held")
            self.assertEqual(waiter.stdout.readline().strip(), "run")
            self.assertTrue(self.held_by_other("b"), "the running command holds slot b")
            waiter.stdin.close()
            waiter.wait(timeout=30)
            self.assertFalse(self.held_by_other("b"), "slot b is free after the command exits")
        finally:
            os.close(ready_r)
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

    def test_refuses_to_take_a_slot_inside_an_exclusive_window(self) -> None:
        # Gates hold slots while they wait for the window's lock: waiting inside it deadlocks the fleet.
        out = subprocess.run([str(SCRIPT), "sh", "-c", "echo ran"],
                             env={**self.env, "KRASNOKAMEN_EXCLUSIVE_WINDOW": "4242"},
                             capture_output=True, text=True, timeout=60)
        self.assertEqual(out.returncode, 75, out.stderr)
        self.assertEqual(out.stdout, "")
        self.assertIn("scene-slot.sh mem-run.sh -x", out.stderr)
        self.assertFalse(any(self.held_by_other(slot) for slot in "abcdef"))

    def test_window_inside_a_held_slot_reuses_it(self) -> None:
        # scene-slot.sh mem-run.sh -x CMD: CMD's own scene-slot.sh runs at once in the inherited slot.
        probe = Path(self.dir) / "probe.py"
        probe.write_text(PROBE)
        inner = f"KRASNOKAMEN_EXCLUSIVE_WINDOW=4242 exec {SCRIPT} {sys.executable} {probe} {self.dir}"
        out = subprocess.run([str(SCRIPT), "sh", "-c", inner], env=self.env,
                             capture_output=True, text=True, timeout=60, check=True)
        self.assertEqual(len(out.stdout.strip()), 1, f"exactly one slot held, got {out.stdout!r}")


if __name__ == "__main__":
    unittest.main()
