"""Original cursor-close oracle fences; not real-client evidence."""
from __future__ import annotations

import unittest
from unittest import mock

from .backends import native_promotion_items as items


def state(*, cursor=0, inventory=True):
    apples = [{"item_id": "minecraft:apple", "count": 3, "slot": 0}] if inventory else []
    return {"inventory": apples, "screen": {"open": True},
            "container": {"container_id": 0, "slots": [
                {"item_id": "minecraft:apple", "count": 3, "slot": 36}],
                "carried": {"item_id": "minecraft:apple" if cursor else "minecraft:air", "count": cursor}}}


class Steps:
    def __init__(self, states):
        self.states = iter(states)
        self.report = {}
        self.calls = []

    def fixture(self, command):
        self.calls.append(("fixture", command))

    def call(self, name, arguments=None):
        self.calls.append((name, arguments))
        return {}

    def check(self, label, condition):
        if not condition:
            raise RuntimeError(label)

    def wait(self, predicate, label, **kwargs):
        current = next(self.states)
        self.check(label, predicate(current))
        return current


class CursorPromotionTests(unittest.TestCase):
    def test_phantom_is_captured_then_rejected_without_repair_click(self):
        steps = Steps([state(), state(), state(cursor=3, inventory=False), state(cursor=3)])
        shots = []
        with mock.patch.object(items, "_set"), mock.patch.object(items, "_click") as click, \
             mock.patch.object(items, "_echo", return_value=state()), \
             mock.patch.object(items, "_capture", side_effect=lambda _, name: shots.append(name)):
            with self.assertRaisesRegex(RuntimeError, "inventory_reopen_cursor_empty_before_any_click"):
                items.cursor_close(steps, (0, 64, 0))
        click.assert_called_once_with(steps, 36)
        self.assertEqual(shots, ["cursor-inventory-before-close", "cursor-inventory-reopened-before-any-click"])
        self.assertEqual([name for name, _ in steps.calls if name == "minecraft_close_screen"],
                         ["minecraft_close_screen"])

    def test_both_original_menus_conserve_three_apples_with_only_pickup_clicks(self):
        steps = Steps([state(), state(), state(cursor=3, inventory=False), state(),
                       state(cursor=3, inventory=False)])
        shots = []
        with mock.patch.object(items, "_set"), mock.patch.object(items, "_click") as click, \
             mock.patch.object(items, "_echo", return_value=state()), \
             mock.patch.object(items, "_open", return_value=state()), \
             mock.patch.object(items, "_capture", side_effect=lambda _, name: shots.append(name)):
            items.cursor_close(steps, (0, 64, 0))
        self.assertEqual(click.call_args_list, [mock.call(steps, 36), mock.call(steps, 36)])
        self.assertEqual(len(shots), 4)
        self.assertEqual(shots[-1], "cursor-crafting_table-reopened-before-any-click")

    def test_placement_does_not_accept_without_unmodified_wire_receipt(self):
        steps = mock.Mock()
        steps.client.promotion_wire_receipt = None
        with self.assertRaisesRegex(RuntimeError, "unmodified graphical wire"):
            items.banner_placement(steps, (0, 64, 0))
        steps.fixture.assert_not_called()


if __name__ == "__main__":
    unittest.main()
