"""A cancelled scope must end even when a descendant ignores SIGTERM (#G5, CS341).

Real scope, real guardian: a targeted TERM to the wrapper used to leave it in `wait` forever,
holding the scope and its build token, because only the direct child got the signal.
"""
import os
import select
import shutil
import signal
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
CHILD = r'''
import os, signal, sys
signal.signal(signal.SIGTERM, signal.SIG_IGN)
pid = os.fork()
if pid == 0:
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    signal.pause()
    os._exit(0)
cg = open('/proc/self/cgroup').read().strip().split('::', 1)[1]
print(f'{pid} {os.getpid()} {os.getppid()} {cg}', flush=True)
os.waitpid(pid, 0)
'''


class CancelledScopeTests(unittest.TestCase):
    def test_a_term_ignoring_descendant_does_not_hold_the_scope(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            slice_name = f'memtest{os.getpid()}.slice'
            subprocess.run(['systemctl', '--user', 'start', slice_name], check=True)
            self.addCleanup(subprocess.run, ['systemctl', '--user', 'stop', slice_name], check=False)
            watchdog = f'memtest{os.getpid()}-watchdog'
            self.addCleanup(subprocess.run, ['systemctl', '--user', 'stop', watchdog + '.service'], check=False)
            source = (HERE / 'mem-run.sh').read_text()
            source = source.replace('dir=/tmp/solaris-mem-slots', f'dir={root}/state', 1)
            source = source.replace('slice=solaris-heavy.slice', f'slice={slice_name}', 1)
            source = source.replace('solaris-heavy-watchdog', watchdog)
            runner = root / 'mem-run.sh'
            runner.write_text(source)
            runner.chmod(0o755)
            for name in ('slot-select.sh', 'heavy-watchdog.py', 'release-build-policy.sh',
                         'measurement-window.py', 'measurement-windows', 'mem-run-guard.py'):
                shutil.copy(HERE / name, root)
            (root / 'targets' / 'slot-a').mkdir(parents=True)
            env = {**os.environ, 'CARGO_TARGET_DIR': str(root / 'targets' / 'slot-a'),
                   'KRASNOKAMEN_SLOT_TARGETS': str(root / 'targets'), 'KRASNOKAMEN_SLOT_MIN_FREE_GB': '0'}
            proc = subprocess.Popen([str(runner), '-m', '1G', 'python3', '-c', CHILD], env=env,
                                    stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                    stderr=subprocess.PIPE, text=True)
            cg = None
            try:
                self.assertTrue(select.select([proc.stdout], [], [], 60)[0], 'scope command did not start')
                line = proc.stdout.readline().strip()
                if not line:
                    # Never read stderr unbounded here: the scope may still be alive with the pipe open.
                    proc.kill()
                    _, error = proc.communicate(timeout=30)
                    self.fail(f'scope command did not start: {error[-2000:]}')
                grandchild, child, wrapper, cg = line.split()
                # The wrapper is the inner scope bash: a targeted TERM, exactly what an agent
                # cancelling one job by pid does. Only the direct child used to receive it.
                os.kill(int(wrapper), signal.SIGTERM)
                try:
                    proc.wait(timeout=90)
                except subprocess.TimeoutExpired:
                    self.fail('the cancelled scope never ended: a TERM-ignoring descendant held the '
                              'wrapper, the scope and its build token (pre-fix behaviour)')
                self.assertNotEqual(proc.returncode, 0, 'a cancelled command is not a successful run')
                for pid in (grandchild, child):
                    with self.assertRaises(ProcessLookupError, msg=f'process {pid} survived in {cg}'):
                        os.kill(int(pid), 0)
            finally:
                if cg:
                    subprocess.run(['systemctl', '--user', 'kill', '--kill-whom=all', '--signal=KILL',
                                    cg.rsplit('/', 1)[-1]], stdout=subprocess.DEVNULL,
                                   stderr=subprocess.DEVNULL, check=False)
                if proc.poll() is None:
                    proc.kill()
                    proc.wait(timeout=10)
                for stream in (proc.stdin, proc.stdout, proc.stderr):
                    if stream:
                        stream.close()


if __name__ == '__main__':
    unittest.main()
