"""Unit tests of the buildings-qa `perimeter` scenario's pure reading of the client's block scan."""
import unittest
from unittest import mock

from . import regressions
from .backends import perimeter_qa as qa

# The core's wall ring for today's deepest blueprint (settlements/layout.rs RING = 30); the gate
# itself learns it from the founded line.
RING = 30


def standing_town(ax=100, ay=70, az=-40, ring=RING):
    """The scan of a finished fortification on flat ground: the wall line and one well."""
    blocks, wells = {}, {}
    for x, z, across in qa.ring_columns(ax, az, ring):
        for step, block in enumerate(qa.wanted(across)):
            blocks[(x, ay + step, z)] = block
    cx, cz = qa.wells(ax, az)[0]
    wells[(cx, ay - 1, cz)] = qa.WATER
    for dx in (-1, 0, 1):
        for dz in (-1, 0, 1):
            if (dx, dz) != (0, 0):
                wells[(cx + dx, ay, cz + dz)] = qa.COBBLE
    return blocks, wells, (ax, ay, az)


class PerimeterScan(unittest.TestCase):
    def test_the_wall_line_columns_follow_the_ring(self):
        columns = qa.ring_columns(0, 0, RING)
        self.assertEqual(len(columns), 4 * (2 * RING + 1) - 4)
        self.assertEqual(len({(x, z) for x, z, _ in columns}), len(columns))
        self.assertEqual(max(abs(x) for x, _, _ in columns), RING)
        self.assertEqual(max(abs(z) for _, z, _ in columns), RING)
        self.assertEqual(sum(1 for *_, across in columns if abs(across) <= qa.GATE_HALF), 28)
        self.assertEqual(sum(1 for *_, across in columns if across == 0), 4)

    def test_a_finished_fortification_is_complete(self):
        ring, wells, (ax, ay, az) = standing_town()
        counts = qa.tally(ring, wells, ax, ay, az, RING)
        self.assertTrue(qa.complete(counts), counts)
        self.assertEqual((counts["doors"], counts["open_columns"], len(counts["wells"])), (4, 0, 1))

    def test_a_missing_wall_column_opens_the_line_and_a_missing_well_fails(self):
        ring, wells, (ax, ay, az) = standing_town()
        for step in range(3):
            del ring[(ax + 9, ay + step, az + RING)]
        counts = qa.tally(ring, wells, ax, ay, az, RING)
        self.assertFalse(qa.complete(counts))
        self.assertEqual(counts["open_columns"], 1)
        self.assertEqual(counts["wall_columns"]["empty"], 1)
        counts = qa.tally(*standing_town()[:1], {}, ax, ay, az, RING)
        self.assertEqual(counts["wells"], [])
        self.assertFalse(qa.complete(counts))

    def test_an_empty_world_is_the_baseline(self):
        counts = qa.tally({}, {}, 0, 70, 0, RING)
        self.assertEqual((counts["ring_blocks"], counts["well_blocks"], counts["doors"]), (0, 0, 0))
        self.assertEqual(counts["open_columns"], 4 * (2 * RING + 1) - 4)


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


class PerimeterStatusRead(unittest.TestCase):
    def test_status_page_names_are_read_without_a_window(self):
        from tools.harness.backends import buildings_qa

        class Probe:
            def __init__(self):
                self.calls = []

            def call(self, name, args=None):
                self.calls.append(name)
                return {}

        page = {"container": {"slots": [{"slot": 0, "name": "Wall section L1 · Prepared"},
                                       {"slot": 10, "name": "Active projects: 1/8"},
                                       {"slot": 99}]}}
        probe = Probe()
        with mock.patch.object(buildings_qa, "_project_page", return_value=page):
            names = qa._status_names(probe)
        self.assertEqual(names, ["Wall section L1 · Prepared", "Active projects: 1/8"])
        self.assertEqual(probe.calls, ["minecraft_close_screen"])
        self.assertEqual(qa.project_labels_from(names), {"Wall section"})


class PerimeterWallRing(unittest.TestCase):
    def test_the_gate_reads_the_ring_the_product_names(self):
        # 20261124T160759: the wall stood at 30 (the core's RING for MAX_SIZE[2] = 11) while the
        # gate scanned 25 and reported an empty wall. The founded line carries the ring now.
        self.assertEqual(qa.founded_ring(
            ["Founded bqa_perimeter (revision 3), wall ring 30."], "bqa_perimeter"), 30)
        self.assertEqual(qa.founded_ring(
            ["Founded bqa_other (revision 1), wall ring 30."], "bqa_perimeter"), 0)
        self.assertEqual(qa.founded_ring(["Founded bqa_perimeter (revision 1)."], "bqa_perimeter"), 0)

    def test_the_line_scanned_moves_with_the_ring(self):
        self.assertEqual(len(qa.ring_columns(128, 0, 30)), 4 * (2 * 30 + 1) - 4)
        self.assertEqual(max(abs(x - 128) for x, _, _ in qa.ring_columns(128, 0, 30)), 30)
        self.assertEqual(max(abs(z) for _, z, _ in qa.ring_columns(128, 0, 30)), 30)
        # A gate is 2 * GATE_HALF + 1 columns wide on each of the four sides.
        gates = [across for _, _, across in qa.ring_columns(128, 0, 30)
                 if abs(across) <= qa.GATE_HALF]
        self.assertEqual(len(gates), 4 * (2 * qa.GATE_HALF + 1))


if __name__ == "__main__":
    unittest.main()
