"""Tests for the `mem-run -x` measurement-window schedule.

python3 tools/dev/test_measurement_window.py

The decision itself lives in tools/dev/measurement-window.py; mem-run.sh runs it
before it takes a slot, so an outside-window `-x` is refused here without opening a
real measurement window (opening one would pause every build on the host, so the
inside-window branch is proved through the helper's exit code, not by starting one).
"""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from datetime import datetime, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
HELPER = HERE / "measurement-window.py"
SCRIPT = HERE / "mem-run.sh"


def helper(config: Path, now: str) -> subprocess.CompletedProcess:
    return subprocess.run(["python3", str(HELPER), "--config", str(config), "--now", now],
                          capture_output=True, text=True, timeout=60)


def clock(now) -> time:
    hour, minute = now.split(":")
    return time(int(hour), int(minute))


class WindowConfig(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)

    def write(self, text: str) -> Path:
        path = self.root / "windows"
        path.write_text(text)
        return path

    def test_the_shipped_schedule_is_valid_and_ends_before_the_nightly_fuzz(self) -> None:
        windows = helper(HERE / "measurement-windows", "00:00")
        self.assertIn(windows.returncode, (0, 75))
        text = (HERE / "measurement-windows").read_text()
        self.assertIn("01:00-02:30", text)
        self.assertIn("03:00", text, "the schedule explains the nightly fuzz it avoids")

    def test_inside_a_window(self) -> None:
        config = self.write("# comment\n01:00-02:30\n")
        result = helper(config, "01:30")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("inside a measurement window", result.stderr)

    def test_before_a_window_names_it(self) -> None:
        config = self.write("01:00-02:30\n")
        result = helper(config, "00:15")
        self.assertEqual(result.returncode, 75, result.stderr)
        self.assertIn("Next window 01:00-02:30 in 0h45m", result.stderr)

    def test_after_every_window_names_tomorrows(self) -> None:
        config = self.write("01:00-02:30\n04:00-05:00\n")
        result = helper(config, "06:00")
        self.assertEqual(result.returncode, 75, result.stderr)
        self.assertIn("Next window 01:00-02:30 in 19h00m", result.stderr)

    def test_a_window_end_is_exclusive(self) -> None:
        config = self.write("01:00-02:30\n")
        self.assertEqual(helper(config, "02:29").returncode, 0)
        self.assertEqual(helper(config, "02:30").returncode, 75)

    def test_refuses_a_malformed_or_overnight_schedule(self) -> None:
        self.assertNotEqual(helper(self.write("01:00\n"), "01:30").returncode, 0)
        self.assertNotEqual(helper(self.write("23:00-01:00\n"), "23:30").returncode, 0)
        self.assertNotEqual(helper(self.write("# only comments\n"), "01:30").returncode, 0)


class WindowInMemRun(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        # A window two hours from the current local clock, so it never covers "now".
        now = datetime.now()
        start = (now.hour * 60 + now.minute + 120) % (24 * 60)
        end = (start + 30) % (24 * 60)
        if start >= end:  # keep the flat schedule's start-before-end rule
            start, end = 1, 2
        self.config = self.root / "windows"
        self.config.write_text(f"{start // 60:02d}:{start % 60:02d}-{end // 60:02d}:{end % 60:02d}\n")
        self.env = {key: value for key, value in os.environ.items() if key != "KRASNOKAMEN_WINDOW_OVERRIDE"}
        self.env["KRASNOKAMEN_MEASUREMENT_WINDOWS"] = str(self.config)
        self.env["TMPDIR"] = str(self.root)

    def test_refuses_ex_work_outside_a_window_before_taking_a_slot(self) -> None:
        result = subprocess.run([str(SCRIPT), "-x", "-m", "1G", "python3", "-c", "print('ran')"],
                                env=self.env, capture_output=True, text=True, timeout=120)
        self.assertEqual(result.returncode, 75, result.stderr)
        self.assertIn("-x refused", result.stderr)
        self.assertIn("Next window", result.stderr)
        self.assertIn(str(self.config), result.stderr)
        self.assertNotIn("mem-run: scope", result.stderr, "-x took the exclusive lock before the schedule check")


if __name__ == "__main__":
    unittest.main()
