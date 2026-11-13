#!/usr/bin/env python3
import json
import math
import pathlib
import subprocess
import sys
import tempfile
import unittest

import compose

SHOTS = '''# comment
a cameras/a.json 10.00 11.00 20
b cameras/b.json 10.50 11.50

c cameras/c.json 20.00 21.00
'''


def write(directory, text):
    path = pathlib.Path(directory) / 'shots.txt'
    path.write_text(text)
    return path


class ShotsAndTicks(unittest.TestCase):
    def test_parse_and_predicted_ticks(self):
        with tempfile.TemporaryDirectory() as d:
            shots = compose.parse_shots(write(d, SHOTS))
        self.assertEqual([s.name for s in shots], ['a', 'b', 'c'])
        ticks = compose.predicted_ticks(shots[0], 2279)
        self.assertEqual(len(ticks), 60)
        # 20 ticks/s at 60 fps: three frames per tick, start tick = first_tick + 10.00 s * 20.
        self.assertEqual(ticks[:4], [2479, 2479, 2479, 2480])
        self.assertEqual(ticks[-1], 2479 + 19)
        self.assertEqual(compose.predicted_ticks(shots[1], 2279)[0], 2279 + 210)

    def test_bad_shots_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(ValueError):
                compose.parse_shots(write(d, 'a cam 5 4\n'))
            with self.assertRaises(ValueError):
                compose.parse_shots(write(d, 'a cam 1 2\na cam 3 4\n'))
            with self.assertRaises(ValueError):
                compose.parse_shots(write(d, '# only a comment\n'))

    def test_real_shots_file(self):
        path = pathlib.Path('/run/media/kaiserroman/games/solaris-work/receipts/G1-shots/plan-v2/shots.txt')
        if not path.exists():
            self.skipTest('receipt not present')
        shots = compose.parse_shots(path)
        self.assertEqual(len(shots), 15)
        self.assertAlmostEqual(sum(s.end - s.start for s in shots), 65.6, places=6)


class EditFrames(unittest.TestCase):
    def test_hard_cut_is_concatenation(self):
        ticks = [[1, 1, 1, 2], [10, 10, 11], [20, 20, 20, 21, 21]]
        rows, total = compose.edit_frames(ticks, 0)
        self.assertEqual(total, 12)
        self.assertEqual([r['frame'] for r in rows], list(range(12)))
        self.assertEqual([r['last_applied_tick'] for r in rows], [1, 1, 1, 2, 10, 10, 11, 20, 20, 20, 21, 21])
        self.assertEqual([r['shot'] for r in rows], [0] * 4 + [1] * 3 + [2] * 5)

    def test_crossfade_picks_one_shot_per_frame(self):
        a = [100 + i for i in range(10)]
        b = [200 + i for i in range(10)]
        c = [300 + i for i in range(10)]
        rows, total = compose.edit_frames([a, b, c], 4)
        self.assertEqual(total, 30 - 2 * 4)
        shots = [r['shot'] for r in rows]
        # Fade a->b spans output frames 6..9: outgoing for the first 2, incoming for the last 2.
        self.assertEqual(shots[:6], [0] * 6)
        self.assertEqual(shots[6:10], [0, 0, 1, 1])
        self.assertEqual(shots[10:12], [1, 1])
        self.assertEqual(shots[12:16], [1, 1, 2, 2])
        self.assertEqual(shots[16:], [2] * 6)
        for r in rows:
            source = (a, b, c)[r['shot']]
            self.assertEqual(r['last_applied_tick'], source[r['shot_frame']])
        # The incoming shot's first frame is shown at its own frame 2 (after the outgoing half).
        self.assertEqual(rows[8]['shot_frame'], 2)

    def test_crossfade_too_long_rejected(self):
        with self.assertRaises(ValueError):
            compose.edit_frames([[1] * 8, [2] * 8], 4)


class FilterGraph(unittest.TestCase):
    def test_xfade_chain_for_n_shots(self):
        g = compose.build_filter([240, 300, 180, 240], 15, '1080p', '/tmp/title.txt', music=None)
        self.assertEqual(g.count('xfade='), 3)
        # Offset k = (sum of earlier lengths - k*crossfade) / 60.
        for offset in ((240 - 15) / 60, (240 + 300 - 30) / 60, (240 + 300 + 180 - 45) / 60):
            self.assertIn(f'offset={offset:.6f}', g)
        self.assertIn('[s0][s1]xfade', g)
        self.assertIn('[x1][s2]xfade', g)
        self.assertIn('[x2][s3]xfade', g)
        self.assertIn('[4:v]format=rgba', g)  # overlay is input N
        self.assertNotIn('[5:a]', g)
        self.assertIn(f'fade=t=out:st={(960 - 45 - 60) / 60:.6f}', g)

    def test_hard_cut_uses_concat(self):
        g = compose.build_filter([60, 60, 60], 0, '4k', '/tmp/t.txt', music='m.wav')
        self.assertIn('[s0][s1][s2]concat=n=3', g)
        self.assertNotIn('xfade', g)
        self.assertIn('[4:a]loudnorm', g)
        self.assertIn('scale=iw*2:ih*2', g)
        self.assertIn('scale=3840:2160', g)

    def test_command_keeps_encode_settings(self):
        cmd = compose.encode_command(['a.mp4', 'b.mp4'], 'o-%06d.png', None, [120, 120], 10, '1080p', 't.txt', 'out.mp4')
        joined = ' '.join(cmd)
        for want in ('-crf 16', '-profile:v high', '-bf 2', '-g 30', '-flags +cgop', '-colorspace bt709', '+faststart'):
            self.assertIn(want, joined)
        self.assertNotIn('-c:a', joined)
        self.assertEqual(cmd[cmd.index('-t') + 1], f'{(230 - 0.5) / 60:.6f}')


