"""Full ATM manifest and restart sampling contracts."""
import json
from pathlib import Path
import tempfile
import unittest

from . import atm_full


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


if __name__ == "__main__":
    unittest.main()
