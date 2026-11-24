"""Unit tests for the battle-live gate's pure parts: marker parsing, fall distance, scenario gate."""

from __future__ import annotations

import unittest
from pathlib import Path

from tools.harness.backends import battle_live
from tools.harness.profiles import PROFILES, ProfileContext


def context(scenario: str, *extra: str) -> ProfileContext:
    return ProfileContext(artifact_dir=Path("/tmp/unused"), scenario_id=scenario, extra_args=list(extra))


LINE = ("2026-11-24T21:00:00.000000Z INFO mc_core::battle::live_players: Battle live respawned: PovRed "
        "side=red regiment=4 file=29 rank=0 at=30.73,94.00,37.83 yaw=12.5 kit_slots=4 kit_from=durable "
        "deaths=1 contact=true changed=true reason=none rule=contact")


class MarkerTest(unittest.TestCase):
    def test_respawn_coordinates_and_contact_flag(self) -> None:
        self.assertEqual(battle_live._parse_at(LINE), [30.73, 94.0, 37.83])
        self.assertIsNotNone(battle_live.CONTACT_TRUE.search(LINE))
        self.assertFalse(bool(battle_live.CONTACT_TRUE.search(LINE.replace("contact=true", "contact=false"))))
        self.assertIsNone(battle_live._parse_at("Battle live respawned: PovRed at=none"))

    def test_fall_distance_needs_both_positions(self) -> None:
        before = {"player": {"x": 1.0, "y": 64.0, "z": 2.0}}
        after = {"player": {"x": 4.0, "y": 64.0, "z": 6.0}}
        self.assertEqual(battle_live._fall_distance(before, after), 5.0)
        self.assertIsNone(battle_live._fall_distance({"player": {"entity_id": 3}}, after))
        self.assertIsNone(battle_live._fall_distance(before, {"player": None}))

    def test_the_profile_and_scenarios_are_registered(self) -> None:
        self.assertIn("battle-live", PROFILES)
        self.assertEqual(battle_live.scenario_ids(),
                         ("battle-live-respawn-line-contact", "battle-live-spectator-follows-respawn"))

    def test_only_its_named_scenarios_run(self) -> None:
        with self.assertRaisesRegex(ValueError, "battle-live runs only"):
            battle_live.run(context("battle-kit-census"))
        with self.assertRaisesRegex(ValueError, "battle-live runs only"):
            battle_live.run(context("battle-live-respawn-line-contact", "--extra"))

    def test_a_report_only_passes_with_every_assertion(self) -> None:
        report: dict = {"assertions": {}}
        battle_live.check(report, "a", True)
        self.assertTrue(all(report["assertions"].values()))
        battle_live.check(report, "b", False, detail=3)
        self.assertFalse(all(report["assertions"].values()))
        self.assertEqual(report["observations"]["b"], 3)


if __name__ == "__main__":
    unittest.main()
