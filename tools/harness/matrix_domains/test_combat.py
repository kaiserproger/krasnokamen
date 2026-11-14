"""Adapter-contract tests for the melee/hostile probes; not server coverage."""
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from . import combat


class MobTargetsBaselineTests(unittest.TestCase):
    def run_probe(self, hit_before_receipt=True):
        """One zombie probe where the first hit lands during the summon."""
        events = []
        health_now = [20.0]
        results = []

        def summon(ctx, kind, **kwargs):
            events.append('summon')
            if hit_before_receipt:
                health_now[0] = 17.0  # first hit, then its spawn receipt arrives
            return 7

        def health(ctx):
            events.append('health')
            return health_now[0]

        def wait_until(predicate, **kwargs):
            results.append(predicate())
            return True

        ctx = SimpleNamespace(factors={'difficulty': 'normal', 'game_mode': 'survival'},
                              state={}, wait_until=wait_until,
                              clients=[SimpleNamespace(name='Proto0')])
        with patch.object(combat, 'summon', summon), patch.object(combat, 'health', health), \
                patch.object(combat, 'condition', lambda *args, **kwargs: events.append('condition')):
            combat.mob_targets(ctx)
        return events, results

    def test_baseline_is_taken_before_the_mob_can_hit(self):
        # Regression: with the baseline read after the summon, a zombie that
        # hit during the spawn receipt left `before` at the already-reduced
        # health, so the probe timed out on real damage (protocol-matrix
        # 20261114T174501, Health 20 -> 17 in the received journal).
        events, results = self.run_probe()
        self.assertEqual(events[:2], ['health', 'summon'])
        self.assertTrue(results[0], 'the first real hit must satisfy the attack wait')

    def test_a_probe_without_any_hit_still_times_out(self):
        events, results = self.run_probe(hit_before_receipt=False)
        self.assertEqual(events[:2], ['health', 'summon'])
        self.assertFalse(results[0], 'no damage must not pass the attack wait')


if __name__ == '__main__':
    unittest.main()
