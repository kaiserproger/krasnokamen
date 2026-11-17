import json
import tempfile
import unittest
from pathlib import Path

from tools.harness.battle_exposure import (D1_MAX_INTERVAL_TICKS, D1_STEP_BLOCKS, analyse_take,
                                           episode_wire_steps, fling_dismounts, main, parse_log, pooled,
                                           wire_positions)


def episode_line(ident=1076, tick=5142, ticks=7, plan='chase', travel=1.985, peak=0.412, outcome='targeted',
                 start=(-12.345, 64.0, 88.1), end=(-14.1, 64.0, 87.2), motion=None):
    motion = motion or f'{tick + 1}-{tick + ticks}'
    return (f'2026-11-17T09:37:41.664429Z  INFO mc_core::battle::exposure: Battle exposure: id={ident} '
            f'tick={tick} ticks={ticks} motion={motion} plan={plan} from=[{start[0]},{start[1]},{start[2]}] '
            f'to=[{end[0]},{end[1]},{end[2]}] travel={travel} peak={peak} outcome={outcome}')


def summary_line(start=5142, episodes=1, bodies=1, member_ticks=7, excess_episodes=0, suppressed=0,
                 capped='false'):
    return (f'2026-11-17T09:41:02.100001Z  INFO mc_core::battle::exposure: Battle exposure summary: '
            f'start={start} episodes={episodes} bodies={bodies} member_ticks={member_ticks} '
            f'excess_episodes={excess_episodes} suppressed={suppressed} capped={capped}')


def take_directory(root, log, name='take'):
    run = Path(root, name)
    run.mkdir(parents=True, exist_ok=True)
    (run / 'server.log').write_text(log)
    return run


def positions_file(root, document):
    path = Path(root, 'positions.json')
    path.write_text(json.dumps(document))
    return path


def fling_file(root, take, dismounts, base=5000, over=None, steps=None):
    path = Path(root, f'fling-{take}.json')
    path.write_text(json.dumps({take: {'battle_start_tick': base, 'dismounts': dismounts,
                                       'steps': steps or {}, 'dismounted_over_2': over or []}}))
    return path


class ParseTest(unittest.TestCase):
    def test_episode_and_summary_are_parsed_with_every_field(self):
        episodes, summaries = parse_log('\n'.join(['unrelated line', episode_line(), summary_line()]))
        self.assertEqual(len(episodes), 1)
        row = episodes[0]
        self.assertEqual((row['id'], row['tick'], row['ticks'], row['plan']), (1076, 5142, 7, 'chase'))
        self.assertEqual(row['motion'], [5143, 5149])
        self.assertEqual(row['from'], [-12.345, 64.0, 88.1])
        self.assertEqual(row['to'], [-14.1, 64.0, 87.2])
        self.assertEqual((row['travel'], row['peak'], row['outcome']), (1.985, 0.412, 'targeted'))
        self.assertEqual(summaries, [{'start': 5142, 'episodes': 1, 'bodies': 1, 'member_ticks': 7,
                                      'excess_episodes': 0, 'suppressed': 0, 'capped': False}])

    def test_a_reordered_or_truncated_line_is_named_not_ignored(self):
        reordered = episode_line().replace('ticks=7 motion=5143-5149', 'motion=5143-5149 ticks=7')
        with self.assertRaisesRegex(ValueError, r'server\.log:1: expected field ticks='):
            parse_log(reordered)
        with self.assertRaisesRegex(ValueError, 'fields, expected 10'):
            parse_log(episode_line().rsplit(' outcome=', 1)[0])

    def test_a_motion_window_that_breaks_the_phase_contract_is_refused(self):
        with self.assertRaisesRegex(ValueError, 'is not tick\\+1\\.\\.tick\\+ticks'):
            parse_log(episode_line(tick=5142, ticks=7, motion='5142-5148'))
        with self.assertRaisesRegex(ValueError, 'is not tick\\+1\\.\\.tick\\+ticks'):
            parse_log(episode_line(tick=5142, ticks=7, motion='5143-5150'))

    def test_unknown_plan_outcome_and_short_episode_are_refused(self):
        with self.assertRaisesRegex(ValueError, 'unknown plan'):
            parse_log(episode_line(plan='charge'))
        with self.assertRaisesRegex(ValueError, 'unknown outcome'):
            parse_log(episode_line(outcome='fine'))
        with self.assertRaisesRegex(ValueError, 'at least one tick'):
            parse_log(episode_line(ticks=0, motion='5143-5142'))

    def test_a_summary_with_a_bad_count_is_refused(self):
        with self.assertRaisesRegex(ValueError, 'capped must be true or false'):
            parse_log(summary_line(capped='yes'))


