"""Minimal read-only Anvil/NBT reader for the vanilla oracle's saved world.

Only what the parity gate needs: block snapshots out of `region/*.mca` and
entity snapshots out of `entities/*.mca` (26.1.2 layout: one directory per
dimension under `dimensions/minecraft/<dimension>/`).
"""
from __future__ import annotations

import json
import struct
import zlib
from pathlib import Path

_END, _BYTE, _SHORT, _INT, _LONG, _FLOAT, _DOUBLE, _BYTES, _STRING, _LIST, _COMPOUND, _INTS, _LONGS = range(13)

# Local `content import` cache (ignored, never in Git): `version.json` plus `reports/blocks.json`
# per Minecraft release, the current one at the root and older ones in subdirectories.
VANILLA_DATA = Path(__file__).resolve().parents[3] / "data" / "vanilla"
_DEFAULTS: dict[int, dict[str, str]] = {}


def default_states(data_version: int) -> dict[str, str]:
    """Block id -> its full default state string, from the local blocks report of that DataVersion.

    26.3 chunks store a block in its default state as the bare id with no properties, while
    Krasnokamen and older chunks name every property; both sides compare as full states.
    """
    if data_version not in _DEFAULTS:
        report = None
        for version_file in [VANILLA_DATA / "version.json", *sorted(VANILLA_DATA.glob("*/version.json"))]:
            try:
                world_version = json.loads(version_file.read_text())["world_version"]
            except (OSError, ValueError, KeyError):
                continue
            if world_version == data_version and (version_file.parent / "reports" / "blocks.json").is_file():
                report = version_file.parent / "reports" / "blocks.json"
                break
        if report is None:
            raise ValueError(f"no local blocks report for DataVersion {data_version} under {VANILLA_DATA} "
                             "(run `krasnokamen content import` for that release)")
        defaults = {}
        for name, block in json.loads(report.read_text()).items():
            for state in block["states"]:
                if state.get("default"):
                    props = state.get("properties") or {}
                    defaults[name] = name + ("[" + ",".join(f"{k}={props[k]}" for k in sorted(props)) + "]"
                                             if props else "")
        _DEFAULTS[data_version] = defaults
    return _DEFAULTS[data_version]


class _Reader:
    def __init__(self, data: bytes):
        self.data, self.pos = data, 0

    def take(self, n: int) -> bytes:
        chunk = self.data[self.pos:self.pos + n]
        if len(chunk) != n:
            raise ValueError("truncated NBT")
        self.pos += n
        return chunk

    def number(self, fmt: str):
        return struct.unpack(fmt, self.take(struct.calcsize(fmt)))[0]

    def text(self) -> str:
        return self.take(self.number(">H")).decode("utf-8", "surrogatepass")

    def payload(self, kind: int):
        simple = {_BYTE: ">b", _SHORT: ">h", _INT: ">i", _LONG: ">q", _FLOAT: ">f", _DOUBLE: ">d"}
        if kind in simple:
            return self.number(simple[kind])
        if kind == _BYTES:
            return list(struct.unpack(f">{(n := self.number('>i'))}b", self.take(n)))
        if kind == _STRING:
            return self.text()
        if kind == _LIST:
            child, count = self.number(">B"), self.number(">i")
            return [self.payload(child) for _ in range(max(count, 0))]
        if kind == _COMPOUND:
            out = {}
            while (child := self.number(">B")) != _END:
                name = self.text()
                out[name] = self.payload(child)
            return out
        if kind == _INTS:
            n = self.number(">i")
            return list(struct.unpack(f">{n}i", self.take(4 * n)))
        if kind == _LONGS:
            n = self.number(">i")
            return list(struct.unpack(f">{n}q", self.take(8 * n)))
        raise ValueError(f"unsupported NBT tag {kind}")


def parse_nbt(data: bytes) -> dict:
    reader = _Reader(data)
    kind = reader.number(">B")
    reader.text()
    return reader.payload(kind)


def read_region(path: Path):
    """Yield (chunk_x, chunk_z, nbt) for every stored chunk of one .mca file."""
    raw = path.read_bytes()
    if len(raw) < 8192:
        return
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
        elif compression == 1:
            import gzip
            body = gzip.decompress(body)
        elif compression != 3:
            raise ValueError(f"{path}: unsupported region compression {compression}")
        yield index & 31, index >> 5, parse_nbt(body)


def _palette_codec(version: int) -> tuple[str, str]:
    if isinstance(version, int) and 0 <= version <= 4790:
        return "Name", "Properties"
    if version == 5023:
        return "id", "properties"
    raise ValueError(f"unsupported chunk DataVersion {version!r}")


def _block_string(entry: dict | str, codec: tuple[str, str], data_version: int | None = None) -> str:
    name_key, props_key = codec
    # A list mixing bare identifiers and compounds (26.3 palettes) stores every
    # non-compound element wrapped as a compound with one empty-named entry.
    if isinstance(entry, dict) and len(entry) == 1 and "" in entry:
        entry = entry[""]
    if isinstance(entry, str) and name_key == "id":
        entry = {"id": entry}
    if not isinstance(entry, dict):
        raise ValueError(f"palette entry does not match {name_key}/{props_key} codec: {entry!r}")
    other_name, other_props = ("id", "properties") if name_key == "Name" else ("Name", "Properties")
    if name_key not in entry or other_name in entry or other_props in entry:
        raise ValueError(f"palette entry does not match {name_key}/{props_key} codec: {entry!r}")
    props = entry.get(props_key)
    if not props:
        if name_key != "id":
            return entry[name_key]
        # The 26.3 codec omits the properties of a default state.
        full = default_states(data_version).get(entry[name_key])
        if full is None:
            raise ValueError(f"palette block {entry[name_key]!r} is not in the DataVersion {data_version} blocks report")
        return full
    return entry[name_key] + "[" + ",".join(f"{k}={props[k]}" for k in sorted(props)) + "]"


