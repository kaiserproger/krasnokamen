"""The fresh-reset helper's guards, checked without running it (the owner prohibits execution).

Every assertion here is about a line the script would execute on the VPS; the script itself is never
launched. TEST-SERVER-FRESH-RESET-WRONG-UNIT-AND-OWNER.
"""
import re
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / 'test-server' / 'deploy-reset.sh'


class FreshResetGuardTests(unittest.TestCase):
    def text(self):
        self.assertTrue(SCRIPT.is_file(), 'tools/dev/test-server/deploy-reset.sh')
        return SCRIPT.read_text()

    def code(self):
        """Executed lines only: a comment may name an anti-pattern to warn about it."""
        return '\n'.join(line for line in self.text().splitlines()
                         if not line.lstrip().startswith('#'))

    def test_it_stops_and_checks_the_solaris_unit_not_krasnokamen(self):
        code = self.code()
        self.assertIn('systemctl stop solaris', code)
        # The unit name comes from the tracked unit file, not from memory (N2).
        unit = (SCRIPT.parent / 'solaris.service').read_text()
        name = re.search(r'^Description=Krasnokamen Minecraft server \(test\)', unit, re.M)
        self.assertIsNotNone(name, 'the tracked unit is the one the helper restarts')
        # `is-active --quiet solaris` would also match solaris-metrics: require the exact unit name.
        self.assertRegex(code, r'is-active --quiet solaris(?=[\s;]|$)')
        self.assertNotIn('is-active --quiet krasnokamen', code)
        # A stopped unit is not proof the process is gone.
        self.assertIn('pgrep -x krasnokamen', code)

    def test_every_exit_guard_precedes_the_destructive_steps(self):
        # Order is the point of the row: a guard after `rm -rf world` protects nothing (N2).
        lines = self.code().splitlines()
        guards = [i for i, line in enumerate(lines) if 'exit 1' in line]
        destructive = [i for i, line in enumerate(lines)
                       if re.match(r'(rm -rf world|mv krasnokamen)', line.strip())]
        self.assertTrue(guards, 'the helper must refuse when a check fails')
        self.assertTrue(destructive, 'the helper replaces the world and the binary')
        first = min(destructive)
        before = [i for i in guards if i < first]
        self.assertGreaterEqual(
            len(before), 4,
            'stop check, survivor check, backup collision, and the archive listing/world entry guard all'
            ' run before rm -rf world/mv')

    def test_it_reads_the_whole_archive_listing_before_judging_it(self):
        code = self.code()
        self.assertIn('listing=$(tar tzf', code)
        self.assertIn('grep -q \'^world/\' <<<"$listing"', code)
        # `tar tzf ... | grep -q` lets grep exit early: tar dies of SIGPIPE under pipefail and the
        # full-stream corruption check is lost.
        for line in code.splitlines():
            if 'tar tzf' in line:
                # `||` is the error branch; a pipe into another reader is the bug.
                self.assertNotIn('tar tzf "$backup" |', line, line)

    def test_it_refuses_to_overwrite_a_backup(self):
        code = self.code()
        self.assertIn('if [ -e "$backup" ]; then', code)
        # Minute-resolution names collide when two resets run inside one minute.
        self.assertIn('%Y%m%d-%H%M%S', code)
        self.assertIn('$$', code)

    def test_it_owns_the_binary_as_the_unit_user(self):
        self.assertIn('chown "${KRASNOKAMEN_SERVER_USER:-krasnokamen}:"', self.code())


if __name__ == '__main__':
    unittest.main()
