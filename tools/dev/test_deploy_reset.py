"""The fresh-reset helper's guards, checked without running it (the owner prohibits execution).

Every assertion here is about a line the script would execute on the VPS; the script itself is never
launched. TEST-SERVER-FRESH-RESET-WRONG-UNIT-AND-OWNER.
"""
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
        self.assertIn('systemctl is-active --quiet solaris', code)
        self.assertNotIn('is-active --quiet krasnokamen', code)
        # A stopped unit is not proof the process is gone.
        self.assertIn('pgrep -x krasnokamen', code)

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
