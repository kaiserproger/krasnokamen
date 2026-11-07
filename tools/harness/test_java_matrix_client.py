"""Shared matrix client semantics must come from real MCP observations."""
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

from .java_matrix_client import JavaMatrixClient
from .matrix_api import MissingCapability


class JavaMatrixClientTest(unittest.TestCase):
    def client(self, mcp):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        return JavaMatrixClient("JMatrix00", mcp, "127.0.0.1:26200", Path(directory.name))

    def test_snapshot_preserves_missing_observations(self):
        mcp = Mock()
        mcp.call_tool.return_value = {"in_play": False, "state_version": 1}
        result = self.client(mcp).snapshot()
        for field in ("position", "inventory", "cursor", "menu", "health", "food", "connected"):
            self.assertIsNone(result[field])

    def test_snapshot_normalizes_actual_menu_and_player(self):
        mcp = Mock()
        mcp.call_tool.return_value = {
            "in_play": True, "state_version": 3, "recent_chat": ["message"],
            "player": {"x": 1, "y": 2, "z": 3, "health": 18, "food": 19},
            "inventory": [{"slot": 0, "item_id": "minecraft:stone", "count": 2}],
            "container": {"container_id": 7, "slots": [], "carried": {"item_id": "minecraft:air", "count": 0}},
        }
        result = self.client(mcp).snapshot()
        self.assertEqual(result["position"], [1, 2, 3])
        self.assertEqual(result["health"], 18)
        # Bounded newest-first GUI messages cannot satisfy cumulative events.
        self.assertIsNone(result["events"])
        self.assertEqual(result["raw"]["recent_chat"], ["message"])
        self.assertEqual(result["menu"]["id"], 7)
        self.assertIsNone(result["menu"]["state_id"])

    def test_connection_health_uses_reported_transport_not_play_state(self):
        self.assertIs(JavaMatrixClient.normalize({"in_play": True, "connected": False})["connected"], False)
        self.assertIs(JavaMatrixClient.normalize({"in_play": False, "connected": True})["connected"], True)
        self.assertIsNone(JavaMatrixClient.normalize({"in_play": True})["connected"])

    def test_wait_uses_zero_argument_predicate_and_version_event(self):
        mcp = Mock()
        mcp.call_tool.return_value = {"state_version": 42}
        predicate = Mock(side_effect=[False, "result"])
        self.assertEqual(self.client(mcp).wait_until(predicate, description="ready"), "result")
        self.assertEqual(predicate.call_args_list[0].args, ())
        wait = [call for call in mcp.call_tool.call_args_list if call.args[0] == "minecraft_wait_for_state_change"]
        self.assertEqual(len(wait), 1)
        self.assertEqual(wait[0].args[1]["observed_version"], 42)

    def test_unsupported_click_kind_fails_before_dispatch(self):
        mcp = Mock()
        with self.assertRaisesRegex(MissingCapability, "quick_craft"):
            self.client(mcp).click_slot(1, kind="quick_craft")
        mcp.call_tool.assert_not_called()

    def test_attack_fences_current_uuid_and_type(self):
        mcp = Mock()
        mcp.call_tool.side_effect = [{"entities": [{"entity_id": 4, "entity_uuid": "uuid", "entity_type": "minecraft:pig"}]}, {"confirmed": True}]
        self.client(mcp).attack(4)
        arguments = mcp.call_tool.call_args.args[1]
        self.assertEqual(arguments["entity_uuid"], "uuid")
        self.assertEqual(arguments["entity_type"], "minecraft:pig")

    def test_teleport_fixture_never_silently_elevates_player(self):
        mcp = Mock()
        with self.assertRaisesRegex(MissingCapability, "fixture"):
            self.client(mcp).teleport_to(1, 2, 3)
        mcp.call_tool.assert_not_called()

    def test_look_uses_received_eye_height_not_standing_constant(self):
        mcp = Mock()
        mcp.call_tool.side_effect = [
            {"player": {"x": 0, "y": 64, "z": 0, "eye_y": 64.4}},
            {},
        ]
        self.client(mcp).look_at(1, 64.4, 0)
        self.assertEqual(mcp.call_tool.call_args.args,
                         ("minecraft_look", {"yaw_deg": -90, "pitch_deg": 0}))

    def test_received_menu_state_id_is_preserved(self):
        result = JavaMatrixClient.normalize({
            "container": {"container_id": 3, "container_state_id": 17, "slots": []}
        })
        self.assertEqual(result["menu"]["state_id"], 17)

    def test_inventory_uses_shared_window_zero_slot_ids(self):
        stacks = [{"slot": slot, "item_id": "minecraft:stone", "count": 1}
                  for slot in (0, 8, 9, 35, 36, 39, 40)]
        normalized = JavaMatrixClient.inventory_slots(stacks)
        self.assertEqual([stack["slot"] for stack in normalized], [36, 44, 9, 35, 8, 5, 45])
        self.assertEqual(stacks[0]["slot"], 0)

    def test_move_dispatches_real_physics_inputs_not_teleport(self):
        mcp = Mock()
        mcp.call_tool.return_value = {}
        self.client(mcp).move(forward=1, strafe=-1, ticks=12, sprint=True)
        mcp.call_tool.assert_called_once_with("minecraft_press_inputs",
                                              {"keys": ["forward", "right", "sprint"], "ticks": 12})

    def test_custom_cursor_fails_before_dispatch(self):
        mcp = Mock()
        with self.assertRaisesRegex(MissingCapability, "custom_cursor"):
            self.client(mcp).use_item_on((1, 2, 3), cursor=(.2, .5, .5))
        mcp.call_tool.assert_not_called()

    def test_off_hand_block_use_is_preserved(self):
        mcp = Mock()
        mcp.call_tool.return_value = {}
        self.client(mcp).use_item_on((1, 2, 3), face="east", hand="off")
        mcp.call_tool.assert_called_once_with("minecraft_use_item_on",
                                              {"x": 1, "y": 2, "z": 3, "face": "east", "hand": "off_hand"})

    def test_unloaded_chunk_is_not_fabricated_as_air(self):
        mcp = Mock()
        mcp.call_tool.side_effect = RuntimeError("client chunk is not loaded at 1,2,3")
        with self.assertRaisesRegex(MissingCapability, "chunk_not_loaded"):
            self.client(mcp).read_block_state((1, 2, 3))

    def test_abort_dig_is_not_silently_completed(self):
        mcp = Mock()
        with self.assertRaisesRegex(MissingCapability, "break_block:abort"):
            self.client(mcp).break_block((1, 2, 3), phase="abort")
        mcp.call_tool.assert_not_called()

    def test_long_tick_wait_is_bounded_by_client_schema_without_sleep(self):
        mcp = Mock()
        mcp.call_tool.return_value = {}
        self.client(mcp).wait_ticks(600)
        self.assertEqual([call.args[1]["ticks"] for call in mcp.call_tool.call_args_list], [255, 255, 90])

    def test_loaded_area_waits_for_each_actual_client_chunk(self):
        mcp = Mock()
        mcp.call_tool.return_value = {"loaded": True}
        self.client(mcp).ensure_loaded(((-1, 64, 15), (16, 70, 16)))
        requests = [call.args[1] for call in mcp.call_tool.call_args_list]
        self.assertEqual([(r["x"], r["z"]) for r in requests],
                         [(-16, 0), (-16, 16), (0, 0), (0, 16), (16, 0), (16, 16)])
        self.assertTrue(all(call.args[0] == "minecraft_wait_for_loaded_block"
                            for call in mcp.call_tool.call_args_list))

    def test_reconnect_can_leave_auth_gate_for_auth_scenario(self):
        mcp = Mock()
        mcp._matrix_platform = None
        mcp._matrix_auth_password = "UnitOnlyPassword"
        mcp.call_tool.side_effect = [{}, {"in_play": True}]
        client = self.client(mcp)
        client.set_auto_auth(False)
        with patch("tools.harness.java_matrix_client.runtime.wait_client_ready_for_connect"), \
                patch("tools.harness.java_clients.authenticate_client") as authenticate:
            client.reconnect()
        authenticate.assert_not_called()
        with self.assertRaises(ValueError):
            client.set_auto_auth("false")


if __name__ == "__main__":
    unittest.main()
