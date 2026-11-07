"""Corrupt chosen block entities inside an exported Anvil world, losslessly.

The harness exports a stopped native world with the shipping CLI, flips one field
of each chosen block entity to a value the core's hydration owner must refuse
(an unknown item id, an out-of-range level), and imports the result again. That is
the real route by which a record Solaris cannot decode reaches a resident chunk: a
vanilla, modded or hand-edited world. Tags keep their exact NBT type, so every
byte outside the edited field is unchanged.
"""
from __future__ import annotations

import struct
import zlib
from pathlib import Path
from typing import Any, Callable

END, BYTE, SHORT, INT, LONG, FLOAT, DOUBLE, BYTE_ARRAY, STRING, LIST, COMPOUND, INT_ARRAY, LONG_ARRAY = range(13)
_FIXED = {BYTE: ">b", SHORT: ">h", INT: ">i", LONG: ">q", FLOAT: ">f", DOUBLE: ">d"}

# A tag is (type id, value). Compound: list of (name, tag); list: (element type, [values]).
Tag = tuple[int, Any]


class _Reader:
    def __init__(self, data: bytes) -> None:
        self.data, self.pos = data, 0

    def take(self, count: int) -> bytes:
        chunk = self.data[self.pos:self.pos + count]
        if len(chunk) != count:
            raise ValueError("truncated NBT")
        self.pos += count
        return chunk

    def number(self, fmt: str) -> Any:
        return struct.unpack(fmt, self.take(struct.calcsize(fmt)))[0]

    def text(self) -> str:
        return self.take(self.number(">H")).decode("utf-8", "surrogatepass")

    def payload(self, kind: int) -> Any:
        if kind in _FIXED:
            return self.number(_FIXED[kind])
        if kind == BYTE_ARRAY:
            return self.take(self.number(">i"))
        if kind == STRING:
            return self.text()
        if kind == LIST:
            child, count = self.number(">B"), self.number(">i")
            return (child, [self.payload(child) for _ in range(max(count, 0))])
        if kind == COMPOUND:
            fields = []
            while (child := self.number(">B")) != END:
                name = self.text()
                fields.append((name, (child, self.payload(child))))
            return fields
        if kind in (INT_ARRAY, LONG_ARRAY):
            count = self.number(">i")
            width = 4 if kind == INT_ARRAY else 8
            return (kind, self.take(count * width))
        raise ValueError(f"unsupported NBT tag {kind}")


def parse(data: bytes) -> tuple[str, Tag]:
    reader = _Reader(data)
    kind = reader.number(">B")
    name = reader.text()
    return name, (kind, reader.payload(kind))


def _text(value: str) -> bytes:
    raw = value.encode("utf-8", "surrogatepass")
    return struct.pack(">H", len(raw)) + raw


def _payload(kind: int, value: Any) -> bytes:
    if kind in _FIXED:
        return struct.pack(_FIXED[kind], value)
    if kind == BYTE_ARRAY:
        return struct.pack(">i", len(value)) + value
    if kind == STRING:
        return _text(value)
    if kind == LIST:
        child, items = value
        return struct.pack(">Bi", child, len(items)) + b"".join(_payload(child, item) for item in items)
    if kind == COMPOUND:
        return b"".join(struct.pack(">B", tag[0]) + _text(name) + _payload(*tag) for name, tag in value) + b"\0"
    if kind in (INT_ARRAY, LONG_ARRAY):
        width = 4 if kind == INT_ARRAY else 8
        return struct.pack(">i", len(value[1]) // width) + value[1]
    raise ValueError(f"unsupported NBT tag {kind}")


def serialize(name: str, tag: Tag) -> bytes:
    return struct.pack(">B", tag[0]) + _text(name) + _payload(*tag)


def field(compound: list[tuple[str, Tag]], name: str) -> Tag:
    matches = [tag for key, tag in compound if key == name]
    if len(matches) != 1:
        raise ValueError(f"expected exactly one {name!r} field, found {len(matches)}")
    return matches[0]


def _read_region(path: Path) -> dict[int, tuple[int, bytes, int]]:
    raw = path.read_bytes()
    chunks: dict[int, tuple[int, bytes, int]] = {}
    for index in range(1024):
        entry = struct.unpack(">I", raw[index * 4:index * 4 + 4])[0]
        offset, sectors = entry >> 8, entry & 0xFF
        if offset == 0 or sectors == 0:
            continue
        start = offset * 4096
        length, compression = struct.unpack(">IB", raw[start:start + 5])
        body = raw[start + 5:start + 4 + length]
        if compression == 2:
            body = zlib.decompress(body)
        else:
            raise ValueError(f"{path}: harness fixture expects zlib chunks, found compression {compression}")
        timestamp = struct.unpack(">I", raw[4096 + index * 4:4096 + index * 4 + 4])[0]
        chunks[index] = (compression, body, timestamp)
    return chunks


def _write_region(path: Path, chunks: dict[int, tuple[bytes, int]]) -> None:
    locations, timestamps, body = bytearray(4096), bytearray(4096), bytearray()
    sector = 2
    for index in sorted(chunks):
        payload, timestamp = chunks[index]
        record = struct.pack(">IB", len(payload) + 1, 2) + payload
        record += b"\0" * (-len(record) % 4096)
        count = len(record) // 4096
        locations[index * 4:index * 4 + 4] = struct.pack(">I", (sector << 8) | count)
        timestamps[index * 4:index * 4 + 4] = struct.pack(">I", timestamp)
        body += record
        sector += count
    path.write_bytes(bytes(locations) + bytes(timestamps) + bytes(body))


Edit = Callable[[list[tuple[str, Tag]]], None]


def corrupt_block_entities(world: Path, edits: dict[tuple[int, int, int], Edit]) -> dict[tuple[int, int, int], str]:
    """Apply `edits[(x, y, z)]` to that block entity's compound; return what each edit hit."""
    applied: dict[tuple[int, int, int], str] = {}
    for region in sorted(world.glob("**/region/r.*.mca")):
        chunks = _read_region(region)
        rewritten: dict[int, tuple[bytes, int]] = {}
        changed = False
        for index, (_, body, timestamp) in chunks.items():
            name, root = parse(body)
            entities = next((tag for key, tag in root[1] if key == "block_entities"), None)
            hit = False
            for entity in (entities[1][1] if entities else []):
                position = tuple(field(entity, axis)[1] for axis in ("x", "y", "z"))
                if position in edits:
                    edits[position](entity)
                    applied[position] = field(entity, "id")[1]
                    hit = True
            if hit:
                body, changed = serialize(name, root), True
            rewritten[index] = (zlib.compress(body), timestamp)
        if changed:
            _write_region(region, rewritten)
    missing = set(edits) - set(applied)
    if missing:
        raise RuntimeError(f"block entities not found in the exported world: {sorted(missing)}")
    return applied
