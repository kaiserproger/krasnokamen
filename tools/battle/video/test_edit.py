#!/usr/bin/env python3
import hashlib
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

import compose
import edit
import overlay

HERE = pathlib.Path(__file__).resolve().parent
FFMPEG = shutil.which('ffmpeg') and shutil.which('ffprobe')


def host(tick):
    return {'sampled_unix_ns': 1_000_000_000 + tick * 50_000_000, 'cgroup_memory_current_bytes': 3 * 2**30,
            'process_rss_bytes': 2**30 + tick * 1000, 'cgroup_cpu_usage_usec': 1000 * tick,
            'cpu_total_ticks': 100 * tick, 'cpu_steal_ticks': 0}


def write_telemetry(path, ticks):
    lines = []
    for t in ticks:
        lines.append(json.dumps({'tick': t, 'wall_start_unix_ns': 1_000_000_000 + t * 50_000_000, 'wall_us': 30000 + t,
                                 'total_us': 25000, 'phase_us': {}, 'host': host(t) if t % 20 == 0 else None,
                                 'battle': {'units': 20000, 'alive': {'1': 10000 - t, '2': 10000 - t // 2}}}))
    path.write_text('\n'.join(lines) + '\n')


def write_frames(path, ticks):
    path.write_text(''.join(json.dumps({'frame': i, 'last_applied_tick': t}) + '\n' for i, t in enumerate(ticks)))


def fake_probe(frames_by_video):
    return lambda video: (frames_by_video[pathlib.Path(video).name], '60/1', 640, 360)


class Fixture:
    def __init__(self, root, lengths=(120, 120), tick_base=(0, 1000), missing_tick=None):
        self.root = root
        self.counts = {}
        for name, n, base in zip('abcdefgh', lengths, tick_base):
            write_frames(root / f'{name}.frames.jsonl', [base + f // 3 for f in range(n)])
            (root / f'{name}.mp4').write_bytes(b'')
            self.counts[f'{name}.mp4'] = n
            ticks = [t for t in range(base, base + n // 3 + 1) if t != missing_tick]
            write_telemetry(root / f'{name}.telemetry.jsonl', ticks)
        (root / 'music.wav').write_bytes(b'')
        (root / 'title.txt').write_text('TITLE\n')

    def shot(self, name, a, b, transition=None, telemetry=True):
        s = {'id': name, 'video': f'{name}.mp4', 'frames': f'{name}.frames.jsonl', 'in': a, 'out': b}
        if telemetry:
            s['telemetry'] = f'{name}.telemetry.jsonl'
        if transition:
            s['transition'] = transition
        return s

    def spec(self, shots, **extra):
        return dict({'resolution': '1080p', 'music': 'music.wav', 'shots': shots}, **extra)

    def plan(self, spec):
        return edit.build_plan(spec, self.root, fake_probe(self.counts))


class PlanTests(unittest.TestCase):
    def setUp(self):
        self._d = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self._d.name)
        self.addCleanup(self._d.cleanup)

    def test_three_shots_frame_arithmetic_and_tick_mapping(self):
        fx = Fixture(self.root, lengths=(120, 120, 120), tick_base=(0, 1000, 2000))
        spec = fx.spec([fx.shot('a', 10, 70, {'type': 'fade', 'frames': 20}),
                        fx.shot('b', 0, 50, {'type': 'cut'}),
                        fx.shot('c', 30, 90, telemetry=False)])
        plan, rows, _ = fx.plan(spec)
        # 60 + 50 + 60 - 20 (one fade, one cut)
        self.assertEqual(plan['frames'], 150)
        self.assertEqual([r['frame'] for r in rows], list(range(150)))
        self.assertEqual(rows[0]['shot'], 'a')
        self.assertEqual(rows[0]['source_frame'], 10)
        self.assertEqual(rows[0]['last_applied_tick'], 3)  # frames[10] = 10 // 3
        # shot b begins at output 40; its first fade frames (j < 10) still belong to a, then b owns them
        self.assertEqual(rows[40]['shot'], 'a')
        self.assertEqual(rows[40]['blend'], {'shot': 'b', 'source_frame': 0, 'last_applied_tick': 1000, 'progress': '0/20'})
        self.assertEqual(rows[49]['shot'], 'a')
        self.assertEqual(rows[50]['shot'], 'b')
        self.assertEqual(rows[50]['blend']['shot'], 'a')
        self.assertEqual(rows[50]['source_frame'], 10)
        self.assertEqual(rows[50]['last_applied_tick'], 1000 + 10 // 3)
        self.assertNotIn('blend', rows[60])
        # b has 50 frames from output 40 to 90, then a hard cut to c
        self.assertEqual(rows[89]['shot'], 'b')
        self.assertEqual(rows[90]['shot'], 'c')
        self.assertEqual(rows[90]['source_frame'], 30)
        self.assertFalse(rows[90]['overlay'])
        self.assertTrue(rows[89]['overlay'])
        self.assertEqual(rows[149]['source_frame'], 89)

    def test_consecutive_transitions_inside_one_shot(self):
        fx = Fixture(self.root, lengths=(60, 60, 60), tick_base=(0, 1000, 2000))
        spec = fx.spec([fx.shot('a', 0, 30, {'type': 'fade', 'frames': 10}),
                        fx.shot('b', 0, 20, {'type': 'fadeblack', 'frames': 10}),
                        fx.shot('c', 0, 30)])
        plan, rows, _ = fx.plan(spec)
        self.assertEqual(plan['frames'], 30 + 20 + 30 - 20)
        self.assertEqual({r['shot'] for r in rows[20:30]}, {'a', 'b'})
        self.assertEqual({r['shot'] for r in rows[30:40]}, {'b', 'c'})

    def test_transitions_that_do_not_fit_are_rejected(self):
        fx = Fixture(self.root, lengths=(60, 60, 60), tick_base=(0, 1000, 2000))
        with self.assertRaisesRegex(edit.EditError, 'do not fit'):
            fx.plan(fx.spec([fx.shot('a', 0, 30, {'type': 'fade', 'frames': 10}),
                             fx.shot('b', 0, 19, {'type': 'fade', 'frames': 10}), fx.shot('c', 0, 30)]))
        with self.assertRaisesRegex(edit.EditError, 'last shot'):
            fx.plan(fx.spec([fx.shot('a', 0, 30), fx.shot('b', 0, 30, {'type': 'fade', 'frames': 5})]))

    def test_join_is_tick_exact_and_fail_closed(self):
        fx = Fixture(self.root, missing_tick=5)  # frame 15..17 of shot a map to tick 5
        with self.assertRaisesRegex(edit.EditError, 'frame 15 .*tick 5 missing from telemetry'):
            fx.plan(fx.spec([fx.shot('a', 0, 60)]))
        # outside the trimmed window the gap is harmless; without overlay it is irrelevant
        fx.plan(fx.spec([fx.shot('a', 20, 60)]))
        fx.plan(fx.spec([fx.shot('a', 0, 60, telemetry=False)]))

    def test_crossfade_tick_comes_from_the_dominant_shot(self):
        fx = Fixture(self.root, lengths=(90, 90), tick_base=(0, 1000))
        spec = fx.spec([fx.shot('a', 0, 60, {'type': 'fade', 'frames': 12}), fx.shot('b', 0, 60)])
        # Fade is output frames 48..59; b owns the second half (frames 54.., source frame j = 6 -> tick 1002).
        # b's telemetry starts at 1002, so the six outgoing-dominant frames still join through shot a.
        write_telemetry(self.root / 'b.telemetry.jsonl', range(1002, 1031))
        plan, rows, _ = fx.plan(spec)
        self.assertEqual({r['shot'] for r in rows[48:54]}, {'a'})
        self.assertEqual({r['shot'] for r in rows[54:60]}, {'b'})
        self.assertEqual(rows[54]['last_applied_tick'], 1002)
        # ... and it fails once the dominant shot lacks its tick.
        write_telemetry(self.root / 'b.telemetry.jsonl', range(1003, 1031))
        with self.assertRaisesRegex(edit.EditError, 'shot b .*tick 1002 missing'):
            fx.plan(spec)

    def test_input_validation(self):
        fx = Fixture(self.root)
        cases = [({'resolution': '720p'}, 'resolution'),
                 ({'bogus': 1}, 'unknown keys'),
                 ({'shots': []}, 'at least one shot')]
        for patch, message in cases:
            spec = dict(fx.spec([fx.shot('a', 0, 60)]), **patch)
            with self.assertRaisesRegex(edit.EditError, message):
                fx.plan(spec)
        with self.assertRaisesRegex(edit.EditError, 'beyond its 120 frames'):
            fx.plan(fx.spec([fx.shot('a', 0, 121)]))
        with self.assertRaisesRegex(edit.EditError, 'unique'):
            fx.plan(fx.spec([fx.shot('a', 0, 10), fx.shot('a', 0, 10)]))
        with self.assertRaisesRegex(edit.EditError, 'has 120 frames|video has'):
            edit.build_plan(fx.spec([fx.shot('a', 0, 60)]), self.root, fake_probe({'a.mp4': 119}))
        with self.assertRaisesRegex(edit.EditError, 'no implicit resampling'):
            edit.build_plan(fx.spec([fx.shot('a', 0, 60)]), self.root, lambda v: (120, '30/1', 640, 360))
        with self.assertRaisesRegex(edit.EditError, 'text 0'):
            fx.plan(fx.spec([fx.shot('a', 0, 60)], texts=[{'file': 'title.txt', 'start': 0, 'end': 5}]))
        bad = self.root / 'bad.frames.jsonl'
        bad.write_text(json.dumps({'frame': 1, 'last_applied_tick': 0}) + '\n')
        with self.assertRaisesRegex(edit.EditError, 'expected frame 0'):
            edit.read_shot_ticks(bad)
        bad.write_text(json.dumps({'frame': 0, 'last_applied_tick': 5}) + '\n' + json.dumps({'frame': 1, 'last_applied_tick': 4}) + '\n')
        with self.assertRaisesRegex(edit.EditError, 'goes back'):
            edit.read_shot_ticks(bad)

    def test_filtergraph_offsets(self):
        fx = Fixture(self.root, lengths=(120, 120, 120), tick_base=(0, 1000, 2000))
        plan, _, _ = fx.plan(fx.spec([fx.shot('a', 10, 70, {'type': 'fade', 'frames': 20}), fx.shot('b', 0, 50, {'type': 'cut'}),
                                      fx.shot('c', 30, 90)], texts=[{'file': 'title.txt', 'start': 0, 'end': 2}]))
        graph = compose.filter_graph(plan, 4, 3)
        self.assertIn('[0:v]trim=start_frame=10:end_frame=70,', graph)
        self.assertIn('xfade=transition=fade:duration=0.333333333:offset=0.666666667', graph)
        self.assertIn('[m1][s2]concat=n=2:v=1:a=0', graph)
        self.assertIn('atrim=duration=2.500000', graph)
        self.assertIn('between(t,0,2)', graph)


@unittest.skipUnless(FFMPEG, 'ffmpeg and ffprobe required')
class EncodeTests(unittest.TestCase):
    def sh(self, *cmd):
        subprocess.run(cmd, check=True, capture_output=True)

    def test_three_shot_edit_encodes_exact_frames_and_repeats_byte_for_byte(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            for name, src in (('a', 'testsrc2'), ('b', 'smptebars'), ('c', 'testsrc')):
                self.sh('ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', f'{src}=size=320x180:rate=60:duration=2', '-c:v', 'libx264',
                        '-threads', '2', '-preset', 'ultrafast', '-g', '30', '-pix_fmt', 'yuv420p', str(root / f'{name}.mp4'))
            self.sh('ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=3', str(root / 'music.wav'))
            for name, base in (('a', 0), ('b', 1000), ('c', 2000)):
                write_frames(root / f'{name}.frames.jsonl', [base + f // 3 for f in range(120)])
                write_telemetry(root / f'{name}.telemetry.jsonl', range(base, base + 41))
            (root / 'title.txt').write_text('TITLE\n')
            spec = {'resolution': '1080p', 'music': 'music.wav', 'texts': [{'file': 'title.txt', 'start': 0, 'end': 1}],
                    'shots': [{'id': 'a', 'video': 'a.mp4', 'frames': 'a.frames.jsonl', 'telemetry': 'a.telemetry.jsonl', 'in': 0, 'out': 70,
                               'transition': {'type': 'fade', 'frames': 20}},
                              {'id': 'b', 'video': 'b.mp4', 'frames': 'b.frames.jsonl', 'telemetry': 'b.telemetry.jsonl', 'in': 10, 'out': 80,
                               'transition': {'type': 'cut'}},
                              {'id': 'c', 'video': 'c.mp4', 'frames': 'c.frames.jsonl', 'in': 30, 'out': 90}]}
            (root / 'spec.json').write_text(json.dumps(spec))
            hashes = []
            for run in ('one', 'two'):
                subprocess.run([sys.executable, str(HERE / 'edit.py'), '--spec', str(root / 'spec.json'), '--out', str(root / run)],
                               check=True, capture_output=True)
                out = root / run
                self.assertEqual(edit.probe_video(out / 'final.mp4')[:3], (180, '60/1', 1920))
                self.assertEqual(len(list((out / 'overlay').glob('*.png'))), 180)
                hashes.append({p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(out.rglob('*')) if p.is_file()
                               and p.name not in ('ffmpeg-command.txt', 'plan.json', 'edit.frames.jsonl') or p.name in ('edit.frames.jsonl',)})
            self.assertEqual(hashes[0], hashes[1])
            # overlay-less shot c: transparent PNGs; overlay shot a: not
            blank = (root / 'one/overlay/overlay-000179.png').read_bytes()
            self.assertEqual(blank, (root / 'one/overlay/overlay-000150.png').read_bytes())
            self.assertNotEqual(blank, (root / 'one/overlay/overlay-000000.png').read_bytes())
            # an unavailable tick aborts before any output beyond the plan files exists
            write_telemetry(root / 'a.telemetry.jsonl', [t for t in range(41) if t != 3])
            r = subprocess.run([sys.executable, str(HERE / 'edit.py'), '--spec', str(root / 'spec.json'), '--out', str(root / 'three')],
                               capture_output=True, text=True)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn('tick 3 missing from telemetry', r.stderr)
            self.assertFalse((root / 'three').exists())


if __name__ == '__main__':
    unittest.main()
