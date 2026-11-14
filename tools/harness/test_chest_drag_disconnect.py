"""Gate control-flow checks; graphical receipts remain required."""
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from . import scenarios
from .backends import native_chest_drag_disconnect as gate, native_v2


class ChestDragTests(unittest.TestCase):
    def test_registered_graphical(self):
        name = "native-v2-chest-drag-disconnect"
        self.assertIn(name, native_v2.RUNNERS)
        self.assertEqual(scenarios.SCENARIOS[name].evidence, "graphical")

    def test_gate_rejects_loss_and_duplicate_after_rejoin(self):
        log = {"item": "minecraft:oak_log", "count": 1, "components": {}}
        empty = {"item": None, "count": 0, "components": {}}
        opened = {"container": {"slot_count": 63, "slots": [
            {"slot": 54, "item_id": "minecraft:oak_log", "count": 1}]}}
        for final_slots, error in (([None] * 63, "chest_rejoin_server_custody_exactly_one"),
                                   ([log, log] + [None] * 61, "chest_rejoin_no_chest_duplicate"),
                                   ([None] * 27 + [log] + [None] * 35, None)):
            with self.subTest(error=error):
                steps = mock.Mock()
                steps.client = SimpleNamespace(p4_window={}, server_address="127.0.0.1:1")
                steps.client.p4_window = {"display": ":1", "window_id": 1}
                steps.remaining.return_value = 30
                steps.call.return_value = {"in_play": False}
                def check(label, condition):
                    if not condition:
                        raise AssertionError(label)
                steps.check.side_effect = check
                snapshots = [{"carried": log, "slots": [None] * 63},
                             {"carried": empty, "slots": final_slots}]
                with mock.patch.object(gate, "_set"), mock.patch.object(gate, "_give"), \
                     mock.patch.object(gate, "_open", return_value=opened), \
                     mock.patch.object(gate, "_echo", return_value={"inventory": [
                         {"item_id": "minecraft:oak_log", "count": 1}]}), \
                     mock.patch.object(gate, "observe_menu", side_effect=snapshots), \
                     mock.patch.object(gate, "Pointer"), mock.patch.object(gate, "center", return_value=(1, 1)), \
                     mock.patch.object(gate.native_client, "capture"):
                    if error:
                        with self.assertRaisesRegex(AssertionError, error):
                            gate.run(steps, (0, 0, 0), Path("/tmp"))
                    else:
                        gate.run(steps, (0, 0, 0), Path("/tmp"))
