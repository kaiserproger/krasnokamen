import json
import tempfile
import unittest
from pathlib import Path

from tools.harness.backends import mob_chase_oracle


def sample(tick, x, z, distance, flags=4, uuid="z"):
    return {
        "game_time": tick,
        "snapshot": {"entities": [
            {"entity_type": "minecraft:zombie", "entity_uuid": uuid, "x": x, "z": z,
             "distance": distance, "received_metadata": {"15": {"kind": "byte", "value": flags}}},
        ]},
    }


class MobChaseOracleTests(unittest.TestCase):
    def test_one_rate_over_the_whole_window(self):
        # 7 blocks in 60 client ticks: the 3-tick server sample period must not change the reading.
        reading = mob_chase_oracle.measure(
            [sample(100, 0., 0., 20.), sample(130, 3.5, 0., 16.5), sample(160, 7., 0., 13.)],
            "minecraft:zombie")
        self.assertEqual(reading["elapsed_client_ticks"], 60)
        self.assertAlmostEqual(reading["blocks_per_client_tick"], 7. / 60., places=9)
        self.assertAlmostEqual(reading["approach_blocks"], 7., places=9)
        self.assertTrue(reading["identity_constant"])
        self.assertEqual((reading["flags_first"], reading["flags_last"]), (4, 4))

    def test_identity_and_distance_are_required(self):
        with self.assertRaisesRegex(ValueError, "at least two samples"):
            mob_chase_oracle.measure([sample(100, 0., 0., 20.)], "minecraft:zombie")
        reading = mob_chase_oracle.measure(
            [sample(100, 0., 0., 20.), sample(160, 7., 0., 13., uuid="other")], "minecraft:zombie")
        self.assertFalse(reading["identity_constant"])
        two = sample(160, 7., 0., 13.)
        two["snapshot"]["entities"].append({"entity_type": "minecraft:zombie", "entity_uuid": "y",
                                            "x": 0., "z": 0., "distance": 1.})
        with self.assertRaisesRegex(ValueError, "exactly one"):
            mob_chase_oracle.measure([sample(100, 0., 0., 20.), two], "minecraft:zombie")
        missing = sample(100, 0., 0., 20.)
        del missing["snapshot"]["entities"][0]["distance"]
        with self.assertRaisesRegex(ValueError, "distance"):
            mob_chase_oracle.measure([missing, sample(160, 7., 0., 13.)], "minecraft:zombie")

    def test_missing_or_non_byte_flags_read_as_zero(self):
        row = sample(100, 0., 0., 20.)
        row["snapshot"]["entities"][0]["received_metadata"] = {"15": {"kind": "int", "value": 4}}
        reading = mob_chase_oracle.measure([row, sample(160, 7., 0., 13., flags=0)], "minecraft:zombie")
        self.assertEqual((reading["flags_first"], reading["flags_last"]), (0, 0))


class MobChaseStepTests(unittest.TestCase):
    class Probe:
        def __init__(self):
            self.checks = {}

        def check(self, name, value):
            self.checks[name] = bool(value)

    def run_step(self, step, samples):
        from tools.harness.backends import scene_shot

        step = {**step, "samples": list(samples)}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name, reading in samples.items():
                (root / f"{name}.json").write_text(json.dumps(reading))
            probe = self.Probe()
            scene_shot.execute(probe, root, [{"mob_chase": step}])
        return probe.checks

    def test_all_four_verdicts_come_from_one_long_window(self):
        step = {"samples": ["a", "b", "c"], "name": "chase", "kind": "minecraft:zombie",
                "min_speed": 0.10, "max_speed": 0.13, "min_approach_blocks": 3, "aggressive_bit": 2}
        good = self.run_step(step, {"a": sample(100, 0., 0., 20.), "b": sample(130, 3.5, 0., 16.5),
                                    "c": sample(160, 7., 0., 13.)})
        self.assertEqual(good, {"chase_speed_zombie": True, "chase_approach_zombie": True,
                                "chase_aggressive_zombie": True, "chase_identity_zombie": True})
        # The historical 3.8 speed: too fast for the band.
        fast = self.run_step(step, {"a": sample(100, 0., 0., 20.), "c": sample(160, 11., 0., 9.)})
        self.assertFalse(fast["chase_speed_zombie"])
        # A wanderer that keeps pace but never closes on the player, with no aggressive bit.
        wander = self.run_step(step, {"a": sample(100, 0., 0., 20., flags=0), "c": sample(160, 7., 0., 20., flags=0)})
        self.assertTrue(wander["chase_speed_zombie"])
        self.assertFalse(wander["chase_approach_zombie"])
        self.assertFalse(wander["chase_aggressive_zombie"])

    def test_the_step_needs_a_native_client_and_a_band(self):
        from tools.harness.backends import scene_shot

        good = {"samples": ["a", "b"], "name": "chase", "kind": "minecraft:zombie",
                "min_speed": 0.10, "max_speed": 0.13, "min_approach_blocks": 3, "aggressive_bit": 2}
        scene_shot.validate_steps([{"mob_chase": good}], "native")
        for bad in ({**good, "min_speed": 0.14}, {**good, "samples": ["a"]},
                    {**good, "aggressive_bit": 9}, {**good, "kind": "zombie"}, {**good, "extra": 1}):
            with self.subTest(step=bad), self.assertRaises(ValueError):
                scene_shot.validate_steps([{"mob_chase": bad}], "native")
        with self.assertRaises(ValueError):
            scene_shot.validate_steps([{"mob_chase": good}], "java")
