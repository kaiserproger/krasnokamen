"""Full ATM manifest and restart sampling contracts."""
import json
import contextlib
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from . import atm_full
from . import __main__ as cli, preflight


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

    def test_rejects_path_escape(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "unsafe resource"):
                atm_full.resource(Path(directory), "mvs:../../../../outside", "structure", ".nbt")

    def test_differential_rejects_natural_and_beard_only_changes(self):
        baseline = {"0,0,0": "minecraft:stone", "1,0,0": "minecraft:dirt"}
        natural = {"minecraft:air", "minecraft:stone", "minecraft:dirt"}
        palette = {"minecraft:stone", "minecraft:dirt", "minecraft:stone_bricks"}
        self.assertEqual(atm_full.differential_candidates(baseline, baseline, natural, palette), [])
        beard = {"0,0,0": "minecraft:dirt", "1,0,0": "minecraft:stone"}
        self.assertEqual(atm_full.differential_candidates(beard, baseline, natural, palette), [])
        placed = {**beard, "2,0,0": "minecraft:stone_bricks[axis=x]"}
        matches = atm_full.differential_candidates(placed, baseline, natural, palette)
        self.assertEqual(matches, [{"position": [2, 0, 0], "state": "minecraft:stone_bricks[axis=x]"}])

    def test_site_proof_records_count_and_fails_below_minimum(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            export, baseline = root / "positive", root / "baseline"
            dimension = export / "dimensions/minecraft/overworld"
            site = {"anchor": [0, 0, 0], "bounds": [0, 0, 0, 7, 0, 0],
                    "chunk": [0, 0], "dimension_dir": str(dimension)}
            row = {"id": "mvs:test", "xz": [0, 0], "dimension": "minecraft:overworld"}
            natural = {"0,0,0": "minecraft:stone"}
            blocks = {f"{i},0,0": "minecraft:stone_bricks" for i in range(8)}
            with mock.patch.object(atm_full, "generated_starts", return_value=[site]), \
                    mock.patch.object(atm_full, "require_chunks"), \
                    mock.patch.object(atm_full, "template_palette", return_value={"minecraft:stone", "minecraft:stone_bricks"}), \
                    mock.patch.object(atm_full, "stand_positions", return_value=[[0, 1, 0]]), \
                    mock.patch.object(atm_full, "read_world", side_effect=[
                        [{"blocks": blocks}], [{"blocks": natural}], [{"blocks": blocks}]]):
                result = atm_full.site_proof(export, baseline, root, row, "before")
            self.assertEqual(row["differential"]["before"]["eligible_blocks"], 8)
            self.assertEqual(row["differential"]["before"]["baseline_self_control_blocks"], 0)
            self.assertEqual(len(result["samples"]), 8)
            with mock.patch.object(atm_full, "generated_starts", return_value=[site]), \
                    mock.patch.object(atm_full, "require_chunks"), \
                    mock.patch.object(atm_full, "template_palette", return_value={"minecraft:stone"}), \
                    mock.patch.object(atm_full, "read_world", side_effect=[
                        [{"blocks": natural}], [{"blocks": natural}]]):
                with self.assertRaisesRegex(ValueError, "only 0 non-natural"):
                    atm_full.site_proof(export, baseline, root, row, "control")
            self.assertEqual(row["differential"]["control"]["eligible_blocks"], 0)
            with self.assertRaisesRegex(ValueError, "missing exported chunks"):
                atm_full.require_chunks(root, [0, 0, 0, 0, 0, 0])


    def test_one_finding_fails_profile_after_collecting_every_definition(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pack = root / "pack"
            pack.mkdir()
            binary = root / "binary"
            binary.write_bytes(b"fixture")
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps({"status": "converted-unproven", "packs": [{
                "id": "mvs", "server_pack": str(pack), "server_files": [],
                "server_sha256": "fixture",
            }]}))
            env = {name: str(binary) for name in (
                "KRASNOKAMEN_SERVER_BINARY", "KRASNOKAMEN_BATTLE_HORIZON_CLIENT",
                "KRASNOKAMEN_BATTLE_HORIZON_CTL", "KRASNOKAMEN_BATTLE_HORIZON_RESOURCES",
                "KRASNOKAMEN_ATM_CACHE")}
            env["KRASNOKAMEN_ATM_MANIFEST"] = str(manifest)
            rows = [{"id": f"mvs:s{i:03}", "dimension": "minecraft:overworld"}
                    for i in range(130)]
            client = mock.Mock()
            client.command.return_value = {}
            proof = {"anchor": [0, 0, 0], "bounds": [0, 0, 0, 0, 0, 0], "chunk": [0, 0],
                     "dimension_dir": str(root), "samples": []}
            with mock.patch.dict(atm_full.os.environ, env), \
                    mock.patch.object(cli, "ARTIFACT_ROOT", root / "receipts"), \
                    mock.patch.object(preflight, "check", return_value=()), \
                    mock.patch.object(atm_full, "inventory", return_value=[]), \
                    mock.patch.object(atm_full, "tree_hash", return_value="fixture"), \
                    mock.patch.object(atm_full, "selected_definitions", return_value=rows), \
                    mock.patch.object(atm_full, "prepare_operator_fixture"), \
                    mock.patch.object(atm_full, "session", side_effect=lambda *a: contextlib.nullcontext(client)), \
                    mock.patch.object(atm_full, "dimension_teleport"), \
                    mock.patch.object(atm_full, "locate_coordinates", side_effect=[ValueError("missing")] + [[0, 0]] * 129), \
                    mock.patch.object(atm_full, "locate_failure", return_value="not found"), \
                    mock.patch.object(atm_full, "export_world", return_value=root), \
                    mock.patch.object(atm_full, "find_site", return_value=proof), \
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
            self.assertEqual(client.command.call_count, 130)


if __name__ == "__main__":
    unittest.main()
