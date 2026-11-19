"""Shared matrix client semantics must come from real MCP observations."""
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock

from .java_matrix_client import JavaMatrixClient
from .matrix_api import MissingCapability


class JavaMatrixClientTest(unittest.TestCase):
    def client(self, mcp):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        return JavaMatrixClient("JMatrix00", mcp, "127.0.0.1:26200", Path(directory.name))

    def test_experience_uses_received_fields(self):
        self.assertEqual(JavaMatrixClient.normalize({"player": {"total_experience": 0,
                         "experience_level": 3, "experience_progress": 0.25}})["xp"],
                         {"total": 0, "level": 3, "progress": 0.25})

    def test_events_use_applied_system_chat_not_gui_history(self):
        messages = ["fence", "same", "same"]
        result = JavaMatrixClient.normalize({"received_system_chat": messages,
                                             "dropped_system_chat": 0,
                                             "recent_chat": ["gui only"]})
        self.assertEqual(result["events"], messages)
        self.assertIsNone(JavaMatrixClient.normalize({"recent_chat": ["gui only"]})["events"])

    def test_incomplete_system_chat_journal_fails_closed(self):
        for messages, dropped in [(["old"], 1), ([], -1), (None, 0), ([], None), (["ok", 1], 0), ([], False)]:
            with self.subTest(messages=messages, dropped=dropped):
                with self.assertRaisesRegex(MissingCapability, "system_chat"):
                    JavaMatrixClient.normalize({"received_system_chat": messages,
                                                "dropped_system_chat": dropped})

    def test_respawn_dispatches_real_mcp_action(self):
        mcp = Mock()
        mcp.call_tool.return_value = {"status": "respawned"}
        client = self.client(mcp)
        client.respawn()
        mcp.call_tool.assert_called_once_with("minecraft_respawn", {"timeout_seconds": 30})

    def test_cumulative_system_chat_requires_lossless_ordered_receipts(self):
        observed = {"received_system_chat": ["first", "first", "last"], "dropped_system_chat": 0}
        self.assertEqual(JavaMatrixClient.normalize(observed)["events"], observed["received_system_chat"])
        for invalid in (None, ["ok", 1], "not a journal"):
            with self.assertRaisesRegex(MissingCapability, "invalid_receipts"):
                JavaMatrixClient.normalize({"received_system_chat": invalid, "dropped_system_chat": 0})
        for dropped in (None, -1, True):
            with self.assertRaisesRegex(MissingCapability, "missing_loss_counter"):
                JavaMatrixClient.normalize({"received_system_chat": [], "dropped_system_chat": dropped})
        with self.assertRaisesRegex(MissingCapability, "lost_receipts:1"):
            JavaMatrixClient.normalize({"received_system_chat": [], "dropped_system_chat": 1})

    def test_connection_health_uses_reported_transport_not_play_state(self):
        self.assertIs(JavaMatrixClient.normalize({"in_play": True, "connected": False})["connected"], False)
        self.assertIs(JavaMatrixClient.normalize({"in_play": False, "connected": True})["connected"], True)
        self.assertIsNone(JavaMatrixClient.normalize({"in_play": True})["connected"])

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

    def fixture_client(self, *, fail_chunk=False):
        home = dict(x=192.5, y=77.0, z=-447.5, yaw=15.0, pitch=20.0, game_mode="creative")
        player = home.copy()
        requests = []
        commands = []
        def command(text):
            commands.append(text)
            parts = text.split()
            if parts[0] == "gamemode":
                player["game_mode"] = parts[1]
            elif parts[0] == "teleport":
                player.update(zip(("x", "y", "z", "yaw", "pitch"), map(float, parts[2:])))
        def call(tool, args):
            if tool == "minecraft_observe":
                return {"player": player.copy(), "state_version": len(commands)}
            if tool == "minecraft_wait_for_loaded_block":
                # Forceload cannot make a chunk outside the view visible. Model a
                # one-chunk view, stricter than the original spawn's render distance.
                self.assertEqual((int(player["x"] // 16), int(player["z"] // 16)),
                                 (args["x"] // 16, args["z"] // 16))
                self.assertEqual(player["game_mode"], "spectator")
                requests.append((args["x"], args["z"]))
                if fail_chunk:
                    raise RuntimeError("chunk publication failed")
                return {"loaded": True}
            raise AssertionError(tool)
        client = self.client(Mock(call_tool=Mock(side_effect=call)))
        client.fixture_command = command
        return client, home, player, requests, commands

    def test_loaded_area_waits_for_each_actual_client_chunk(self):
        client, home, player, requests, commands = self.fixture_client()
        client.ensure_loaded(((-1, 64, 15), (16, 70, 16)))
        self.assertEqual(requests, [(-16, 0), (-16, 16), (0, 0), (0, 16), (16, 0), (16, 16)])
        self.assertEqual(player, home)
        self.assertEqual(commands[0], "gamemode spectator JMatrix00")
        self.assertEqual(commands[-1], "gamemode creative JMatrix00")

    def test_remote_armadillo_fixture_loads_before_arena_teleport(self):
        client, home, player, requests, _ = self.fixture_client()
        client.ensure_loaded(((-16, 199, -16), (16, 215, 16)))
        self.assertEqual(requests, [(x, z) for x in (-16, 0, 16) for z in (-16, 0, 16)])
        self.assertEqual(player, home)

    def test_fixture_observer_restores_actor_when_chunk_publication_fails(self):
        client, home, player, _, _ = self.fixture_client(fail_chunk=True)
        with self.assertRaisesRegex(RuntimeError, "chunk publication failed"):
            client.ensure_loaded(((-16, 199, -16), (16, 215, 16)))
        self.assertEqual(player, home)


if __name__ == "__main__":
    unittest.main()
