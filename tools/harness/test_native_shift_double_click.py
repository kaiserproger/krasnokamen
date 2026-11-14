import tomllib
import unittest
from pathlib import Path
from unittest.mock import MagicMock, call, patch

from tools.harness import scenarios
from tools.harness.backends import client_probes, java_p5, native_v2, shift_correction_wire, shift_matrix
from tools.harness.backends import native_shift_double_click as scenario


class ShiftDoubleClickHarnessTests(unittest.TestCase):
    def test_external_world_path_is_toml_quoted_and_absolute(self):
        artifact = Path('/tmp/receipts with "quotes"/run')
        parsed = tomllib.loads('[data]\nworld_dir = ' + client_probes.world_dir_toml(artifact))
        self.assertEqual(parsed['data']['world_dir'], str(artifact / 'world'))

    def test_scenario_is_registered_as_windowed_native_runner(self):
        name = 'native-v2-shift-double-click'
        entry = scenarios.SCENARIOS[name]
        self.assertEqual(entry.evidence, 'graphical')
        self.assertEqual(entry.scenario_id, name)
        self.assertIn(name, native_v2.NATIVE_SCENARIO_IDS)
        self.assertIn(name, native_v2.RUNNERS)
        java_name = 'java-v2-p5-shift-double-click'
        self.assertIn(java_name, java_p5.SCENARIO_IDS)
        self.assertEqual(scenarios.SCENARIOS[java_name].evidence, 'graphical')

    def test_stone_total_avoids_virtual_result_and_includes_cursor(self):
        empty = {"item": None, "count": 0}
        stone = {"item": "minecraft:stone", "count": 10}
        image = {"container_id": 0, "slots": [stone, stone, empty], "carried": stone}
        self.assertEqual(shift_matrix._total(image), 20)
        image["container_id"] = 1
        self.assertEqual(shift_matrix._total(image), 30)

    def test_correction_wire_arms_only_matching_chest_click(self):
        wire = shift_correction_wire.CorrectionWire.__new__(shift_correction_wire.CorrectionWire)
        wire.command, wire.pending, wire.click_id = 'debug give minecraft:stone 0 0', False, 18
        wire.close_events, wire.close_id = None, 19
        wire.receipt = []
        wire._on_click(None, 'play', 18, bytes([1, 5, 0, 0]))
        self.assertTrue(wire.pending)
        self.assertEqual(wire.receipt[0]['slot'], 0)
        wire.pending = False
        with self.assertRaisesRegex(RuntimeError, 'expected chest slot 0'):
            wire._on_click(None, 'play', 18, bytes([1, 5, 0, 1]))

    def test_physical_input_and_authoritative_negative_control(self):
        empty = {"count": 0, "item": None}
        stone = lambda count: {"count": count, "item": "minecraft:stone"}
        before = {"slots": [dict(empty) for _ in range(46)]}
        before["slots"][9] = stone(10)
        for index in range(36, 45):
            before["slots"][index] = stone(64)
        moved = {"slots": [dict(slot) for slot in before["slots"]]}
        moved["slots"][36] = dict(empty)
        moved["slots"][10] = stone(64)
        steps = MagicMock()
        steps.report = {}
        steps.wait.return_value = {"screen": {"open": True}, "container": {"container_id": 0}}
        def check(label, condition):
            if not condition:
                raise AssertionError(label)
        steps.check.side_effect = check
        pointer = MagicMock()
        pointer.x.XKeysymToKeycode.return_value = 50
        pointer.xt.XTestFakeKeyEvent.return_value = 1
        with patch.object(scenario, "Pointer", return_value=pointer), \
             patch.object(shift_matrix, "slot_center", return_value=(10, 10)), \
             patch.object(scenario, "_echo"), \
             patch.object(shift_matrix.native_client, "capture"), \
             patch.object(scenario, "observe_menu", side_effect=[before, moved]):
            scenario.run(steps, Path("/tmp/artifact"), {})
        self.assertEqual(pointer.button.call_args_list, [call(True), call(False), call(True), call(False)])
        self.assertEqual(pointer.xt.XTestFakeKeyEvent.call_count, 2)
        with patch.object(scenario, "Pointer", return_value=pointer), \
             patch.object(shift_matrix, "slot_center", return_value=(10, 10)), \
             patch.object(scenario, "_echo"), \
             patch.object(shift_matrix.native_client, "capture"), \
             patch.object(scenario, "observe_menu", side_effect=[before, before]):
            with self.assertRaisesRegex(AssertionError, "shift_hotbar_first_stack_moved"):
                scenario.run(steps, Path("/tmp/artifact"), {})


if __name__ == '__main__':
    unittest.main()
