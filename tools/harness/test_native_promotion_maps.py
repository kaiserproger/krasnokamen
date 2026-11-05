"""Map scene/oracle fences, not real-client acceptance evidence."""
from __future__ import annotations

import unittest
from pathlib import Path
from unittest import mock

from .backends import native_promotion_maps as maps


def steps():
    result = mock.Mock(report={}, artifact_dir=Path("unused-map-artifact"))
    result.client.p5_armor_actor = object()
    def check(label, condition):
        if not condition:
            raise RuntimeError(label)
    result.check.side_effect = check
    return result


class MapPromotionTests(unittest.TestCase):
    def test_pumpkin_scene_keeps_an_ordinary_offhand_clone_before_equipment(self):
        primary = steps()
        with mock.patch.object(maps, "ProbeSteps") as scope, \
             mock.patch.object(maps, "_create", return_value=5), \
             mock.patch.object(maps, "_clone") as clone, mock.patch.object(maps, "_give"), \
             mock.patch.object(maps, "_echo", return_value={"inventory": []}), \
             mock.patch.object(maps, "_held_shot"), mock.patch.object(maps, "_click") as click:
            actor = scope.return_value.__enter__.return_value
            actor.wait.return_value = {"inventory": []}
            maps._players(primary, (0, 201, 0), True)
        self.assertEqual(clone.call_args_list, [mock.call(actor, 5, 10), mock.call(actor, 5, 0)])
        inputs = [call for call in actor.call.call_args_list if call.args[0] == "minecraft_press_inputs"]
        self.assertEqual(inputs, [
            mock.call("minecraft_press_inputs", {"keys": ["swap_offhand"], "ticks": 1})])
        self.assertEqual(click.call_args_list, [mock.call(actor, 36), mock.call(actor, 5)])
        labels = [call.args[1] for call in primary.wait.call_args_list]
        self.assertEqual(labels, ["map_observer_sees_carrier_of_held_clone",
                                  "map_pumpkin_carrier_hidden_from_other_player"])

    def test_pocket_map_probe_rejects_same_map_id_before_starting_actor_fixture(self):
        primary = steps()
        with mock.patch.object(maps, "_create", return_value=5), \
             mock.patch.object(maps, "ProbeSteps") as scope:
            with self.assertRaisesRegex(RuntimeError, "map_distinct_ids_for_same_item_type"):
                maps._players(primary, (0, 201, 0), False)
        scope.assert_not_called()

    def test_pocket_clone_uses_supported_hotbar_fixture_and_ordinary_inventory_clicks(self):
        primary = steps()
        primary.call.return_value = {
            "item_id": "minecraft:filled_map", "components": {"minecraft:map_id": 5}}
        with mock.patch.object(maps, "_echo"), mock.patch.object(maps, "_click") as click:
            maps._clone(primary, 5, 10)
        primary.fixture.assert_called_once_with("debug give minecraft:filled_map[map_id=5] 1 0")
        self.assertEqual(click.call_args_list, [mock.call(primary, 36), mock.call(primary, 10)])
        primary.call.assert_any_call("minecraft_open_inventory")
        primary.call.assert_any_call("minecraft_close_screen")

    def test_banner_review_does_not_hide_the_banner_under_the_player_pointer(self):
        primary = steps()
        current = {"player": {"x": .5, "y": 201, "z": .5}}
        primary.observe.return_value = current
        def fixture(command):
            if command.startswith("tp @s "):
                current["player"].update(zip(("x", "y", "z"), map(float, command.split()[2:])))
        primary.fixture.side_effect = fixture
        primary.wait.side_effect = lambda predicate, label, **kwargs: primary.check(label, predicate(current))
        shots = []
        with mock.patch.object(maps, "_create", return_value=0), \
             mock.patch.object(maps, "_locked", return_value=1), \
             mock.patch.object(maps, "_set"), mock.patch.object(maps, "_use"), \
             mock.patch.object(maps, "_marker_count", side_effect=(1, 0)), \
             mock.patch.object(maps, "_map", return_value={"colors": [0] * 16384}), \
             mock.patch.object(maps, "_held_shot", side_effect=lambda *_: shots.append(current["player"]["x"])):
            maps._banners(primary, (0, 201, 0), True, {})
        self.assertEqual(len(shots), 2)
        self.assertTrue(all(abs(x) >= 8 for x in shots), shots)
        self.assertEqual(current["player"]["x"], .5)


if __name__ == "__main__":
    unittest.main()
