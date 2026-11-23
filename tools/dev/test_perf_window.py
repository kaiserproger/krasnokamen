"""The maintained perf-window recipe: fixed counter set, frame-pointer call graph, AMD events.

This is a recipe contract, not a perf run: the box capture needs a live pinned server, so the
test checks the refusals and the exact flags/counters the next capture must reproduce
(PERF-WINDOW-HAS-NO-CALL-GRAPH-AND-NO-MAINTAINED-SCRIPT,
PERF-WINDOW-READS-AMD-L2-ALIAS-AS-LAST-LEVEL-MISS-RATE).
"""
import os
import subprocess
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / 'perf-window.sh'


class PerfWindowRecipeTests(unittest.TestCase):
    def text(self):
        # A missing script is a failed assertion, not an unreadable file: the registered ref has to
        # be RED when the fix is reversed, and the reversal deletes the script.
        self.assertTrue(SCRIPT.exists(), 'tools/dev/perf-window.sh is the maintained capture recipe')
        return SCRIPT.read_text()

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
            (('999999999',), 'no process'),
        ):
            done = self.run_script(*args)
            self.assertEqual(done.returncode, 2, done.stderr)
            self.assertIn(message, done.stderr)
        done = self.run_script(str(os.getpid()), '0')
        self.assertEqual(done.returncode, 2)
        self.assertIn('seconds must be a positive integer', done.stderr)

    def test_the_counter_set_is_the_amd_last_level_events_not_the_l2_aliases(self):
        text = self.text()
        # Prose may name the aliases to warn about them; only the executed lines must not use them.
        code = '\n'.join(line for line in text.splitlines() if not line.lstrip().startswith('#'))
        for event in ('ls_dmnd_fills_from_sys.all', 'l2_request_g1.all', 'l2_misses.all'):
            self.assertIn(event, code)
        # `cache-references`/`cache-misses` alias to L2 on AMD64 and must not return as
        # last-level evidence.
        self.assertNotIn('cache-references', code)
        self.assertNotIn('cache-misses', code)

    def test_the_capture_is_a_frame_pointer_call_graph_with_per_thread_attribution(self):
        text = self.text()
        self.assertIn('--call-graph fp', text)
        self.assertIn('-g ', text)
        self.assertIn('--per-thread', text)
        # Inclusive callers, not self time only.
        self.assertIn('graph,0.5,caller,0.5', text)
        # The top-down verdict and the enabled/running ratio column.
        self.assertIn('-M PipelineL1', text)
        self.assertIn('-x,', text)
        # A capture without caller frames is refused, not quoted.
        self.assertIn('no caller frames', text)


if __name__ == '__main__':
    unittest.main()
