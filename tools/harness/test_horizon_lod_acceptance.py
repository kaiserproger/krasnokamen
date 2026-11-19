"""The horizon-lod acceptance's own comparison logic on a synthetic world: a faithful
reply scores 100%, a flat or shifted fake fails the thresholds the profile applies."""
from __future__ import annotations

import argparse
import tempfile
import unittest
from pathlib import Path

from . import horizon_lod_wire as hz
from .backends import horizon_lod as accept
from .backends import horizon_lod_live as live

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

    def test_seed_warning_needs_warn_horizon_and_the_seed(self):
        warn = "WARN mc_edge::server: Horizon far terrain exposes stored terrain that can help recover the world seed"
        self.assertEqual(accept.seed_warnings(warn), [warn])
        self.assertEqual(accept.seed_warnings("WARN ignoring unknown config section key=horizon_lod"), [])
        self.assertEqual(accept.seed_warnings("INFO Horizon far terrain seed"), [])
        self.assertEqual(accept.seed_warnings("WARN the world seed is public"), [])


def _picture(directory: Path, name: str, terrain: bool) -> str:
    from PIL import Image

    image = Image.new("RGB", (1280, 720), (150, 190, 255))
    if terrain:
        for x in range(1280):
            for y in range(250, 440):
                image.putpixel((x, y), (40, 120, 40))
    path = directory / name
    image.save(path)
    return str(path)


def _session(directory: Path, name: str, source: str, quads: int, terrain: bool) -> dict:
    final = {"source": source, "effective_distance": 64, "target_tiles": 2400,
             "sampled_target_tiles": 2400, "resident_quads": quads}
    return {"final": final, "decode_warnings": [], "client_exit": 0, "server_exit": 0,
            "screenshot": _picture(directory, f"{name}.png", terrain)}


def _judged(only: str, sessions: dict) -> list[str]:
    args = argparse.Namespace(only=only, view_distance=4, min_quad_ratio=5.0, min_changed_fraction=0.30)
    checks = accept.Checks()
    live.judge(args, sessions, checks)
    return checks.failed()


class LiveVerdictTests(unittest.TestCase):
    """The live profile cannot pass without a real ON session that used the server's far terrain."""

    def test_off_only_is_never_acceptance(self):
        with tempfile.TemporaryDirectory() as directory:
            off = _session(Path(directory), "off", "server-terrain", 25000, False)
            self.assertEqual(_judged("off", {"off": off}), ["off_only_is_a_control_not_acceptance"])

    def test_a_server_without_the_service_fails_the_pair(self):
        with tempfile.TemporaryDirectory() as directory:
            where = Path(directory)
            sessions = {"off": _session(where, "off", "server-terrain", 25000, False),
                        "on": _session(where, "on", "server-terrain", 25100, False)}
            failed = _judged("pair", sessions)
            self.assertIn("on_client_accepted_the_offer", failed)
            self.assertIn("on_resident_far_geometry_exceeds_off", failed)
            self.assertIn("on_screenshot_shows_far_terrain_off_does_not", failed)

    def test_a_pair_without_an_on_session_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            off = _session(Path(directory), "off", "server-terrain", 25000, False)
            self.assertEqual(_judged("pair", {"off": off}), ["on_session"])
        self.assertEqual(_judged("pair", {}), ["on_session"])

    def test_a_server_that_serves_far_terrain_passes_the_pair(self):
        with tempfile.TemporaryDirectory() as directory:
            where = Path(directory)
            sessions = {"off": _session(where, "off", "server-terrain", 25000, False),
                        "on": _session(where, "on", "server", 384000, True)}
            self.assertEqual(_judged("pair", sessions), [])


if __name__ == "__main__":
    unittest.main()
