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

    def test_camera_gap_measures_against_the_followed_eye(self) -> None:
        camera = {"entity": {"position": [4.0, 65.62, 6.0]}}
        self.assertEqual(battle_live._camera_gap(camera, {"player": {"eye": [4.0, 65.62, 6.0]}}), 0.0)
        # A camera left on the old body is far behind the holder it should follow.
        self.assertGreater(battle_live._camera_gap(camera, {"player": {"block_pos": [204.0, 64.0, 6.0]}}),
                           battle_live.CAMERA_FOLLOW_BLOCKS)
        self.assertIsNone(battle_live._camera_gap({"entity": {}}, {"player": {"eye": [0.0, 0.0, 0.0]}}))
        self.assertIsNone(battle_live._camera_gap(camera, {"player": None}))

    def test_point_prefers_the_block_position(self) -> None:
        self.assertEqual(battle_live._point({"player": {"block_pos": [1, 2, 3], "x": 9.0}}), [1.0, 2.0, 3.0])
        self.assertEqual(battle_live._point({"player": {"x": 1.0, "y": 64.0, "z": 2.0}}), [1.0, 64.0, 2.0])
        self.assertIsNone(battle_live._point({"player": {"health": 20.0}}))

    def test_the_respawn_assertions_are_named_for_the_rows(self) -> None:
        source = Path(battle_live.__file__).read_text()
        for name in ("respawn_is_in_contact", "respawn_is_not_the_side_centre",
                     "respawn_near_where_he_fell", "spectator_camera_followed_the_respawn"):
            self.assertIn(name, source)
        # The respawn is the client's own action, as in the crash-recovery kit.
        self.assertIn('holder.driver("respawn")', source)

    def test_judge_fails_the_step_on_any_failed_assertion(self) -> None:
        good = {"assertions": {"a": True, "b": True}, "passed": True}
        battle_live.judge(good)
        self.assertTrue(good["passed"])
        for bad in ({"assertions": {"a": True, "b": False}},
                    {"assertions": {"a": True}, "incomplete": ["no position"]},
                    {"assertions": {}}):
            with self.assertRaisesRegex(RuntimeError, "assertions failed"):
                battle_live.judge(dict(bad))

    def test_the_respawn_witness_reads_the_wal_archive(self) -> None:
        source = Path(battle_live.__file__).read_text()
        self.assertIn('"--crash-handoff"', source)
        self.assertIn("_respawn_enemy_witness", source)
        self.assertIn("holder_sees_an_enemy_after_respawn", source)
        missing = battle_live._respawn_enemy_witness(Path("/nonexistent-run"))
        self.assertFalse(missing["archive_exists"])
        self.assertIsNone(missing["min_enemy_distance"])

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
