"""Integration: mem-run must retain its scope until a killed harness's tree is gone.

The harness command is started through `sh -c` so the runner does not classify it as
a compile and the test never queues behind the host's three build tokens: its subject
is scope lifetime, not token admission. `token=none` in the runner's scope line is
asserted, so a future change that reintroduces a token wait fails here instead of
hanging until the bound.
"""
import json
import os
import select
import signal
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RUNNER = Path(os.environ.get('MEMRUN_TEST_SCRIPT', '/run/media/kaiserroman/games/solaris-work/solaris/tools/dev/mem-run.sh'))
# Hidden from the runner's token classification on purpose (see the module docstring).
HARNESS = 'exec python3 -m tools.harness run test -p mc-core --lib'
# The stage-writes-the-FIFO event, not a timeout: generous enough for a loaded host.
START_BOUND = 120


class HarnessScopeTests(unittest.TestCase):
    def check_signal(self, sig):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fifo = root / 'ready'
            os.mkfifo(fifo)
            ready = os.open(fifo, os.O_RDWR | os.O_NONBLOCK)
            cargo = root / 'cargo'
            cargo.write_text('''#!/usr/bin/python3
import os
pid = os.fork()
if pid == 0:
    os.setsid()
    with open(os.environ['TEST_READY'], 'w') as f:
        f.write(f"{os.getpid()} {os.getppid()} {os.getppid()}\\n")
        f.flush()
    r, w = os.pipe()
    os.read(r, 1)
else:
    os.waitpid(pid, 0)
''')
            cargo.chmod(0o755)
            env = os.environ.copy()
            env.update(PATH=f'{root}:' + env['PATH'], TEST_READY=str(fifo),
                       KRASNOKAMEN_VALIDATION_ROOT=str(root / 'receipts'))
            stderr = root / 'runner.err'
            grandchild = stage = harness = None
            pidfd = None
            with stderr.open('wb') as log:
                proc = subprocess.Popen([str(RUNNER), '-m', '1G', 'sh', '-c', HARNESS],
                                        cwd=ROOT, env=env, stdout=subprocess.DEVNULL, stderr=log)
                try:
                    self.assertTrue(select.select([ready], [], [], START_BOUND)[0],
                                    f'stage did not start: {stderr.read_text(errors="replace")[-2000:]}')
                    grandchild, stage, _ = map(int, os.read(ready, 128).split())
                    self.assertIn('token=none', stderr.read_text(errors='replace'),
                                  'the subject is scope lifetime: this command must take no build token')
                    with open(f'/proc/{stage}/stat') as f:
                        harness = int(f.read().split(') ', 1)[1].split()[1])
                    cgroup = Path('/sys/fs/cgroup' + Path(f'/proc/{stage}/cgroup').read_text().strip().split('::', 1)[1])
                    unit = cgroup.name
                    pidfd = os.pidfd_open(grandchild)
                    os.kill(harness, sig)
                    self.assertNotEqual(proc.wait(timeout=30), 0)
                    self.assertTrue(select.select([pidfd], [], [], 10)[0], 'detached grandchild survived')
                    if sig != signal.SIGKILL:
                        receipt = json.loads(next((root / 'receipts').glob('*/result.json')).read_text())
                        self.assertEqual(receipt['status'], 'interrupted')
                        self.assertIn(signal.Signals(sig).name, receipt['error'])
                    active = subprocess.run(['systemctl', '--user', 'is-active', '--quiet', unit], check=False)
                    self.assertNotEqual(active.returncode, 0, 'scope still active')
                finally:
                    if grandchild:
                        try:
                            os.kill(grandchild, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                    if stage:
                        try:
                            os.killpg(stage, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                    if proc.poll() is None:
                        proc.kill()
                        proc.wait()
                    if pidfd is not None:
                        os.close(pidfd)
                    os.close(ready)

    def test_term_harness(self):
        self.check_signal(signal.SIGTERM)

    def test_kill_harness(self):
        self.check_signal(signal.SIGKILL)


if __name__ == '__main__':
    unittest.main()