class WireJoinTest(unittest.TestCase):
    """The one interval semantics of the campaign join."""

    def steps(self, rows):
        document = {'battle_start_tick': 5000, 'ids': {'1076': [
            [tick, round(x * 4096), 0, round(z * 4096)] for tick, x, z in rows]}}
        with tempfile.TemporaryDirectory() as root:
            return wire_positions(positions_file(root, document))[1076]

    def test_a_fling_inside_the_window_is_a_d1_step(self):
        steps = self.steps([(5140, 0.0, 0.0), (5144, 3.0, 0.0)])
        found = episode_wire_steps({'motion': [5143, 5149]}, steps)
        self.assertEqual(len(found), 1)
        self.assertTrue(found[0]['d1_step'])
        self.assertAlmostEqual(found[0]['blocks'], 3.0, places=3)
        self.assertEqual(found[0]['interval'], 4)

    def test_a_step_that_covers_a_short_window_entirely_is_not_lost(self):
        # one packet interval swallows a 1-tick exposure: the short window must still count
        steps = self.steps([(5140, 0.0, 0.0), (5146, 4.0, 0.0)])
        found = episode_wire_steps({'motion': [5143, 5143]}, steps)
        self.assertEqual(len(found), 1)
        self.assertTrue(found[0]['covers_window'])
        self.assertTrue(found[0]['spans_start'])
        self.assertTrue(found[0]['spans_end'])
        self.assertTrue(found[0]['d1_step'])

    def test_a_moving_body_within_the_bound_is_not_a_d1_step(self):
        steps = self.steps([(5143, 0.0, 0.0), (5145, 1.0, 0.0), (5147, 2.0, 0.0)])
        found = episode_wire_steps({'motion': [5143, 5149]}, steps)
        self.assertEqual(len(found), 2)
        self.assertFalse(any(step['d1_step'] for step in found))

    def test_a_slow_fling_over_too_many_ticks_is_not_a_d1_step(self):
        steps = self.steps([(5143, 0.0, 0.0), (5143 + D1_MAX_INTERVAL_TICKS + 1, 3.0, 0.0)])
        found = episode_wire_steps({'motion': [5143, 5155]}, steps)
        self.assertEqual([step['d1_step'] for step in found], [False])

    def test_a_step_outside_the_window_does_not_count(self):
        # every packet is before the window, so no interval reaches it
        steps = self.steps([(5100, 0.0, 0.0), (5102, 5.0, 0.0), (5104, 5.5, 0.0)])
        self.assertEqual(episode_wire_steps({'motion': [5143, 5149]}, steps), [])

    def test_the_bound_constants_are_the_runbook_ones(self):
        self.assertEqual((D1_STEP_BLOCKS, D1_MAX_INTERVAL_TICKS), (2.0, 6))


