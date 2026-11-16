"""Read-only structure proof from the server's own offline Anvil export."""
from __future__ import annotations

import gzip
import json
from pathlib import Path

from .quirk_parity.anvil import parse_nbt, read_region, read_world


def template_blocks(pack: Path, structure: str) -> dict[str, list[str]]:
    """Trace this structure's pools; don't let another pack's palette satisfy it."""
    def resource(identifier: str, kind: str, suffix: str) -> Path:
        namespace, name = identifier.split(":", 1)
        path = pack / "data" / namespace / kind / (name + suffix)
        if not path.resolve().is_relative_to(pack.resolve()):
            raise ValueError(f"unsafe resource identifier {identifier}")
        return path

    definition = json.loads(resource(structure, "worldgen/structure", ".json").read_text())
    pending, seen, templates = [definition["start_pool"]], set(), set()
    while pending:
        identifier = pending.pop()
        if identifier in seen or identifier == "minecraft:empty":
            continue
        seen.add(identifier)
        pool = json.loads(resource(identifier, "worldgen/template_pool", ".json").read_text())
        pending.append(pool.get("fallback", "minecraft:empty"))
        def elements(value):
            if isinstance(value, dict):
                if "location" in value and isinstance(value["location"], str):
                    templates.add(value["location"])
                for child in value.values():
                    elements(child)
            elif isinstance(value, list):
                for child in value:
                    elements(child)
        elements(pool["elements"])
    # Structural material, not stone/dirt/water or foliage that terrain alone can supply.
    markers: dict[str, list[str]] = {}
    for identifier in sorted(templates):
        nbt = parse_nbt(gzip.decompress(resource(identifier, "structure", ".nbt").read_bytes()))
        palettes = nbt.get("palettes", [nbt.get("palette", [])])
        for palette in palettes:
            for block in palette:
                name = block["Name"]
                if not name.startswith("minecraft:"):
                    raise ValueError(f"{identifier}: requires custom block {name}")
                if any(part in name for part in ("_planks", "_bricks", "_stairs", "_slab", "_fence",
                                                  "_wall", "_door", "_trapdoor", "lantern", "barrel",
                                                  "chest", "cobblestone")):
                    markers.setdefault(name, []).append(identifier)
    if not markers:
        raise ValueError(f"{structure}: no constructed template block marker; choose a building")
    return markers


def generated_starts(export: Path, identifier: str) -> list[dict]:
    found = []
    # Export layout is version-dependent; xPos/zPos and the start anchor are authoritative.
    for region in sorted(export.rglob("r.*.*.mca")):
        if region.parent.name != "region":
            continue
        for _cx, _cz, chunk in read_region(region):
            start = chunk.get("structures", {}).get("starts", {}).get(identifier)
            if not start or start.get("id") != identifier:
                continue
            box, anchor = start.get("BB"), start.get("KrasnokamenLocatePos")
            if not isinstance(box, list) or len(box) != 6 or not isinstance(anchor, list) or len(anchor) != 3:
                raise ValueError(f"{identifier}: export lost native bounds/anchor")
            if (start.get("ChunkX"), start.get("ChunkZ")) != (chunk.get("xPos"), chunk.get("zPos")):
                raise ValueError(f"{identifier}: start stored in wrong chunk")
            if any(box[i] > box[i + 3] for i in range(3)):
                raise ValueError(f"{identifier}: inverted generated bounds")
            if (box[3]-box[0]+1)*(box[4]-box[1]+1)*(box[5]-box[2]+1) > 2_000_000:
                raise ValueError(f"{identifier}: evidence volume exceeds two million blocks")
            found.append({"id": identifier, "anchor": anchor, "bounds": box,
                          "chunk": [chunk["xPos"], chunk["zPos"]], "dimension_dir": str(region.parent.parent)})
    return found


def prove_site(export: Path, pack: Path, identifier: str, candidate: list[int]) -> dict:
    starts = [s for s in generated_starts(export, identifier)
              if abs(s["anchor"][0] - candidate[0]) <= 32 and abs(s["anchor"][2] - candidate[1]) <= 32]
    if not starts:
        raise ValueError(f"{identifier}: no persisted generated start near {candidate}; locate is not proof")
    site = min(starts, key=lambda s: abs(s["anchor"][0]-candidate[0]) + abs(s["anchor"][2]-candidate[1]))
    markers = template_blocks(pack, identifier)
    blocks = read_world(Path(site["dimension_dir"]), [site["bounds"]])[0]["blocks"]
    matches = [{"position": [int(v) for v in xyz.split(",")], "state": state,
                "templates": markers[state.split("[")[0]]}
               for xyz, state in sorted(blocks.items()) if state.split("[")[0] in markers]
    if len(matches) < 8 or len({m["state"].split("[")[0] for m in matches}) < 2:
        raise ValueError(f"{identifier}: generated bounds lack eight constructed blocks of two template kinds")
    # The picture shows the generated start, not distant pieces beyond client view distance.
    local = [m for m in matches if all(abs(m["position"][axis]-site["anchor"][axis]) <= 32 for axis in (0, 2))]
    local.sort(key=lambda m: sum((a-b)**2 for a,b in zip(m["position"], site["anchor"])))
    if len(local) < 8:
        raise ValueError(f"{identifier}: insufficient constructed blocks near the generated start")
    kinds = {}
    for match in local:
        kinds.setdefault(match["state"].split("[")[0], match)
    if len(kinds) < 2:
        raise ValueError(f"{identifier}: generated start has fewer than two constructed block kinds")
    samples = list(kinds.values())[:16]
    samples += [m for m in local if m not in samples][:16-len(samples)]
    site.update(matching_blocks=len(matches), samples=samples,
                scope="persisted exact-ID start plus template-palette samples, not full mod fidelity")
    return site
