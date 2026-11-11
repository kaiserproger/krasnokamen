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

    def test_all_behaviours_pass_requires_five_passes(self) -> None:
        good = {name: "PASS" for name in
                ("door_walk", "sleep_return", "farmer_harvest", "miner_deposit", "lumberjack")}
        self.assertTrue(village_qa.all_behaviours_pass(good))
        bad = dict(good, miner_deposit="FAIL")
        self.assertFalse(village_qa.all_behaviours_pass(bad))
        inconclusive = dict(good, lumberjack="INCONCLUSIVE")
        self.assertFalse(village_qa.all_behaviours_pass(inconclusive))

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

    STATIONS = {"farmer": (201, 78, -447), "miner": (207, 78, -449),
                "lumberjack": (207, 78, -435)}

    @staticmethod
    def _row(x: float, z: float, plate: str, profession: int) -> dict:
        return {"x": x, "y": 78.0, "z": z, "received_metadata": {
            "2": {"kind": "optional_component", "value": plate},
            "19": {"kind": "villager", "value": [2, profession, 1]}}}

    def test_job_comes_from_client_data_not_summon_role(self) -> None:
        # ot1z1c9j: summoned as the miner, the allocator made it the lumberjack.
        row = self._row(205.44, -434.22, "Николай · лесоруб · работа", 7)
        self.assertEqual(village_qa.villager_job(row), "lumberjack")
        self.assertEqual(village_qa.villager_job(
            self._row(0, 0, "Дмитрий · шахтёр · работа", 13)), "miner")
        self.assertEqual(village_qa.villager_job(
            self._row(0, 0, "Андрей · фермер · отдых", 5)), "farmer")
        # Nameplate and the outfit the client renders must agree.
        self.assertIsNone(village_qa.villager_job(
            self._row(0, 0, "Николай · лесоруб · работа", 0)))
        self.assertIsNone(village_qa.villager_job(
            self._row(0, 0, "Наталья · безработный · работа", 0)))
        self.assertIsNone(village_qa.villager_job({"x": 0, "z": 0}))

    def test_return_needs_the_same_job_at_that_jobs_station(self) -> None:
        # ot1z1c9j morning: 2.08 from the fletcher, 5.9 from the mine its
        # summon role pointed at.
        row = self._row(205.44, -434.22, "Николай · лесоруб · работа", 7)
        self.assertTrue(village_qa.returned_to_job(row, "lumberjack", self.STATIONS))
        self.assertFalse(village_qa.returned_to_job(row, "miner", self.STATIONS))
        far = self._row(204.5, -434.5, "Николай · лесоруб · работа", 7)
        self.assertFalse(village_qa.returned_to_job(far, "lumberjack", self.STATIONS))
        jobless = self._row(207.5, -434.5, "Николай · безработный · работа", 0)
        self.assertFalse(village_qa.returned_to_job(jobless, "lumberjack", self.STATIONS))

    def test_one_villager_per_job_so_the_bed_sleeper_always_works(self) -> None:
        # a0rmeano & co.: a fourth, jobless villager took the only bed.
        roles = village_qa.VILLAGER_ROLES
        self.assertEqual(len(roles), len(set(roles)))
        self.assertEqual(set(roles), set(village_qa.JOB_WIRE_PROFESSION))
        self.assertEqual(set(roles), set(village_qa.JOB_ROLES.values()))

    MINE = ((206, 71, -450), (216, 78, -440))
    # u2wl2u3f morning (every 4th poll, 45 s window): the woken lumberjack walked
    # ~100 blocks round the fixture cliff toward a hillside tree, never near its
    # station and never yet beside a log.
    U2WL2U3F_LUMBERJACK = [
        (194.48, 78.69, -446.52), (197.42, 78.0, -446.12), (196.44, 76.94, -448.49),
        (193.77, 77.0, -450.48), (190.84, 77.0, -451.52), (187.69, 77.0, -451.53),
        (184.09, 77.0, -451.53), (180.04, 77.0, -451.53), (176.55, 77.0, -452.26),
        (176.47, 77.0, -455.86), (176.47, 77.0, -459.91), (176.47, 77.0, -463.96),
        (176.47, 77.0, -467.56), (176.47, 77.0, -470.71), (176.47, 77.0, -475.22),
        (175.97, 79.23, -478.48), (172.37, 79.75, -478.54), (167.51, 82.0, -479.54),
        (163.26, 85.75, -478.55), (161.46, 86.99, -475.65), (164.33, 86.99, -472.3),
        (165.46, 87.99, -467.6), (165.46, 87.99, -463.55), (165.46, 87.99, -459.5),
        (165.46, 86.99, -456.2), (165.45, 86.99, -452.15), (165.45, 86.99, -449.0),
        (165.45, 86.99, -446.75), (165.45, 86.99, -446.75), (166.41, 86.99, -442.74),
        (166.44, 86.99, -441.84)]

    def _work(self, job: str) -> village_qa.WorkReturn:
        return village_qa.WorkReturn(job, self.STATIONS, self.MINE)

    @staticmethod
    def _logs(*cells: tuple[int, int, int]) -> list[dict]:
        return [{"x": x, "y": y, "z": z, "block_id": "minecraft:oak_log"} for x, y, z in cells]

    def test_back_at_the_station_counts_for_every_job(self) -> None:
        work = self._work("lumberjack")
        work.see(self._row(205.44, -434.22, "Николай · лесоруб · работа", 7))
        self.assertIn("station", work.evidence)
        lost = self._work("lumberjack")
        lost.see(self._row(207.5, -434.5, "Николай · безработный · работа", 0))
        self.assertIsNone(lost.evidence)

    def test_miner_back_at_work_inside_the_mine_not_short_of_it(self) -> None:
        work = self._work("miner")
        work.see(self._row(209.5, -446.5, "Дмитрий · шахтёр · работа", 13))
        self.assertIn("inside the mine", work.evidence)
        # u2wl2u3f: the miner stood at the foot of the mine step the whole day.
        stuck = self._work("miner")
        for _ in range(3):
            stuck.see(self._row(204.73, -448.51, "Дмитрий · шахтёр · работа", 13))
        self.assertIsNone(stuck.evidence)

    def test_lumberjack_back_at_work_when_it_cuts_the_tree_it_stopped_at(self) -> None:
        work = self._work("lumberjack")
        row = self._row(205.5, -430.5, "Николай · лесоруб · работа", 7)
        work.see(row)
        self.assertIsNone(work.scan_spot(), "a moving villager is not scanned")
        work.see(row)
        spot = work.scan_spot()
        self.assertIsNotNone(spot)
        work.scanned(spot, self._logs((207, 78, -430), (207, 79, -430), (215, 78, -430)))
        self.assertEqual(work.watched, [(207, 78, -430), (207, 79, -430)])
        work.logs_now({(207, 78, -430): "minecraft:oak_log", (207, 79, -430): "minecraft:oak_log"})
        self.assertIsNone(work.evidence, "standing beside a tree is not cutting it")
        work.logs_now({(207, 78, -430): "minecraft:air", (207, 79, -430): "minecraft:oak_log"})
        self.assertIn("cut the log at [207, 78, -430]", work.evidence)

    def test_a_log_vanishing_after_the_lumberjack_left_is_not_its_cut(self) -> None:
        work = self._work("lumberjack")
        here = self._row(205.5, -430.5, "Николай · лесоруб · работа", 7)
        work.see(here)
        work.see(here)
        work.scanned(work.scan_spot(), self._logs((207, 78, -430)))
        work.see(self._row(195.5, -446.5, "Николай · лесоруб · работа", 7))
        work.logs_now({(207, 78, -430): "minecraft:air"})
        self.assertIsNone(work.evidence)

    def test_replay_u2wl2u3f_lumberjack_still_on_its_way_at_the_window_end(self) -> None:
        work = self._work("lumberjack")
        for x, y, z in self.U2WL2U3F_LUMBERJACK:
            work.see(dict(self._row(x, z, "Сергей · лесоруб · работа", 7), y=y))
            if (spot := work.scan_spot()) is not None:
                work.scanned(spot, [])  # the recording holds no scan; nothing proven
        self.assertIsNone(work.evidence)
        self.assertGreater(work.travelled, 90.0, "it was walking to work, not idle")

    def test_woken_lumberjack_idle_beside_the_bed_fails(self) -> None:
        # umbaup5f: stood up beside the bed and never moved all morning.
        work = self._work("lumberjack")
        for _ in range(20):
            work.see(self._row(195.475, -446.519, "Михаил · лесоруб · работа", 7))
            if (spot := work.scan_spot()) is not None:
                work.scanned(spot, [{"x": 194, "y": 78, "z": -447, "block_id": "minecraft:red_bed"},
                                    {"x": 192, "y": 78, "z": -446, "block_id": "minecraft:cobblestone"}])
        self.assertIsNone(work.evidence)
        self.assertEqual(work.watched, [])
        self.assertLess(work.travelled, 0.01)


if __name__ == "__main__":
    unittest.main()
