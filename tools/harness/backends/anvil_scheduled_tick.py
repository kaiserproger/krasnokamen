"""Plant a saved, due gravity block tick into an exported Anvil world, losslessly.

The HS-3 fixture needs world data a running native server never writes: an
unsupported sand block whose chunk `block_ticks` list carries a due tick for it.
Vanilla saves exactly that whenever it stops between a support removal and the
gravity tick, which is the report the row came from. The native engine instead
converts a gravity cell to a falling entity inside the placing write
(`falling::start`), and no code path schedules a gravity block tick, so a
`/setblock` fixture can never leave one behind - frozen or not.

This module therefore edits the world `world export-anvil` just wrote: it turns
the block under the sand into air and appends the sand's own `block_ticks`
entry. The `world import-anvil` that follows is the real route such a record
takes into a resident chunk (`mc-world-conversion/src/import.rs`), so the scene
tests the product's restore path rather than a command's behaviour.

Every other byte outside the edited cell and the appended entry is preserved:
the section palette is only indexed (no entry is added, so every other cell
keeps its value), the block-state array is re-packed at its own bit width, and
the entry is appended to the existing list, whose element type is kept.
"""
from __future__ import annotations

import struct
import zlib
from pathlib import Path
from typing import Any

from .anvil_corrupt import (
    COMPOUND,
    INT,
    LIST,
    LONG_ARRAY,
    STRING,
    _read_region,
    _write_region,
    field,
    parse,
    serialize,
)

# Vanilla packs block states as a minimum of four bits per cell, `64 // bits`
# cells per big-endian long, no entry spanning two longs (1.16+ layout).
MIN_BITS = 4
AIR = "minecraft:air"


def _palette(block_states: list[tuple[str, Any]]) -> list[list[tuple[str, Any]]]:
    return field(block_states, "palette")[1][1]


def _palette_names(palette: list[list[tuple[str, Any]]]) -> list[str]:
    return [str({key: tag[1] for key, tag in entry}.get("Name", "")) for entry in palette]


def _bits(palette_size: int) -> int:
    return max(MIN_BITS, (palette_size - 1).bit_length())


def _cell(x: int, y: int, z: int) -> int:
    return (y & 15) * 256 + (z & 15) * 16 + (x & 15)


def _raw(tag: Any, kind: int) -> bytes:
    """The bytes of an array tag; the codec stores them as (kind, bytes) inside the tag's payload."""
    value = tag[1]
    if isinstance(value, tuple) and value and value[0] == kind:
        return value[1]
    return value


def _longs(block_states: list[tuple[str, Any]]) -> list[int]:
    raw = _raw(field(block_states, "data"), LONG_ARRAY)
    return list(struct.unpack(f">{len(raw) // 8}q", raw))


