"""Static tests of the settlement storage real-client leg; not real-client evidence."""
from __future__ import annotations

import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from .backends import client_probes
from .backends import native_settlement_storage as storage


class SettlementStorageLegTests(unittest.TestCase):
    def test_storage_route_uses_physical_dashboard_controls(self) -> None:
        leg = mock.Mock()
        result = storage._open_storage_from_dashboard(leg)
        self.assertEqual(leg.mock_calls, [
            mock.call.main("main_dialog_storage"),
            mock.call.button("Settlement", "main_settlement"),
            mock.call.menu("Settlement", "settlement_dashboard_open"),
            mock.call.slot(16, "dashboard_resources"),
            mock.call.menu("Resources", "resources_open"),
            mock.call.slot(52, "resources_storage"),
            mock.call.menu("Storage · storeqa", "storage_menu_open"),
        ])
        self.assertEqual(result, leg.menu.return_value)

    def test_resident_card_requires_named_chest_screen(self) -> None:
        state = {"screen": {"class": storage.CONTAINER, "title": "Resident · Anna"},
                 "container": {"menu_class": "net.minecraft.world.inventory.ChestMenu", "slot_count": 90}}
        self.assertTrue(storage.in_resident_card(state))
        state["screen"]["title"] = "Residents / households"
        self.assertFalse(storage.in_resident_card(state))
        state["screen"]["title"] = "Resident · Anna"
        state["container"]["slot_count"] = 63
        self.assertFalse(storage.in_resident_card(state))

    def test_lore_flattens_text_components(self) -> None:
        components = {"minecraft:lore": [
            {"text": "In storage: 70", "italic": False, "color": "gray"},
            "plain",
            {"text": "", "extra": [{"text": "a"}, "b"]},
        ]}
        self.assertEqual(storage.lore(components), ["In storage: 70", "plain", "ab"])
        self.assertEqual(storage.lore({}), [])

    def test_since_takes_only_lines_newer_than_the_marker(self) -> None:
        # recent_chat lists the newest line first.
        lines = ["Your inventory is full.", "<HarnessEntities> storage-qa full abc", "Your inventory is full."]
        self.assertEqual(storage.since(lines, "storage-qa full abc"), ["Your inventory is full."])
        self.assertEqual(storage.since(lines, "absent"), [])

    def test_icon_total_reads_the_uncapped_name_total(self) -> None:
        self.assertEqual(storage.icon_total({"name": "Cobblestone ×120", "count": 99}), 120)
        with self.assertRaisesRegex(RuntimeError, "×total"):
            storage.icon_total({"name": "Cobblestone", "count": 1})

    def test_villager_profession_reads_received_metadata(self) -> None:
        row = {"received_metadata": {"0": {"kind": "byte", "value": 0},
                                     "18": {"kind": "villager", "value": [2, 5, 1]}}}
        self.assertEqual(storage.villager_profession(row), 5)
        self.assertIsNone(storage.villager_profession({"received_metadata": {}}))

    def test_short_id_and_hover_target_match_whole_numbers(self) -> None:
        self.assertEqual(storage.short_id((3 << 32) | 7), 7)
        hovers = {"a": "Storage #12 L1\n1 2 3 .. 4 5 6", "b": "Farmer workshop #1 L2\n1 2 3 .. 4 5 6"}
        self.assertEqual(storage.hover_target(hovers, 1), "b")
        self.assertIsNone(storage.hover_target({"a": "Storage #12 L1"}, 1))
        # A shared cell opens its first building only; the viewer's line is skipped.
        shared = {"a": "You are here: 1 2\nStorage #1 L1\n1 2 3 .. 4 5 6\nFarmer workshop #2 L2\n1 2 3 .. 4 5 6",
                  "b": "You are here: 1 2\nFarmer workshop #2 L2\n1 2 3 .. 4 5 6"}
        self.assertEqual(storage.hover_target(shared, 2), "b")
        self.assertEqual(storage.hover_target(shared, 1), "a")

    def test_icon_readers_ignore_other_menus(self) -> None:
        inventory = {"container": {"slot_count": 46, "slots": [
            {"slot": 36, "item_id": storage.COBBLE, "count": 64, "name": "Cobblestone"}]}}
        self.assertIsNone(storage.icon(inventory, storage.COBBLE))
        self.assertEqual(storage.own(inventory, storage.COBBLE), 0)
        self.assertEqual(storage.stocked(inventory, storage.COBBLE), 0)
        menu = {"container": {"slot_count": 90, "slots": [
            {"slot": 3, "item_id": storage.COBBLE, "count": 70, "name": "Cobblestone ×70"},
            {"slot": 89, "item_id": storage.COBBLE, "count": 64, "name": "Cobblestone"}]}}
        self.assertEqual(storage.stocked(menu, storage.COBBLE), 70)
        self.assertEqual(storage.own(menu, storage.COBBLE), 64)

    def test_farmer_id_comes_from_the_local_vanilla_report(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            reports = Path(root) / "reports"
            reports.mkdir()
            (reports / "registries.json").write_text(json.dumps({"minecraft:villager_profession": {
                "entries": {"minecraft:none": {"protocol_id": 0}, "minecraft:farmer": {"protocol_id": 5}}}}))
            with mock.patch.dict(os.environ, {"KRASNOKAMEN_SERVER_VANILLA_DATA_DIR": root}):
                self.assertEqual(storage.farmer_id(), 5)

    def test_workshop_refresh_waits_for_second_station(self) -> None:
        leg = mock.Mock()
        leg.detail = {}
        steps = leg.steps
        leg.menu.return_value = {"container": {"slots": [{"slot": 0, "name": "#2 Farmer workshop"}]}}
        leg.components.return_value = {}
        def wait(_predicate: object, label: str, *args: object, **kwargs: object) -> None:
            if label == "ready_for_level_2":
                raise RuntimeError("stop after refresh")
        leg.wait.side_effect = wait
        with self.assertRaisesRegex(RuntimeError, "stop after refresh"):
            storage._building(leg, 2, ["a", "b"], (196, 77, -445))
        calls = leg.mock_calls
        send = next(i for i, call in enumerate(calls) if call == mock.call.steps.fixture(
            "setblock 196 77 -445 minecraft:composter"))
        fence = next(i for i, call in enumerate(calls) if call[0] == "steps.call"
                     and call[1][0] == "minecraft_wait_for_block_state")
        refresh = next(i for i, call in enumerate(calls) if call == mock.call.slot(
            storage.BUILDING_ICON, "refresh_workshop"))
        self.assertLess(send, fence)
        self.assertLess(fence, refresh)

    def test_scenario_requires_window_before_server_resolution(self) -> None:
        ctx = mock.Mock(scenario_id="native-v2-settlement-storage", client="native", windowed=False)
        with mock.patch.object(client_probes, "_server_binary") as binary:
            with self.assertRaisesRegex(ValueError, "requires --windowed"):
                client_probes.run(ctx, "entities")
        binary.assert_not_called()

    def test_run_refuses_without_owned_window_or_guest(self) -> None:
        steps = mock.Mock()
        steps.client = mock.Mock(spec=[])
        with self.assertRaisesRegex(RuntimeError, "owned windowed Xvfb"):
            storage.run(steps, (0, 64, 0))
        steps.client = mock.Mock(spec=["p4_window"])
        steps.client.p4_window = {"display": ":99", "window_id": 1}
        with self.assertRaisesRegex(RuntimeError, "StorageGuest"):
            storage.run(steps, (0, 64, 0))


if __name__ == "__main__":
    unittest.main()
