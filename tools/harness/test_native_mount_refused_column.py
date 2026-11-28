import unittest

from tools.harness.backends.native_mount_refused_column import _scan_visited_refused_column


class GroundScanEvidenceTest(unittest.TestCase):
    def test_requires_completed_untruncated_scan_of_exact_column(self):
        scan = {"reply": "land", "truncated": False,
                "unheld_cells": [[191, 76, -448], None, None, None]}
        self.assertTrue(_scan_visited_refused_column({"boat_ground_scan": scan}, 11, -28))
        self.assertFalse(_scan_visited_refused_column({"boat_ground_scan": scan}, 11, -29))
        self.assertFalse(_scan_visited_refused_column(
            {"boat_ground_scan": {**scan, "reply": "scanning"}}, 11, -28))
        self.assertFalse(_scan_visited_refused_column(
            {"boat_ground_scan": {**scan, "truncated": True}}, 11, -28))
        self.assertFalse(_scan_visited_refused_column({}, 11, -28))


if __name__ == "__main__":
    unittest.main()
