"""Fail-closed parser/budget regressions; live workload receipts remain separate."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools.harness import server_load_profile as p

SCRIPT = '''mc-core-tick 1.000000000: 1000000 cpu-clock:
    1234 __rust_alloc (/server)
    2345 hashbrown::map::HashMap<K,V>::get (/server)
    3456 hashbrown::map::HashMap<K,V>::get (/server)

mc-core-tick 1.001000000: 1000000 cpu-clock:
    4567 core::str::cmp (/server)
    5678 tick (/server)

'''


class ProfileTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.out = Path(self.temp.name)
        self.rows = [{'tick': n, 'wall_start_unix_ns': n * 1_000_000, 'wall_us': 2000}
                     for n in (1, 2)]

    def capture(self, exits=None):
        (self.out / 'perf-1.data').touch()
        p.dump(self.out / 'perf-1.json', {'unix_minus_monotonic_ns': 0})
        p.dump(self.out / 'perf-1.exit.json', exits or {'server': 0, 'perf': 0})

    def test_period_to_ms_per_tick_and_recursive_inclusive_not_double_counted(self):
        self.capture()
        with patch.object(p.subprocess, 'check_output', return_value=SCRIPT):
            report = p.summarize(self.out, self.rows, 50)
        self.assertEqual(report['cpu_ms_per_tick'], 1)
        self.assertEqual(report['self'][0]['ms_per_tick'], .5)
        self.assertEqual(report['inclusive'][1]['ms_per_tick'], .5)
        self.assertEqual(report['flagged_inclusive_ms_per_tick'],
                         {'string_hash_compare': .5, 'allocation': .5, 'map_lookup': .5})
        self.assertEqual(report['status'], 'pass')

    def test_slow_window_takes_middle_snapshot_before_600_ticks(self):
        self.assertTrue(p.midpoint_reached(60, 1, 30_000_000_001))
        self.assertFalse(p.midpoint_reached(60, 1, 29_000_000_001))
        self.assertFalse(p.midpoint_reached(0, None, 0))

    def test_generic_type_names_are_not_executed_operations(self):
        function = 'rayon_core::registry::run<std::collections::HashMap<alloc::string::String, alloc::vec::Vec<u8>>>::get'
        operation = p.classification_name(function)
        self.assertEqual(operation, 'rayon_core::registry::run::get')
        self.assertFalse(any(pattern.search(operation) for pattern in p.FLAGS.values()))
        self.assertTrue(p.FLAGS['map_lookup'].search(p.classification_name('<hashbrown::map::HashMap<K,V>>::get')))

    def test_budget_negative_control_same_samples_fail_tighter_budget(self):
        self.capture()
        with patch.object(p.subprocess, 'check_output', return_value=SCRIPT):
            self.assertEqual(p.summarize(self.out, self.rows, 1)['status'], 'budget_exceeded')

    def test_missing_perf_is_not_a_pass(self):
        with self.assertRaisesRegex(ValueError, 'no perf samples'):
            p.summarize(self.out, self.rows, 50)

    def test_no_call_stacks_is_not_a_pass(self):
        self.capture()
        with patch.object(p.subprocess, 'check_output', return_value='mc-core-tick 1.0: 1000 cpu-clock:\n  1234 tick (/s)\n\n'):
            with self.assertRaisesRegex(ValueError, 'call stacks'):
                p.summarize(self.out, self.rows, 50)

    def test_failed_capture_is_not_a_pass(self):
        self.capture({'server': 0, 'perf': 255})
        with self.assertRaisesRegex(ValueError, 'failed capture'):
            p.summarize(self.out, self.rows, 50)

    def test_tick_gap_is_not_a_pass(self):
        self.rows[1]['tick'] = 3
        with self.assertRaisesRegex(ValueError, 'discontinuous'):
            p.summarize(self.out, self.rows, 50)

    def test_clock_regression_is_not_a_pass(self):
        self.rows[1]['wall_start_unix_ns'] = 1
        with self.assertRaisesRegex(ValueError, 'clock regressed'):
            p.summarize(self.out, self.rows, 50)

    def test_expected_sigint_after_clean_server_is_a_complete_capture(self):
        self.capture({'server': 0, 'perf': -2})
        with patch.object(p.subprocess, 'check_output', return_value=SCRIPT):
            self.assertEqual(p.summarize(self.out, self.rows, 50)['status'], 'pass')

    def test_preparation_and_smoke_are_not_measurements(self):
        work = self.out / 'workload'
        work.mkdir()
        for status in ('prepared', 'smoke'):
            p.dump(work / 'result.json', {'status': status})
            with self.assertRaisesRegex(ValueError, 'not measured'):
                p.source_windows(self.out, 'coreperf')

    def test_failed_workload_keeps_diagnostic_functions_without_passing(self):
        self.capture()
        work = self.out / 'workload'
        run = work / 'run'
        run.mkdir(parents=True)
        p.dump(work / 'result.json', {'status': 'failed'})
        p.dump(run / 'result.json', {'start_unix_ns': 1_000_000, 'end_unix_ns': 4_000_000})
        (run / 'samples.jsonl').write_text(json.dumps({'metrics': {'samples': self.rows}}) + '\n')
        p.dump(self.out / 'command-exit.json', {'exit': 1, 'mid_snapshot': True})
        argv = ['profile', '--analyze', '--out', str(self.out), '--kind', 'coreperf']
        with patch.object(p.sys, 'argv', argv), \
                patch.object(p.subprocess, 'check_output', return_value=SCRIPT), \
                patch('builtins.print'):
            self.assertEqual(p.main(), 1)
        result = json.loads((self.out / 'profile-analysis.json').read_text())
        self.assertEqual(result['status'], 'failed')
        self.assertTrue(result['diagnostic_only'])
        self.assertTrue(result['windows']['measurement']['self'])


if __name__ == '__main__':
    unittest.main()
