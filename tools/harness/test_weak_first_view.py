"""The first-view benchmark cannot silently escape its requested envelope."""
from pathlib import Path
import unittest
from unittest.mock import patch

from tools.harness.profiles import ProfileContext
from tools.harness.backends import weak_first_view


class WeakFirstViewTests(unittest.TestCase):
    def context(self, **overrides):
        values = dict(artifact_dir=Path('/unused'), client='native', windowed=True,
                      extra_args=['--server-cpus', '0', '--client-cpus', '1-3', '--memory', '2G'])
        values.update(overrides)
        return ProfileContext(**values)

    def test_server_and_observer_sets_stay_inside_the_runner_affinity(self):
        with patch.object(weak_first_view.os, 'sched_getaffinity', return_value={0, 1, 2, 3}):
            args = weak_first_view.validate_args(self.context())
            self.assertEqual(args.server_cpus, '0')
            self.assertEqual(args.client_cpus, '1-3')
            self.assertEqual(args.memory, '2G')
            with self.assertRaisesRegex(ValueError, 'outside the measurement envelope'):
                weak_first_view.validate_args(self.context(extra_args=[
                    '--server-cpus', '4', '--client-cpus', '1-3', '--memory', '2G']))

    def test_headless_or_java_does_not_substitute_for_a_usable_view(self):
        for override in ({'windowed': False}, {'client': 'java'}, {'platform': 'fabric'}):
            with self.subTest(override=override), self.assertRaisesRegex(ValueError, 'requires'):
                weak_first_view.validate_args(self.context(**override))

    def test_default_envelope_uses_one_allowed_cpu_and_real_ram_cap(self):
        with patch.object(weak_first_view.os, 'sched_getaffinity', return_value={8, 9, 10, 11}):
            args = weak_first_view.validate_args(self.context(extra_args=[]))
            self.assertEqual(args.server_cpus, '8')
            observer = set(weak_first_view.common.parse_cpu_list(args.client_cpus))
            self.assertTrue(observer and observer <= {9, 10, 11})
            self.assertEqual(args.memory, '2G')

    def test_malformed_envelope_fails_as_a_receiptable_error(self):
        with self.assertRaisesRegex(ValueError, 'invalid envelope arguments'):
            weak_first_view.validate_args(self.context(extra_args=['--server-cpus']))


if __name__ == '__main__':
    unittest.main()
