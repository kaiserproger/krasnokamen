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
import time
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
IGNORING = r'''
import os, signal
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
HONOURING = r'''
import os, signal, sys
signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
cg = open('/proc/self/cgroup').read().strip().split('::', 1)[1]
print(f'- {os.getpid()} {os.getppid()} {cg}', flush=True)
signal.pause()
'''


def wait_scope_empty(cg: str, timeout: float = 60) -> None:
    """Wait on the scope's cgroup.events until `populated 0` (or the cgroup is gone)."""
    path = Path('/sys/fs/cgroup') / cg.lstrip('/') / 'cgroup.events'
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            fd = os.open(path, os.O_RDONLY)
        except FileNotFoundError:
            return
        try:
            poller = select.poll()
            poller.register(fd, select.POLLPRI | select.POLLERR)
            os.lseek(fd, 0, os.SEEK_SET)
            if b'populated 0\n' in os.read(fd, 4096):
                return
            poller.poll(max(0, int((deadline - time.monotonic()) * 1000)))
        finally:
            os.close(fd)
    raise AssertionError(f'the scope {cg} never emptied (cgroup.events did not report populated 0)')


class CancelledScopeTests(unittest.TestCase):
    def sandbox(self):
        """A private mem-run.sh with its own state dir, slice and watchdog name."""
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
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
        for name in ('slot-select.sh', 'slot-lock.py', 'heavy-watchdog.py', 'release-build-policy.sh',
                     'measurement-window.py', 'measurement-windows', 'mem-run-guard.py', 'reserved_cpus.py'):
            shutil.copy(HERE / name, root)
        (root / 'targets' / 'slot-a').mkdir(parents=True)
        env = {**os.environ, 'CARGO_TARGET_DIR': str(root / 'targets' / 'slot-a'),
               'KRASNOKAMEN_SLOT_TARGETS': str(root / 'targets'), 'KRASNOKAMEN_SLOT_MIN_FREE_GB': '0',
               'KRASNOKAMEN_MEMRUN_CANCEL_GRACE': '1'}
        return root, runner, env

    def start(self, runner, env, *, child, compile=False):
        """Start one scope and return (process, grandchild, child, wrapper, cgroup) at its first line."""
        if compile:
            # A build-token command whose "cargo" is our child: the token is taken before the scope.
            binary = runner.parent / 'bin'
            binary.mkdir(exist_ok=True)
            (binary / 'cargo').write_text('#!/usr/bin/env bash\nexec python3 -c "$TEST_CHILD"\n')
            (binary / 'cargo').chmod(0o755)
            env = {**env, 'PATH': f'{binary}:{env["PATH"]}', 'TEST_CHILD': child}
            argv = [str(runner), '-m', '1G', 'cargo', 'test']
        else:
            argv = [str(runner), '-m', '1G', 'python3', '-c', child]
        proc = subprocess.Popen(argv, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, text=True)
        self.addCleanup(self.stop, proc)
        if not select.select([proc.stdout], [], [], 120)[0]:
            _, error = proc.communicate(timeout=30)
            self.fail(f'scope command did not start: {error[-2000:]}')
        line = proc.stdout.readline().strip()
        self.assertTrue(line, 'scope command did not start')
        grandchild, child_pid, wrapper, cg = line.split()
        return proc, grandchild, child_pid, wrapper, cg

    def stop(self, proc):
        if proc.poll() is None:
            proc.kill()
            proc.wait(timeout=10)
        for stream in (proc.stdin, proc.stdout, proc.stderr):
            if stream:
                stream.close()

    def scope_kill(self, cg):
        subprocess.run(['systemctl', '--user', 'kill', '--kill-whom=all', '--signal=KILL',
                        cg.rsplit('/', 1)[-1]], stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL, check=False)

    def test_a_term_ignoring_descendant_does_not_hold_the_scope(self):
        root, runner, env = self.sandbox()
        proc, grandchild, child, wrapper, cg = self.start(runner, env, child=IGNORING)
        # The wrapper is the inner scope bash: a targeted TERM, exactly what an agent
        # cancelling one job by pid does. Only the direct child used to receive it.
        os.kill(int(wrapper), signal.SIGTERM)
        try:
            proc.wait(timeout=90)
            wait_scope_empty(cg)
        except subprocess.TimeoutExpired:
            self.scope_kill(cg)
            self.fail('the cancelled scope never ended: a TERM-ignoring descendant held the '
                      'wrapper, the scope and its build token (pre-fix behaviour)')
        self.assertNotEqual(proc.returncode, 0, 'a cancelled command is not a successful run')
        for pid in (grandchild, child):
            if pid == '-':
                continue
            # The scope is empty; a task an init has not reaped yet is already dead and takes no CPU.
            deadline = time.monotonic() + 10
            while True:
                try:
                    os.kill(int(pid), 0)
                except ProcessLookupError:
                    break
                if time.monotonic() >= deadline:
                    self.fail(f'process {pid} survived in {cg} after the scope emptied')
                time.sleep(0.05)

    def test_a_term_honouring_child_ends_the_scope_before_the_escalation(self):
        root, runner, env = self.sandbox()
        env['KRASNOKAMEN_MEMRUN_CANCEL_GRACE'] = '60'
        proc, _, child, wrapper, cg = self.start(runner, env, child=HONOURING)
        os.kill(int(wrapper), signal.SIGTERM)
        try:
            # Well inside the 60 s escalation window: only the forwarded TERM can end this scope.
            proc.wait(timeout=20)
        except subprocess.TimeoutExpired:
            self.scope_kill(cg)
            self.fail('a TERM-honouring child did not end its scope without the guardian escalation')
        wait_scope_empty(cg)
        self.assertNotEqual(proc.returncode, 0, 'a cancelled command is not a successful run')

    def test_the_token_is_held_during_the_scope_and_free_after_a_cancellation(self):
        root, runner, env = self.sandbox()
        proc, grandchild, child, wrapper, cg = self.start(runner, env, child=IGNORING, compile=True)
        token = root / 'state' / 'tokens' / 'token-1.lock'
        self.assertNotEqual(subprocess.run(['flock', '-n', str(token), 'true']).returncode, 0,
                            'a compile scope must hold a build token while it runs')
        os.kill(int(wrapper), signal.SIGTERM)
        try:
            proc.wait(timeout=90)
            wait_scope_empty(cg)
        except subprocess.TimeoutExpired:
            self.scope_kill(cg)
            self.fail('the cancelled compile scope never ended')
        self.assertEqual(subprocess.run(['flock', '-w', '10', str(token), 'true']).returncode, 0,
                         'the build token was not returned after the scope emptied')


if __name__ == '__main__':
    unittest.main()
