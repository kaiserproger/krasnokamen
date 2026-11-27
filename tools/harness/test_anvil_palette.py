"""Synthetic chunk palette codec and audit contracts."""
import struct
import unittest
from pathlib import Path
from unittest import mock

from .backends import anvil_corrupt as nbt
from .quirk_parity import anvil


def chunk(version, name_key, properties_key):
    palette = [(name_key, (nbt.STRING, "minecraft:oak_log")),
               (properties_key, (nbt.COMPOUND, [("axis", (nbt.STRING, "x"))]))]
    root = [("DataVersion", (nbt.INT, version)),
            ("sections", (nbt.LIST, (nbt.COMPOUND, [[
                ("Y", (nbt.BYTE, 0)),
                ("block_states", (nbt.COMPOUND, [
                    ("palette", (nbt.LIST, (nbt.COMPOUND, [palette]))),
                ])),
            ]])))]
    return anvil.parse_nbt(nbt.serialize("", (nbt.COMPOUND, root)))


class AnvilPaletteTests(unittest.TestCase):
    def test_mixed_version_world_decodes_and_audits_chunk_counts(self):
        chunks = [((0, 0), chunk(4790, "Name", "Properties")),
                  ((1, 0), chunk(5023, "id", "properties"))]
        audit = {}
        with mock.patch.object(anvil, "_regions", side_effect=lambda directory, wanted: iter(chunks) if directory.name == "region" else iter(())):
            result = anvil.read_world(Path("/synthetic"), [[0, 0, 0, 31, 0, 0]], palette_audit=audit)
        self.assertEqual(result[0]["blocks"]["0,0,0"], "minecraft:oak_log[axis=x]")
        self.assertEqual(result[0]["blocks"]["16,0,0"], "minecraft:oak_log[axis=x]")
        self.assertEqual(audit, {"4790": {"encoding": "Name/Properties", "chunks": 1},
                                 "5023": {"encoding": "id/properties", "chunks": 1}})

    def test_5023_bare_identifier_palette(self):
        root = [("DataVersion", (nbt.INT, 5023)),
                ("sections", (nbt.LIST, (nbt.COMPOUND, [[
                    ("Y", (nbt.BYTE, 0)),
                    ("block_states", (nbt.COMPOUND, [
                        ("palette", (nbt.LIST, (nbt.STRING, ["minecraft:stone"]))),
                    ])),
                ]])))]
        data = anvil.parse_nbt(nbt.serialize("", (nbt.COMPOUND, root)))
        with mock.patch.dict(anvil._DEFAULTS, {5023: {"minecraft:stone": "minecraft:stone"}}), \
                mock.patch.object(anvil, "_regions", side_effect=lambda directory, wanted: iter([((0, 0), data)]) if directory.name == "region" else iter(())):
            result = anvil.read_world(Path("/synthetic"), [[0, 0, 0, 0, 0, 0]])
        self.assertEqual(result[0]["blocks"]["0,0,0"], "minecraft:stone")

    def test_5023_mixed_palette_expands_default_states(self):
        # Real 26.3 vanilla save (redstone-lever-lamp, 2026-11-27): a palette mixing bare ids and
        # compounds is a compound list, bare elements wrapped in one empty-named entry, and a
        # block in its default state is the bare id: `{'': 'minecraft:redstone_lamp'}` is lit=false.
        palette = [[("", (nbt.STRING, "minecraft:air"))],
                   [("id", (nbt.STRING, "minecraft:lever")),
                    ("properties", (nbt.COMPOUND, [("face", (nbt.STRING, "floor")),
                                                   ("powered", (nbt.STRING, "false")),
                                                   ("facing", (nbt.STRING, "north"))]))],
                   [("", (nbt.STRING, "minecraft:redstone_lamp"))]]
        root = [("DataVersion", (nbt.INT, 5023)),
                ("sections", (nbt.LIST, (nbt.COMPOUND, [[
                    ("Y", (nbt.BYTE, 0)),
                    ("block_states", (nbt.COMPOUND, [
                        ("palette", (nbt.LIST, (nbt.COMPOUND, palette))),
                        ("data", (nbt.LONG_ARRAY, (None, struct.pack(">256q", (2 << 8) | (1 << 4), *[0] * 255)))),
                    ])),
                ]])))]
        data = anvil.parse_nbt(nbt.serialize("", (nbt.COMPOUND, root)))
        defaults = {"minecraft:air": "minecraft:air", "minecraft:redstone_lamp": "minecraft:redstone_lamp[lit=false]",
                    "minecraft:lever": "minecraft:lever[face=wall,facing=north,powered=false]"}
        with mock.patch.dict(anvil._DEFAULTS, {5023: defaults}), \
                mock.patch.object(anvil, "_regions", side_effect=lambda directory, wanted: iter([((0, 0), data)]) if directory.name == "region" else iter(())):
            result = anvil.read_world(Path("/synthetic"), [[0, 0, 0, 2, 0, 0]])
        self.assertEqual(result[0]["blocks"], {"1,0,0": "minecraft:lever[face=floor,facing=north,powered=false]",
                                               "2,0,0": "minecraft:redstone_lamp[lit=false]"})

    def test_5023_unknown_bare_block_fails_closed(self):
        with mock.patch.dict(anvil._DEFAULTS, {5023: {}}):
            with self.assertRaisesRegex(ValueError, "not in the DataVersion 5023 blocks report"):
                anvil._block_string({"": "minecraft:redstone_lamp"}, ("id", "properties"), 5023)


if __name__ == "__main__":
    unittest.main()
