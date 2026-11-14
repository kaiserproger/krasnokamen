"""Unit checks for the bounded chat window; graphical gates prove the real path."""
import json
import unittest

from .backends.menu_observer import holds, observe_menu


class MenuObserverTests(unittest.TestCase):
    def test_uses_snapshot_newer_than_fence_when_old_messages_are_evicted(self):
        old = {"container_id": 0, "carried": {"item": None, "count": 0}, "slots": [None] * 46}
        new = {**old, "carried": {"item": "minecraft:oak_log", "count": 1}}

        class Steps:
            report = {}
            marker = ""
            def call(self, name, args):
                if not args.get("command"):
                    self.marker = args["message"]
            def wait(self, predicate, label):
                lines = [self.marker] if label.endswith("_fence") else [
                    "KRASNOKAMEN_MENU " + json.dumps(new), self.marker,
                    "KRASNOKAMEN_MENU " + json.dumps(old),
                ]
                state = {"recent_chat": lines}
                assert predicate(state)
                return state
            def check(self, label, condition):
                assert condition, label

        steps = Steps()
        self.assertEqual(observe_menu(steps, "test"), new)
        self.assertEqual(steps.report["server_menu_snapshots"]["test"], new)

    def test_holds_ignores_reported_components(self):
        stack = {"item": "minecraft:oak_log", "count": 1, "components": {"minecraft:custom_name": "x"}}
        self.assertTrue(holds(stack, "minecraft:oak_log", 1))
        self.assertFalse(holds(stack, "minecraft:oak_log", 2))
        self.assertFalse(holds({"item": None, "count": 0, "components": {}}, "minecraft:oak_log", 1))
        self.assertFalse(holds(None, "minecraft:oak_log", 1))
