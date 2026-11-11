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

    DOOR = (10, 80, 5)
    HOUSE = ((6, 79, 2), (10, 83, 8))

    def _watch(self) -> village_qa.NightDoorWatch:
        return village_qa.NightDoorWatch([self.DOOR], {self.DOOR: self.HOUSE})

    def _walk_out(self, watch: village_qa.NightDoorWatch, final_open: bool) -> None:
        # One long step from the interior to far outside: the door cell is never sampled.
        watch.observe({"v": (8.5, 80.0, 6.5)}, set(), 0.0)
        watch.observe({"v": (16.0, 80.0, 9.5)}, {self.DOOR}, 1.0)
        watch.observe({"v": (19.0, 80.0, 9.5)}, {self.DOOR} if final_open else set(), 2.0)

    def test_building_zone_separates_interior_wall_and_outside(self) -> None:
        lo, hi = self.HOUSE
        self.assertEqual(village_qa.building_zone(lo, hi, (8.5, 80.0, 5.5)), "in")
        self.assertEqual(village_qa.building_zone(lo, hi, (8.5, 80.6, 5.5)), "in")
        self.assertEqual(village_qa.building_zone(lo, hi, (10.4, 80.0, 5.5)), "wall")
        self.assertEqual(village_qa.building_zone(lo, hi, (6.4, 80.0, 5.5)), "wall")
        self.assertEqual(village_qa.building_zone(lo, hi, (12.0, 80.0, 5.5)), "out")
        self.assertEqual(village_qa.building_zone(lo, hi, (8.5, 77.0, 5.5)), "out")

    def test_crossing_is_seen_however_far_apart_the_observations_are(self) -> None:
        crossings = village_qa.BuildingCrossings({self.DOOR: self.HOUSE})
        self.assertEqual(crossings.observe({"v": (8.5, 80.0, 6.5)}), [])
        self.assertEqual(crossings.observe({"v": (16.0, 80.0, 9.5)}), [("v", self.DOOR, "out")])
        self.assertEqual(crossings.observe({"v": (8.0, 80.0, 3.5)}), [("v", self.DOOR, "in")])
        # Standing in the door cell is neither side: no flip until it leaves.
        self.assertEqual(crossings.observe({"v": (10.5, 80.0, 5.5)}), [])
        self.assertEqual(crossings.observe({"v": (8.0, 80.0, 3.5)}), [])

    def test_walking_around_the_building_is_not_a_crossing(self) -> None:
        crossings = village_qa.BuildingCrossings({self.DOOR: self.HOUSE})
        self.assertEqual(crossings.observe({"v": (14.0, 80.0, 0.5)}), [])
        self.assertEqual(crossings.observe({"v": (3.0, 80.0, 9.5)}), [])

    def test_door_walk_miss_from_the_recorded_run_is_a_crossing(self) -> None:
        # Run 20261111T115144-village-qa-tr3_lp49: two entity lists ~6 blocks apart straddled the
        # house door at x=196 with the second sample 3 blocks off the door row; the door-plane
        # sampler reported crossed=[] although the farmer had walked out through it.
        house = ((192, 77, -448), (196, 81, -444))
        crossings = village_qa.BuildingCrossings({(196, 78, -446): house})
        self.assertEqual(crossings.observe({"f6a5": (194.43, 78.6, -445.77)}), [])
        self.assertEqual(crossings.observe({"f6a5": (200.53, 77.0, -442.54)}),
                         [("f6a5", (196, 78, -446), "out")])

    def test_night_door_closed_behind_the_villager_passes(self) -> None:
        watch = self._watch()
        self._walk_out(watch, final_open=False)
        self.assertEqual(watch.verdict(self.DOOR, set())[0], "PASS")

    def test_night_door_left_open_with_nobody_near_fails(self) -> None:
        watch = self._watch()
        self._walk_out(watch, final_open=True)
        # Still open well past the allowance with the villager far away.
        watch.observe({"v": (19.0, 80.0, 9.5)}, {self.DOOR}, 2.0 + village_qa.DOOR_LEFT_OPEN_SECONDS + 1.0)
        verdict, evidence = watch.verdict(self.DOOR, {self.DOOR})
        self.assertEqual(verdict, "FAIL")
        self.assertIn("left open", evidence)

    def test_night_door_open_beside_its_villager_is_not_left_open(self) -> None:
        watch = self._watch()
        self._walk_out(watch, final_open=False)
        beside = {"v": (11.5, 80.0, 5.5)}
        watch.observe(beside, {self.DOOR}, 4.0)
        watch.observe(beside, {self.DOOR}, 4.0 + village_qa.DOOR_LEFT_OPEN_SECONDS * 2)
        self.assertEqual(watch.verdict(self.DOOR, set())[0], "PASS")

    def test_night_door_crossing_between_polls_still_counts(self) -> None:
        watch = self._watch()
        # The open and the close both fall between two polls: the door is never read open.
        watch.observe({"v": (8.5, 80.0, 6.5)}, set(), 0.0)
        watch.observe({"v": (16.0, 80.0, 9.5)}, set(), 1.0)
        self.assertEqual(watch.opened, set())
        self.assertEqual(watch.verdict(self.DOOR, set())[0], "PASS")

    def test_night_door_without_a_crossing_is_not_a_pass(self) -> None:
        watch = self._watch()
        watch.observe({"v": (14.0, 80.0, 5.5)}, set(), 0.0)
        self.assertEqual(watch.verdict(self.DOOR, set())[0], "FAIL")

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
