import importlib.util
import json
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name('prepare-arena.py')
spec = importlib.util.spec_from_file_location('prepare_arena', SCRIPT)
arena = importlib.util.module_from_spec(spec)
spec.loader.exec_module(arena)

FILL = re.compile(r'^fill (-?\d+) (-?\d+) (-?\d+) (-?\d+) (-?\d+) (-?\d+) (\S+)$')
SETBLOCK = re.compile(r'^setblock (-?\d+) (-?\d+) (-?\d+) (\S+)$')


def boxes(commands):
    for phase, command in commands:
        match = FILL.match(command)
        if match:
            x1, y1, z1, x2, y2, z2 = map(int, match.groups()[:6])
            yield phase, (min(x1, x2), min(y1, y2), min(z1, z2)), (max(x1, x2), max(y1, y2), max(z1, z2)), match.group(7)


class PrepareArenaTests(unittest.TestCase):
    def test_plan_is_a_pure_function_of_seed(self):
        a = arena.plan(42, True)
        self.assertEqual(a, arena.plan(42, True))
        self.assertEqual(arena.command_digest(a), arena.command_digest(arena.plan(42, True)))
        self.assertNotEqual(arena.command_digest(a), arena.command_digest(arena.plan(43, True)))
        self.assertLess(len(arena.plan(42, False)), len(a))
        # The command stream is the receipt: a change to the layout must be deliberate.
        self.assertEqual(sum(1 for phase, _ in a if phase == 'decor'), arena.DECOR_TUFTS + arena.DECOR_FLOWERS)

    def test_every_command_is_well_formed_and_inside_the_loaded_square(self):
        load = arena.LOAD_HALF
        for phase, command in arena.plan(42, True):
            match = FILL.match(command) or SETBLOCK.match(command)
            if match:
                coords = list(map(int, match.groups()[:-1]))
                xs, zs = coords[0::3], coords[2::3]
                self.assertTrue(all(-load <= v < load for v in xs + zs), command)
                self.assertTrue(all(-64 <= v < 320 for v in coords[1::3]), command)
            else:
                self.assertTrue(command.startswith(('gamerule ', 'time ', 'weather ', 'difficulty ', 'forceload add ', 'setworldspawn ')), command)

    def test_every_fill_fits_the_raised_block_limit_and_forceload_the_chunk_limit(self):
        for phase, low, high, block in boxes(arena.plan(42, True)):
            volume = (high[0] - low[0] + 1) * (high[1] - low[1] + 1) * (high[2] - low[2] + 1)
            self.assertLessEqual(volume, arena.FILL_LIMIT, (phase, low, high))
        for phase, command in arena.plan(42, True):
            if command.startswith('forceload add'):
                x1, z1, x2, z2 = map(int, command.split()[2:6])
                self.assertLessEqual(((x2 - x1 + 1) // 16) * ((z2 - z1 + 1) // 16), 256, command)
                self.assertEqual((x2 - x1 + 1) % 16 + (z2 - z1 + 1) % 16, 0)

    def test_battle_field_is_dead_flat_and_nothing_stands_on_it(self):
        flat = arena.FLAT
        for seed in (1, 42, 99):
            for x in range(-flat + 1, flat, 7):
                for z in range(-flat + 1, flat, 7):
                    self.assertEqual(arena.surface(seed, x, z), arena.GRASS_Y, (seed, x, z))
            trees, boulders, taken = arena.place_features(seed)
            self.assertGreaterEqual(len(trees), 50)
            self.assertGreaterEqual(len(boulders), 10)
            for x, z, radius in taken:
                self.assertGreaterEqual(max(abs(x), abs(z)) - radius, flat)
                self.assertLessEqual(max(abs(x), abs(z)) + radius, arena.HALF - 4)
        # The staging bounds of `battle setup` at the spawn chunk are [-176, 192]; 200 is clear of them.
        self.assertGreaterEqual(flat, 192 + 8)

    def test_decor_stays_off_trees_and_on_the_surface(self):
        seed = 42
        _, _, taken = arena.place_features(seed)
        for phase, command in arena.plan(seed, True):
            if phase != 'decor':
                continue
            x, y, z, block = SETBLOCK.match(command).groups()
            x, y, z = int(x), int(y), int(z)
            self.assertEqual(y, arena.surface(seed, x, z) + 1)
            self.assertTrue(all((x - tx) ** 2 + (z - tz) ** 2 > (tr + 1) ** 2 for tx, tz, tr in taken), command)

    def test_rim_and_hill_rise_gently_and_hill_is_landmark(self):
        seed = 42
        top = max(arena.surface(seed, x, z) for x in range(-arena.HALF, arena.HALF, 4) for z in range(-arena.HALF, arena.HALF, 4))
        self.assertEqual(top, arena.GRASS_Y + max(arena.HILL['height'], arena.STEPS))
        # Neighbouring columns never differ by more than one block except at hill/terrace walls of one block.
        for x in range(-arena.HALF, arena.HALF - 1):
            for z in range(-arena.HALF, arena.HALF, 9):
                self.assertLessEqual(abs(arena.surface(seed, x, z) - arena.surface(seed, x + 1, z)), 3, (x, z))
        self.assertGreater(arena.HILL['x'] - arena.HILL['radius'], arena.FLAT)
        self.assertLess(arena.HILL['x'] + arena.HILL['radius'], arena.HALF)

    def test_terrace_boundaries_are_nested_so_columns_have_no_gaps(self):
        for seed in (3, 42):
            for side in range(4):
                for segment in range(2 * arena.HALF // arena.SEGMENT):
                    starts = [arena.rim_start(seed, side, segment, k) for k in range(1, arena.STEPS + 1)]
                    self.assertEqual(starts, sorted(set(starts)))
                    self.assertGreaterEqual(starts[0], arena.FLAT)
                    self.assertLessEqual(starts[-1], arena.HALF - 1)

    def test_dry_run_prints_the_digest_without_starting_a_server(self):
        with tempfile.TemporaryDirectory() as directory:
            run = Path(directory) / 'run'
            result = subprocess.run([str(SCRIPT), '--pin', '/none', '--cache', '/none', '--run-dir', str(run), '--dry-run'],
                                    text=True, capture_output=True, check=True)
            payload = json.loads(result.stdout)
            self.assertEqual(payload['command_sha256'], arena.command_digest(arena.plan(42, True)))
            self.assertFalse(run.exists())


if __name__ == '__main__':
    unittest.main()
