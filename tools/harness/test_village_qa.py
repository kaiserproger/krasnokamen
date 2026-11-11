"""Fail-closed unit tests for the village-qa helper predicates.

These cover the pure geometry/ledger helpers shared with the real-client
acceptance backend; the graphical acceptance itself stays in
`tools/harness/backends/village_qa.py` and runs only under Main's GO.
"""
from __future__ import annotations

import unittest

from .backends import village_qa


class VillageQaHelpersTests(unittest.TestCase):
    def test_bed_cells_are_east_foot_then_head(self) -> None:
        foot, head = village_qa.bed_cells(192, 77, -448)
        self.assertEqual(foot, (193, 78, -447))
        self.assertEqual(head, (194, 78, -447))
        self.assertTrue(village_qa.bed_facing_valid(foot, head, "east"))
        self.assertFalse(village_qa.bed_facing_valid(head, foot, "east"))
        self.assertFalse(village_qa.bed_facing_valid(foot, head, "west"))

    def test_headroom_needs_two_air_blocks(self) -> None:
        self.assertTrue(village_qa.house_headroom_ok(78, 81))
        self.assertFalse(village_qa.house_headroom_ok(78, 80))

    def test_door_crossing_flips_sides(self) -> None:
        self.assertTrue(village_qa.door_crossed(195.4, 196.6, 196))
        self.assertTrue(village_qa.door_crossed(196.6, 195.4, 196))
        self.assertFalse(village_qa.door_crossed(195.4, 195.6, 196))

    def test_chest_has_matches_any_wanted(self) -> None:
        self.assertTrue(village_qa.chest_has({"minecraft:cobblestone": 3}, village_qa.MINING_ITEMS))
        self.assertTrue(village_qa.chest_has({"minecraft:oak_log": 1}, {"minecraft:oak_log"}))
        self.assertFalse(village_qa.chest_has({"minecraft:wheat": 5}, village_qa.MINING_ITEMS))
        self.assertFalse(village_qa.chest_has({}, village_qa.MINING_ITEMS))

    def test_time_set_phase_budget(self) -> None:
        phases = list(village_qa.TIME_SET_PHASES)
        self.assertEqual(len(phases), 5)
        self.assertTrue(all(command.startswith("time set ") for command in phases))
        commands = [f"fill 0 0 0 1 1 1 stone", *phases]
        self.assertEqual(village_qa.count_time_sets(commands), 5)

    def test_all_behaviours_pass_requires_six_passes(self) -> None:
        good = {name: "PASS" for name in
                ("door_walk", "night_doors", "sleep_return", "farmer_harvest", "miner_deposit", "lumberjack")}
        self.assertTrue(village_qa.all_behaviours_pass(good))
        bad = dict(good, miner_deposit="FAIL")
        self.assertFalse(village_qa.all_behaviours_pass(bad))
        self.assertFalse(village_qa.all_behaviours_pass(dict(good, night_doors="FAIL")))
        inconclusive = dict(good, lumberjack="INCONCLUSIVE")
        self.assertFalse(village_qa.all_behaviours_pass(inconclusive))

    def _house_door_walk(self, watch: village_qa.NightDoorWatch, door: tuple[int, int, int],
                         final_open: bool) -> None:
        y, z = float(door[1]), door[2] + 0.5
        watch.observe({"v": (door[0] - 2.0, y, z)}, set(), 0.0)
        watch.observe({"v": (door[0] - 0.8, y, z)}, {door}, 1.0)
        watch.observe({"v": (door[0] + 0.8, y, z)}, {door}, 2.0)
        # The villager walks away to its bed; the door either closed behind it or not.
        watch.observe({"v": (door[0] + 6.0, y, z)}, {door} if final_open else set(), 3.0)

    def test_night_door_closed_behind_the_villager_passes(self) -> None:
        door = (10, 80, 5)
        watch = village_qa.NightDoorWatch([door])
        self._house_door_walk(watch, door, final_open=False)
        self.assertEqual(watch.verdict(door, set())[0], "PASS")

    def test_night_door_left_open_with_nobody_near_fails(self) -> None:
        door = (10, 80, 5)
        watch = village_qa.NightDoorWatch([door])
        self._house_door_walk(watch, door, final_open=True)
        # Still open well past the allowance with the villager far away.
        watch.observe({"v": (door[0] + 6.0, 80.0, door[2] + 0.5)}, {door},
                      3.0 + village_qa.DOOR_LEFT_OPEN_SECONDS + 1.0)
        verdict, evidence = watch.verdict(door, {door})
        self.assertEqual(verdict, "FAIL")
        self.assertIn("left open", evidence)

    def test_night_door_open_beside_its_villager_is_not_left_open(self) -> None:
        door = (10, 80, 5)
        watch = village_qa.NightDoorWatch([door])
        self._house_door_walk(watch, door, final_open=False)
        beside = {"v": (door[0] + 1.0, 80.0, door[2] + 0.5)}
        watch.observe(beside, {door}, 4.0)
        watch.observe(beside, {door}, 4.0 + village_qa.DOOR_LEFT_OPEN_SECONDS * 2)
        self.assertEqual(watch.verdict(door, set())[0], "PASS")

    def test_night_door_crossing_between_polls_still_counts(self) -> None:
        door = (10, 80, 5)
        watch = village_qa.NightDoorWatch([door])
        y, z = 80.0, door[2] + 0.5
        # The open and the close both fall between two polls: the door is never read open.
        watch.observe({"v": (door[0] - 0.8, y, z)}, set(), 0.0)
        watch.observe({"v": (door[0] + 0.8, y, z)}, set(), 1.0)
        self.assertEqual(watch.opened, set())
        self.assertEqual(watch.verdict(door, set())[0], "PASS")

    def test_night_door_without_a_crossing_is_not_a_pass(self) -> None:
        door = (10, 80, 5)
        watch = village_qa.NightDoorWatch([door])
        watch.observe({"v": (door[0] - 5.0, 80.0, door[2] + 0.5)}, set(), 0.0)
        self.assertEqual(watch.verdict(door, set())[0], "FAIL")

    def test_trees_need_five_column_separation(self) -> None:
        self.assertTrue(village_qa.trees_separated([(0, 0, 0), (5, 0, 0)]))
        self.assertFalse(village_qa.trees_separated([(0, 0, 0), (2, 0, 0)]))

    def test_radius_covers_fixtures_and_clamps(self) -> None:
        self.assertEqual(village_qa.required_radius((0, 0, 0), [(3, 0, 4)]), 6)
        self.assertEqual(village_qa.required_radius((0, 0, 0), [(300, 0, 0)]), 256)
        self.assertGreaterEqual(village_qa.required_radius((0, 0, 0), [(0, 0, 0)]), 1)

    def test_sleep_pose_matches_core_offset(self) -> None:
        self.assertTrue(village_qa.is_sleep_pose(78.6875, 78))
        self.assertFalse(village_qa.is_sleep_pose(79.5, 78))

    def test_sleep_requires_received_sleeping_dimensions_not_just_proximity(self) -> None:
        row = {"x": 2.5, "y": 78.6875, "z": 3.5, "health": 20,
               "bounding_box": {"min_y": 78.6875, "max_y": 80.6375}}
        self.assertFalse(village_qa.sleeping_at_bed(row, (2, 78, 3)))
        row["bounding_box"] = {"min_y": 78.6875, "max_y": 78.8875}
        self.assertTrue(village_qa.sleeping_at_bed(row, (2, 78, 3)))
        row["y"] += 0.2
        self.assertTrue(village_qa.sleeping_at_bed(row, (2, 78, 3)))
        row["health"] = 0
        self.assertFalse(village_qa.sleeping_at_bed(row, (2, 78, 3)))


if __name__ == "__main__":
    unittest.main()
