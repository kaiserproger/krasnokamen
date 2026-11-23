"""The maintained perf-window recipe: fixed counter set, frame-pointer call graph, AMD events.

This is a recipe contract, not a perf run: the box capture needs a live pinned server, so the
test checks the refusals and the exact flags/counters the next capture must reproduce
(PERF-WINDOW-HAS-NO-CALL-GRAPH-AND-NO-MAINTAINED-SCRIPT,
PERF-WINDOW-READS-AMD-L2-ALIAS-AS-LAST-LEVEL-MISS-RATE).

Every content assertion reads the script with its comments stripped: the header explains the
history (`cache-misses` alias to L2, the old `-F 999` recipe) and prose must never satisfy a
check about what the script executes.
"""
import os
import subprocess
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / 'perf-window.sh'
# Verified on real captures: receipts/G2/laptop-perf-20k/report-{fp,dwarf}.txt hold caller lines,
# the flat BattleBaseline reports do not.
CALLERS = r'^ +(\||-{2}[0-9])'


class PerfWindowRecipeTests(unittest.TestCase):
    def text(self):
        # A missing script is a failed assertion, not an unreadable file: the registered ref has to
        # be RED when the fix is reversed, and the reversal deletes the script.
        self.assertTrue(SCRIPT.exists(), 'tools/dev/perf-window.sh is the maintained capture recipe')
        return SCRIPT.read_text()

    def code(self):
        """The executed lines: comments (whole-line and trailing) cannot satisfy a content check."""
        lines = []
        for line in self.text().splitlines():
            stripped = line.lstrip()
            if stripped.startswith('#'):
                continue
            lines.append(line.split('# ', 1)[0] if ' # ' in line else line)
        return '\n'.join(lines)

    def run_script(self, *args):
        self.assertTrue(SCRIPT.exists(), 'tools/dev/perf-window.sh is the maintained capture recipe')
        return subprocess.run([str(SCRIPT), *args], capture_output=True, text=True, timeout=60)

    def test_the_script_is_executable_and_refuses_an_unusable_invocation(self):
        self.text()
        self.assertTrue(os.access(SCRIPT, os.X_OK))
        for args, message in (
            ((), 'usage:'),
            (('not-a-pid',), 'is not a pid'),
            (('999999999',), 'no process'),
        ):
            done = self.run_script(*args)
            self.assertEqual(done.returncode, 2, done.stderr)
            self.assertIn(message, done.stderr)
        done = self.run_script(str(os.getpid()), '0')
        self.assertEqual(done.returncode, 2)
        self.assertIn('seconds must be a positive integer', done.stderr)

    def test_the_counter_set_is_the_amd_last_level_events_not_the_l2_aliases(self):
        code = self.code()
        # The last-level number and the L2 context counters, by the names this host advertises.
        for event in ('ls_dmnd_fills_from_sys.all', 'l2_request_g1.all', 'l2_cache_req_stat.ls_rd_blk_c'):
            self.assertIn(event, code)
        # `cache-references`/`cache-misses` alias to L2 on AMD64 and must not return as
        # last-level evidence; `l2_misses.all` is not an event of this host's PMU.
        self.assertNotIn('cache-references', code)
        self.assertNotIn('cache-misses', code)
        self.assertNotIn('l2_misses', code)

    def test_the_capture_is_a_frame_pointer_call_graph_with_per_thread_attribution(self):
        code = self.code()
        self.assertIn('--call-graph fp', code)
        self.assertIn('-g ', code)
        self.assertIn('--per-thread', code)
        # Inclusive callers, not self time only.
        self.assertIn('graph,0.5,caller,0.5', code)
        # The top-down verdict and the enabled/running ratio column.
        self.assertIn('-M PipelineL1', code)
        self.assertIn('-x,', code)
        # Per-thread record buffers exhaust an 8 MiB memlock limit without a cap.
        self.assertIn('-m "$mmap_pages"', code)
        # The three perf windows run one after another, never nested.
        self.assertEqual(code.count('-- sleep "$seconds"'), 3)
        # A capture without caller frames is refused, not quoted; the pattern is the verified one.
        self.assertIn('no caller frames', code)
        self.assertIn(CALLERS, code)


if __name__ == '__main__':
    unittest.main()
