import unittest

from tools.harness.backends.mob_motion_oracle import measure


def sample(tick, zombie, spider):
    return {"game_time": tick,
            "snapshot": {"entities": [
                {"entity_type": "minecraft:zombie", "entity_uuid": "z", "x": zombie, "z": 0},
                {"entity_type": "minecraft:spider", "entity_uuid": "s", "x": spider, "z": 0},
            ]}}


class MobMotionOracleTests(unittest.TestCase):
    def test_tick_rates_and_stable_identity(self):
        samples = [sample(100, 0, 0), sample(120, 2, 1), sample(140, 4, 2)]
        result = measure(samples, ["minecraft:zombie", "minecraft:spider"])
        self.assertEqual(result["minecraft:zombie"]["segments"][0]["elapsed_client_ticks"], 20)
        self.assertEqual(result["minecraft:spider"]["total_displacement_blocks"], 2)
        samples[2]["snapshot"]["entities"][1]["entity_uuid"] = "other"
        with self.assertRaisesRegex(ValueError, "identity changed"):
            measure(samples, ["minecraft:spider"])

    def test_nonadvancing_game_time_fails(self):
        with self.assertRaisesRegex(ValueError, "client game time did not advance"):
            measure([sample(100, 0, 0), sample(100, 1, 1), sample(120, 2, 2)], ["minecraft:zombie"])


def chase_sample(tick, z, yaw, vz=0.23):
    return {"game_time": tick, "snapshot": {"entities": [{
        "entity_type": "minecraft:zombie", "entity_uuid": "z",
        "x": 5.2, "y": 64.0, "z": z, "yaw": yaw,
        "velocity": {"x": 0.0, "y": 0.0, "z": vz},
    }]}}


class ChaseFacingOracleTests(unittest.TestCase):
    """Vanilla `MoveControl` turns the body toward the wanted point; the row's build turned it
    toward the resolved step, so on a deflected approach the two bearings differ by ~45-60
    degrees and a per-sample gap tells them apart."""

    player = [9.5, 71.0, 4.5]

    def wanted(self, z):
        from tools.harness.backends.mob_motion_oracle import bearing
        return bearing(self.player[0] - 5.2, self.player[2] - z)

    def test_wanted_bearing_passes_while_step_bearing_fails(self):
        from tools.harness.backends.mob_motion_oracle import chase_facing

        wanted = [chase_sample(100 + 3 * i, 0.5 + 0.1 * i, self.wanted(0.5 + 0.1 * i)) for i in range(5)]
        green = chase_facing(wanted, "minecraft:zombie", self.player)
        self.assertLess(green["max_gap_to_wanted_deg"], 15)
        self.assertGreater(green["max_step_deflection_deg"], 30)
        self.assertEqual(green["moving_samples"], 4)
        step = [chase_sample(100 + 3 * i, 0.5 + 0.1 * i, 0.0) for i in range(5)]
        red = chase_facing(step, "minecraft:zombie", self.player)
        self.assertGreater(red["max_gap_to_wanted_deg"], 30)

    def test_yaw_rate_counts_the_client_ticks_between_samples(self):
        from tools.harness.backends.mob_motion_oracle import chase_facing

        samples = [chase_sample(100, 0.5, 0.0), chase_sample(110, 0.5, 90.0), chase_sample(120, 0.5, 90.0)]
        result = chase_facing(samples, "minecraft:zombie", self.player)
        self.assertAlmostEqual(result["max_yaw_change_deg_per_tick"], 9.0)

    def test_requires_one_subject_and_advancing_time(self):
        from tools.harness.backends.mob_motion_oracle import chase_facing

        samples = [chase_sample(100 + 3 * i, 0.5, 0.0) for i in range(3)]
        samples[1]["snapshot"]["entities"].append(dict(samples[1]["snapshot"]["entities"][0], entity_uuid="other"))
        with self.assertRaisesRegex(ValueError, "expected exactly one"):
            chase_facing(samples, "minecraft:zombie", self.player)
        with self.assertRaisesRegex(ValueError, "did not advance"):
            chase_facing([chase_sample(100, 0.5, 0.0), chase_sample(100, 0.5, 0.0),
                          chase_sample(103, 0.5, 0.0)], "minecraft:zombie", self.player)
        with self.assertRaisesRegex(ValueError, "at least three samples"):
            chase_facing([chase_sample(100, 0.5, 0.0)], "minecraft:zombie", self.player)