class NoBacktrack(unittest.TestCase):
    @staticmethod
    def shots(*spans):
        return [compose.Shot(f's{i}', f'c{i}', a, b) for i, (a, b) in enumerate(spans)]

    @staticmethod
    def cam(yaw_target, attach=None):
        # position at origin, looking along (sin, cos) of the requested yaw
        look = [math.sin(math.radians(yaw_target)), -0.5, math.cos(math.radians(yaw_target))]
        key = {'position': [0, 0, 0], 'look_target': look}
        return key, key, attach

    def ticks(self, shots):
        return [compose.predicted_ticks(s, 0) for s in shots]

    def test_trims_head_to_previous_end_and_orders_by_start(self):
        shots = self.shots((10, 14), (8, 12), (13, 17))  # file order s0 s1 s2; by START: s1 s0 s2
        kept, table = compose.plan_no_backtrack(shots, self.ticks(shots), [self.cam(0)] * 3, 90, 120)
        self.assertEqual([shots[i].name for i, _ in kept], ['s1', 's0', 's2'])
        # s1 runs 8..12 s; s0 (10..14) is trimmed by 2 s = 120 frames, 2 s remain: kept (>= 120 frames).
        self.assertEqual(kept[1], (0, 120))
        # s2 starts at 13 s, before s0's end (14 s): trimmed to 14 s, 3 s remain.
        self.assertEqual(kept[2], (2, 60))
        self.assertEqual([r['delta_ticks'] for r in table], [None, 0, 0])
        self.assertEqual([r['delta_ticks_original'] for r in table[1:]], [-40, -20])

    def test_no_backward_step_after_planning(self):
        shots = self.shots((10, 14), (11, 15), (12, 16), (30, 34))
        ticks = self.ticks(shots)
        kept, _ = compose.plan_no_backtrack(shots, ticks, [self.cam(0)] * 4, 90, 120)
        previous_last = None
        for i, trim in kept:
            visible = ticks[i][trim:]
            if previous_last is not None:
                self.assertGreaterEqual(visible[0], previous_last + 1)
            previous_last = visible[-1]

    def test_short_remainder_is_dropped(self):
        shots = self.shots((10, 14), (13, 15), (20, 24))  # s1 would keep 1 s
        kept, table = compose.plan_no_backtrack(shots, self.ticks(shots), [self.cam(0)] * 3, 90, 120)
        self.assertEqual([i for i, _ in kept], [0, 2])
        self.assertEqual(table[1]['status'], 'dropped')
        self.assertIsNone(table[1]['in_tick'])

    def test_sharp_angle_keeps_backtrack(self):
        shots = self.shots((10, 14), (11, 15), (12, 16))
        # s1 turns 120 degrees from s0: exception, kept whole. s2 attached to an entity: class change, exception.
        cams = [self.cam(0), self.cam(120), self.cam(130, attach=7)]
        kept, table = compose.plan_no_backtrack(shots, self.ticks(shots), cams, 90, 120)
        self.assertEqual(kept, [(0, 0), (1, 0), (2, 0)])
        self.assertEqual([r['exception'] for r in table], [False, True, True])
        self.assertEqual(table[1]['delta_ticks'], -60)
        # A turn below the threshold is trimmed instead.
        kept, _ = compose.plan_no_backtrack(shots[:2], self.ticks(shots[:2]), [self.cam(0), self.cam(80)], 90, 60)
        self.assertEqual(kept[1], (1, 180))

    def test_trim_in_filter_graph(self):
        g = compose.build_filter([240, 120], 12, '1080p', '/tmp/t.txt', None, trims=[0, 60])
        self.assertIn('[1:v]trim=start_frame=60,setpts=PTS-STARTPTS,fps=60', g)
        self.assertIn('[0:v]fps=60', g)


class MapCommand(unittest.TestCase):
    def test_map_from_recorded_frames_and_wrong_take_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            d = pathlib.Path(d)
            shots = write(d, 'a cam 1.0 1.1\nb cam 2.0 2.1\n')
            frames = d / 'frames'
            frames.mkdir()
            for name, first in (('a', 2299), ('b', 2319)):
                (frames / f'{name}.frames.jsonl').write_text(
                    ''.join(json.dumps({'frame': i, 'last_applied_tick': first + i // 3}) + '\n' for i in range(6)))
            script = str(pathlib.Path(compose.__file__))
            run = lambda *extra: subprocess.run([sys.executable, script, 'map', '--shots-file', str(shots), '--first-tick', '2279',
                                                 '--frames-dir', str(frames), '--out', str(d / 'edit.jsonl'), *extra],
                                                capture_output=True, text=True)
            r = run()
            self.assertEqual(r.returncode, 0, r.stderr)
            rows = [json.loads(x) for x in (d / 'edit.jsonl').read_text().splitlines()]
            self.assertEqual([x['last_applied_tick'] for x in rows], [2299, 2299, 2299, 2300, 2300, 2300, 2319, 2319, 2319, 2320, 2320, 2320])
            (d / 'edit.jsonl').unlink()
            r = subprocess.run([sys.executable, script, 'map', '--shots-file', str(shots), '--first-tick', '3000',
                                '--frames-dir', str(frames), '--out', str(d / 'edit.jsonl')], capture_output=True, text=True)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn('wrong take', r.stderr)


if __name__ == '__main__':
    unittest.main()
