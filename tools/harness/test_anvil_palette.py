"""Synthetic chunk palette codec and audit contracts."""
import unittest
from pathlib import Path
from unittest import mock

from .backends import anvil_corrupt as nbt
from .quirk_parity import anvil


def chunk(version, name_key, properties_key, *, mixed=False):
    palette = [(name_key, (nbt.STRING, "minecraft:oak_log")),
               (properties_key, (nbt.COMPOUND, [("axis", (nbt.STRING, "x"))]))]
    if mixed:
        palette.append(("Name" if name_key == "id" else "id", (nbt.STRING, "minecraft:stone")))
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

    def test_wrong_or_mixed_spelling_fails_with_chunk_and_section(self):
        for version, name, props, mixed in ((4790, "id", "properties", False),
                                             (5023, "Name", "Properties", False),
                                             (5023, "id", "properties", True),
                                             (4790, "Name", "Properties", True)):
            with self.subTest(version=version, name=name, mixed=mixed):
                data = chunk(version, name, props, mixed=mixed)
                with mock.patch.object(anvil, "_regions", return_value=iter([((0, 0), data)])):
                    with self.assertRaisesRegex(ValueError, r"chunk 0,0 section 0: palette entry"):
                        anvil.read_world(Path("/synthetic"), [[0, 0, 0, 0, 0, 0]])

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

    def test_unknown_version_fails_closed(self):
        with mock.patch.object(anvil, "_regions", return_value=iter([((0, 0), chunk(4900, "Name", "Properties"))])):
            with self.assertRaisesRegex(ValueError, "chunk 0,0: unsupported chunk DataVersion 4900"):
                anvil.read_world(Path("/synthetic"), [[0, 0, 0, 0, 0, 0]])


if __name__ == "__main__":
    unittest.main()
