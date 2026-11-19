"""The horizon-lod acceptance's own comparison logic on a synthetic world: a faithful
reply scores 100%, a flat or shifted fake fails the thresholds the profile applies."""
from __future__ import annotations

import unittest

from . import horizon_lod_wire as hz
from .backends import horizon_lod as accept

STONE, GRASS, WATER = 1, 9, 86


def height(x: int, z: int) -> int:
    return 60 + (x * 7 + z * 13) % 9


class World:
    """Duck-types `Oracle`: block states from a pure height function."""

    def state(self, x: int, y: int, z: int) -> int:
        top = height(x, z)
        return STONE if y < top - 1 else GRASS if y == top - 1 else 0

    def top(self, x: int, z: int, skip: set[int]) -> int | None:
        return height(x, z)


def tile(request: hz.Request, shift: int = 0, flat: bool = False) -> hz.Reply:
    columns = []
    for index in range(hz.SIDE * hz.SIDE):
        x = request.x + (index % hz.SIDE) * request.stride
        z = request.z + (index // hz.SIDE) * request.stride
        h = 64 if flat else max(height(x + dx, z + dz) for dx in range(request.stride)
                                for dz in range(request.stride)) + shift
        columns.append(hz.Column(hz.Surface(h, GRASS, None, "minecraft:plains"),
                                 (hz.Span(h - 1, h, GRASS, "minecraft:plains", 0),)))
    return hz.Reply(0, False, tuple(columns))


def score(reply: hz.Reply, request: hz.Request) -> dict:
    return accept.compare_tile(reply, request, World(), set())


class CompareTests(unittest.TestCase):
    def test_faithful_stride_one_tile_scores_everything(self):
        request = hz.Request(0, 96, -48, 1)
        stats = score(tile(request), request)
        self.assertEqual((stats["with_surface"], stats["exact_top_state"], stats["top_within_2"]),
                         (256, 256, 256))

    def test_flat_fake_fails_the_exact_state_threshold(self):
        request = hz.Request(0, 96, -48, 1)
        stats = score(tile(request, flat=True), request)
        self.assertLess(stats["exact_top_state"] / 256, 0.95)

    def test_shifted_fake_fails_both_thresholds(self):
        request = hz.Request(0, 96, -48, 1)
        stats = score(tile(request, shift=5), request)
        self.assertLess(stats["exact_top_state"] / 256, 0.95)
        self.assertLess(stats["top_within_2"] / 256, 0.80)

    def test_coarse_tile_stays_inside_its_footprint_and_a_shifted_one_does_not(self):
        request = hz.Request(0, 128, 0, 4)
        self.assertEqual(score(tile(request), request)["footprint_loose"], 256)
        self.assertLess(score(tile(request, shift=9), request)["footprint_loose"], 256 * 0.9)

    def test_aligned_tiles_sit_beyond_the_view_distance(self):
        spawn = (-4, -12)
        for stride in (2, 4):
            first = accept.aligned_tile(stride, spawn, 4, 8)
            self.assertEqual(first[0] % stride, 0)
            self.assertEqual(first[1] % stride, 0)
            self.assertGreaterEqual(accept.beyond_chunks(first, stride, spawn, 4) * 2, stride * stride)

    def test_seed_warning_needs_the_key_and_the_seed(self):
        warn = "2026 WARN krasnokamen: [horizon_lod] enabled: far tiles help recover the world seed"
        self.assertEqual(accept.seed_warnings(warn), [warn])
        self.assertEqual(accept.seed_warnings("WARN ignoring unknown config section key=horizon_lod"), [])
        self.assertEqual(accept.seed_warnings("INFO horizon_lod seed"), [])


if __name__ == "__main__":
    unittest.main()
