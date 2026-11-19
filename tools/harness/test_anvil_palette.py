"""Synthetic chunk palette codec and audit contracts."""
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
        with mock.patch.object(anvil, "_regions", side_effect=lambda directory, wanted: iter([((0, 0), data)]) if directory.name == "region" else iter(())):
            result = anvil.read_world(Path("/synthetic"), [[0, 0, 0, 0, 0, 0]])
        self.assertEqual(result[0]["blocks"]["0,0,0"], "minecraft:stone")


if __name__ == "__main__":
    unittest.main()