def _section_blocks(section: dict, codec: tuple[str, str], data_version: int | None = None):
    states = section.get("block_states") or {}
    palette = [_block_string(entry, codec, data_version) for entry in states.get("palette", [])]
    if not palette:
        return
    if len(palette) == 1:
        for index in range(4096):
            yield index, palette[0]
        return
    bits = max(4, (len(palette) - 1).bit_length())
    per_long = 64 // bits
    mask = (1 << bits) - 1
    data = [value & 0xFFFFFFFFFFFFFFFF for value in states["data"]]
    for index in range(4096):
        word = data[index // per_long]
        yield index, palette[(word >> ((index % per_long) * bits)) & mask]


def _chunks_of(boxes):
    wanted = set()
    for x1, _y1, z1, x2, _y2, z2 in boxes:
        for chunk_x in range(x1 >> 4, (x2 >> 4) + 1):
            for chunk_z in range(z1 >> 4, (z2 >> 4) + 1):
                wanted.add((chunk_x, chunk_z))
    return wanted


def _regions(directory: Path, wanted):
    """Yield (chunk_x, chunk_z, nbt) for wanted chunks, parsing each region once."""
    by_region: dict[tuple[int, int], set] = {}
    for chunk in wanted:
        by_region.setdefault((chunk[0] >> 5, chunk[1] >> 5), set()).add(chunk)
    for (rx, rz), chunks in by_region.items():
        file = directory / f"r.{rx}.{rz}.mca"
        if not file.is_file():
            continue
        for cx, cz, nbt in read_region(file):
            chunk = (cx + (rx << 5), cz + (rz << 5))
            if chunk in chunks:
                yield chunk, nbt


def read_world(dimension_dir: Path, boxes, *, palette_audit: dict | None = None) -> list[dict]:
    """Per-box {"blocks": {"x,y,z": state}, "entities": [raw NBT]}.

    Blocks are non-air only; entities are those whose position is inside the box.
    """
    cells = [{"blocks": {}, "entities": [], "containers": {}} for _ in boxes]
    wanted = _chunks_of(boxes)
    for (chunk_x, chunk_z), nbt in _regions(dimension_dir / "region", wanted):
        version = nbt.get("DataVersion")
        try:
            codec = _palette_codec(version)
        except ValueError as error:
            raise ValueError(f"chunk {chunk_x},{chunk_z}: {error}") from error
        if palette_audit is not None:
            entry = palette_audit.setdefault(str(version), {"encoding": "/".join(codec), "chunks": 0})
            entry["chunks"] += 1
        mine = [i for i, b in enumerate(boxes)
                if b[0] >> 4 <= chunk_x <= b[3] >> 4 and b[2] >> 4 <= chunk_z <= b[5] >> 4]
        for entity in nbt.get("block_entities", []):
            items = entity.get("Items")
            if not items:
                continue
            x, y, z = entity["x"], entity["y"], entity["z"]
            for i in mine:
                x1, y1, z1, x2, y2, z2 = boxes[i]
                if x1 <= x <= x2 and y1 <= y <= y2 and z1 <= z <= z2:
                    totals = cells[i]["containers"].setdefault(f"{x},{y},{z}", {})
                    for stack in items:
                        totals[stack["id"]] = totals.get(stack["id"], 0) + stack.get("count", stack.get("Count", 1))
        for section in nbt.get("sections", []):
            base_y = section["Y"] * 16
            relevant = [i for i in mine if boxes[i][1] <= base_y + 15 and boxes[i][4] >= base_y]
            if not relevant:
                continue
            try:
                for index, name in _section_blocks(section, codec, version):
                    if name in ("minecraft:air", "minecraft:cave_air", "minecraft:void_air"):
                        continue
                    x = chunk_x * 16 + (index & 15)
                    z = chunk_z * 16 + ((index >> 4) & 15)
                    y = base_y + (index >> 8)
                    for i in relevant:
                        x1, y1, z1, x2, y2, z2 = boxes[i]
                        if x1 <= x <= x2 and y1 <= y <= y2 and z1 <= z <= z2:
                            cells[i]["blocks"][f"{x},{y},{z}"] = name
            except (KeyError, TypeError, ValueError) as error:
                raise ValueError(f"chunk {chunk_x},{chunk_z} section {section['Y']}: {error}") from error
    for (chunk_x, chunk_z), nbt in _regions(dimension_dir / "entities", wanted):
        for entity in nbt.get("Entities", []):
            x, y, z = entity.get("Pos", [0.0, 0.0, 0.0])
            for i, (x1, y1, z1, x2, y2, z2) in enumerate(boxes):
                if x1 <= x < x2 + 1 and y1 <= y < y2 + 1 and z1 <= z < z2 + 1:
                    cells[i]["entities"].append(entity)
    return cells
