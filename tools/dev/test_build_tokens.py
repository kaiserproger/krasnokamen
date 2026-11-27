"""No build-token admission in mem-run.sh: python3 tools/dev/test_build_tokens.py

Owner D127 (2026-11-27) removed build tokens: a compile, gate or run no longer waits
for another command, and no priority lane or disk floor queues one. What remains is
the shared CPU jobserver (shares cores, never queues), each scope's MemoryMax, the
OOM watchdog and the scene slots.

The file name and `BuildTokenTests.setUp` stay because `test_scene_slot.py` imports
them for its isolated runner sandbox. The tests prove the removal: the runner text
has no token assignment, no token lock directory appears, and several concurrent
compile commands all start at once.
"""

from __future__ import annotations

import os
import select
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
BOUND = 60  # seconds; only a failure ever waits this long
STUB = '''#!/usr/bin/env python3
import os, sys
print(f"run {os.getpid()}", flush=True)
for _ in sys.stdin:
    pass
'''


class BuildTokenTests(unittest.TestCase):
    """An isolated runner copy with its own state dir, slice and watchdog."""

    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        slice_name = f'memtok{os.getpid()}.slice'
        watchdog = f'memtok{os.getpid()}-watchdog'
        subprocess.run(['systemctl', '--user', 'start', slice_name], check=True)
        self.addCleanup(subprocess.run, ['systemctl', '--user', 'stop', slice_name], check=False)
        self.addCleanup(subprocess.run, ['systemctl', '--user', 'stop', watchdog + '.service'], check=False)
        source = Path(os.environ.get('MEMRUN_TEST_SCRIPT', str(HERE / 'mem-run.sh'))).read_text()
        self.assertIn('dir=/tmp/solaris-mem-slots', source)
        self.assertIn('slice=solaris-heavy.slice', source)
        self.source = source
        source = source.replace('dir=/tmp/solaris-mem-slots', f'dir={self.root / "state"}', 1)
        source = source.replace('slice=solaris-heavy.slice', f'slice={slice_name}', 1)
        source = source.replace('solaris-heavy-watchdog', watchdog)
        self.runner = self.root / 'mem-run.sh'
        self.runner.write_text(source)
        self.runner.chmod(0o755)
        for name in ('slot-select.sh', 'slot-gc', 'heavy-watchdog.py', 'mem-run-guard.py', 'slot-lock.py',
                     'sccache.sh', 'release-build-policy.sh', 'measurement-window.py', 'measurement-windows',
                     'reserved_cpus.py'):
            if (HERE / name).exists():
                shutil.copy(HERE / name, self.root)
        bin_dir = self.root / 'bin'
        bin_dir.mkdir()
        (bin_dir / 'cargo').write_text(STUB.replace('/usr/bin/env python3', os.sys.executable, 1))
        (bin_dir / 'cargo').chmod(0o755)
        self.tree = self.root / 'wt'
        self.tree.mkdir()
        self.state_dir = self.root / 'state'
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith('KRASNOKAMEN_')}
        self.env.update(PATH=f'{bin_dir}:{os.environ["PATH"]}',
                        CARGO_TARGET_DIR=str(self.root / 'targets' / 'slot-a'),
                        KRASNOKAMEN_SLOT_TARGETS=str(self.root / 'targets'),
                        KRASNOKAMEN_DISK_GC_BELOW_GB='0', KRASNOKAMEN_DISK_REFUSE_BELOW_GB='0',
                        KRASNOKAMEN_WINDOW_OVERRIDE='1', KRASNOKAMEN_RELEASE_BUILD_ALLOW='1')
        (self.root / 'targets' / 'slot-a').mkdir(parents=True)
        self.count = 0

    def start(self, *command: str) -> subprocess.Popen:
        self.count += 1
        log = self.root / f'runner-{self.count}.log'
        with open(log, 'w') as stderr:
            return subprocess.Popen([str(self.runner), '-m', '1G', *command], cwd=self.tree, env=self.env,
                                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr, text=True)

    def stop(self, proc: subprocess.Popen) -> None:
        if proc.poll() is None:
            proc.kill()
        proc.wait(timeout=BOUND)

    def test_the_runner_has_no_token_admission(self) -> None:
        self.assertNotRegex(self.source, r'^build_tokens=')
        self.assertNotIn('KRASNOKAMEN_BUILD_TOKEN', self.source)
        self.assertNotIn('slot-lock.py', self.source, 'the runner no longer takes a token lock')

    def test_concurrent_compile_commands_all_start_at_once(self) -> None:
        procs = [self.start('cargo', 'build', '-p', f'crate{n}') for n in range(4)]
        for proc in procs:
            self.addCleanup(self.stop, proc)
        for proc in procs:
            readable, _, _ = select.select([proc.stdout], [], [], BOUND)
            self.assertTrue(readable, 'a compile waited instead of starting at once')
            line = proc.stdout.readline().split()
            self.assertEqual(line[0], 'run', line)
        self.assertFalse((self.state_dir / 'tokens').exists(), 'no token lock dir may appear')
        for proc in procs:
            proc.stdin.close()
        for proc in procs:
            self.assertEqual(proc.wait(timeout=BOUND), 0)


if __name__ == '__main__':
    unittest.main()
