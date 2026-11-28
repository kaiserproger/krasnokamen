"""A construction-only proof must never hide unexplained ALL-item changes.

Also fixes the R6b budget and cadence defects: the running-tick window follows
the audited checkpoint day phase, and only the work shift pays frozen censuses.
"""
import unittest

from . import retained_storehouse as gate
from .retained_storehouse import construction_acceptance

# R6b baseline: checkpoint tick 383732, climate day_time 276733 (phase 12733),
# next adult work shift at 290000, certified storehouse cursor 64 of 142.
CHECKPOINT_DAY_TIME = 276_733
NEXT_SHIFT = 290_000


def require(name):
    function = getattr(gate, name, None)
    if function is None:
        raise AssertionError(f'retained gate has no {name}: the day-phase window and cadence are missing')
    return function


def schedule(day_time, total, progressed=False):
    """Replay the gate's cadence over one run window."""
    next_batch = require('next_batch')
    rows, elapsed = [], 0
    while elapsed < total:
        size, full = next_batch(day_time, total - elapsed, progressed)
        if size <= 0:
            raise AssertionError(f'cadence returned a non-advancing batch at day_time {day_time}')
        elapsed += size
        day_time += size
        rows.append({'batch': size, 'full': full, 'day_time': day_time, 'phase': day_time % gate.DAY_TICKS,
                     'work': gate.in_work_shift(day_time)})
    return rows


class RetainedWindowTests(unittest.TestCase):
    def test_window_reaches_the_next_work_shift_from_the_checkpoint_phase(self):
        window = require('run_window')(CHECKPOINT_DAY_TIME)
        self.assertEqual(window['checkpoint_day_phase'], 12_733)
        self.assertEqual(window['next_shift_start'], NEXT_SHIFT)
        self.assertEqual(window['gap_ticks'], NEXT_SHIFT - CHECKPOINT_DAY_TIME)
        self.assertEqual(window['remaining_placements'], 78)
        self.assertEqual(window['work_ticks'], 3_806)  # ceil(78*1488/61) * 2 margin
        self.assertEqual(window['window_ticks'], 17_073)

    def test_window_contains_the_whole_placement_work_window(self):
        window = require('run_window')(CHECKPOINT_DAY_TIME)
        end = CHECKPOINT_DAY_TIME + window['window_ticks']
        self.assertTrue(gate.in_work_shift(end))
        self.assertGreaterEqual(end - NEXT_SHIFT, window['work_ticks'])

    def test_work_window_crosses_rest_gaps_without_spending_the_placement_budget(self):
        window = require('run_window')(CHECKPOINT_DAY_TIME, 400, cap_ticks=100_000)
        end = CHECKPOINT_DAY_TIME + window['window_ticks']
        worked = sum(1 for tick in range(window['next_shift_start'], end) if gate.in_work_shift(tick))
        self.assertEqual(worked, window['work_ticks'])
        self.assertGreater(window['work_ticks'], gate.WORK_SHIFT_END - gate.WORK_SHIFT_START)

    def test_explicit_cap_rejects_a_window_that_does_not_fit(self):
        with self.assertRaises(ValueError):
            require('run_window')(CHECKPOINT_DAY_TIME, cap_ticks=6_000)  # the R6b ceiling
        with self.assertRaises(ValueError):
            require('run_window')(CHECKPOINT_DAY_TIME, cap_ticks=0)


class RetainedCadenceTests(unittest.TestCase):
    def test_off_shift_runs_large_batches_without_a_full_census(self):
        next_batch = require('next_batch')
        self.assertEqual(next_batch(CHECKPOINT_DAY_TIME, 17_073, False), (2_000, False))
        self.assertEqual(next_batch(CHECKPOINT_DAY_TIME + 2_000, 11_267, False), (2_000, False))

    def test_off_shift_batch_is_clamped_to_the_shift_boundary_with_a_full_census(self):
        next_batch = require('next_batch')
        self.assertEqual(next_batch(NEXT_SHIFT - 1_267, 15_000, False), (1_267, True))
        self.assertEqual(next_batch(NEXT_SHIFT - 2_000, 15_000, False), (2_000, True))

    def test_work_shift_keeps_the_fine_cadence_and_full_censuses(self):
        next_batch = require('next_batch')
        self.assertEqual(next_batch(NEXT_SHIFT, 3_806, False), (20, True))
        self.assertEqual(next_batch(NEXT_SHIFT + 20, 3_786, True), (200, True))

    def test_run_never_spends_a_batch_beyond_the_window(self):
        window = require('run_window')(CHECKPOINT_DAY_TIME)
        rows = schedule(CHECKPOINT_DAY_TIME, window['window_ticks'])
        self.assertEqual(sum(row['batch'] for row in rows), window['window_ticks'])

    def test_only_the_boundary_and_work_shift_pay_a_full_census(self):
        window = require('run_window')(CHECKPOINT_DAY_TIME)
        rows = schedule(CHECKPOINT_DAY_TIME, window['window_ticks'])
        off_shift = [row for row in rows if not row['work']]
        self.assertEqual(len(off_shift), 6)  # 6 x 2000 before the clamped boundary batch
        self.assertTrue(all(row['batch'] == 2_000 for row in off_shift))
        self.assertTrue(all(not row['full'] for row in off_shift))
        self.assertTrue(all(row['full'] for row in rows if row['work']))
        self.assertEqual(rows[6]['day_time'], NEXT_SHIFT)
        self.assertEqual(rows[6]['batch'], 1_267)
        self.assertTrue(rows[6]['full'])


class RetainedAcceptanceTests(unittest.TestCase):
    def test_missing_construction_item_rejects_gate(self):
        balance = {'status': 'FAIL', 'per_item': {'minecraft:cobblestone': {'unexplained': -1}},
                   'unexplained_delta': {'minecraft:cobblestone': -1}}
        construction = {'touched_item_kinds': ['minecraft:cobblestone'], 'construction_kinds_on_ground': {}}
        result = construction_acceptance(balance, construction)
        self.assertEqual(result['status'], 'FAIL')
        self.assertEqual(result['unexplained_delta'], {'minecraft:cobblestone': -1})

    def test_unsupported_nonconstruction_remains_visible_and_all_item_failed(self):
        balance = {'status': 'FAIL', 'per_item': {'minecraft:egg': {'unexplained': 2}},
                   'unexplained_delta': {'minecraft:egg': 2}}
        construction = {'touched_item_kinds': ['minecraft:cobblestone'], 'construction_kinds_on_ground': {}}
        result = construction_acceptance(balance, construction)
        self.assertEqual(result['status'], 'PASS')
        self.assertEqual(result['scope'], 'retained_stall_construction_only')
        self.assertEqual(result['all_item_status'], 'FAIL')
        self.assertEqual(result['all_item_unexplained_delta'], {'minecraft:egg': 2})
        self.assertEqual(balance['status'], 'FAIL')

    def test_transient_construction_ground_rejects_gate(self):
        balance = {'status': 'FAIL', 'per_item': {}, 'unexplained_delta': {}}
        construction = {'touched_item_kinds': ['minecraft:dirt'],
                        'construction_kinds_on_ground': {'minecraft:dirt': {'published_journal_witness': 12}}}
        self.assertEqual(construction_acceptance(balance, construction)['status'], 'FAIL')


if __name__ == '__main__':
    unittest.main()