class TakeTest(unittest.TestCase):
    def complete_take(self, root, log, positions=None, dismounts=None, over=None):
        run = take_directory(root, log)
        path = positions_file(root, positions) if positions is not None else None
        if dismounts is None:
            return analyse_take('take', run, path)
        source = fling_file(root, 'take', dismounts, over=over)
        return analyse_take('take', run, path, fling_dismounts(source)['take'], json.loads(source.read_text())['take'])

    def test_a_take_reports_server_numbers_and_marks_its_burst(self):
        log = '\n'.join([
            'Battle started tick=5000 ground: bodies=2000 inside=0 floating=0 wet=0',
            episode_line(),
            episode_line(ident=1077, tick=5300, ticks=3, plan='flank', outcome='reseated'),
            summary_line(start=5142, episodes=2, bodies=2, member_ticks=10),
        ])
        take = None
        with tempfile.TemporaryDirectory() as root:
            take = analyse_take('take', take_directory(root, log))
            self.assertTrue(take['records'])
            self.assertEqual((take['episodes'], take['bodies_at_risk'], take['exposed_member_ticks']), (2, 2, 10))
            self.assertEqual(take['plans'], {'chase': 1, 'flank': 1})
            self.assertEqual(take['outcomes'], {'targeted': 1, 'reseated': 1})
            self.assertTrue(take['complete'])
            self.assertTrue(take['usable_for_d1'])
            self.assertEqual(take['max_lines_per_close_tick'], 1)
            self.assertNotIn('wire', take)

    def test_a_take_without_records_is_not_zero_exposure(self):
        with tempfile.TemporaryDirectory() as root:
            take = analyse_take('take', take_directory(root, 'Battle started tick=5000\n'))
            self.assertFalse(take['records'])
            self.assertFalse(take['complete'])
            self.assertIn('no summary line', take['incomplete'])

    def test_a_summary_without_its_episode_lines_is_incomplete_and_unusable(self):
        with tempfile.TemporaryDirectory() as root:
            take = analyse_take('take', take_directory(
                root, '\n'.join([episode_line(), summary_line(episodes=12, bodies=4, member_ticks=90)])))
            self.assertEqual(take['episodes'], 1)
            self.assertEqual(take['reported_episodes'], 12)
            self.assertFalse(take['complete'])
            self.assertFalse(take['usable_for_d1'])
            self.assertTrue(any('summary episodes' in reason for reason in take['incomplete']))

    def test_two_summaries_mean_the_run_restarted(self):
        with tempfile.TemporaryDirectory() as root:
            take = analyse_take('take', take_directory(root, '\n'.join([
                episode_line(tick=5142), summary_line(start=5142, episodes=1),
                episode_line(ident=2000, tick=6000), summary_line(start=6000, episodes=1),
                summary_line(start=5142, episodes=1),
            ])))
            self.assertFalse(take['complete'])
            self.assertTrue(any('summaries' in reason for reason in take['incomplete']))

    def test_the_cap_makes_a_take_unusable_for_a_bound(self):
        with tempfile.TemporaryDirectory() as root:
            take = analyse_take('take', take_directory(
                root, '\n'.join([episode_line(), summary_line(episodes=1, suppressed=44, capped='true')])))
            self.assertTrue(take['capped'])
            self.assertEqual(take['suppressed'], 44)
            self.assertFalse(take['usable_for_d1'])
            self.assertTrue(any('suppressed' in reason for reason in take['incomplete']))

    def test_an_uncovered_wire_tail_is_incomplete(self):
        positions = {'battle_start_tick': 5000, 'ids': {'1076': [[5140, 0, 0, 0], [5144, round(1 * 4096), 0, 0]]}}
        with tempfile.TemporaryDirectory() as root:
            take = self.complete_take(root, '\n'.join([episode_line(), summary_line()]), positions)
            self.assertFalse(take['complete'])
            self.assertTrue(any('uncovered wire tail' in reason for reason in take['incomplete']))

    def test_the_join_reports_a_flinging_at_risk_body(self):
        positions = {'battle_start_tick': 5000, 'ids': {
            '1076': [[5140, 0, 0, 0], [5144, round(4 * 4096), 0, 0], [5200, round(4.5 * 4096), 0, 0]],
            '1077': [[5140, 0, 0, 0], [5144, round(9 * 4096), 0, 0]],
            '9999': [[5143, 0, 0, 0], [5200, round(2 * 4096), 0, 0]],
        }}
        with tempfile.TemporaryDirectory() as root:
            take = self.complete_take(root, '\n'.join([episode_line(), summary_line()]), positions,
                                      dismounts=[[142, '1076', 'horse_died'], [900, '1077', 'left_view']])
            self.assertEqual(take['wire']['d1_steps'], 1)
            self.assertEqual(take['wire']['bodies_with_d1_step'], 1)
            self.assertEqual(take['wire']['episodes_without_wire_step'], 0)
            self.assertEqual(take['wire_dismounts']['exposed_of_dismounted'], 1)
            self.assertEqual(take['wire_dismounts']['exposure_without_wire_dismount'], [])

    def test_a_left_view_is_not_a_dismount(self):
        with tempfile.TemporaryDirectory() as root:
            table = fling_dismounts(fling_file(root, 'take', [[142, '1076', 'horse_died'],
                                                              [900, '1077', 'left_view']]))
            self.assertEqual(table['take'], {1076: 5142})

    def test_a_phantom_exposure_without_a_wire_dismount_is_named(self):
        with tempfile.TemporaryDirectory() as root:
            take = self.complete_take(root, '\n'.join([episode_line(), summary_line()]),
                                      dismounts=[[142, '9999', 'other']])
            self.assertEqual(take['wire_dismounts']['exposure_without_wire_dismount'], [1076])
            self.assertEqual(take['wire_dismounts']['exposed_of_dismounted'], 0)
            self.assertFalse(take['usable_for_d1'])

    def test_an_episode_starting_before_its_dismount_means_a_phase_mismatch(self):
        with tempfile.TemporaryDirectory() as root:
            take = self.complete_take(root, '\n'.join([episode_line(tick=5142), summary_line(start=5142)]),
                                      dismounts=[[150, '1076', 'horse_died']])
            self.assertEqual(take['wire_dismounts']['episodes_starting_before_dismount'], 1)
            self.assertFalse(take['usable_for_d1'])

    def test_the_wire_d1_count_comes_from_all_dismounted_steps_not_the_filter(self):
        with tempfile.TemporaryDirectory() as root:
            take = self.complete_take(root, '\n'.join([episode_line(), summary_line()]),
                                      dismounts=[[142, '1076', 'horse_died']],
                                      over=[[0, 1, 2.5, False], [20, 2, 3.1, True]])
            self.assertEqual(take['d1']['dismounted_over_2'], 2)

    def test_a_missing_battle_start_tick_is_refused(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root, 'fling.json')
            path.write_text(json.dumps({'take': {'dismounts': []}}))
            with self.assertRaisesRegex(ValueError, 'battle_start_tick'):
                fling_dismounts(path)

    def test_pooling_sums_bodies_steps_and_usability(self):
        takes = [
            {'name': 'yaw-1', 'records': True, 'bodies_at_risk': 3, 'exposed_member_ticks': 10,
             'server_excess_episodes': 0, 'capped': False, 'complete': True,
             'wire': {'d1_steps': 0, 'bodies_with_d1_step': 0}},
            {'name': 'yaw-2', 'records': True, 'bodies_at_risk': 4, 'exposed_member_ticks': 20,
             'server_excess_episodes': 1, 'capped': False, 'complete': True,
             'wire': {'d1_steps': 2, 'bodies_with_d1_step': 1}},
        ]
        pool = pooled(takes)
        self.assertEqual((pool['bodies_at_risk'], pool['d1_steps'], pool['server_excess_episodes']), (7, 2, 1))
        self.assertTrue(pool['complete'])


