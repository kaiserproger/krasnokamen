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

    def test_says_after_reads_recent_chat_newest_first(self) -> None:
        # The client lists recent_chat newest first: the count line is the newest, the fence is older.
        chat = ["3 more datapack warning(s), see the server log", "Skipping new datapack file/nometa",
                "Reloading!", "Round2Begin", "Skipping new datapack file/brokenzip.zip", "Reloading!"]
        self.assertTrue(datapack_reload.chat_says_after(chat, "Round2Begin", "more datapack warning(s)"))
        self.assertTrue(datapack_reload.chat_says_after(chat, "Round2Begin", "Skipping new datapack file/nometa"))
        # A line from before the fence (older, so later in the list) never counts.
        self.assertFalse(datapack_reload.chat_says_after(chat, "Round2Begin", "Skipping new datapack file/brokenzip.zip"))
        self.assertFalse(datapack_reload.chat_says_after(chat[1:], "Round2Begin", "more datapack warning(s)"))
        self.assertFalse(datapack_reload.chat_says_after(chat[:3], "Round2Begin", "Reloading!x"))
        self.assertFalse(datapack_reload.chat_says_after(chat[:3], "Round2Begin", "Reloading!"))


if __name__ == "__main__":
    unittest.main()
