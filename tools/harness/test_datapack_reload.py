"""Offline admission and registration checks for the datapack hot-reload gate."""
from pathlib import Path
import json
import tempfile
import unittest

from . import profiles, scenarios
from .backends import datapack_reload


class DatapackReloadTests(unittest.TestCase):
    def test_scenario_resolves_to_graphical_profile(self) -> None:
        scenario = scenarios.SCENARIOS["native-datapack-reload"]
        self.assertEqual(scenario.profile, "datapack-reload")
        self.assertEqual(scenario.evidence, "graphical")
        self.assertEqual(profiles.PROFILES[scenario.profile]["scope"], "real-client")
        self.assertEqual(profiles.PROFILES[scenario.profile]["run"], profiles.run_datapack_reload)

    def test_rejects_non_windowed_or_non_native_clients_before_launch(self) -> None:
        for client, windowed in (("java", True), ("native", False)):
            with self.subTest(client=client, windowed=windowed):
                ctx = profiles.ProfileContext(Path("."), client=client, windowed=windowed)
                with self.assertRaisesRegex(ValueError, "requires --client native --windowed"):
                    datapack_reload.run(ctx)

    def test_the_pack_replaces_the_bone_meal_recipe_and_is_a_valid_directory_pack(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = datapack_reload.write_pack(Path(directory))
            self.assertEqual(root, Path(directory) / "datapacks" / "gems")
            metadata = json.loads((root / "pack.mcmeta").read_text())
            self.assertEqual(metadata["pack"]["min_format"], metadata["pack"]["max_format"])
            recipe = json.loads((root / "data/minecraft/recipe/bone_meal.json").read_text())
            self.assertEqual(recipe["ingredients"], ["minecraft:bone"])
            self.assertEqual((recipe["result"]["id"], recipe["result"]["count"]),
                             datapack_reload.PACK_RESULT)
        self.assertNotEqual(datapack_reload.PACK_RESULT, datapack_reload.VANILLA_RESULT)


if __name__ == "__main__":
    unittest.main()
