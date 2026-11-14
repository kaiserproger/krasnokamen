"""CARGO_INCREMENTAL policy mem-run exports (lane and profile).

Isolated like test_mem_run_orphans: a copy of the runner with a private state dir, slice and
watchdog, so even the -x window only touches this test's own scopes. No Cargo build runs; the
child is `sh -c printf` and reads its own environment.
Run: python3 tools/dev/test_mem_run_build_env.py (MEMRUN_TEST_SCRIPT selects another runner copy)."""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
PROBE = 'printf "%s\\n" "$CARGO_INCREMENTAL"'


class BuildEnvTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        slice_name = f'memenv{os.getpid()}.slice'
        self.watchdog = f'memenv{os.getpid()}-watchdog'
        subprocess.run(['systemctl', '--user', 'start', slice_name], check=True)
        self.addCleanup(subprocess.run, ['systemctl', '--user', 'stop', slice_name], check=False)
        self.addCleanup(subprocess.run, ['systemctl', '--user', 'stop', self.watchdog + '.service'], check=False)
        self.source = (Path(os.environ.get('MEMRUN_TEST_SCRIPT', str(HERE / 'mem-run.sh')))).read_text()
        self.source = self.source.replace('dir=/tmp/solaris-mem-slots', f'dir={self.root}/state', 1)
        self.source = self.source.replace('slice=solaris-heavy.slice', f'slice={slice_name}', 1)
        self.source = self.source.replace('solaris-heavy-watchdog', self.watchdog)
        for name in ('slot-select.sh', 'heavy-watchdog.py', 'mem-run-guard.py'):
            if (HERE / name).exists():
                shutil.copy(HERE / name, self.root)
        (self.root / 'targets' / 'slot-a').mkdir(parents=True)
        self.env = os.environ | {'CARGO_TARGET_DIR': str(self.root / 'targets' / 'slot-a'),
                                 'SOLARIS_SLOT_TARGETS': str(self.root / 'targets'), 'SOLARIS_SLOT_MIN_FREE_GB': '0'}

    def effective(self, *runner_args, tail=()):
        runner = self.root / 'mem-run-copy.sh'
        runner.write_text(self.source)
        runner.chmod(0o755)
        out = subprocess.run([str(runner), *runner_args, 'sh', '-c', PROBE, 'sh', *tail],
                             env=self.env, check=True, capture_output=True, text=True, timeout=120).stdout
        return out.strip()

    def test_incremental_policy_by_lane_and_profile(self):
        cases = [
            (('-m', '1G'), (), '1'),                      # debug/test build
            (('-m', '1G'), ('--release',), '1'),          # normal-lane release build
            (('-m', '1G'), ('--profile', 'fast'), '0'),   # custom profile
            (('-m', '1G'), ('--release', '--profile=x'), '0'),
            (('-p', '-m', '1G'), (), '0'),                # perf lane
            (('-p', '-m', '1G'), ('--release',), '0'),
            (('-x', '-m', '1G'), (), '0'),                # quiet window
            (('-x', '-m', '1G'), ('--release',), '0'),
        ]
        for runner_args, tail, expected in cases:
            with self.subTest(runner_args=runner_args, tail=tail):
                self.assertEqual(self.effective(*runner_args, tail=tail), expected)


if __name__ == '__main__':
    unittest.main()
