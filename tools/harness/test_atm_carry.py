"""Synthetic local fixtures only; no downloaded mod or Mojang bytes."""
import gzip
import hashlib
import json
from pathlib import Path
import struct
import tempfile
import unittest
import zipfile
import zlib

from . import atm_carry, atm_evidence, atm_partial, profiles, scenarios
from .backends import atm_structures


def nbt(value):
    def text(s):
        raw = s.encode()
        return struct.pack(">H", len(raw)) + raw
    def tag(v):
        if isinstance(v, str):
            return 8, text(v)
        if isinstance(v, int):
            return 3, struct.pack(">i", v)
        if isinstance(v, list):
            children = [tag(c) for c in v]
            kind = children[0][0] if children else 0
            assert all(k == kind for k, _ in children)
            return 9, bytes([kind]) + struct.pack(">i", len(v)) + b"".join(b for _, b in children)
        return 10, b"".join(bytes([tag(c)[0]]) + text(k) + tag(c)[1] for k, c in v.items()) + b"\0"
    kind, payload = tag(value)
    return bytes([kind]) + b"\0\0" + payload


def region(path, chunk):
    compressed = zlib.compress(nbt(chunk))
    body = struct.pack(">I", len(compressed)+1) + b"\2" + compressed
    sectors = (len(body)+4095)//4096
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(struct.pack(">I", (2 << 8) | sectors) + bytes(8192-4) + body.ljust(sectors*4096, b"\0"))


class CarryTests(unittest.TestCase):
    def test_jar_identity_licence_format_and_json_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            jar = Path(directory) / "test.jar"
            def fixture(licence="MIT", fmt=48, data='{"type":"minecraft:jigsaw"}'):
                metadata = f'license="{licence}"\n[[mods]]\nmodId="test"\nversion="1"\n'.encode()
                with zipfile.ZipFile(jar, "w") as archive:
                    archive.writestr("META-INF/neoforge.mods.toml", metadata)
                    archive.writestr("pack.mcmeta", json.dumps({"pack": {"pack_format": fmt}}))
                    archive.writestr("data/test/worldgen/structure/tower.json", data)
                return {"id": "test", "version": "1", "licence": "MIT", "sha256": atm_carry.digest(jar),
                        "licence_entry": "META-INF/neoforge.mods.toml",
                        "licence_sha256": hashlib.sha256(metadata).hexdigest()}
            good = fixture()
            evidence, files = atm_carry.inspect_jar(jar, good)
            self.assertEqual(evidence["licence"], "MIT")
            self.assertEqual(len(files), 1)
            for change, message in (({"licence": "CC-BY-NC-ND-4.0"}, "unapproved"),
                                    ({"fmt": 15}, "format 48"), ({"data": "{"}, "invalid source JSON")):
                spec = fixture(**change)
                with self.assertRaisesRegex(ValueError, message):
                    atm_carry.inspect_jar(jar, spec)
            spec = fixture()
            with self.assertRaisesRegex(ValueError, "SHA-256"):
                atm_carry.inspect_jar(jar, {**spec, "sha256": "0"*64})
            with self.assertRaisesRegex(ValueError, "licence evidence changed"):
                atm_carry.inspect_jar(jar, {**spec, "licence_sha256": "0"*64})

    def test_converter_losses_and_missing_files_are_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            name = "data/test/worldgen/structure/test.json"
            self.assertIn("source data missing", " ".join(atm_carry.conversion_errors({}, {name: b"{}"}, root)))
            (root/name).parent.mkdir(parents=True)
            (root/name).write_text("{}")
            self.assertEqual(atm_carry.conversion_errors({}, {name: b"{}"}, root), [])
            for key in ("unsupported", "rules_lost", "processors_dropped", "modifiers_left"):
                report = {"worldgen_types": {key: {"test:rule": 1}}}
                self.assertIn(key, " ".join(atm_carry.conversion_errors(report, {name: b"{}"}, root)))


