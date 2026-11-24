"""Unit tests of the buildings-qa `perimeter` scenario's pure reading of the client's block scan."""
import unittest

from . import regressions
from .backends import perimeter_qa as qa


def standing_town(ax=100, ay=70, az=-40):
    """The scan of a finished fortification on flat ground: the wall line and one well."""
    ring, wells = {}, {}
    for x, z, across in qa.ring_columns(ax, az):
        for step, block in enumerate(qa.wanted(across)):
            ring[(x, ay + step, z)] = block
    cx, cz = qa.wells(ax, az)[0]
    wells[(cx, ay - 1, cz)] = qa.WATER
    for dx in (-1, 0, 1):
        for dz in (-1, 0, 1):
            if (dx, dz) != (0, 0):
                wells[(cx + dx, ay, cz + dz)] = qa.COBBLE
    return ring, wells, (ax, ay, az)


class PerimeterScan(unittest.TestCase):
    def test_the_wall_line_has_two_hundred_columns_and_four_gatehouses(self):
        columns = qa.ring_columns(0, 0)
        self.assertEqual(len(columns), 200)
        self.assertEqual(len({(x, z) for x, z, _ in columns}), 200)
        self.assertEqual(sum(1 for *_, across in columns if abs(across) <= qa.GATE_HALF), 28)
        self.assertEqual(sum(1 for *_, across in columns if across == 0), 4)

    def test_a_finished_fortification_is_complete(self):
        ring, wells, (ax, ay, az) = standing_town()
        counts = qa.tally(ring, wells, ax, ay, az)
        self.assertTrue(qa.complete(counts), counts)
        self.assertEqual((counts["doors"], counts["open_columns"], len(counts["wells"])), (4, 0, 1))

    def test_a_missing_wall_column_opens_the_line_and_a_missing_well_fails(self):
        ring, wells, (ax, ay, az) = standing_town()
        for step in range(3):
            del ring[(ax + 9, ay + step, az + qa.RING)]
        counts = qa.tally(ring, wells, ax, ay, az)
        self.assertFalse(qa.complete(counts))
        self.assertEqual(counts["open_columns"], 1)
        self.assertEqual(counts["wall_columns"]["empty"], 1)
        counts = qa.tally(*standing_town()[:1], {}, ax, ay, az)
        self.assertEqual(counts["wells"], [])
        self.assertFalse(qa.complete(counts))

    def test_an_empty_world_is_the_baseline(self):
        counts = qa.tally({}, {}, 0, 70, 0)
        self.assertEqual((counts["ring_blocks"], counts["well_blocks"], counts["doors"]), (0, 0, 0))
        self.assertEqual(counts["open_columns"], 200)


class PerimeterRefs(unittest.TestCase):
    def test_a_perimeter_verdict_runs_that_select(self):
        command = regressions._profile_command("buildings-qa", "perimeter.enclosed")
        self.assertIn("BUILDINGS_QA_SELECT=perimeter", command)
        self.assertIn("project_named", qa.VERDICTS)
        self.assertIn("BUILDINGS_QA_SELECT=perimeter",
                      regressions._profile_command("buildings-qa", "perimeter.project_named"))

    def test_an_unknown_perimeter_verdict_is_refused(self):
        with self.assertRaises(ValueError):
            regressions._profile_command("buildings-qa", "perimeter.moat")


class PerimeterProjectNames(unittest.TestCase):
    """SETTLEMENT-LAYOUT-PROJECT-SHOWS-UNKNOWN-BLUEPRINT: the layout kinds carry display names."""

    def test_a_layout_project_named_during_the_build_passes(self):
        self.assertTrue(qa.project_named({"Wall section", "House"}))
        self.assertTrue(qa.project_named({"Gate", "Road"}))
        self.assertTrue(qa.project_named({"Well"}))

    def test_an_unknown_blueprint_label_fails_even_beside_a_named_one(self):
        self.assertFalse(qa.project_named({"Wall section", "Unknown blueprint"}))
        self.assertFalse(qa.project_named({"Unknown blueprint"}))

    def test_no_layout_project_observed_never_passes(self):
        self.assertFalse(qa.project_named(set()))
        self.assertFalse(qa.project_named({"House", "Road", "Street lamp"}))

    def test_status_slot_names_carry_the_project_label(self):
        names = ["Active projects: 1/8", "Wall section L1 · Prepared",
                 "Well L1 · Prepared 0/24", "Медовая поляна"]
        self.assertEqual(qa.project_labels_from(names), {"Wall section", "Well"})
        self.assertTrue(qa.project_named(qa.project_labels_from(names)))
        unknown = qa.project_labels_from(["Unknown blueprint L1 · Prepared 0/24"])
        self.assertFalse(qa.project_named(unknown))


if __name__ == "__main__":
    unittest.main()
