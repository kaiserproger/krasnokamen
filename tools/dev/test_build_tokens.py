"""Build tokens in mem-run.sh: python3 tools/dev/test_build_tokens.py

Isolated like test_mem_run_build_env: a copy of the runner with a private state dir (so private
token locks), slice and watchdog. No Cargo runs: `cargo` is a stub on PATH that prints its token and
then blocks on stdin, so the test decides when each "build" ends. No sleeps: every wait is on a pipe
(a stub's line, slot-lock.py's queued byte) or a process exit; the 60 s bounds only catch a failure.
MEMRUN_TEST_SCRIPT selects another runner copy (the negative control removes the token exec)."""
from __future__ import annotations

import fcntl
import os
import select
import shutil
import signal
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
BOUND = 60  # seconds; only a failure ever waits this long
STUB = '''#!/usr/bin/env python3
import os, subprocess, sys
print(f"run {os.environ.get('SOLARIS_BUILD_TOKEN', '-')} {os.getpid()}", flush=True)
for line in sys.stdin:
    if line.strip() == "nest":  # a nested compile through the runner, like the ledger's cargo nextest
        out = subprocess.run([os.environ["TEST_RUNNER"], "-m", "1G", "cargo", "build"],
                             stdin=subprocess.DEVNULL, capture_output=True, text=True)
        print(f"nested {out.returncode} {out.stdout.strip()}", flush=True)
'''


class Command:
    def __init__(self, proc: subprocess.Popen, ready: int, log: Path):
        self.proc, self.ready, self.log = proc, ready, log
        self.state: str | None = None  # 'run' or 'queued'
        self.token: str | None = None

    def read_run_line(self, test: unittest.TestCase) -> None:
        line = self.proc.stdout.readline().split()
        test.assertTrue(line, f'runner exited without starting its command: {self.log.read_text()}')
        test.assertEqual(line[0], 'run', line)
        self.state, self.token = 'run', line[1]


