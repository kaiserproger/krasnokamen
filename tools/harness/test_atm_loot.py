"""Authored fixtures only: generated-table provenance and real menu projection."""
import copy
import os
import json
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from . import atm_loot
from .backends import atm_structures
from .test_atm_carry import region


class ContainerProofTests(unittest.TestCase):
    def test_native_screenshot_gate_cannot_skip_required_loot_probe(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "result.json"
            manifest.write_text(json.dumps({"status": "converted-unproven", "packs": [
                {"container_loot_probe": {"block": "minecraft:barrel", "table": "demo:barrel"}}
            ]}))
            ctx = SimpleNamespace(client="native", windowed=True, extra_args=[], platform=None, timeout_seconds=30)
            with patch.dict(os.environ, {"KRASNOKAMEN_ATM_MANIFEST": str(manifest)}):
                with self.assertRaisesRegex(ValueError, "Horizon barrel-open"):
                    atm_structures.run(ctx)

    def test_saved_unopened_table_and_barrel_inside_start_are_required(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pack = root / "pack"
            table = pack / "data/demo/loot_table/barrel.json"
            table.parent.mkdir(parents=True)
            table.write_text(json.dumps({"pools": [{"entries": [{"type": "minecraft:item", "name": "minecraft:iron_ingot"}]}]}))
            entity = {"id": "minecraft:barrel", "x": 0, "y": 64, "z": 0, "LootTable": "demo:barrel", "LootTableSeed": 17}
            chunk = {"xPos": 0, "zPos": 0, "block_entities": [entity], "sections": [
                {"Y": 3, "block_states": {"palette": [{"Name": "minecraft:stone"}]}},
                {"Y": 4, "block_states": {"palette": [{"Name": "minecraft:air"}, {"Name": "minecraft:barrel"}], "data": [1] + [0]*255}},
            ]}
            region_file = root / "world/region/r.0.0.mca"
            region(region_file, chunk)
            site = {"id": "demo:structure", "anchor": [0, 64, 0], "bounds": [0, 64, 0, 8, 70, 8], "dimension_dir": str(root / "world")}
            probe = {"block": "minecraft:barrel", "table": "demo:barrel"}
            proof = atm_loot.container_candidate(site, pack, probe)
            self.assertEqual(proof["position"], [0, 64, 0])
            self.assertEqual(proof["saved_block_entity"]["LootTableSeed"], 17)
            self.assertEqual(proof["allowed_items"], ["minecraft:iron_ingot"])
            for update in ({"LootTable": "demo:wrong"}, {"Items": [{"id": "minecraft:iron_ingot", "count": 7}]}, {"x": 100}):
                bad = copy.deepcopy(chunk)
                bad["block_entities"][0].update(update)
                region(region_file, bad)
                with self.assertRaisesRegex(ValueError, "no reachable generated unopened"):
                    atm_loot.container_candidate(site, pack, probe)

    def test_player_inventory_empty_menu_and_unallowed_items_cannot_pass(self):
        slots = [{"slot": index, "item": None} for index in range(300, 327)]
        screen = {"screen": "container", "slots": [{"slot": 0, "item": "minecraft:iron_ingot", "count": 64}] + slots}
        with self.assertRaisesRegex(ValueError, "loot is empty"):
            atm_loot.rolled_slots(screen, ["minecraft:iron_ingot"])
        slots[0].update(item="minecraft:iron_ingot", count=7)
        self.assertEqual(len(atm_loot.rolled_slots(screen, ["minecraft:iron_ingot"])), 27)
        slots[0]["item"] = "minecraft:diamond"
        with self.assertRaisesRegex(ValueError, "outside its loot table"):
            atm_loot.rolled_slots(screen, ["minecraft:iron_ingot"])
        slots[0]["item"] = "minecraft:iron_ingot"
        slots[0]["count"] = 0
        with self.assertRaisesRegex(ValueError, "outside its loot table"):
            atm_loot.rolled_slots(screen, ["minecraft:iron_ingot"])
        screen["slots"].pop()
        with self.assertRaisesRegex(ValueError, "complete 27-slot"):
            atm_loot.rolled_slots(screen, ["minecraft:iron_ingot"])


if __name__ == "__main__":
    unittest.main()
