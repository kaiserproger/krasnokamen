"""Disk exhaustion guard in mem-run.sh: python3 tools/dev/test_disk_guard.py

Owner D124 (2026-11-27) replaced the "keep 250 GiB free plus the next run's
footprint" launch standstill with an exhaustion guard: below
KRASNOKAMEN_DISK_REFUSE_BELOW_GB (default 60 GiB) a new heavy command is refused
(exit 75); below KRASNOKAMEN_DISK_GC_BELOW_GB (default 120 GiB) an ordinary command
first runs slot-gc --delete on idle slots and rechecks. A battle run writes 15-45 GB,
which is why 60 GiB is the hard margin.

The thresholds are absolute whole GiB, so each case selects a branch on this host's
real free space (an absurd threshold instead of a fake df). A sandboxed runner copy
keeps its own state dir, slice and watchdog; `true` proves the allow path and the
refusal happens before any scope starts.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
# Everything mem-run.sh sources or executes beside itself; copies of optional ones are skipped.
SIBLINGS = ('slot-select.sh', 'release-build-policy.sh', 'slot-gc', 'slot-lock.py', 'mem-run-guard.py',
            'heavy-watchdog.py', 'measurement-window.py', 'measurement-windows', 'reserved_cpus.py', 'sccache.sh')


class DiskGuard(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = self.root = Path(tmp.name)
        slice_name = f'memdisk{os.getpid()}.slice'
        subprocess.run(['systemctl', '--user', 'start', slice_name], check=True)
        self.addCleanup(subprocess.run, ['systemctl', '--user', 'stop', slice_name], check=False)
        watchdog = f'memdisk{os.getpid()}-watchdog'
        self.addCleanup(subprocess.run, ['systemctl', '--user', 'stop', watchdog + '.service'], check=False)
        source = (HERE / 'mem-run.sh').read_text()
        self.assertIn('dir=/tmp/solaris-mem-slots', source)
        self.assertIn('slice=solaris-heavy.slice', source)
        source = source.replace('dir=/tmp/solaris-mem-slots', f'dir={root}/state', 1)
        source = source.replace('slice=solaris-heavy.slice', f'slice={slice_name}', 1)
        source = source.replace('solaris-heavy-watchdog', watchdog)
        self.runner = root / 'mem-run.sh'
        self.runner.write_text(source)
        self.runner.chmod(0o755)
        for name in SIBLINGS:
            if (HERE / name).exists():
                shutil.copy(HERE / name, root)
        (root / 'targets' / 'slot-a').mkdir(parents=True)
        self.env = os.environ.copy()
        self.env.update(CARGO_TARGET_DIR=str(root / 'targets' / 'slot-a'),
                        KRASNOKAMEN_SLOT_TARGETS=str(root / 'targets'),
                        # Off by default; the cases below opt in.
                        KRASNOKAMEN_DISK_GC_BELOW_GB='0', KRASNOKAMEN_DISK_REFUSE_BELOW_GB='0')

    def run_true(self, *flags: str, **env: str):
        return subprocess.run([str(self.runner), *flags, '-m', '1G', 'true'], env=self.env | env,
                              capture_output=True, text=True, timeout=120)

    def test_refuses_below_the_hard_floor_before_starting_anything(self) -> None:
        done = self.run_true(KRASNOKAMEN_DISK_REFUSE_BELOW_GB='100000')
        self.assertEqual(done.returncode, 75, done.stderr)
        self.assertIn('exhaustion guard', done.stderr)
        self.assertIn('refusing to start', done.stderr)
        self.assertNotIn('mem-run: scope', done.stderr, 'the refusal must precede slot selection and the scope')

    def test_a_measurement_lane_is_also_refused_below_the_floor(self) -> None:
        done = self.run_true('-x', KRASNOKAMEN_DISK_REFUSE_BELOW_GB='100000', KRASNOKAMEN_WINDOW_OVERRIDE='1')
        self.assertEqual(done.returncode, 75, done.stderr)
        self.assertIn('exhaustion guard', done.stderr)

    def test_runs_slot_gc_below_the_gc_threshold_and_starts_anyway(self) -> None:
        done = self.run_true(KRASNOKAMEN_DISK_GC_BELOW_GB='100000')
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn('running slot-gc --delete on idle slots', done.stderr)
        self.assertIn('mem-run: scope', done.stderr)

    def test_allows_a_command_above_both_thresholds(self) -> None:
        done = self.run_true()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertNotIn('slot-gc', done.stderr)
        self.assertNotIn('exhaustion guard', done.stderr)
        self.assertIn('mem-run: scope', done.stderr)

    def test_a_non_integer_threshold_is_refused_not_evaluated(self) -> None:
        done = self.run_true(KRASNOKAMEN_DISK_GC_BELOW_GB='lots')
        self.assertEqual(done.returncode, 75, done.stderr)
        self.assertIn('must be whole GiB', done.stderr)
        self.assertNotIn('mem-run: scope', done.stderr)


if __name__ == '__main__':
    unittest.main()