class BuildTokenTests(unittest.TestCase):
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
        self.state_dir = self.root / 'state'
        for old, new in (('dir=/tmp/solaris-mem-slots', f'dir={self.state_dir}'),
                         ('slice=solaris-heavy.slice', f'slice={slice_name}')):
            self.assertIn(old, source)
            source = source.replace(old, new, 1)
        source = source.replace('solaris-heavy-watchdog', watchdog)
        self.runner = self.root / 'mem-run.sh'
        self.runner.write_text(source)
        self.runner.chmod(0o755)
        for name in ('slot-select.sh', 'heavy-watchdog.py', 'mem-run-guard.py', 'slot-lock.py', 'sccache.sh'):
            shutil.copy(HERE / name, self.root)
        bin_dir = self.root / 'bin'
        bin_dir.mkdir()
        (bin_dir / 'cargo').write_text(STUB.replace('/usr/bin/env python3', sys.executable, 1))
        (bin_dir / 'cargo').chmod(0o755)
        self.tree = self.root / 'wt'
        self.tree.mkdir()
        self.env = {key: value for key, value in os.environ.items()
                    if key not in ('SOLARIS_BUILD_TOKEN', 'SOLARIS_BUILD_TOKEN_PID', 'SOLARIS_SCENE_SLOT')}
        self.env.update(PATH=f'{bin_dir}:{os.environ["PATH"]}', CARGO_TARGET_DIR=str(self.root / 'target'),
                        SOLARIS_SLOT_TARGETS=str(self.root / 'targets'), SOLARIS_SLOT_MIN_FREE_GB='0',
                        TEST_RUNNER=str(self.runner))
        self.count = 0

    def start(self, *command: str) -> Command:
        """Start `mem-run.sh -m 1G <command>`; slot-lock.py writes b'w' to its ready pipe once queued."""
        self.count += 1
        ready_r, ready_w = os.pipe()
        log = self.root / f'runner-{self.count}.log'
        with open(log, 'w') as stderr:
            proc = subprocess.Popen([str(self.runner), '-m', '1G', *command], cwd=self.tree,
                                    env=self.env | {'SOLARIS_BUILD_TOKEN_READY_FD': str(ready_w)}, pass_fds=(ready_w,),
                                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr, text=True)
        os.close(ready_w)
        cmd = Command(proc, ready_r, log)
        self.addCleanup(self.stop, cmd)
        return cmd

    def stop(self, cmd: Command) -> None:
        if cmd.proc.poll() is None:
            cmd.proc.kill()  # the scope guardian kills the command's scope
            cmd.proc.wait(timeout=BOUND)
        for stream in (cmd.proc.stdin, cmd.proc.stdout):
            try:
                stream.close()
            except BrokenPipeError:
                pass
        if cmd.ready is not None:
            os.close(cmd.ready)
            cmd.ready = None

    def settle(self, cmds: list[Command]) -> None:
        """Wait until every command has started (its stub printed `run`) or reported itself queued."""
        while any(cmd.state is None for cmd in cmds):
            watched = {}
            for cmd in cmds:
                if cmd.state is None:
                    watched[cmd.proc.stdout] = cmd
                    if cmd.ready is not None:
                        watched[cmd.ready] = cmd
            readable, _, _ = select.select(list(watched), [], [], BOUND)
            self.assertTrue(readable, f'no runner started or queued within {BOUND} s')
            for stream in readable:
                cmd = watched[stream]
                if cmd.state is not None:
                    continue
                if stream is cmd.proc.stdout:
                    cmd.read_run_line(self)
                else:
                    byte = os.read(cmd.ready, 1)  # b'' = slot-lock.py won at once and closed it
                    os.close(cmd.ready)
                    cmd.ready = None
                    if byte == b'w':
                        cmd.state = 'queued'

    def wait_run(self, cmd: Command) -> None:
        readable, _, _ = select.select([cmd.proc.stdout], [], [], BOUND)
        self.assertTrue(readable, f'queued runner did not start within {BOUND} s')
        cmd.read_run_line(self)

    def finish(self, cmd: Command) -> None:
        cmd.proc.stdin.close()
        self.assertEqual(cmd.proc.wait(timeout=BOUND), 0, cmd.log.read_text())

    def held_tokens(self) -> list[str]:
        held = []
        for token in '123':
            fd = os.open(self.state_dir / 'tokens' / f'token-{token}.lock', os.O_RDONLY)
            try:
                fcntl.flock(fd, fcntl.LOCK_SH | fcntl.LOCK_NB)
            except BlockingIOError:
                held.append(token)
            finally:
                os.close(fd)
        return held

    def wait_tokens_free(self) -> None:
        """The scope guardian returns a token when the scope empties, just after the runner exits:
        block on each token lock (an alarm bounds a failure) instead of sampling it."""
        def expired(*_):
            raise AssertionError(f'a build token stayed held {BOUND} s after its runner exited')
        previous = signal.signal(signal.SIGALRM, expired)
        signal.alarm(BOUND)
        try:
            for token in '123':
                fd = os.open(self.state_dir / 'tokens' / f'token-{token}.lock', os.O_RDONLY)
                try:
                    fcntl.flock(fd, fcntl.LOCK_EX)
                finally:
                    os.close(fd)
        finally:
            signal.alarm(0)
            signal.signal(signal.SIGALRM, previous)

    def hold_three(self, extra: int = 0) -> tuple[list[Command], list[Command]]:
        cmds = [self.start('cargo', 'test', '-p', f'crate{n}') for n in range(3 + extra)]
        self.settle(cmds)
        running = [cmd for cmd in cmds if cmd.state == 'run']
        queued = [cmd for cmd in cmds if cmd.state == 'queued']
        self.assertEqual(len(running), 3, f'exactly three compile commands run, got {len(running)}')
        self.assertEqual(sorted(cmd.token for cmd in running), ['1', '2', '3'])
        self.assertEqual(self.held_tokens(), ['1', '2', '3'])
        return running, queued

    def test_fourth_compile_waits_for_a_freed_token_and_a_kill_frees_one(self):
        running, queued = self.hold_three(extra=1)
        self.assertEqual(len(queued), 1)
        fourth = queued[0]
        self.assertIsNone(fourth.proc.poll())
        status = subprocess.run([str(HERE / 'fleet-status'), '--mem-dir', str(self.state_dir)], env=self.env,
                                capture_output=True, text=True, timeout=BOUND, check=True).stdout
        self.assertIn('BUILD TOKENS (3/3 in use, 1 waiting)', status)
        self.assertEqual(status.count('cargo test -p crate'), 4, status)
        self.assertIn(f'waiting pid {fourth.proc.pid}', status)
        self.assertEqual(status.count(f'in {self.tree}'), 4, status)
        # A normal exit hands its token to the waiter.
        first = running.pop(0)
        self.finish(first)
        self.wait_run(fourth)
        self.assertEqual(fourth.token, first.token)
        running.append(fourth)
        # So does killing a holder (the runner pid: wrapper and scope go with it).
        fifth = self.start('cargo', 'build')
        self.settle([fifth])
        self.assertEqual(fifth.state, 'queued')
        victim = running.pop(0)
        victim.proc.send_signal(signal.SIGKILL)
        victim.proc.wait(timeout=BOUND)
        self.wait_run(fifth)
        self.assertEqual(fifth.token, victim.token)
        for cmd in running + [fifth]:
            self.finish(cmd)
        self.wait_tokens_free()

    def test_nested_compile_and_gate_run_while_all_tokens_are_held(self):
        running, _ = self.hold_three()
        # A gate on a pinned binary takes no token.
        gate = subprocess.run([str(self.runner), '-m', '1G', 'sh', '-c', 'echo "gate ${SOLARIS_BUILD_TOKEN:-none}"'],
                              cwd=self.tree, env=self.env, capture_output=True, text=True, timeout=BOUND)
        self.assertEqual(gate.returncode, 0, gate.stderr)
        self.assertEqual(gate.stdout.strip(), 'gate none')
        # A compile nested under a holder reuses the holder's token instead of waiting for a fourth.
        outer = running[1]
        outer.proc.stdin.write('nest\n')
        outer.proc.stdin.flush()
        readable, _, _ = select.select([outer.proc.stdout], [], [], BOUND)
        self.assertTrue(readable, 'nested compile blocked on a second token')
        nested = outer.proc.stdout.readline().split()
        self.assertEqual(nested[:3], ['nested', '0', 'run'], nested)
        self.assertEqual(nested[3], outer.token)
        self.assertEqual(self.held_tokens(), ['1', '2', '3'])
        for cmd in running:
            self.finish(cmd)


if __name__ == '__main__':
    unittest.main()
