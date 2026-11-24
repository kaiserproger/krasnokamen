import copy
import random
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools.coreperf.constantinople import (
    FOUNDED_LINE, census, population_plan, nbt, stored_item_total, preservation, region_chunks,
)


def terrain_fixture():
    return {"beds": [], "workstations": [], "farmland": [{"position": [0, 63, -2], "above": "air"}],
            "outdoor": [[x, 64, z, "grass_block"] for x in range(-24, 25) for z in range(-24, 25)]}


class ConstantinopleTests(unittest.TestCase):
    def test_empty_region_placeholder_is_not_a_truncated_record(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "r.-1.-10.mca"
            path.write_bytes(b"")
            self.assertEqual(list(region_chunks(path)), [])
            path.write_bytes(b"\0")
            with self.assertRaisesRegex(ValueError, "truncated region"):
                list(region_chunks(path))

    def test_placement_is_seeded_and_input_order_independent(self):
        terrain = terrain_fixture()
        before = copy.deepcopy(terrain)
        first = population_plan(terrain, 12, 8, 3, 1453)
        random.Random(5).shuffle(terrain["outdoor"])
        second = population_plan(terrain, 12, 8, 3, 1453)
        self.assertEqual(first, second)
        self.assertEqual(before["beds"], terrain["beds"])
        self.assertEqual(first["seeded_farmland"], 1)
        self.assertNotEqual(first["villagers"], population_plan(before, 12, 8, 3, 1454)["villagers"])
        self.assertEqual(len({v["uuid"] for v in first["villagers"] + first["animals"]}), 20)
        self.assertEqual(len({tuple(v["home"]) for v in first["villagers"]}), 12)
        self.assertEqual(len({tuple(v["job_site"]) for v in first["villagers"]}), 12)

    def test_target_tops_up_preserved_villagers_and_does_not_steal_their_pois(self):
        terrain = terrain_fixture()
        terrain["existing_villagers"] = [{"uuid": "00000000-0000-0000-0000-000000000001",
                                          "position": [10.5, 64, 10.5], "home": [10, 64, 10],
                                          "job_site": [12, 64, 10]}]
        terrain["beds"] = [{"position": [10, 64, 10], "facing": "east"}]
        terrain["workstations"] = [{"position": [12, 64, 10], "profession": "farmer"}]
        result = population_plan(terrain, 5, 0, 1, 1453)
        self.assertEqual(result["target_villagers"], 5)
        self.assertEqual(len(result["existing_villagers"]), 1)
        self.assertEqual(len(result["villagers"]), 4)
        self.assertNotIn([10, 64, 10], [v["home"] for v in result["villagers"]])
        self.assertNotIn([12, 64, 10], [v["job_site"] for v in result["villagers"]])
        with self.assertRaisesRegex(ValueError, "below"):
            population_plan(terrain, 0, 0, 1, 1453)

    def test_reused_home_spawns_never_overlap_later_module_blocks(self):
        terrain = terrain_fixture()
        homes = [[x, 64, z] for x in (-16, -8, 0, 8, 16) for z in (-8, 8)]
        terrain["beds"] = [{"position": home, "facing": "east"} for home in homes]
        terrain["workstations"] = [{"position": [x + 2, y, z], "profession": "fisherman"}
                                   for x, y, z in homes]
        occupied = {(x + dx, z) for x, _, z in homes for dx in (-1, 0, 2)}
        terrain["outdoor"] = [p for p in terrain["outdoor"] if (p[0], p[2]) not in occupied]
        result = population_plan(terrain, 100, 0, 1, 0)
        spawns = {tuple(v["position"]) for v in result["villagers"]}
        self.assertEqual(len(spawns), 100)
        self.assertTrue(spawns.isdisjoint(tuple(v["job_site"]) for v in result["villagers"]))

    def test_existing_farmland_plants_are_not_obstructions(self):
        for plant in ("melon_stem", "attached_melon_stem", "pumpkin_stem",
                      "attached_pumpkin_stem", "torchflower_crop", "pitcher_crop"):
            with self.subTest(plant=plant):
                terrain = terrain_fixture()
                terrain["farmland"][0]["above"] = plant
                result = population_plan(terrain, 0, 0, 1, 1453)
                self.assertEqual(result["seeded_farmland"], 1)

    def test_census_requires_canonical_export_entity_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            world = Path(directory)
            (world / "region").mkdir()
            (world / "region/r.0.0.mca").write_bytes(bytes(8192))
            with self.assertRaisesRegex(ValueError, "entities"):
                census(world)

    def test_preservation_counts_legacy_and_modern_records_across_dimensions(self):
        identity = "00000000-0000-0000-0000-000000000007"
        player = {"UUID": [0, 0, 0, 7], "Inventory": [{"id": "minecraft:stone", "Count": 7}]}
        shadow = {"UUID": player["UUID"], "Inventory": [{"id": "minecraft:stone", "Count": 99}]}
        bodies = [{"id": "minecraft:villager", "Inventory": [{"id": "minecraft:bread", "Count": 3}],
                   "Passengers": [{"id": "minecraft:pig"}]}]
        blocks = [{"id": "minecraft:chest", "Items": [{"id": "minecraft:stone", "Count": 5}]},
                  {"id": "minecraft:sign"}]
        nether = {"block_entities": [{"id": "minecraft:chest",
                                      "Items": [{"id": "minecraft:stone", "count": 2}]}]}
        with tempfile.TemporaryDirectory() as directory:
            source, native = (Path(directory) / name for name in ("source", "native"))
            records = {
                source / "region/r.0.0.mca": {"Level": {"Entities": bodies, "TileEntities": blocks}},
                source / "DIM-1/region/r.0.0.mca": nether,
                native / "dimensions/minecraft/overworld/region/r.0.0.mca": {"block_entities": blocks},
                native / "dimensions/minecraft/overworld/entities/r.0.0.mca": {"Entities": bodies},
                native / "dimensions/minecraft/the_nether/region/r.0.0.mca": nether,
            }
            for path in records:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.touch()
            for world, level in ((source, b"embedded"), (native, b"empty")):
                (world / "level.dat").write_bytes(level)
            for world, name, raw in ((source, "playerdata", b"shadow"), (native, "players/data", b"player")):
                path = world / name / (identity + ".dat")
                path.parent.mkdir(parents=True)
                path.write_bytes(raw)
            decoded = {b"embedded": {"Data": {"Player": player}}, b"empty": {},
                       b"shadow": shadow, b"player": player}
            with patch("tools.coreperf.constantinople.nbt", side_effect=decoded.__getitem__), \
                 patch("tools.coreperf.constantinople.region_chunks",
                       side_effect=lambda path: iter([(0, 0, records[path])])):
                result = preservation(source, native)
                self.assertEqual(result["status"], "PASS")
                self.assertEqual(result["source"]["counts"], {"entities": 2, "block_entities": 3,
                                 "container_items": 10, "players": 1, "player_items": 7})
                decoded[b"player"] = {**player, "Inventory": [{"id": "minecraft:stone", "count": 6}]}
                result = preservation(source, native)
                self.assertEqual(result["status"], "FAIL")
                self.assertEqual(result["differences"]["player_items"], {"source": 7, "native": 6})
                self.assertIn("player_items_by_uuid", result["differences"])

    def test_preservation_counts_nested_containers_and_equipment_not_derived_items(self):
        old = {"Items": [{"id": "minecraft:shulker_box", "Count": 1, "tag": {
            "BlockEntityTag": {"Items": [{"id": "minecraft:stone", "Count": 8}]}}}],
               "ArmorItems": [{}, {"id": "minecraft:iron_helmet", "Count": 1}],
               "SelectedItem": {"id": "minecraft:stone", "Count": 64}}
        modern = {"Items": [{"id": "minecraft:shulker_box", "count": 1, "components": {
            "minecraft:container": [{"slot": 0, "item": {"id": "minecraft:stone", "count": 8}}]}}],
                  "equipment": {"head": {"id": "minecraft:iron_helmet", "count": 1}}}
        self.assertEqual(stored_item_total(old, "source"), 10)
        self.assertEqual(stored_item_total(modern, "native"), 10)
        with self.assertRaisesRegex(ValueError, r"chest\.Items\[0\]"):
            stored_item_total({"Items": [{"id": "minecraft:stone", "Count": "8"}]}, "chest")

    def test_preservation_counts_legacy_equipment_physical_items_once(self):
        armor = {"id": "minecraft:diamond_horse_armor", "Count": 1}
        saddle = {"id": "minecraft:saddle", "Count": 1}
        old = {"id": "minecraft:horse", "ArmorItem": armor, "SaddleItem": saddle,
               "ArmorItems": [{}, {}, armor, {}]}
        modern = {"id": "minecraft:horse", "equipment": {
            "body": {"id": "minecraft:diamond_horse_armor", "count": 1},
            "saddle": {"id": "minecraft:saddle", "count": 1}}}
        self.assertEqual(stored_item_total(old, "source"), 2)
        self.assertEqual(stored_item_total(modern, "native"), 2)
        distinct = {**old, "ArmorItems": [{}, {}, {"id": "minecraft:stone", "Count": 1}, {}]}
        self.assertEqual(stored_item_total(distinct, "distinct"), 3)
        self.assertEqual(stored_item_total({**old, "id": "mod:horse"}, "opaque"), 3)
        for kind in ("minecraft:pig", "minecraft:strider"):
            with self.subTest(kind=kind):
                self.assertEqual(stored_item_total({"id": kind, "Saddle": 1}, "legacy"), 1)
                self.assertEqual(stored_item_total({"id": kind, "Saddle": 0}, "unsaddled"), 0)
                self.assertEqual(stored_item_total({"id": kind, "Saddle": 1,
                    "equipment": {"saddle": {"id": "minecraft:saddle", "count": 1}}}, "modern"), 1)

    def test_routes_are_closed_cardinal_edges_and_avoid_authored_pois(self):
        result = population_plan(terrain_fixture(), 20, 8, 3, 1453)
        pois = {tuple(v[field]) for v in result["villagers"] for field in ("home", "job_site")}
        for route in result["street_routes"]:
            self.assertGreater(len(route), 2)
            for a, b in zip(route, route[1:] + route[:1]):
                self.assertEqual(abs(a[0] - b[0]) + abs(a[2] - b[2]), 1)
                self.assertLessEqual(abs(a[1] - b[1]), 1)
                self.assertNotIn(tuple(a), pois)
        self.assertEqual({a["kind"] for a in result["animals"]}, {"sheep", "chicken", "pig", "cow"})

    def test_obstructed_farmland_and_insufficient_space_fail_closed(self):
        terrain = terrain_fixture()
        terrain["farmland"][0]["above"] = "stone"
        with self.assertRaisesRegex(ValueError, "obstructed"):
            population_plan(terrain, 0, 0, 1, 1)
        terrain["outdoor"] = [[x, 64, 0, "stone"] for x in range(20)]
        terrain["farmland"] = []
        with self.assertRaisesRegex(ValueError, "only 0 villagers fit"):
            population_plan(terrain, 1, 0, 1, 1)

    def test_nbt_rejects_truncation_and_duplicate_names(self):
        self.assertEqual(nbt(b"\x0a\x00\x00\x00"), {})
        with self.assertRaises(ValueError):
            nbt(b"\x0a\x00\x00")
        with self.assertRaisesRegex(ValueError, "duplicate"):
            nbt(b"\x0a\x00\x00\x01\x00\x01a\x00\x01\x00\x01a\x00\x00")

    def test_the_founded_line_parser_matches_the_products_message(self):
        # The settle phase waits for `founded == names`: a message the regex does not match never
        # advances it (review D123 wave 2, M1). The product side is pinned by
        # `settlement_commands::the_founded_line_names_only_the_settlement_and_its_revision`.
        match = FOUNDED_LINE.search("Founded bqa_perimeter (revision 2).")
        self.assertIsNotNone(match)
        self.assertEqual(match[1], "bqa_perimeter")
        self.assertIsNone(FOUNDED_LINE.search("Founded bqa_perimeter (revision 2), wall ring 30."))


if __name__ == "__main__":
    unittest.main()
