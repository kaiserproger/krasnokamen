import importlib.util
import json
import struct
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("natural_site", Path(__file__).with_name("natural-site.py"))
natural_site = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(natural_site)


def chunks_of(command):
    _, _, x0, z0, x1, z1 = command.split()
    x0, z0, x1, z1 = map(int, (x0, z0, x1, z1))
    return (x1 // 16 - x0 // 16 + 1) * (z1 // 16 - z0 // 16 + 1), (x0, z0, x1, z1)


class PlanTests(unittest.TestCase):
    def test_no_block_edits_and_spawn_last(self):
        commands = natural_site.plan(1000, -2000, 368, 71)
        for command in commands:
            self.assertFalse(command.split()[0] in ("fill", "setblock", "clone", "place"), command)
        self.assertEqual(commands[-1], "setworldspawn 1000 71 -2000")

    def test_forceload_covers_square_within_vanilla_cap(self):
        commands = [c for c in natural_site.plan(1000, -2000, 368, 71) if c.startswith("forceload add")]
        covered = set()
        for command in commands:
            count, (x0, z0, x1, z1) = chunks_of(command)
            self.assertLessEqual(count, 256, command)
            covered |= {(cx, cz) for cx in range(x0 // 16, x1 // 16 + 1) for cz in range(z0 // 16, z1 // 16 + 1)}
        expected = {(cx, cz) for cx in range((1000 - 368) // 16, (1000 + 368) // 16 + 1)
                    for cz in range((-2000 - 368) // 16, (-2000 + 368) // 16 + 1)}
        self.assertEqual(covered, expected)

    def test_one_probe_per_chunk(self):
        probes = natural_site.chunk_probes(0, 0, 32)
        self.assertEqual(len(probes), 5 * 5)
        self.assertIn("execute if block 8 319 8 minecraft:air", probes)


class MapTests(unittest.TestCase):
    def test_map_marks_water_tree_and_field(self):
        width, depth, margin = 6, 5, 1
        stats = {"area": {"x0": -3, "z0": -2, "width": width, "depth": depth, "margin": margin},
                 "field": {"width": 4, "depth": 3}, "centre": [2, 64, 2],
                 "ground_p1": 63, "ground_p99": 66, "ground_min": 63, "ground_max": 66,
                 "surroundings": {"top_max": 70}}
        raw = bytearray()
        for i in range(width * depth):
            flag = 1 if i == 0 else 4 if i == 1 else 0
            raw += struct.pack("<hhB", 64 + i % 3, 70, flag)
        with tempfile.TemporaryDirectory() as tmp:
            scan = Path(tmp)
            (scan / "stats.json").write_text(json.dumps(stats))
            (scan / "columns.bin").write_bytes(bytes(raw))
            result = natural_site.render_map(scan, scan / "map.png", scale=2)
            from PIL import Image
            image = Image.open(scan / "map.png")
            self.assertEqual(image.size, (12, 10))
            self.assertEqual(result["size"], [12, 10])
            self.assertEqual(image.getpixel((0, 0)), (40, 90, 220))
            self.assertEqual(image.getpixel((2, 0)), (20, 70, 25))
            self.assertEqual(image.getpixel((2, 2)), (255, 255, 255))

    def test_map_refuses_truncated_columns(self):
        with tempfile.TemporaryDirectory() as tmp:
            scan = Path(tmp)
            (scan / "stats.json").write_text(json.dumps({"area": {"width": 2, "depth": 2, "margin": 0}}))
            (scan / "columns.bin").write_bytes(b"\0" * 7)
            with self.assertRaises(ValueError):
                natural_site.render_map(scan, scan / "map.png")


if __name__ == "__main__":
    unittest.main()
