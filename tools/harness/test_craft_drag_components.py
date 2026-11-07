"""Exercise the scenario call sites with the server's component-bearing stacks."""
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from PIL import Image

from .backends import drag_variants, java_craft_drag, native_craft_drag_paced


class Steps:
    def __init__(self):
        self.client = SimpleNamespace(p4_window={})
        self.report = {"assertions": {}}
        self.initial = {"screen": {"open": True}, "container": {
            "container_id": 0, "slot_count": 46,
            "slots": [{"slot": 36, "item_id": "minecraft:oak_log", "count": 1}],
            "carried": {"count": 0}}}
        self.moved = {"container": {"slot_count": 46, "slots": [], "carried": {
            "item_id": "minecraft:oak_log", "count": 1, "origin": "server_observed"}},
            "inventory": []}
        self.final = {"inventory": [{"item_id": "minecraft:oak_log", "count": 1}]}

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False

    def observe(self):
        return self.initial

    def wait(self, predicate, label):
        state = self.initial if "inventory_open" in label else self.moved
        assert predicate(state), label
        return state

    def call(self, *args, **kwargs):
        return {}

    def fixture(self, *args):
        pass

    def remaining(self):
        return 30

    def check(self, label, condition):
        self.report["assertions"][label] = bool(condition)
        if not condition:
            raise AssertionError(label)


class CraftDragComponentsTests(unittest.TestCase):
    def test_java_and_native_scenarios_accept_component_bearing_server_stacks(self):
        stack = {"item": "minecraft:oak_log", "count": 1, "components": {"minecraft:custom_name": "x"}}
        empty = {"item": None, "count": 0, "components": {}}
        def server(_steps, label):
            closed = label.endswith("closed")
            return {"carried": empty if closed else stack,
                    "slots": [None, stack] + [None] * 44 if closed else [None] * 46}

        with tempfile.TemporaryDirectory() as directory:
            image = Path(directory) / "screen.png"
            Image.new("RGB", (640, 480)).save(image)
            for paced in (False, True):
                with self.subTest(client="java", paced=paced):
                    steps = Steps()
                    steps.observe = lambda: steps.final
                    with mock.patch.object(java_craft_drag, "ProbeSteps", return_value=steps), \
                         mock.patch.object(java_craft_drag, "_prepare"), \
                         mock.patch.object(java_craft_drag, "_echo", return_value=steps.moved), \
                         mock.patch.object(java_craft_drag, "observe_menu", side_effect=server), \
                         mock.patch.object(java_craft_drag, "Pointer"), \
                         mock.patch.object(java_craft_drag.java_p5, "capture", return_value=str(image)), \
                         mock.patch.object(drag_variants, "run"):
                        java_craft_drag.run(steps.client, Path(directory), 30, paced=paced)
                    self.assertTrue(steps.report["assertions"]["java_drag_up_server_keeps_cursor"])
                    if not paced:
                        self.assertTrue(steps.report["assertions"]["java_unfenced_server_conserves_log"])
            steps = Steps()
            with mock.patch.object(native_craft_drag_paced, "_echo", return_value=steps.moved), \
                 mock.patch.object(native_craft_drag_paced, "observe_menu", side_effect=server), \
                 mock.patch.object(native_craft_drag_paced, "Pointer"), \
                 mock.patch.object(native_craft_drag_paced, "center", return_value=(1, 1)), \
                 mock.patch.object(native_craft_drag_paced.native_client, "capture", side_effect=[
                     {"frame_id": 1}, {"frame_id": 2}, {"frame_id": 3}]), \
                 mock.patch.object(drag_variants, "run"):
                native_craft_drag_paced.run(steps, Path(directory), {})
            self.assertTrue(steps.report["assertions"]["native_drag_up_server_keeps_cursor"])
