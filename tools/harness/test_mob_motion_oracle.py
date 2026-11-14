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
