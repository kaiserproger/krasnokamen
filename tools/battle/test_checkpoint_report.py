import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("checkpoint_report", Path(__file__).with_name("checkpoint-report.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class CheckpointReportTests(unittest.TestCase):
    def test_join_preserves_missing_ticks_and_unknown_population(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            telemetry, log = root / "ticks", root / "server"
            telemetry.write_text("\n".join(json.dumps(row) for row in [
                {"tick": 1199, "wall_us": 7000, "phase_us": {"journal_build_us": 10, "journal_queue_us": 2}},
                {"tick": 1200, "wall_us": 85000, "phase_us": {"journal_build_us": 30, "journal_queue_us": 4}},
                {"tick": 1202, "wall_us": 9000, "phase_us": {"journal_build_us": 20, "journal_queue_us": 6}},
            ]))
            log.write_text("Battle running tick=1195 side1=10000 side2=10000 resident=20000 active=20000 nonzero_velocity=19500 horses=4000 regiments=20\n"
                           "checkpoint capture ms: tick=1200 data=50.00 owners=1.00 scheduler=2.00 chunks=3.00 absent=4.00 dimensions=5.00 job=6.00\n"
                           "Battle running tick=1208 side1=9999 side2=10000 resident=19999 active=19999 nonzero_velocity=19000 horses=4000 regiments=20\n")
            boundaries = root / "boundaries"
            boundaries.write_text(json.dumps({"paused_sample_start_tick": 1199, "paused_sample_end_tick": 1200,
                                              "battle_sample_start_tick": 1201, "recorded_last_tick": 1202}))
            result = module.report(telemetry, log, boundaries)
            self.assertEqual(result["captures"][0]["capture_ms"], 71.0)
            self.assertEqual(result["captures"][0]["wall_ms"], 85.0)
            self.assertEqual(result["captures"][0]["nearest_status"]["side1"], 10000)
            self.assertEqual(result["captures"][0]["status_tick_gap"], -5)
            self.assertEqual(result["ordinary_median_ms"], 8.0)
            self.assertEqual(result["journal_owner_phase_us"]["journal_build_us"]["median_us"], 20)
            self.assertEqual(result["journal_owner_phase_us"]["journal_queue_us"]["max_us"], 6)
            self.assertEqual(result["measurement_phases"]["paused"]["samples"], 2)
            self.assertEqual(result["measurement_phases"]["paused"]["p95_ms"], 85.0)
            self.assertEqual(result["measurement_phases"]["battle"]["missing_ticks"], 1)
            self.assertEqual(result["missing_ranges"], [[1201, 1201]])
            self.assertFalse(result["population_and_dirty_count_verified"])

    def test_rejects_duplicate_ticks_and_missing_capture(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            telemetry, log = root / "ticks", root / "server"
            row = json.dumps({"tick": 1, "wall_us": 20})
            telemetry.write_text(row + "\n" + row + "\n")
            log.write_text("")
            with self.assertRaisesRegex(ValueError, "duplicate"):
                module.report(telemetry, log)
            telemetry.write_text(row)
            with self.assertRaisesRegex(ValueError, "no checkpoint captures"):
                module.report(telemetry, log)

    def test_rejects_partial_phase_boundary(self):
        with self.assertRaisesRegex(ValueError, "invalid phase boundaries"):
            module.phase_summary({}, 100, None)


if __name__ == "__main__":
    unittest.main()
