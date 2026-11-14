"""Offline admission and registration checks for the graphical composter gate."""
from pathlib import Path
import unittest

from . import profiles, scenarios
from .backends import composter_chance


class ComposterChanceTests(unittest.TestCase):
    def test_scenario_resolves_to_graphical_profile(self) -> None:
        scenario = scenarios.SCENARIOS["native-composter-chance"]
        self.assertEqual(scenario.profile, "composter-chance")
        self.assertEqual(profiles.PROFILES[scenario.profile]["scope"], "real-client")
        self.assertEqual(profiles.PROFILES[scenario.profile]["run"], profiles.run_composter_chance)

    def test_rejects_non_windowed_or_non_native_clients_before_launch(self) -> None:
        for client, windowed in (("java", True), ("native", False)):
            with self.subTest(client=client, windowed=windowed):
                ctx = profiles.ProfileContext(Path("."), client=client, windowed=windowed)
                with self.assertRaisesRegex(ValueError, "requires --client native --windowed"):
                    composter_chance.run(ctx)


if __name__ == "__main__":
    unittest.main()
