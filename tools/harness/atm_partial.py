"""Derive explicitly partial MVS pilot packs; preserve the full conversion.

No-loot variants are generation-test-only. Containers' LootTable references
remain unchanged. The crystal-only fixture retains its two pools and templates.
"""
from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path
import shutil

from .atm_carry import digest, inventory, tree_hash, write_json

REASON = "unsupported: Nether placement, HIGHEST_LAND/biome-radius"
NETHER_TYPE = "moogs_structures:moogs_structures_generic_nether_jigsaw_structure"


def exclude_nether(pack: Path) -> list[dict]:
    removed, identifiers, tags = [], set(), set()
    for path in sorted((pack / "data").glob("*/worldgen/structure/**/*.json")):
        value = json.loads(path.read_text())
        if value.get("type") != NETHER_TYPE:
            continue
        relative = path.relative_to(pack).as_posix()
        parts = path.relative_to(pack).parts
        identifiers.add(parts[1] + ":" + "/".join(parts[4:])[:-5])
        biomes = value.get("biomes")
        if isinstance(biomes, str) and biomes.startswith("#"):
            tags.add(biomes[1:])
        removed.append({"path": relative, "sha256": digest(path), "reason": REASON})
    if not identifiers:
        raise ValueError("no custom Moog Nether structures found; refusing misleading partial label")
    for path in sorted((pack / "data").glob("*/worldgen/structure_set/**/*.json")):
        value = json.loads(path.read_text())
        members = {entry["structure"] for entry in value["structures"]}
        if not members & identifiers:
            continue
        if members - identifiers:
            raise ValueError(f"mixed Overworld/Nether structure set needs an explicit policy: {path}")
        removed.append({"path": path.relative_to(pack).as_posix(), "sha256": digest(path), "reason": REASON})
    removed_names = {entry["path"] for entry in removed}
    remaining = [path.read_text() for path in (pack / "data").rglob("*.json")
                 if path.relative_to(pack).as_posix() not in removed_names]
    for identifier in sorted(tags):
        ns, name = identifier.split(":", 1)
        path = pack / f"data/{ns}/tags/worldgen/biome/{name}.json"
        if path.is_file() and not any(json.dumps("#" + identifier) in text for text in remaining):
            removed.append({"path": path.relative_to(pack).as_posix(), "sha256": digest(path), "reason": REASON + "; now-unreferenced biome tag"})
    for row in removed:
        (pack / row["path"]).unlink()
    return removed


def variants(full: Path, output: Path, *, crystal_only: bool = False) -> dict:
    manifest = json.loads(full.read_text())
    if manifest.get("status") != "converted-unproven" or len(manifest.get("packs", [])) != 1 or manifest["packs"][0]["id"] != "mvs":
        raise ValueError("requires the full single-pack MVS carry manifest")
    original = manifest["packs"][0]
    source = Path(original["server_pack"])
    if inventory(source) != original["server_files"]:
        raise ValueError("full MVS conversion changed after receipt")
    results = {}
    for no_loot in ((True,) if crystal_only else (False, True)):
        suffix = "crystal-noloot" if crystal_only else "no-nether" + ("-no-loot" if no_loot else "")
        name = f"mvs-{original['version']}-partial-{suffix}"
        directory = output / name
        target = directory / name
        shutil.copytree(source, target)
        if crystal_only:
            keep = {
                "data/mvs/worldgen/structure/crystal.json",
                "data/mvs/worldgen/structure_set/crystal.json",
                "data/mvs/worldgen/template_pool/crystal/temp.json",
                "data/mvs/worldgen/template_pool/crystal/lower.json",
                "data/mvs/structure/1_21_0/crystal/base.nbt",
                "data/mvs/structure/1_21_0/crystal/lower.nbt",
            }
            if not keep <= original["server_files"].keys():
                raise ValueError("MVS crystal dependency closure changed")
            excluded = []
            for path in sorted((target / "data").rglob("*")):
                if path.is_file() and path.relative_to(target).as_posix() not in keep:
                    excluded.append({"path": path.relative_to(target).as_posix(), "sha256": digest(path),
                                     "reason": "generation-test-only: outside crystal's two-pool/two-template closure; loot excluded"})
                    path.unlink()
        else:
            excluded = exclude_nether(target)
        if no_loot and not crystal_only:
            for path in sorted((target / "data").glob("*/loot_table/**/*.json")):
                excluded.append({"path": path.relative_to(target).as_posix(), "sha256": digest(path),
                                 "reason": "generation-test-only: datapack container loot admission not implemented"})
                path.unlink()
        metadata = json.loads((target / "pack.mcmeta").read_text())
        metadata["pack"]["description"] = name + ": PARTIAL, EXPERIMENTAL" + (", GENERATION TEST ONLY; NO LOOT CLAIM" if no_loot else "")
        write_json(target / "pack.mcmeta", metadata)
        result = copy.deepcopy(manifest)
        pack = result["packs"][0]
        pack.update(id=name, source_mod_id="mvs", partial=True, generation_only=no_loot,
                    excluded_files=excluded, full_conversion=str(full), full_conversion_sha256=digest(full),
                    server_pack=str(target), server_files=inventory(target))
        pack["server_sha256"] = tree_hash(pack["server_files"])
        if crystal_only:
            pack["structures"] = ["mvs:crystal"]
        pack["excluded_count"] = len(excluded)
        pack["retained_data_count"] = sum(path.startswith("data/") for path in pack["server_files"])
        result.update(partial=True, generation_only=no_loot,
                      exclusions="Only crystal generation closure; ALL loot excluded" if crystal_only else
                      "Custom Nether structures and their structure sets; unused local biome tags" + ("; ALL loot tables" if no_loot else ""))
        write_json(directory / "result.json", result)
        results[name] = {"manifest": str(directory / "result.json"), "server_sha256": pack["server_sha256"],
                         "excluded_files": excluded, "excluded_count": len(excluded),
                         "retained_data_count": pack["retained_data_count"], "generation_only": no_loot}
    write_json(output / "variants.json", results)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--full", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--crystal-only", action="store_true", help="generation-only two-template MVS crystal fixture")
    args = parser.parse_args()
    output = args.output.resolve()
    if "targets" not in output.parts:
        parser.error("derived pack bytes must remain beneath targets/")
    output.mkdir(parents=True, exist_ok=False)
    try:
        results = variants(args.full.resolve(), output, crystal_only=args.crystal_only)
    except (ValueError, OSError, KeyError) as error:
        write_json(output / "result.json", {"status": "failed", "error": str(error)})
        return 1
    print(json.dumps({name: value["manifest"] for name, value in results.items()}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
