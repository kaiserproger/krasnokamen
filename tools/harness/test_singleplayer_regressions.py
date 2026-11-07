"""Focused checks for singleplayer regression registration and event setup."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools.harness import regressions, scenarios
from tools.harness.backends import singleplayer_regressions as sp


class SingleplayerRegressionTests(unittest.TestCase):
    def test_both_scenarios_are_graphical_native_ledger_checks(self):
        for name in (sp.FIRST, sp.LEASE):
            with self.subTest(name=name):
                entry = scenarios.SCENARIOS[name]
                self.assertEqual(entry.evidence, "graphical")
                self.assertEqual(entry.scenario_id, name)
                command = regressions._client_command(entry.profile, name)
                self.assertIn("--windowed", command)
                self.assertIn("native", command)

    def test_wait_path_does_not_create_unstarted_world(self):
        with tempfile.TemporaryDirectory() as temp:
            world = Path(temp) / "world"
            database = world / "native" / "state.redb"
            seen = []

            def appeared(path, timeout, condition):
                seen.append(path)
                self.assertFalse(path.exists())
                if path == database:
                    path.touch()
                else:
                    path.mkdir()
                self.assertTrue(condition()[0])

            with patch.object(sp.runtime, "wait_for_path_condition", appeared):
                sp._wait_path(database, 30)
            self.assertEqual(seen, [world, world / "native", database])


if __name__ == "__main__":
    unittest.main()
