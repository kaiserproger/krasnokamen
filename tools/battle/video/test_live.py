#!/usr/bin/env python3
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

import compose
import live
import overlay

TICK0, WALL0 = 1000, 1_700_000_000_000_000_000  # telemetry: tick 1000 + k at WALL0 + k * 50 ms
LAPTOP_AHEAD_NS = 1_500_000_000_000             # laptop clock 25 min ahead of the host


def telemetry_clock(count=400):
    return [WALL0 + k * 50_000_000 for k in range(count)], [TICK0 + k for k in range(count)]


def showinfo_log(first_wall_ns, count, step_ns=16_666_667):
    lines = ['ffmpeg version x']
    for n in range(count):
        wall = first_wall_ns + n * step_ns
        lines.append(f'[Parsed_showinfo_0 @ 0x7f] n:{n:4d} pts:{wall // 1000} pts_time:{wall // 1000 / 1e6:.6f} duration:  16667')
    return '\n'.join(lines) + '\n'


def anchors(offset=-LAPTOP_AHEAD_NS, markers=None):
    return {'clock': [{'laptop_unix_ns': WALL0 + LAPTOP_AHEAD_NS, 'host_minus_laptop_ns': offset, 'rtt_ns': 8_000_000},
                      {'laptop_unix_ns': WALL0 + LAPTOP_AHEAD_NS + 10**10, 'host_minus_laptop_ns': offset - 10**6, 'rtt_ns': 9_000_000}],
            'markers': markers}


