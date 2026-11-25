"""Synthetic contracts for the saved-gravity-tick Anvil fixture editor."""
import struct
import tempfile
import unittest
import zlib
from pathlib import Path

from .backends import anvil_corrupt as nbt
from .backends import anvil_scheduled_tick as fixture


NAME = "minecraft:"
AIR, STONE, SAND = "minecraft:air", "minecraft:stone", "minecraft:sand"


def _palette(names):
    return [("Name", (nbt.STRING, name)) for name in names]


def _pack(cells, palette_size):
    """The vanilla 1.16+ block-state array for a dense cell list."""
    bits = max(4, (palette_size - 1).bit_length())
    per_long = 64 // bits
    longs = [0] * ((len(cells) + per_long - 1) // per_long)
    for cell, value in enumerate(cells):
        slot, shift = divmod(cell, per_long)
        longs[slot] |= value << (shift * bits)
    return struct.pack(f">{len(longs)}q", *longs)


def chunk(section_y, cells, palette, ticks=(), x_pos=0, z_pos=0):
    section = [
        ("Y", (nbt.BYTE, section_y)),
        ("block_states", (nbt.COMPOUND, [
            ("palette", (nbt.LIST, (nbt.COMPOUND, [_palette([name]) for name in palette]))),
            ("data", (nbt.LONG_ARRAY, (nbt.LONG_ARRAY, _pack(cells, len(palette))))),
        ])),
    ]
    root = [
        ("DataVersion", (nbt.INT, 4790)),
        ("xPos", (nbt.INT, x_pos)),
        ("zPos", (nbt.INT, z_pos)),
        ("yPos", (nbt.INT, -4)),
        ("Status", (nbt.STRING, "minecraft:full")),
        ("sections", (nbt.LIST, (nbt.COMPOUND, [section]))),
        ("block_ticks", (nbt.LIST, (nbt.COMPOUND, list(ticks)))),
    ]
    return root


def write_world(directory, root):
    region = directory / "dimensions/minecraft/overworld/region"
    region.mkdir(parents=True, exist_ok=True)
    path = region / "r.0.0.mca"
    nbt._write_region(path, {0: (zlib.compress(nbt.serialize("", (nbt.COMPOUND, root))), 0)})
    return directory


def planted_cells(palette, cells):
    return _pack(cells, len(palette))


class ScheduledTickFixtureTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        directory = Path(self.tmp.name) / "world-export-0"
        palette = [AIR, STONE, SAND]
        # Section 0 (y 0..15): stone support above the sand, sand on top of it.
        cells = [0] * 4096
        cells[fixture._cell(3, 7, 3)] = 1
        cells[fixture._cell(3, 8, 3)] = 1
        cells[fixture._cell(3, 9, 3)] = 2
        self.root = chunk(0, cells, palette)
        self.world = write_world(directory, self.root)
        self.clear = (3, 8, 3)
        self.tick = (3, 9, 3)

    def _read(self):
        region = self.world / "dimensions/minecraft/overworld/region/r.0.0.mca"
        chunks = nbt._read_region(region)
        return nbt.parse(chunks[0][1])[1]

    def _sections(self, root):
        return nbt.field(root[1], "sections")[1][1]

    def _names(self, section):
        return fixture._palette_names(fixture._palette(nbt.field(section, "block_states")[1]))

    def test_clear_cell_become_air_and_the_tick_is_appended(self):
        report = fixture.plant_due_gravity_tick(
            self.world, clear=self.clear, tick=self.tick, delay=600)
        root = self._read()
        section = self._sections(root)[0]
        names = self._names(section)
        self.assertEqual(names[fixture.read_cell(section, *self.clear)], AIR)
        self.assertEqual(names[fixture.read_cell(section, *self.tick)], SAND)
        self.assertEqual(fixture.read_cell(section, 3, 7, 3), names.index(STONE))
        entries = nbt.field(root[1], "block_ticks")[1][1]
        self.assertEqual(len(entries), 1)
        values = {key: tag[1] for key, tag in entries[0]}
        self.assertEqual(values, {"i": SAND, "x": 3, "y": 9, "z": 3, "t": 600, "p": 0})
        self.assertEqual(report["block_ticks_before"], 0)
        self.assertEqual(report["cleared"]["palette"], [1, 0])
        self.assertEqual(report["tick"], {"i": SAND, "x": 3, "y": 9, "z": 3,
                                          "t": 600, "p": 0, "palette": 2})

    def test_second_cell_of_the_same_section_keeps_its_value(self):
        fixture.plant_due_gravity_tick(self.world, clear=self.clear, tick=self.tick)
        section = self._sections(self._read())[0]
        names = self._names(section)
        # The support below the cleared cell and the rest of the palette survive.
        self.assertEqual(names[fixture.read_cell(section, 3, 7, 3)], STONE)
        self.assertEqual(fixture.read_cell(section, 0, 0, 0), 0)

    def test_existing_list_keeps_its_entries(self):
        palette = [AIR, STONE, SAND]
        cells = [0] * 4096
        cells[fixture._cell(3, 8, 3)] = 1
        cells[fixture._cell(3, 9, 3)] = 2
        kept = [fixture.tick_entry((1, 1, 1), STONE, 4, 1)]
        world = write_world(Path(self.tmp.name) / "second", chunk(0, cells, palette, ticks=kept))
        report = fixture.plant_due_gravity_tick(world, clear=self.clear, tick=self.tick)
        chunks = nbt._read_region(world / "dimensions/minecraft/overworld/region/r.0.0.mca")
        root = nbt.parse(chunks[0][1])[1]
        entries = nbt.field(root[1], "block_ticks")[1][1]
        self.assertEqual(report["block_ticks_before"], 1)
        self.assertEqual([{key: tag[1] for key, tag in entry}["y"] for entry in entries], [1, 9])

    def test_region_with_several_chunks_keeps_the_others(self):
        palette = [AIR, STONE, SAND]
        cells = [0] * 4096
        cells[fixture._cell(3, 8, 3)] = 1
        cells[fixture._cell(3, 9, 3)] = 2
        world = Path(self.tmp.name) / "many"
        region = world / "dimensions/minecraft/overworld/region"
        region.mkdir(parents=True)
        chunks = {}
        for index, (x_pos, z_pos) in enumerate(((0, 0), (1, 0), (0, 1))):
            root = chunk(0, cells, palette, x_pos=x_pos, z_pos=z_pos)
            chunks[index] = (zlib.compress(nbt.serialize("", (nbt.COMPOUND, root))), index)
        nbt._write_region(region / "r.0.0.mca", chunks)
        fixture.plant_due_gravity_tick(world, clear=self.clear, tick=self.tick)
        re_read = nbt._read_region(region / "r.0.0.mca")
        self.assertEqual(len(re_read), 3)
        for index, position in ((1, (1, 0)), (2, (0, 1))):
            _, body, _ = re_read[index]
            root = nbt.parse(body)[1]
            self.assertEqual((nbt.field(root[1], "xPos")[1], nbt.field(root[1], "zPos")[1]), position)
            self.assertEqual(len(nbt.field(root[1], "block_ticks")[1][1]), 0)
        _, body, _ = re_read[0]
        self.assertEqual(len(nbt.field(nbt.parse(body)[1][1], "block_ticks")[1][1]), 1)

    def test_chunk_outside_the_world_is_refused(self):
        with self.assertRaises(RuntimeError):
            fixture.plant_due_gravity_tick(self.world, clear=(80, 8, 3), tick=(80, 9, 3))

    def test_a_cell_holding_another_block_is_refused(self):
        with self.assertRaises(RuntimeError):
            fixture.plant_due_gravity_tick(self.world, clear=(3, 9, 3), tick=self.tick)

    def test_a_tick_cell_without_the_block_is_refused(self):
        with self.assertRaises(RuntimeError):
            fixture.plant_due_gravity_tick(self.world, clear=self.clear, tick=(4, 9, 4))

    def test_negative_section_coordinates_round_trip(self):
        palette = [AIR, STONE, SAND]
        cells = [0] * 4096
        cells[fixture._cell(3, 13, 3)] = 1
        cells[fixture._cell(3, 14, 3)] = 1
        cells[fixture._cell(3, 15, 3)] = 2
        root = chunk(-1, cells, palette)
        world = write_world(Path(self.tmp.name) / "negative", root)
        fixture.plant_due_gravity_tick(world, clear=(3, -2, 3), tick=(3, -1, 3))
        chunks = nbt._read_region(world / "dimensions/minecraft/overworld/region/r.0.0.mca")
        section = self._sections(nbt.parse(chunks[0][1])[1])[0]
        names = self._names(section)
        self.assertEqual(names[fixture.read_cell(section, 3, -2, 3)], AIR)
        self.assertEqual(names[fixture.read_cell(section, 3, -1, 3)], SAND)
        self.assertEqual(names[fixture.read_cell(section, 3, -3, 3)], STONE)

    def test_missing_air_index_is_refused(self):
        cells = [0] * 4096
        cells[fixture._cell(3, 8, 3)] = 0
        cells[fixture._cell(3, 9, 3)] = 1
        world = write_world(Path(self.tmp.name) / "noair", chunk(0, cells, [STONE, SAND]))
        with self.assertRaises(ValueError):
            fixture.plant_due_gravity_tick(world, clear=self.clear, tick=self.tick)


if __name__ == "__main__":
    unittest.main()
