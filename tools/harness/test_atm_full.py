"""Full ATM manifest and restart sampling contracts."""
import json
import gzip
import contextlib
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from . import atm_full
from . import __main__ as cli, preflight
from .backends import anvil_corrupt as nbt


class FullSelectionTests(unittest.TestCase):
    def test_exact_inventory_and_deterministic_restart_sample(self):
        with tempfile.TemporaryDirectory() as directory:
            pack = Path(directory)
            structures = pack / "data/mvs/worldgen/structure"
            pools = pack / "data/mvs/worldgen/template_pool"
            templates = pack / "data/mvs/structure"
            for path in (structures, pools, templates):
                path.mkdir(parents=True)
            (templates / "piece.nbt").write_bytes(b"template")
            (pools / "start.json").write_text(json.dumps({"elements": [{"element": {"location": "mvs:piece"}}]}))
            for index in range(130):
                typ = atm_full.OVERWORLD if index < 127 else atm_full.NETHER
                (structures / f"s{index:03}.json").write_text(json.dumps({"type": typ, "start_pool": "mvs:start"}))
            rows = atm_full.selected_definitions(pack)
            self.assertEqual(len(rows), 130)
            selected = atm_full.restart_selection(rows)
            self.assertEqual(len(selected), 16)
            self.assertEqual(selected, {f"mvs:s{i:03}" for i in range(0, 127, 10)} |
                             {"mvs:s127", "mvs:s128", "mvs:s129"})
            self.assertEqual(atm_full.control_selection(rows, "all"), {row["id"] for row in rows})
            with self.assertRaisesRegex(ValueError, "absent, 1 or all"):
                atm_full.control_selection(rows, "invalid")
            self.assertEqual(rows[0]["templates"], ["mvs:piece"])
            (structures / "s129.json").unlink()
            with self.assertRaisesRegex(ValueError, "127 Overworld and 3 Nether"):
                atm_full.selected_definitions(pack)

    def test_template_version_selection_and_ambiguity(self):
        element = {"location": "mvs:fallback", "locations": {
            "1.21-1.21.4": "mvs:old", "1.21.5-26.3": "mvs:current"}}
        self.assertEqual(atm_full.selected_template(element), "mvs:current")
        element["locations"]["26.3"] = "mvs:overlap"
        with self.assertRaisesRegex(ValueError, "ambiguous"):
            atm_full.selected_template(element)

    def test_locate_failure_must_name_the_requested_definition(self):
        messages = {"styled": [{"translate": "commands.locate.structure.invalid",
                                "with": [{"text": "mvs:missing"}]}]}
        self.assertEqual(atm_full.locate_failure(messages, "mvs:missing"),
                         "identifier not admitted by the server")
        self.assertIsNone(atm_full.locate_failure(messages, "mvs:other"))

    def test_differential_rejects_natural_and_beard_only_changes(self):
        baseline = {"0,0,0": "minecraft:stone", "1,0,0": "minecraft:dirt"}
        natural = {"minecraft:air", "minecraft:stone", "minecraft:dirt"}
        palette = {"minecraft:stone", "minecraft:dirt", "minecraft:stone_bricks"}
        self.assertEqual(atm_full.differential_candidates(baseline, baseline, baseline, natural, palette), [])
        beard = {"0,0,0": "minecraft:dirt", "1,0,0": "minecraft:stone"}
        self.assertEqual(atm_full.differential_candidates(beard, baseline, beard, natural, palette), [])
        placed = {**beard, "2,0,0": "minecraft:stone_bricks[axis=x]"}
        matches = atm_full.differential_candidates(placed, baseline, placed, natural, palette)
        self.assertEqual(matches, [{"position": [2, 0, 0], "state": "minecraft:stone_bricks[axis=x]"}])
        later = {"0,0,0": "minecraft:snow[layers=1]", "1,0,0": "minecraft:stone_bricks",
                 "2,0,0": "minecraft:diamond_block", "3,0,0": "minecraft:cobblestone",
                 "4,0,0": "minecraft:ice", "5,0,0": "minecraft:snow_block"}
        discovery = {key: value for key, value in later.items() if key != "3,0,0"}
        self.assertEqual(atm_full.differential_candidates(
            later, {}, discovery, {"minecraft:air"},
            {"minecraft:snow", "minecraft:ice", "minecraft:stone_bricks", "minecraft:cobblestone", "minecraft:snow_block"}),
            [{"position": [1, 0, 0], "state": "minecraft:stone_bricks"},
             {"position": [5, 0, 0], "state": "minecraft:snow_block"}])

    def test_template_size_controls_differential_threshold(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "data/mvs/structure/start.nbt"
            path.parent.mkdir(parents=True)
            def template(count):
                palette = [[("Name", (nbt.STRING, "minecraft:stone_bricks"))]]
                blocks = [[("state", (nbt.INT, 0))] for _ in range(count)]
                raw = nbt.serialize("", (nbt.COMPOUND, [
                    ("palette", (nbt.LIST, (nbt.COMPOUND, palette))),
                    ("blocks", (nbt.LIST, (nbt.COMPOUND, blocks))),
                ]))
                path.write_bytes(gzip.compress(raw))
            row = {"id": "mvs:test", "templates": ["mvs:start"], "xz": [0, 0],
                   "dimension": "minecraft:overworld"}
            export, baseline = root / "positive", root / "baseline"
            site = {"anchor": [0, 0, 0], "bounds": [0, 0, 0, 7, 0, 0],
                    "chunk": [0, 0], "dimension_dir": str(export / "dimensions/minecraft/overworld")}
            for count, placed, expected in ((6, 6, True), (6, 5, False), (0, 0, False), (12, 8, True)):
                with self.subTest(count=count, placed=placed):
                    template(count)
                    palette, variants = atm_full.template_metadata(root, row)
                    row.update(template_palette=sorted(palette), template_variant_counts=variants)
                    blocks = {f"{i},0,0": "minecraft:stone_bricks" for i in range(placed)}
                    with mock.patch.object(atm_full, "generated_starts", return_value=[site]), \
                            mock.patch.object(atm_full, "require_chunks"), \
                            mock.patch.object(atm_full, "stand_positions", return_value=[[0, 1, 0]]), \
                            mock.patch.object(atm_full, "read_world", side_effect=[
                                [{"blocks": blocks}], [{"blocks": {}}], [{"blocks": blocks}], [{"blocks": blocks}]]):
                        if expected:
                            atm_full.site_proof(export, baseline, root / "discovery", root, row, "before", control_count=0)
                            self.assertEqual(row["differential"]["before"]["minimum"], min(8, count))
                        else:
                            with self.assertRaisesRegex(ValueError, "unprovable" if count == 0 else "only 5"):
                                atm_full.site_proof(export, baseline, root / "discovery", root, row, "before", control_count=0)

    def test_natural_types_and_control_set_effective_threshold(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "data/mvs/structure/start.nbt"
            path.parent.mkdir(parents=True)
            palette = [[("Name", (nbt.STRING, name))] for name in
                       ("minecraft:stone", "minecraft:stone_bricks", "minecraft:snow", "minecraft:ice")]
            raw = nbt.serialize("", (nbt.COMPOUND, [
                ("palette", (nbt.LIST, (nbt.COMPOUND, palette))),
                ("blocks", (nbt.LIST, (nbt.COMPOUND, [[("state", (nbt.INT, i))] for i in [0, 1, 1, 1, 1, 2, 3]]))),
            ]))
            path.write_bytes(gzip.compress(raw))
            row = {"id": "mvs:rock", "templates": ["mvs:start"], "xz": [0, 0],
                   "dimension": "minecraft:overworld"}
            palette_types, variants = atm_full.template_metadata(root, row)
            row.update(template_palette=sorted(palette_types), template_variant_counts=variants)
            export, baseline = root / "positive", root / "baseline"
            site = {"anchor": [0, 0, 0], "bounds": [0, 0, 0, 4, 0, 0],
                    "chunk": [0, 0], "dimension_dir": str(export / "dimensions/minecraft/overworld")}
            natural = {"0,0,0": "minecraft:stone"}
            for placed, control, error in ((4, 0, None), (3, 0, "only 3"),
                                           (0, 0, "only 0"), (4, 4, "placement control"),
                                           (4, None, "placement control")):
                with self.subTest(placed=placed, control=control):
                    blocks = {f"{i},0,0": "minecraft:stone_bricks" for i in range(placed)}
                    with mock.patch.object(atm_full, "generated_starts", return_value=[site]), \
                            mock.patch.object(atm_full, "require_chunks"), \
                            mock.patch.object(atm_full, "stand_positions", return_value=[[0, 1, 0]]), \
                            mock.patch.object(atm_full, "read_world", side_effect=[
                                [{"blocks": blocks}], [{"blocks": natural}], [{"blocks": blocks}], [{"blocks": blocks}]]):
                        if error:
                            with self.assertRaisesRegex(ValueError, error):
                                atm_full.site_proof(export, baseline, root / "discovery", root, row, "before", control_count=control)
                        else:
                            atm_full.site_proof(export, baseline, root / "discovery", root, row, "before", control_count=control)
                            self.assertEqual(row["differential"]["before"]["start_template_blocks"], 4)
                            self.assertEqual(row["differential"]["before"]["minimum"], 4)
                            self.assertEqual(row["differential"]["before"]["control_eligible_blocks"], 0)

    def test_control_count_requires_matching_start_and_dimension(self):
        site = {"anchor": [1, 2, 3], "bounds": [0, 0, 0, 4, 4, 4]}
        row = {"id": "mvs:test", "dimension": "minecraft:overworld", "initial_site": site}
        controls = {"mvs:test": {"count": 0, "dimension": "minecraft:overworld", "site": dict(site)}}
        self.assertEqual(atm_full.placement_control_count(row, controls), 0)
        controls["mvs:test"]["site"]["anchor"] = [3, 2, 1]
        self.assertIsNone(atm_full.placement_control_count(row, controls))
        self.assertIsNone(atm_full.placement_control_count(row, {}))

    def test_unobstructed_alternative_and_control_anchor(self):
        dimension = "/world/dimensions/minecraft/overworld"
        pile = {"id": "mvs:pile", "anchor": [0, 64, 0], "bounds": [0, 63, 0, 6, 66, 4],
                "chunk": [0, 0], "dimension_dir": dimension}
        farm = {"id": "mvs:farm", "anchor": [6, 63, 6], "bounds": [-5, 62, -5, 11, 66, 11],
                "chunk": [0, 0], "dimension_dir": dimension}
        alternative = {"id": "mvs:pile", "anchor": [100, 64, 100], "bounds": [100, 63, 100, 106, 66, 104],
                       "chunk": [6, 6], "dimension_dir": dimension}
        row = {"id": "mvs:pile", "dimension": "minecraft:overworld",
               "locate_attempts": [{"xz": [0, 0]}, {"xz": [100, 100]}]}
        site, rejected = atm_full.choose_unobstructed(row, [pile, farm, alternative])
        self.assertEqual(site, alternative)
        self.assertEqual(rejected[0]["conflicts"][0]["id"], "mvs:farm")
        self.assertIsNone(atm_full.choose_unobstructed(row, [pile, farm])[0])
        control = {"site": alternative}
        self.assertEqual(atm_full.choose_unobstructed(row, [pile, farm, alternative], control)[0], alternative)
        control["site"] = {**alternative, "bounds": [99, 63, 100, 106, 66, 104]}
        self.assertIsNone(atm_full.choose_unobstructed(row, [pile, farm, alternative], control)[0])

    def test_start_index_preserves_other_ids_and_rejects_bad_bounds(self):
        with tempfile.TemporaryDirectory() as directory:
            region = Path(directory) / "dimensions/minecraft/overworld/region"
            region.mkdir(parents=True)
            (region / "r.0.0.mca").touch()
            def start(identifier, bounds):
                return {"id": identifier, "ChunkX": 0, "ChunkZ": 0,
                        "BB": bounds, "KrasnokamenLocatePos": [0, 64, 0]}
            chunk = {"xPos": 0, "zPos": 0, "structures": {"starts": {
                "mvs:pile": start("mvs:pile", [0, 63, 0, 6, 66, 4]),
                "mvs:farm": start("mvs:farm", [-5, 62, -5, 11, 66, 11]),
            }}}
            with mock.patch.object(atm_full, "read_region", return_value=iter([(0, 0, chunk)])):
                found = atm_full.index_starts(Path(directory))
            self.assertEqual({item["id"] for item in found}, {"mvs:pile", "mvs:farm"})
            chunk["structures"]["starts"]["mvs:farm"]["BB"] = [9, 0, 0, 0, 1, 1]
            with mock.patch.object(atm_full, "read_region", return_value=iter([(0, 0, chunk)])):
                with self.assertRaisesRegex(ValueError, "mvs:farm: invalid exported start"):
                    atm_full.index_starts(Path(directory))

    def test_template_metadata_uses_smallest_and_excludes_nonproof_blocks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "data/mvs/structure"
            path.mkdir(parents=True)
            palette = [[("Name", (nbt.STRING, name))] for name in
                       ("minecraft:stone", "minecraft:air", "minecraft:jigsaw",
                        "minecraft:structure_void", "minecraft:structure_block")]
            for name, indices in (("small", [0, 0, 1, 2, 3, 4]), ("large", [0] * 10)):
                blocks = [[("state", (nbt.INT, index))] for index in indices]
                raw = nbt.serialize("", (nbt.COMPOUND, [
                    ("palette", (nbt.LIST, (nbt.COMPOUND, palette))),
                    ("blocks", (nbt.LIST, (nbt.COMPOUND, blocks))),
                ]))
                (path / f"{name}.nbt").write_bytes(gzip.compress(raw))
            palette_types, variants = atm_full.template_metadata(
                root, {"id": "mvs:test", "templates": ["mvs:large", "mvs:small"]})
            self.assertEqual(min(sum(variant.values()) for variant in variants), 2)
            self.assertEqual(palette_types, {"minecraft:stone"})

    def test_site_proof_records_count_and_fails_below_minimum(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            export, baseline = root / "positive", root / "baseline"
            dimension = export / "dimensions/minecraft/overworld"
            site = {"anchor": [0, 0, 0], "bounds": [0, 0, 0, 7, 0, 0],
                    "chunk": [0, 0], "dimension_dir": str(dimension)}
            row = {"id": "mvs:test", "xz": [0, 0], "dimension": "minecraft:overworld",
                   "template_palette": ["minecraft:stone", "minecraft:stone_bricks"],
                   "template_variant_counts": [{"minecraft:stone_bricks": 8}]}
            natural = {"0,0,0": "minecraft:stone"}
            blocks = {f"{i},0,0": "minecraft:stone_bricks" for i in range(8)}
            with mock.patch.object(atm_full, "generated_starts", return_value=[site]), \
                    mock.patch.object(atm_full, "require_chunks"), \
                    mock.patch.object(atm_full, "stand_positions", return_value=[[0, 1, 0]]), \
                    mock.patch.object(atm_full, "read_world", side_effect=[
                        [{"blocks": blocks}], [{"blocks": natural}], [{"blocks": blocks}], [{"blocks": blocks}]]):
                result = atm_full.site_proof(export, baseline, root / "discovery", root, row, "before", control_count=0)
            self.assertEqual(row["differential"]["before"]["eligible_blocks"], 8)
            self.assertEqual(row["differential"]["before"]["baseline_self_control_blocks"], 0)
            self.assertEqual(len(result["samples"]), 8)
            with mock.patch.object(atm_full, "generated_starts", return_value=[site]), \
                    mock.patch.object(atm_full, "require_chunks"), \
                    mock.patch.object(atm_full, "read_world", side_effect=[
                        [{"blocks": natural}], [{"blocks": natural}], [{"blocks": natural}]]):
                with self.assertRaisesRegex(ValueError, "only 0 non-natural"):
                    atm_full.site_proof(export, baseline, root / "discovery", root, row, "control", control_count=0)
            self.assertEqual(row["differential"]["control"]["eligible_blocks"], 0)
            with mock.patch.object(atm_full, "generated_starts", return_value=[site]), \
                    mock.patch.object(atm_full, "require_chunks", side_effect=[
                        None, None, ValueError("missing discovery chunks")]):
                with self.assertRaisesRegex(ValueError, "missing discovery chunks"):
                    atm_full.site_proof(export, baseline, root / "discovery", root, row, "before", control_count=0)
            with self.assertRaisesRegex(ValueError, "missing exported chunks"):
                atm_full.require_chunks(root, [0, 0, 0, 0, 0, 0])


    def test_one_finding_fails_profile_after_collecting_every_definition(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pack = root / "pack"
            pack.mkdir()
            pin_dir = root / "pin"
            pin_dir.mkdir()
            binary = pin_dir / "binary"
            binary.write_bytes(b"fixture")
            (pin_dir / "manifest.json").write_text(json.dumps({
                "head": "test-head", "diff_sha256": "candidate-diff",
                "binary_sha256": atm_full.digest(binary)}))
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps({"status": "converted-unproven", "packs": [{
                "id": "mvs", "server_pack": str(pack), "server_files": [],
                "server_sha256": "fixture",
            }]}))
            control_report = root / "control.json"
            control_report.write_text(json.dumps({
                "control_subset": True, "seed": atm_full.SEED,
                "differential_rule": "unobstructed-discovery-v2",
                "manifest_sha256": atm_full.digest(manifest), "server_pin_head": "test-head",
                "server_sha256": "control-binary", "server_pin_diff_sha256": "control-diff",
                "definitions": []}))
            env = {name: str(binary) for name in (
                "KRASNOKAMEN_SERVER_BINARY", "KRASNOKAMEN_BATTLE_HORIZON_CLIENT",
                "KRASNOKAMEN_BATTLE_HORIZON_CTL", "KRASNOKAMEN_BATTLE_HORIZON_RESOURCES",
                "KRASNOKAMEN_ATM_CACHE")}
            env["KRASNOKAMEN_ATM_MANIFEST"] = str(manifest)
            env["KRASNOKAMEN_ATM_CONTROL_REPORT"] = str(control_report)
            rows = [{"id": f"mvs:s{i:03}", "dimension": "minecraft:overworld"}
                    for i in range(130)]
            client = mock.Mock()
            client.command.return_value = {}
            proof = {"anchor": [0, 0, 0], "bounds": [0, 0, 0, 0, 0, 0], "chunk": [0, 0],
                     "dimension_dir": str(root), "samples": []}
            def fake_session(args, config, port, label, report):
                (args.output / label).mkdir()
                return contextlib.nullcontext(client)
            def choose(row, starts, control):
                rejected = [{"anchor": [0, 0, 0], "bounds": [0, 0, 0, 1, 1, 1],
                             "conflicts": [{"id": "mvs:farm", "bounds": [0, 0, 0, 2, 2, 2]}]}]
                if row["id"] == "mvs:s001":
                    return (None if len(row["locate_attempts"]) == 1 else proof, rejected)
                return proof, []


            with mock.patch.dict(atm_full.os.environ, env), \
                    mock.patch.object(cli, "ARTIFACT_ROOT", root / "receipts"), \
                    mock.patch.object(preflight, "check", return_value=()), \
                    mock.patch.object(atm_full, "inventory", return_value=[]), \
                    mock.patch.object(atm_full, "tree_hash", return_value="fixture"), \
                    mock.patch.object(atm_full, "selected_definitions", return_value=rows), \
                    mock.patch.object(atm_full, "template_metadata", return_value=({"minecraft:stone"}, [{"minecraft:stone": 8}])), \
                    mock.patch.object(atm_full, "prepare_operator_fixture"), \
                    mock.patch.object(atm_full, "session", side_effect=fake_session), \
                    mock.patch.object(atm_full, "dimension_teleport"), \
                    mock.patch.object(atm_full, "locate_coordinates", side_effect=[ValueError("missing")] + [[0, 0]] * 130), \
                    mock.patch.object(atm_full, "locate_failure", return_value="not found"), \
                    mock.patch.object(atm_full, "export_world", return_value=root), \
                    mock.patch.object(atm_full, "find_site", return_value=proof), \
                    mock.patch.object(atm_full, "index_starts", return_value=[]), \
                    mock.patch.object(atm_full, "choose_unobstructed", side_effect=choose), \
                    mock.patch.object(atm_full, "site_proof", return_value=proof), \
                    mock.patch.object(atm_full, "read_world", return_value=[{"blocks": {}}]), \
                    mock.patch.object(atm_full, "photograph", return_value={}):
                code = cli.cmd_run(["atm-full"])
            receipt_dir = next((root / "receipts").iterdir())
            receipt = json.loads((receipt_dir / "result.json").read_text())
            report = json.loads((receipt_dir / "full-atm/result.json").read_text())
            self.assertNotEqual(code, 0)
            self.assertEqual(receipt["status"], "failed")
            self.assertEqual(receipt["details"]["scenario"], report)
            self.assertEqual(report["status"], "findings")
            self.assertFalse(report["full_target_accepted"])
            self.assertEqual(len(report["definitions"]), 130)
            self.assertEqual(report["definitions"][0]["reason"], "not found")
            self.assertTrue(all(row["status"] == "verified" for row in report["definitions"][1:]))
            self.assertEqual(len(report["definitions"][1]["locate_attempts"]), 2)
            self.assertEqual(report["definitions"][1]["obstructed_starts"][0]["conflicts"][0]["id"], "mvs:farm")
            self.assertEqual(client.command.call_count, 131)


if __name__ == "__main__":
    unittest.main()
