"""Keep the unloaded-eye failure distinct from an initial join failure."""
import tempfile
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

from .backends import unloaded_eye
from .backends.probe_steps import ProbeSteps


class UnloadedEyeTests(unittest.TestCase):
    def test_post_teleport_disconnect_names_own_assertion_and_releases_stream(self):
        held, release = threading.Event(), threading.Event()
        held.set()
        states = iter([
            {"in_play": True, "player": {"x": 0.5}},
            {"in_play": False, "player": None},
        ])

        def call_tool(name, arguments):
            if name == "minecraft_observe":
                return next(states)
            self.assertEqual(name, "minecraft_send_chat")
            self.assertEqual(arguments["message"], "tp @s 1000.5 176.0 1000.5")
            return {}

        client = SimpleNamespace(
            scene_proxy=SimpleNamespace(held=held, release=release),
            call_tool=Mock(side_effect=call_tool),
        )
        with tempfile.TemporaryDirectory() as directory:
            artifact = Path(directory)
            probe = ProbeSteps(client, artifact, 10, "native-unloaded-eye")
            probe.observe()
            with patch("tools.harness.backends.scene_shot.execute", return_value=[]):
                with self.assertRaisesRegex(RuntimeError, "assertion failed: unloaded_eye_session_survives$"):
                    unloaded_eye.execute(probe, artifact, [{"tp": unloaded_eye.DESTINATION}], "native")
            self.assertTrue(probe.report["assertions"]["in_play"])
            self.assertFalse(probe.report["assertions"]["unloaded_eye_session_survives"])
            self.assertTrue(release.is_set())


if __name__ == "__main__":
    unittest.main()
