"""Real scope lifetime regression, isolated from the shared runner and watchdog."""
import fcntl
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
import os
pid = os.fork()
if pid == 0:
    os.read(0, 1)
    os._exit(0)
cg = open('/proc/self/cgroup').read().strip().split('::', 1)[1]
print(f'{pid} {os.getpid()} {os.getppid()} {cg}', flush=True)
os.waitpid(pid, 0)
'''

class ScopeLifetimeTests(unittest.TestCase):
    def test_killing_runner_kills_scope_children(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            slice_name = f'memtest{os.getpid()}.slice'
            subprocess.run(['systemctl', '--user', 'start', slice_name], check=True)
            self.addCleanup(subprocess.run, ['systemctl', '--user', 'stop', slice_name], check=False)
            watchdog = f'memtest{os.getpid()}-watchdog'
            self.addCleanup(subprocess.run, ['systemctl', '--user', 'stop', watchdog + '.service'], check=False)
            source = Path(os.environ.get('MEMRUN_TEST_SCRIPT', str(HERE / 'mem-run.sh'))).read_text()
            source = source.replace('dir=/tmp/solaris-mem-slots', f'dir={root}/state', 1)
            source = source.replace('slice=solaris-heavy.slice', f'slice={slice_name}', 1)
            source = source.replace('solaris-heavy-watchdog', watchdog)
            runner = root / 'mem-run.sh'
            runner.write_text(source)
            runner.chmod(0o755)
            shutil.copy(HERE / 'slot-select.sh', root)
            shutil.copy(HERE / 'heavy-watchdog.py', root)
            if (HERE / 'mem-run-guard.py').exists():
                shutil.copy(HERE / 'mem-run-guard.py', root)
            (root / 'targets' / 'slot-a').mkdir(parents=True)
            (root / 'targets' / 'slot-b').mkdir()
            env = os.environ.copy()
            env.update(CARGO_TARGET_DIR=str(root / 'targets' / 'slot-a'),
                       KRASNOKAMEN_SLOT_TARGETS=str(root / 'targets'), KRASNOKAMEN_SLOT_MIN_FREE_GB='0')
            proc = subprocess.Popen([str(runner), '-m', '1G', 'python3', '-c', CHILD],
                                    stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                    stderr=subprocess.PIPE, text=True, env=env)
            sibling_env = env | {'CARGO_TARGET_DIR': str(root / 'targets' / 'slot-b')}
            sibling = subprocess.Popen([str(runner), '-m', '1G', 'python3', '-c', CHILD],
                                       stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE, text=True, env=sibling_env)
            sibling_cg = None
            sibling_fd = None
            fds = []
            cg = None
            try:
                self.assertTrue(select.select([sibling.stdout], [], [], 30)[0], 'sibling did not start')
                sibling_line = sibling.stdout.readline().strip()
                self.assertTrue(sibling_line, 'sibling scope failed to start')
                sibling_child, _, _, sibling_cg = sibling_line.split()
                sibling_fd = os.pidfd_open(int(sibling_child))
                other_env = env | {'CARGO_TARGET_DIR': str(root / 'other')}
                normal = subprocess.run([str(runner), '-m', '1G', 'true'], env=other_env,
                                        capture_output=True, text=True, timeout=30)
                self.assertEqual(normal.returncode, 0, normal.stderr)
                self.assertIn('true\t', (root / 'state' / 'peaks').read_text())
                perf = subprocess.run([str(runner), '-p', '-m', '1G', 'bash', '-c', 'exit 37'],
                                      env=other_env, capture_output=True, text=True, timeout=30)
                self.assertEqual(perf.returncode, 37, perf.stderr)
                term_script = ('import signal,sys; '
                               'signal.signal(signal.SIGTERM, lambda *_: sys.exit(23)); '
                               'print("ready", flush=True); signal.pause()')
                term = subprocess.Popen([str(runner), '-m', '1G', 'python3', '-c', term_script],
                                        env=other_env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                try:
                    self.assertTrue(select.select([term.stdout], [], [], 30)[0], 'TERM test did not start')
                    self.assertEqual(term.stdout.readline().strip(), 'ready')
                    term.terminate()
                    _, term_err = term.communicate(timeout=30)
                    self.assertEqual(term.returncode, 23, term_err)
                finally:
                    if term.poll() is None:
                        term.kill()
                        term.communicate(timeout=10)
                ready, _, _ = select.select([proc.stdout], [], [], 30)
                if not ready:
                    proc.kill()
                    proc.wait()
                    self.fail('scope command did not start')
                line = proc.stdout.readline().strip()
                if not line:
                    self.fail(f'scope command failed: {proc.stderr.read()}')
                child, command, wrapper, cg = line.split()
                fds = [os.pidfd_open(int(pid)) for pid in (child, command, wrapper)]
                print(f'outer={proc.pid} wrapper={wrapper} command={command} child={child} scope={cg}', flush=True)
                os.kill(proc.pid, signal.SIGKILL)
                proc.wait(timeout=10)
                # If the slot becomes available, no old process may still own it.
                with open(root / 'state' / 'a', 'w') as lock:
                    fcntl.flock(lock, fcntl.LOCK_EX)
                    self.assertTrue(select.select([fds[0]], [], [], 0)[0],
                                    'slot released while its previous child was alive')
                self.assertFalse(select.select([sibling_fd], [], [], 0)[0], 'neighbor scope was killed')
                for pid, fd in zip((child, command, wrapper), fds):
                    ready, _, _ = select.select([fd], [], [], 3)
                    self.assertTrue(ready, f'process {pid} survived in {cg}')
            finally:
                if sibling_cg:
                    subprocess.run(['systemctl', '--user', 'kill', '--kill-whom=all', '--signal=KILL', sibling_cg.rsplit('/', 1)[-1]],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
                if sibling.poll() is None:
                    sibling.kill()
                sibling.wait(timeout=10)
                if sibling_fd is not None:
                    os.close(sibling_fd)
                for stream in (sibling.stdin, sibling.stdout, sibling.stderr):
                    if stream:
                        stream.close()
                if cg:
                    subprocess.run(['systemctl', '--user', 'kill', '--kill-whom=all', '--signal=KILL', cg.rsplit('/', 1)[-1]],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
                if proc.poll() is None:
                    proc.kill()
                    proc.wait(timeout=10)
                for fd in fds:
                    os.close(fd)
                for stream in (proc.stdin, proc.stdout, proc.stderr):
                    if stream:
                        stream.close()
            quiet = subprocess.run([str(runner), '-x', '-m', '1G', 'true'], env=other_env,
                                   capture_output=True, text=True, timeout=30)
            self.assertEqual(quiet.returncode, 0, quiet.stderr)

if __name__ == '__main__':
    unittest.main()