class LiveMap(unittest.TestCase):
    def setUp(self):
        self.dir = pathlib.Path(tempfile.mkdtemp())
        self.first_wall = WALL0 + LAPTOP_AHEAD_NS + 1_010_000_000  # first captured frame, laptop clock: host tick 1020
        (self.dir / 'c.ffmpeg.log').write_text(showinfo_log(self.first_wall, 300))

    def write(self, data):
        (self.dir / 'c.anchors.json').write_text(json.dumps(data))
        return live.anchor(self.dir / 'c.anchors.json', self.dir / 'c.ffmpeg.log', 300, telemetry_clock())

    def test_parse_showinfo_is_in_order_and_exact(self):
        walls = live.parse_showinfo(self.dir / 'c.ffmpeg.log')
        self.assertEqual(len(walls), 300)
        self.assertEqual(walls[0], self.first_wall // 1000 * 1000)
        (self.dir / 'bad.log').write_text(showinfo_log(self.first_wall, 3).replace('n:   1', 'n:   5'))
        with self.assertRaises(ValueError):
            live.parse_showinfo(self.dir / 'bad.log')

    def test_offset_interpolates_between_measurements(self):
        clock = anchors()['clock']
        self.assertEqual(live.host_minus_laptop(clock, clock[0]['laptop_unix_ns']), -LAPTOP_AHEAD_NS)
        self.assertEqual(live.host_minus_laptop(clock, clock[0]['laptop_unix_ns'] + 5 * 10**9), -LAPTOP_AHEAD_NS - 500_000)
        self.assertEqual(live.host_minus_laptop(clock[:1], 12345), -LAPTOP_AHEAD_NS)
        # No extrapolation outside the measured interval.
        self.assertEqual(live.host_minus_laptop(clock, 0), -LAPTOP_AHEAD_NS)
        self.assertEqual(live.host_minus_laptop(clock, 10**19), -LAPTOP_AHEAD_NS - 10**6)

    def test_anchored_with_marker(self):
        # Frame 0 is host wall WALL0 + 1 s: clock tick 1020; the marker says the event (tick 1022) shows at frame 30.
        # Frame 30 is host wall +0.5 s later: clock tick 1030, so the clip is shifted by -8 ticks: too much.
        ticks, report = self.write(anchors(markers=[{'event': 'battle start', 'server_tick': 1030, 'captured_frame': 30}]))
        self.assertTrue(report['anchored'], report)
        self.assertEqual(report['marker_shift_ticks'], 0)
        self.assertEqual(len(ticks), 300)
        self.assertEqual(ticks[0], 1020)
        self.assertEqual(ticks[60], 1020 + 20)  # 60 frames = 1 s = 20 ticks
        # Marker 2 ticks later than the clock map says: all frames shift by +2.
        ticks, report = self.write(anchors(markers=[{'event': 'battle start', 'server_tick': 1032, 'captured_frame': 30}]))
        self.assertEqual((report['marker_shift_ticks'], ticks[0]), (2, 1022))

    def test_unanchored_without_marker_or_with_bad_marker(self):
        for markers, why in ((None, 'no marker'), ([], 'no marker'),
                             ([{'event': 'e', 'server_tick': 1100, 'captured_frame': 30}], 'clock offset wrong'),
                             ([{'event': 'e', 'server_tick': 1030, 'captured_frame': 30},
                               {'event': 'f', 'server_tick': 1113, 'captured_frame': 270}], 'drift'),
                             ([{'event': 'e', 'server_tick': 1030, 'captured_frame': 900}], 'outside the clip')):
            ticks, report = self.write(anchors(markers=markers))
            self.assertIsNone(ticks, markers)
            self.assertFalse(report['anchored'])
            self.assertIn(why, report['reason'])

    def test_uncovered_clip_is_unanchored_not_estimated(self):
        # Offset puts the clip 20 s before the first telemetry tick.
        ticks, report = self.write(anchors(offset=-LAPTOP_AHEAD_NS - 21 * 10**9,
                                           markers=[{'event': 'e', 'server_tick': 1030, 'captured_frame': 30}]))
        self.assertIsNone(ticks)
        self.assertIn('outside the telemetry', report['reason'])

    def test_unanchored_rows_render_a_label_panel(self):
        with tempfile.TemporaryDirectory() as d:
            d = pathlib.Path(d)
            t = d / 't.jsonl'
            t.write_text(''.join(json.dumps({'tick': k, 'wall_start_unix_ns': 10**18 + k * 50_000_000, 'wall_us': 1000, 'host': None}) + '\n' for k in range(3)))
            f = d / 'f.jsonl'
            f.write_text(json.dumps({'frame': 0, 'last_applied_tick': None, 'unanchored': True}) + '\n' + json.dumps({'frame': 1, 'last_applied_tick': 1}) + '\n')
            overlay.write_overlay(t, f, d / 'o')
            self.assertEqual((d / 'o' / 'overlay-000000.png').read_bytes(), overlay.render_unanchored())
            self.assertNotEqual((d / 'o' / 'overlay-000001.png').read_bytes(), overlay.render_unanchored())
            f2 = d / 'f2.jsonl'
            f2.write_text(json.dumps({'frame': 0, 'last_applied_tick': None}) + '\n')
            with self.assertRaises(ValueError):
                overlay.write_overlay(t, f2, d / 'o2')


class LiveInEdit(unittest.TestCase):
    def test_unanchored_ticks_flow_into_the_edit_map_and_plan(self):
        rows, total = compose.edit_frames([[5] * 10, [None] * 10], 0)
        self.assertEqual([r['unanchored'] for r in rows], [False] * 10 + [True] * 10)
        shots = [compose.Shot('a', 'c', 10, 14), compose.Shot('live1', 'live', 12, 13), compose.Shot('b', 'c', 20, 24)]
        ticks = [compose.predicted_ticks(shots[0], 0), [None] * 90, compose.predicted_ticks(shots[2], 0)]
        cam = ({'position': [0, 0, 0], 'look_target': [0, 0, 1]},) * 2 + (None,)
        kept, table = compose.plan_no_backtrack(shots, ticks, [(*cam[:2], None), None, (*cam[:2], None)], 60, 40, 120)
        self.assertEqual([i for i, _ in kept], [0, 1, 2])
        self.assertEqual(table[1]['status'], 'unanchored')
        self.assertGreater(table[2]['delta_ticks'], 0)  # live clip did not become the previous shot

    def test_live_filter_resamples_before_trim(self):
        g = compose.build_filter([120, 60], 0, '1080p', '/tmp/t.txt', None, trims=[30, 0], ends=[150, None])
        self.assertIn('[0:v]fps=60,trim=start_frame=30:end_frame=150,setpts=PTS-STARTPTS,scale', g)
        self.assertIn('[1:v]fps=60,scale', g)


if __name__ == '__main__':
    unittest.main()