def read_cell(section: list[tuple[str, Any]], x: int, y: int, z: int) -> int:
    """Palette index of one cell of a parsed section compound."""
    block_states = field(section, "block_states")[1]
    palette = _palette(block_states)
    if not any(key == "data" for key, _ in block_states):
        if len(palette) != 1:
            raise ValueError(f"section without block-state data carries {len(palette)} palette entries")
        return 0
    bits = _bits(len(palette))
    per_long = 64 // bits
    cell = _cell(x, y, z)
    value = (_longs(block_states)[cell // per_long] >> ((cell % per_long) * bits)) & ((1 << bits) - 1)
    return int(value)


def write_cell(section: list[tuple[str, Any]], x: int, y: int, z: int, index: int) -> None:
    """Point one cell of a parsed section compound at `index`; every other cell keeps its value."""
    block_states = field(section, "block_states")[1]
    palette = _palette(block_states)
    bits = _bits(len(palette))
    per_long = 64 // bits
    cell = _cell(x, y, z)
    slot, shift = divmod(cell, per_long)
    shift *= bits
    mask = (1 << bits) - 1
    if index & ~mask:
        raise ValueError(f"palette index {index} does not fit {bits} bits")
    longs = _longs(block_states)
    longs[slot] = (longs[slot] & ~(mask << shift)) | (index << shift)
    raw = struct.pack(f">{len(longs)}q", *longs)
    block_states[:] = [(key, (LONG_ARRAY, (LONG_ARRAY, raw)) if key == "data" else tag)
                       for key, tag in block_states]


def _section(sections: list[list[tuple[str, Any]]], y: int) -> list[tuple[str, Any]]:
    for section in sections:
        if field(section, "Y")[1] == y >> 4:
            return section
    raise RuntimeError(f"exported chunk has no section for y={y}")


def tick_entry(position: tuple[int, int, int], block: str, delay: int, priority: int) -> list[tuple[str, Any]]:
    """The chunk `block_ticks` element (vanilla `ScheduledTick` fields; `t` is a relative delay)."""
    x, y, z = position
    return [
        ("i", (STRING, block)),
        ("x", (INT, x)),
        ("y", (INT, y)),
        ("z", (INT, z)),
        ("t", (INT, delay)),
        ("p", (INT, priority)),
    ]


def plant_due_gravity_tick(
    world: Path,
    *,
    clear: tuple[int, int, int],
    tick: tuple[int, int, int],
    block: str = "minecraft:sand",
    clear_block: str = "minecraft:stone",
    delay: int = 600,
    priority: int = 0,
) -> dict[str, Any]:
    """Clear `clear` to air and append a due `block` tick for `tick` in that chunk's `block_ticks`.

    `clear` must currently hold `clear_block` and `tick` must hold `block`, so a
    fixture that silently placed something else fails here instead of passing a
    scene on an empty assertion.
    """
    chunk_x, chunk_z = clear[0] >> 4, clear[2] >> 4
    regions = sorted(world.glob("dimensions/minecraft/overworld/region/*.mca"))
    if not regions:
        regions = sorted(world.glob("**/region/*.mca"))
    if not regions:
        raise RuntimeError(f"exported world {world} holds no region files")
    for region in regions:
        chunks = _read_region(region)
        for index, (_, body, timestamp) in chunks.items():
            name, root = parse(body)
            if (field(root[1], "xPos")[1], field(root[1], "zPos")[1]) != (chunk_x, chunk_z):
                continue
            sections = field(root[1], "sections")[1][1]
            cleared = _section(sections, clear[1])
            cleared_states = field(cleared, "block_states")[1]
            cleared_names = _palette_names(_palette(cleared_states))
            air_index = cleared_names.index(AIR)
            expected = cleared_names.index(clear_block)
            found = read_cell(cleared, *clear)
            if found != expected:
                raise RuntimeError(
                    f"exported chunk ({chunk_x}, {chunk_z}) holds {cleared_names[found]} at {clear}, "
                    f"not {clear_block}")
            write_cell(cleared, *clear, air_index)

            held = _section(sections, tick[1])
            held_names = _palette_names(_palette(field(held, "block_states")[1]))
            at_tick = read_cell(held, *tick)
            if held_names[at_tick] != block:
                raise RuntimeError(
                    f"exported chunk ({chunk_x}, {chunk_z}) holds {held_names[at_tick]} at {tick}, "
                    f"not {block}")

            entry = tick_entry(tick, block, delay, priority)
            existing = [key for key, _ in root[1] if key == "block_ticks"]
            if existing:
                element, entries = field(root[1], "block_ticks")[1]
                if element not in (COMPOUND, 0):
                    raise RuntimeError(f"block_ticks list carries element type {element}, not a compound")
                before = len(entries)
                entries.append(entry)
                root[1][:] = [
                    (key, (LIST, (COMPOUND, entries)) if key == "block_ticks" else tag)
                    for key, tag in root[1]
                ]
            else:
                before = 0
                root[1].append(("block_ticks", (LIST, (COMPOUND, [entry]))))

            edited = serialize(name, root)
            rewritten = {
                other: (zlib.compress(edited if other == index else stored), stamp)
                for other, (_, stored, stamp) in chunks.items()
            }
            _write_region(region, rewritten)
            return {
                "region": region.name,
                "chunk": [chunk_x, chunk_z],
                "cleared": {"position": list(clear), "was": clear_block, "now": AIR,
                            "palette": [found, air_index]},
                "tick": {"i": block, "x": tick[0], "y": tick[1], "z": tick[2],
                         "t": delay, "p": priority, "palette": at_tick},
                "block_ticks_before": before,
            }
    raise RuntimeError(f"exported world {world} has no chunk ({chunk_x}, {chunk_z})")