class MainTest(unittest.TestCase):
    def test_no_records_exits_two_and_never_prints_a_pass(self):
        with tempfile.TemporaryDirectory() as root:
            run = take_directory(root, 'Battle started tick=5000\n')
            out = Path(root, 'report.json')
            code = main(['--take', f'yaw-1={run}', '--out', str(out)])
            self.assertEqual(code, 2)
            self.assertEqual(json.loads(out.read_text())['pooled']['bodies_at_risk'], 0)

    def test_a_reported_take_exits_zero_and_writes_the_numbers(self):
        with tempfile.TemporaryDirectory() as root:
            run = take_directory(root, '\n'.join([episode_line(), summary_line()]))
            out = Path(root, 'report.json')
            self.assertEqual(main(['--take', f'yaw-1={run}', '--out', str(out)]), 0)
            report = json.loads(out.read_text())
            self.assertEqual(report['takes'][0]['bodies_at_risk'], 1)
            self.assertEqual(report['pooled']['bodies_at_risk'], 1)

    def test_inputs_for_an_unknown_take_are_refused(self):
        with tempfile.TemporaryDirectory() as root:
            run = take_directory(root, episode_line() + '\n')
            with self.assertRaises(SystemExit):
                main(['--take', f'yaw-1={run}', '--positions', f'yaw-9={Path(root)}'])
            with self.assertRaises(SystemExit):
                main(['--take', f'yaw-1={run}', '--fling-json', f'yaw-9={Path(root)}'])

    def test_malformed_take_arguments_are_refused(self):
        with self.assertRaises(SystemExit):
            main(['--take', 'nopath'])


if __name__ == '__main__':
    unittest.main()
