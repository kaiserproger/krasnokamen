"""Authored fixtures only: generated-table provenance and real menu projection."""
import copy
import os
import json
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from . import atm_horizon, atm_loot
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
            proof = atm_loot.container_candidates(site, pack, probe)[0]
            self.assertEqual(proof["position"], [0, 64, 0])
            self.assertEqual(proof["saved_block_entity"]["LootTableSeed"], 17)
            self.assertEqual(proof["allowed_items"], ["minecraft:iron_ingot"])
            for update in ({"LootTable": "demo:wrong"}, {"Items": [{"id": "minecraft:iron_ingot", "count": 7}]}, {"x": 100}):
                bad = copy.deepcopy(chunk)
                bad["block_entities"][0].update(update)
                region(region_file, bad)
                with self.assertRaisesRegex(ValueError, "no reachable generated unopened"):
                    atm_loot.container_candidates(site, pack, probe)

    def test_crystal_barrel_standable_diagonals_above_waterlogged_stairs(self):
        # Crystal lower.nbt: local (6,0,7), translated to the counted world.
        x, y, z = 3504, 60, -5057
        blocks = {(x, y, z): "minecraft:barrel", (x, y + 1, z): "minecraft:water"}
        for dx, dz in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            blocks[x + dx, y, z + dz] = "minecraft:stone"  # terrain in absent y=0 cells
            blocks[x + dx, y + 1, z + dz] = "minecraft:stone_brick_stairs"
        for dx in (-1, 1):
            for dz in (-1, 1):
                blocks[x + dx, y, z + dz] = "minecraft:stone"
                blocks[x + dx, y + 1, z + dz] = "minecraft:polished_andesite"
        block = lambda pos: blocks.get(tuple(pos), "minecraft:air")
        stands = atm_loot.stand_positions([x, y, z], block)
        self.assertIn([3503.5, 62, -5057.5], stands)
        self.assertNotIn([3504.5, 60, -5057.5], stands)
        self.assertNotIn([3505.5, 61, -5056.5], stands)

    def test_clip_ray_distance_and_next_container_are_required(self):
        class Client:
            def __init__(self):
                self.receipt = {}
                self.mode = "spectator"
                self.position = None
                self.target = None

            def command(self, text, _response):
                self.mode = text.split()[1]

            def teleport(self, pos):
                self.position = pos

            def call(self, command, *args):
                if command == "state":
                    return {"gamemode": self.mode, "player": {"position": self.position, "on_ground": True}}
                if command == "look-at":
                    self.target = args
                    return {}
                if command == "looking-at":
                    if self.position[0] == 0.5:
                        return {"kind": "block", "block": {"pos": [1, 1, 0], "distance": 1, "face": "west"}}
                    return {"kind": "block", "block": {"pos": [4, 0, 0], "distance": 4.5, "face": "up"}}
                return {}

        client = Client()
        rows = [{"position": [0, 0, 0], "stands": [[0.5, 1, 0.5]]},
                {"position": [4, 0, 0], "stands": [[4.5, 1, 0.5]]}]
        chosen = atm_loot.select_reachable(client, rows)
        self.assertEqual(chosen["position"], [4, 0, 0])
        self.assertEqual(client.receipt["container_selection"]["candidate_order"], [[0, 0, 0], [4, 0, 0]])
        self.assertEqual(len(chosen["selection_attempts"]), 2)
        with patch.object(client, "call", return_value={"kind": "block", "block": {
            "pos": [4, 0, 0], "distance": 4.5001, "face": "up"}}):
            self.assertIsNone(atm_loot.visible_face(client, [4, 0, 0]))

    def test_control_socket_is_short_private_and_removed_on_failure(self):
        with self.assertRaisesRegex(RuntimeError, "test failure"):
            with atm_horizon.control_socket() as socket:
                directory = socket.parent
                self.assertLess(len(os.fsencode(socket)), 108)
                self.assertEqual(directory.stat().st_mode & 0o777, 0o700)
                socket.write_text("temporary")
                raise RuntimeError("test failure")
        self.assertFalse(directory.exists())

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
