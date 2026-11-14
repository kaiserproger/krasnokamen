"""Build environment mem-run exports: CARGO_INCREMENTAL (lane and profile) and RUSTC_WRAPPER (sccache).

Isolated like test_mem_run_orphans: a copy of the runner with a private state dir, slice and
watchdog, so even the -x window only touches this test's own scopes. No Cargo build runs; the
child is `sh -c printf` and reads its own environment. The sccache unit and binary are variables in
the runner copy; the test points them at a private transient unit and private stub files, so it never
depends on or touches the real solaris-sccache.service.
Run: python3 tools/dev/test_mem_run_build_env.py (MEMRUN_TEST_SCRIPT selects another runner copy)."""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
PROBE = 'printf "%s|%s|%s\\n" "$CARGO_INCREMENTAL" "$RUSTC_WRAPPER" "$SCCACHE_CLIENT_SIDE"'


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
        for name in ('slot-select.sh', 'heavy-watchdog.py', 'mem-run-guard.py', 'sccache.sh'):
            if (HERE / name).exists():
                shutil.copy(HERE / name, self.root)
        (self.root / 'targets' / 'slot-a').mkdir(parents=True)
        self.env = os.environ | {'CARGO_TARGET_DIR': str(self.root / 'targets' / 'slot-a'),
                                 'SOLARIS_SLOT_TARGETS': str(self.root / 'targets'), 'SOLARIS_SLOT_MIN_FREE_GB': '0'}

    def start_unit(self):
        unit = f'memenv{os.getpid()}-sccache'
        fifo = self.root / 'unit-fifo'  # the unit blocks on this FIFO until stopped: no timer
        os.mkfifo(fifo)
        subprocess.run(['systemd-run', '--user', '--quiet', f'--unit={unit}', 'cat', str(fifo)], check=True)
        self.addCleanup(subprocess.run, ['systemctl', '--user', 'stop', unit + '.service'], check=False)
        return unit + '.service'

    def effective(self, *runner_args, tail=(), unit='solaris-sccache-absent-test.service', binary=None, cmd=None):
        runner = self.root / 'mem-run-copy.sh'
        source = self.source.replace('sccache_unit=solaris-sccache.service', f'sccache_unit={unit}', 1)
        self.assertIn(f'sccache_unit={unit}', source)
        stub = binary or self.root / 'sccache-stub'
        source = source.replace('sccache_bin="$HOME/.cargo/bin/sccache"', f'sccache_bin={stub}', 1)
        self.assertIn(f'sccache_bin={stub}', source)
        runner.write_text(source)
        runner.chmod(0o755)
        out = subprocess.run([str(runner), *runner_args, *(cmd or []), 'sh', '-c', PROBE, 'sh', *tail],
                             env=self.env, check=True, capture_output=True, text=True, timeout=120).stdout
        return out.strip().split('|')

    def make_stub(self, executable=True):
        stub = self.root / 'sccache-stub'
        stub.write_text('#!/bin/sh\n')
        stub.chmod(0o755 if executable else 0o644)
        return stub

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
                self.assertEqual(self.effective(*runner_args, tail=tail)[0], expected)

    def test_wrapper_follows_service_and_binary(self):
        wrapper = str(self.root / 'sccache.sh')
        unit = self.start_unit()
        stub = self.make_stub()
        self.assertEqual(self.effective('-m', '1G', unit=unit)[1:], [wrapper, '1'])
        # the lanes that disable incremental still get the cache
        self.assertEqual(self.effective('-p', '-m', '1G', unit=unit)[1:], [wrapper, '1'])
        # service inactive
        self.assertEqual(self.effective('-m', '1G')[1:], ['', ''])
        # service active but sccache not executable / missing
        self.make_stub(executable=False)
        self.assertEqual(self.effective('-m', '1G', unit=unit)[1:], ['', ''])
        stub.unlink()
        self.assertEqual(self.effective('-m', '1G', unit=unit)[1:], ['', ''])

    def test_explicit_env_override_from_old_briefs_wins(self):
        unit = self.start_unit()
        self.make_stub()
        self.assertEqual(self.effective('-m', '1G', unit=unit, cmd=['env', 'RUSTC_WRAPPER='])[1], '')
        self.assertEqual(self.effective('-m', '1G', unit=unit, cmd=['env', 'RUSTC_WRAPPER=/bin/true'])[1], '/bin/true')


if __name__ == '__main__':
    unittest.main()