class PartialTests(unittest.TestCase):
    def test_crystal_fixture_accounts_every_exclusion_and_keeps_nbt_unchanged(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "full"
            retained = [
                "worldgen/structure/crystal.json", "worldgen/structure_set/crystal.json",
                "worldgen/template_pool/crystal/temp.json", "worldgen/template_pool/crystal/lower.json",
                "structure/1_21_0/crystal/base.nbt", "structure/1_21_0/crystal/lower.nbt",
            ]
            excluded = ["loot_table/crystal.json", "worldgen/structure/sunzi_gate.json"]
            for name in retained + excluded:
                path = source / "data/mvs" / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"original LootTable:mvs:crystal" if name.endswith(".nbt") else b"{}")
            atm_carry.write_json(source / "pack.mcmeta", {"pack": {"pack_format": 121}})
            before = atm_carry.inventory(source)
            full = root / "full.json"
            atm_carry.write_json(full, {"status": "converted-unproven", "packs": [{
                "id": "mvs", "version": "5.1.3", "server_pack": str(source), "server_files": before,
                "structures": ["mvs:other"],
            }]})
            results = atm_partial.variants(full, root / "partial", crystal_only=True)
            row = results["mvs-5.1.3-partial-crystal-noloot"]
            self.assertEqual(row["excluded_count"], 2)
            self.assertEqual(row["retained_data_count"], 6)
            self.assertTrue(row["generation_only"])
            self.assertEqual({entry["path"] for entry in row["excluded_files"]},
                             {"data/mvs/" + name for name in excluded})
            result = json.loads(Path(row["manifest"]).read_text())
            pack = result["packs"][0]
            self.assertEqual(pack["structures"], ["mvs:crystal"])
            for name in retained:
                self.assertEqual(pack["server_files"]["data/mvs/" + name], before["data/mvs/" + name])
            self.assertEqual(atm_carry.inventory(source), before)

    def test_mixed_structure_set_cannot_silently_drop_overworld_member(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            atm_carry.write_json(root / "data/mvs/worldgen/structure/nether.json",
                                 {"type": atm_partial.NETHER_TYPE})
            atm_carry.write_json(root / "data/mvs/worldgen/structure_set/mixed.json",
                                 {"structures": [{"structure": "mvs:nether"}, {"structure": "mvs:crystal"}]})
            before = atm_carry.inventory(root)
            with self.assertRaisesRegex(ValueError, "mixed Overworld/Nether"):
                atm_partial.exclude_nether(root)
            self.assertEqual(atm_carry.inventory(root), before)


class EvidenceTests(unittest.TestCase):
    def test_exact_generated_start_and_template_blocks_are_required(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pack, export = root/"pack", root/"export"
            atm_carry.write_json(pack/"data/test/worldgen/structure/tower.json", {"start_pool": "test:tower"})
            atm_carry.write_json(pack/"data/test/worldgen/template_pool/tower.json",
                {"elements": [{"element": {"location": "test:tower"}}]})
            template = pack/"data/test/structure/tower.nbt"
            template.parent.mkdir(parents=True)
            template.write_bytes(gzip.compress(nbt({"palette": [{"Name": "minecraft:oak_planks"},
                                                                   {"Name": "minecraft:stone_bricks"}]})))
            start = {"id": "test:tower", "ChunkX": 0, "ChunkZ": 0, "BB": [0,0,0,15,31,15],
                     "KrasnokamenLocatePos": [0,0,0]}
            chunk = {"xPos": 0, "zPos": 0, "structures": {"starts": {"test:tower": start}},
                     "sections": [{"Y": y, "block_states": {"palette": [{"Name": name}]}}
                         for y, name in enumerate(("minecraft:oak_planks", "minecraft:stone_bricks"))]}
            path = export/"region/r.0.0.mca"
            region(path, chunk)
            proof = atm_evidence.prove_site(export, pack, "test:tower", [0,0])
            self.assertEqual(proof["matching_blocks"], 8192)
            self.assertEqual(len(proof["samples"]), 16)
            # Negative control: same actual blocks, no generated start, cannot pass.
            chunk["structures"]["starts"] = {}
            region(path, chunk)
            with self.assertRaisesRegex(ValueError, "no persisted generated start"):
                atm_evidence.prove_site(export, pack, "test:tower", [0,0])
            # An unrelated expected structure must never satisfy the selected ID.
            chunk["structures"]["starts"] = {"minecraft:village": {**start, "id": "minecraft:village"}}
            region(path, chunk)
            with self.assertRaisesRegex(ValueError, "no persisted generated start"):
                atm_evidence.prove_site(export, pack, "test:tower", [0,0])
            # Start alone is not proof: terrain rather than the template fails.
            chunk["structures"]["starts"] = {"test:tower": start}
            chunk["sections"] = [{"Y": 0, "block_states": {"palette": [{"Name": "minecraft:dirt"}]}}]
            region(path, chunk)
            with self.assertRaisesRegex(ValueError, "lack eight constructed blocks"):
                atm_evidence.prove_site(export, pack, "test:tower", [0,0])

    def test_locate_and_graphical_admission(self):
        self.assertEqual(atm_structures.locate_coordinates(["The nearest test:tower is at [-42, ~, 77]"], "test:tower"), [-42,77])
        with self.assertRaisesRegex(ValueError, "did not return"):
            atm_structures.locate_coordinates(["The nearest minecraft:village is at [0, ~, 0]"], "test:tower")
        scenario = scenarios.SCENARIOS["native-atm-structures"]
        self.assertEqual(profiles.PROFILES[scenario.profile]["run"], profiles.run_atm_structures)
        for client, windowed in (("java", True), ("native", False)):
            with self.assertRaisesRegex(ValueError, "requires --client native --windowed"):
                atm_structures.run(profiles.ProfileContext(Path("."), client=client, windowed=windowed))


if __name__ == "__main__":
    unittest.main()
