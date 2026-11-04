"""Evidence scanning must not turn generated/natural geometry into stage proof."""
import unittest
from unittest.mock import Mock, patch

from .backends import civilization_seed_qa as qa


class CivilizationEvidenceTests(unittest.TestCase):
    def scan(self, buildings, response):
        probe = Mock()
        probe.call.return_value = response
        with patch.object(qa.village, "_listed_buildings", return_value=buildings):
            evidence = qa.building_blocks(probe, [])
        return probe, evidence

    def test_retains_beds_and_furnaces_with_truncation_flag(self):
        building = {"natural": False, "min": [0, 64, 0], "max": [2, 67, 2]}
        blocks = [{"block_id": name} for name in (
            "minecraft:red_bed", "minecraft:furnace", "minecraft:blast_furnace", "minecraft:stone")]
        probe, evidence = self.scan([building], {"blocks": blocks, "truncated": True})
        self.assertEqual(evidence, [{"building": building, "blocks": blocks[:3], "truncated": True}])
        probe.call.assert_called_once_with("minecraft_scan_blocks", {
            "min_x": 0, "min_y": 64, "min_z": 0, "max_x": 2, "max_y": 67, "max_z": 2})

    def test_natural_zone_does_not_become_building_evidence(self):
        probe, evidence = self.scan([{"natural": True}], {})
        probe.call.assert_not_called()
        self.assertEqual(evidence, [])

    def test_oversize_scan_is_explicitly_unobserved(self):
        building = {"natural": False, "min": [0, 64, 0], "max": [31, 95, 31]}
        probe, evidence = self.scan([building], {})
        probe.call.assert_not_called()
        self.assertEqual(evidence, [{"building": building, "skipped": "scan volume exceeds 4096"}])

    def test_empty_scan_never_proves_beds_or_furnaces(self):
        building = {"natural": False, "min": [0, 64, 0], "max": [2, 67, 2]}
        _, evidence = self.scan([building], {"blocks": [], "truncated": False})
        self.assertEqual(evidence[0]["blocks"], [])
        self.assertFalse(evidence[0]["truncated"])

    def test_waypoint_uses_observed_ground_and_clear_dry_headroom(self):
        def cell(y, empty, dry=True):
            return {"y": y, "collision_empty": empty, "fluid": {"empty": dry}}
        self.assertEqual(qa.standable_height([cell(70, False), cell(71, True),
                                              cell(72, True)], 69), 71)
        self.assertIsNone(qa.standable_height([cell(70, False), cell(71, True),
                                               cell(72, False)], 69))
        self.assertIsNone(qa.standable_height([cell(70, False), cell(71, True, False),
                                               cell(72, True)], 69))

    def test_storage_floor_may_be_below_chest_without_overlapping_building(self):
        floor = [{"x": x, "y": 76, "z": z, "collision_empty": False}
                 for x in range(131, 134) for z in range(-218, -215)]
        self.assertEqual(qa.storage_bounds((132, 78, -217), floor, []),
                         ([131, 76, -218], [133, 79, -216]))
        self.assertIsNone(qa.storage_bounds((132, 78, -217), floor[:-1], []))
        self.assertIsNone(qa.storage_bounds((132, 78, -217), floor,
                                           [{"min": [132, 76, -217], "max": [132, 78, -217]}]))

    def test_iron_capture_opens_furnace_and_requires_visible_output(self):
        probe = Mock()
        empty = {"container": {"container_id": 4,
                               "menu_class": "net.minecraft.world.inventory.FurnaceMenu", "slots": []}}
        full = {"container": {"container_id": 4,
                              "menu_class": "net.minecraft.world.inventory.FurnaceMenu",
                              "slots": [{"slot": 2, "item_id": "minecraft:iron_ingot", "count": 2}]}}

        def wait(predicate, label, **kwargs):
            self.assertFalse(predicate(empty))
            self.assertTrue(predicate(full))
            return full

        probe.wait.side_effect = wait
        with patch.object(qa.native_client, "capture", return_value={"retained_path": "iron.png"}) as capture:
            result = qa.capture_iron_menu(probe, "artifact", (1, 2, 3), 600)
        self.assertEqual(result["container"], full["container"])
        capture.assert_called_once_with(probe.client, "artifact", "civilization-iron-000600")
        self.assertEqual([call.args[0] for call in probe.call.call_args_list],
                         ["minecraft_close_screen", "minecraft_set_hotbar_slot",
                          "minecraft_use_item_on", "minecraft_close_screen"])

    def test_observer_is_safe_only_after_ordinary_claim_and_stock_transfer(self):
        probe = Mock()
        player = {"player": {"x": 1.0, "y": 70.0, "z": 1.0}}
        probe.wait.return_value = player
        census = 0

        def call(name, args):
            nonlocal census
            if name == "minecraft_list_entities":
                census += 1
                if census == 2:
                    raise RuntimeError("observation begins")
            return {}

        probe.call.side_effect = call
        detail = {"fixture_commands": []}
        probe.report = detail
        events = []
        with patch.object(qa, "walk_to_village", return_value=player), \
             patch.object(qa.village, "_send", side_effect=[
                 ["Settlements: Test."],
                 ["owner=autonomous anchor=[1, 70, 1]"],
                 ["Claimed Test"]]), \
             patch.object(qa.village, "_recent_chat", side_effect=lambda lines, _: lines), \
             patch.object(qa.village, "_founding_storage", return_value=(1, 70, 1)), \
             patch.object(qa.village, "_open_chest", return_value={}), \
             patch.object(qa, "fund", side_effect=lambda *args: events.append("stock")), \
             patch.object(qa.item_merge, "_console",
                          side_effect=lambda _, command: events.append(command)), \
             patch.object(qa.native_client, "capture", return_value={}):
            with self.assertRaisesRegex(RuntimeError, "observation begins"):
                qa.scenario(probe, detail, "artifact", Mock(), "server.log", Mock())
        self.assertEqual(events, ["stock", f"gamemode creative {qa.village.USERNAME}"])
        self.assertEqual(detail["fixture_commands"],
                         [f"console: gamemode creative {qa.village.USERNAME}"])


if __name__ == "__main__":
    unittest.main()
