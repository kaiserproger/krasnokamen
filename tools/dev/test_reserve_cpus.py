"""Reservation identity: a dead owner must not keep pinning later mem-run scopes (#G4, CS341).

Real scopes, but every run here is `true` under a 1G cap, so the test only exercises the
reservation file protocol and the two scripts that read and write it.
"""
import importlib.util
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
# Under mem-run the harness environment carries MIMALLOC_* switches whose banner lands on the
# child's stderr; the tests read the script's own first line, so the children get a clean env.
CLEAN_ENV = {key: value for key, value in os.environ.items() if not key.startswith('MIMALLOC_')}


def protocol():
    spec = importlib.util.spec_from_file_location('reserved_cpus_under_test', HERE / 'reserved_cpus.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def starttime(pid: int) -> str:
    """Field 22 of /proc/<pid>/stat: what tells a live owner from a reused pid."""
    return Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[19]


def dead_pid() -> int:
    done = subprocess.Popen(['python3', '-c', 'pass'])
    done.wait(timeout=30)
    return done.pid


class ReserveCpusTests(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.state = self.root / 'state'
        source = (HERE / 'reserve-cpus').read_text()
        self.script = self.root / 'reserve-cpus'
        self.script.write_text(source.replace('dir=/tmp/solaris-mem-slots', f'dir={self.state}', 1))
        self.script.chmod(0o755)
        shutil.copy(HERE / 'reserved_cpus.py', self.root)

    def reserve(self, *args, **kwargs):
        return subprocess.run([str(self.script), *args], capture_output=True, text=True, env=CLEAN_ENV, **kwargs)

    def holder(self):
        """A command that outlives the reservation-file check until the test closes its stdin."""
        return subprocess.Popen([str(self.script), '6,7', '--', 'python3', '-c', 'import sys; sys.stdin.read()'],
                                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                                env=CLEAN_ENV)

    def test_a_reservation_records_its_live_owner(self):
        proc = self.holder()
        try:
            # The script announces the reservation after writing the file: wait for that line, not a sleep.
            self.assertIn('reserved 6,7 for', proc.stderr.readline())
            cpus, owner = (self.state / 'reserved-cpus').read_text().splitlines()[:2]
            self.assertEqual(cpus, '6,7')
            pid, start = owner.split()
            self.assertEqual(int(pid), proc.pid)
            self.assertEqual(starttime(int(pid)), start)
        finally:
            proc.stdin.close()
            proc.wait(timeout=30)
        self.assertFalse((self.state / 'reserved-cpus').exists(), 'release removes the file')

    def test_a_dead_owner_is_cleared_instead_of_blocking_the_next_reservation(self):
        self.state.mkdir()
        corpse = dead_pid()
        (self.state / 'reserved-cpus').write_text(f'6,7\n{corpse} {corpse}\n')
        done = self.reserve('8', '--', 'true')
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn('clearing stale reservation 6,7', done.stderr)
        self.assertFalse((self.state / 'reserved-cpus').exists(), 'released after the command')

    def test_the_locked_create_refuses_a_second_reservation_and_release_removes_only_its_own(self):
        # ReviewD123Harness2 item 6: the flock-guarded create and the own-file release were only
        # exercised through the protocol module, never through the script.
        first = self.holder()
        try:
            self.assertIn('reserved 6,7 for', first.stderr.readline())
            second = self.reserve('8', '--', 'true')
            self.assertEqual(second.returncode, 75, second.stderr)
            self.assertIn('CPUs already reserved', second.stderr)
            # A foreign file under the same lock: the first reservation's release must not touch it.
            (self.state / 'reserved-cpus').write_text('8\n' + (self.state / 'reserved-cpus').read_text().split(chr(10))[1] + chr(10))
        finally:
            first.stdin.close()
            first.wait(timeout=30)
        self.assertTrue((self.state / 'reserved-cpus').exists(),
                        'a reservation that replaced ours is not ours to remove')

    def test_a_one_line_file_from_an_older_writer_is_honoured(self):
        self.state.mkdir()
        (self.state / 'reserved-cpus').write_text('6,7\n')
        done = self.reserve('8', '--', 'true')
        self.assertEqual(done.returncode, 75, done.stderr)
        self.assertIn('CPUs already reserved: 6,7', done.stderr)
        self.assertEqual((self.state / 'reserved-cpus').read_text(), '6,7\n')


class ReservedCpusProtocolTests(unittest.TestCase):
    """The shared file protocol: judge an owner, and never unlink bytes that changed meanwhile."""

    def setUp(self) -> None:
        self.protocol = protocol()
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.state = Path(tmp.name)

    def test_a_corpse_is_stale_and_a_live_owner_is_kept(self):
        dead = dead_pid()
        self.assertEqual(self.protocol.verdict(f'6,7\n{dead} 1\n'), 'stale 6,7')
        live = f'6,7\n{os.getpid()} {starttime(os.getpid())}\n'
        self.assertEqual(self.protocol.verdict(live), 'keep 6,7')
        # An older one-line writer cannot be judged: honour it, exactly as mem-run always has.
        self.assertEqual(self.protocol.verdict('6,7\n'), 'keep 6,7')
        self.assertEqual(self.protocol.verdict('not a cpu list\n'), 'none')
        self.assertEqual(self.protocol.verdict(''), 'none')

    def test_a_present_unparseable_owner_is_stale_not_immortal(self):
        # ReviewD123Harness2 item 3: only a MISSING owner line means an older writer. A line that
        # claims an owner but cannot be checked must not keep the CPUs pinned forever.
        for line in ('garbage', '12', '12 34 56', 'x 1'):
            with self.subTest(owner=line):
                self.assertEqual(self.protocol.verdict(f'6,7\n{line}\n'), 'stale 6,7')
        self.assertEqual(self.protocol.verdict('6,7\n\n'), 'keep 6,7', 'a blank owner line is an older writer')

    def test_a_reservation_rewritten_after_the_verdict_is_not_unlinked(self):
        # TOCTOU: judge the corpse, then a live reservation replaces the file before the unlink.
        corpse = f'6,7\n{dead_pid()} 1\n'
        (self.state / 'reserved-cpus').write_text(corpse)
        self.assertEqual(self.protocol.verdict(corpse), 'stale 6,7')
        live = f'8\n{os.getpid()} {starttime(os.getpid())}\n'
        (self.state / 'reserved-cpus').write_text(live)
        self.assertFalse(self.protocol.unlink_if_unchanged(self.state, corpse),
                         'the bytes that were judged were not the bytes on disk')
        self.assertEqual((self.state / 'reserved-cpus').read_text(), live, 'the live reservation survives')

    def test_clear_removes_a_corpse_and_keeps_a_live_reservation(self):
        (self.state / 'reserved-cpus').write_text(f'6,7\n{dead_pid()} 1\n')
        self.assertIn('clearing stale reservation 6,7', self.protocol.clear(self.state))
        self.assertFalse((self.state / 'reserved-cpus').exists())
        live = f'6,7\n{os.getpid()} {starttime(os.getpid())}\n'
        (self.state / 'reserved-cpus').write_text(live)
        self.assertEqual(self.protocol.clear(self.state), '')
        self.assertEqual((self.state / 'reserved-cpus').read_text(), live)


class MemRunReservationTests(unittest.TestCase):
    """The scope's side: stale reservations are logged and cleared, live ones are logged."""

    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = self.root = Path(tmp.name)
        slice_name = f'memtest{os.getpid()}.slice'
        subprocess.run(['systemctl', '--user', 'start', slice_name], check=True)
        self.addCleanup(subprocess.run, ['systemctl', '--user', 'stop', slice_name], check=False)
        watchdog = f'memtest{os.getpid()}-watchdog'
        self.addCleanup(subprocess.run, ['systemctl', '--user', 'stop', watchdog + '.service'], check=False)
        source = (HERE / 'mem-run.sh').read_text()
        source = source.replace('dir=/tmp/solaris-mem-slots', f'dir={root}/state', 1)
        source = source.replace('slice=solaris-heavy.slice', f'slice={slice_name}', 1)
        source = source.replace('solaris-heavy-watchdog', watchdog)
        self.runner = root / 'mem-run.sh'
        self.runner.write_text(source)
        self.runner.chmod(0o755)
        for name in ('slot-select.sh', 'heavy-watchdog.py', 'release-build-policy.sh',
                     'measurement-window.py', 'measurement-windows', 'reserved_cpus.py'):
            shutil.copy(HERE / name, root)
        shutil.copy(HERE / 'mem-run-guard.py', root)
        (root / 'targets' / 'slot-a').mkdir(parents=True)
        self.env = os.environ.copy()
        self.env.update(CARGO_TARGET_DIR=str(root / 'targets' / 'slot-a'),
                        KRASNOKAMEN_SLOT_TARGETS=str(root / 'targets'),
                        KRASNOKAMEN_DISK_GC_BELOW_GB='0', KRASNOKAMEN_DISK_REFUSE_BELOW_GB='0')
        self.state = root / 'state'

    def run_true(self):
        return subprocess.run([str(self.runner), '-m', '1G', 'true'], env=self.env,
                              capture_output=True, text=True, timeout=120)

    def test_a_stale_reservation_is_named_cleared_and_does_not_pin_the_scope(self):
        self.state.mkdir()
        corpse = dead_pid()
        (self.state / 'reserved-cpus').write_text(f'0\n{corpse} 1\n')
        done = self.run_true()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn('ignoring stale CPU reservation', done.stderr)
        self.assertNotIn('reserved CPUs', done.stderr.split('ignoring stale CPU reservation')[0])
        self.assertFalse((self.state / 'reserved-cpus').exists(), 'the corpse is removed')

    def test_a_live_reservation_logs_the_cpus_it_excludes(self):
        self.state.mkdir()
        (self.state / 'reserved-cpus').write_text(f'0\n{os.getpid()} {starttime(os.getpid())}\n')
        done = self.run_true()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn('reserved CPUs 0 are excluded from this scope', done.stderr)

    def test_a_reservation_covering_the_whole_pool_is_named_not_silently_dropped(self):
        self.state.mkdir()
        ncpu = os.cpu_count() or 1
        (self.state / 'reserved-cpus').write_text(f'0-{ncpu - 1}\n{os.getpid()} {starttime(os.getpid())}\n')
        done = self.run_true()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn('cover this whole pool', done.stderr)
        self.assertIn('keeps cpus=', done.stderr)

    def test_a_failing_verdict_helper_is_named_and_the_first_line_honoured(self):
        # ReviewD123Harness2 3(b) / ReviewD123Harness1 6: a crashed helper used to read as "none" and
        # a real reservation was silently ignored. It must fail closed: name the helper, show its
        # stderr, and honour the file's first line (the CPU list) as a reservation.
        (self.root / 'reserved_cpus.py').write_text(
            "import sys\nprint('no module named reserved_cpus', file=sys.stderr)\nsys.exit(3)\n")
        self.state.mkdir()
        (self.state / 'reserved-cpus').write_text(f'0\n{os.getpid()} {starttime(os.getpid())}\n')
        done = self.run_true()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn('cannot judge', done.stderr)
        self.assertIn("honouring its first line '0' as a CPU list", done.stderr)
        self.assertIn('no module named reserved_cpus', done.stderr)
        self.assertIn('reserved CPUs 0 are excluded', done.stderr)
        self.assertTrue((self.state / 'reserved-cpus').exists(), 'the reservation was left in place')

    def test_a_garbage_first_line_with_a_failing_helper_refuses_the_scope(self):
        # Sonnet G: fail-closed must not honour a first line that is not a CPU list.
        (self.root / 'reserved_cpus.py').write_text(
            "import sys\nprint('no module named reserved_cpus', file=sys.stderr)\nsys.exit(3)\n")
        self.state.mkdir()
        (self.state / 'reserved-cpus').write_text('not a cpu list\n1 2\n')
        done = self.run_true()
        self.assertEqual(done.returncode, 75, done.stderr)
        self.assertIn('first line is not a CPU list', done.stderr)

    def test_a_non_numeric_cancel_grace_is_named_before_the_scope_starts(self):
        # ReviewD123Harness2 item 4: `nan` passes the guardian's float() and breaks its select().
        self.state.mkdir()
        for value in ('nan', 'inf', '-1', '0', 'soon'):
            with self.subTest(grace=value):
                env = {**self.env, 'KRASNOKAMEN_MEMRUN_CANCEL_GRACE': value}
                done = subprocess.run([str(self.runner), '-m', '1G', 'true'], env=env,
                                      capture_output=True, text=True, timeout=120)
                self.assertEqual(done.returncode, 75, done.stderr)
                self.assertIn('KRASNOKAMEN_MEMRUN_CANCEL_GRACE must be a finite positive number', done.stderr)
                self.assertNotIn('failed to start scope guardian', done.stderr)


if __name__ == '__main__':
    unittest.main()
