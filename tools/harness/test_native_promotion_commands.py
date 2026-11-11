"""Fail-closed checks for the command promotion oracle, not graphical receipts."""
import unittest
from unittest import mock

from .backends import native_promotion_commands as commands


class CommandPromotionTests(unittest.TestCase):
    def test_requires_window(self):
        steps = mock.Mock()
        steps.client.p4_window = None
        with self.assertRaisesRegex(RuntimeError, "windowed Xvfb"):
            commands.run(steps, (0, 0, 0))

    def test_score_requires_fresh_unique_marker(self):
        class Steps:
            def fixture(self, command):
                self.command = command

            def check(self, label, condition):
                if not condition:
                    raise RuntimeError(label)

        steps = Steps()
        with mock.patch.object(commands, "_echo", return_value={"recent_chat": ["old reply"]}):
            with self.assertRaisesRegex(RuntimeError, "expected_score"):
                commands._require_score(steps, "Alice", "deaths", 2, "expected_score")
        with mock.patch.object(commands, "_echo", side_effect=lambda *_: {
                "recent_chat": [steps.command.split()[-1]]}):
            commands._require_score(steps, "Alice", "deaths", 2, "expected_score")
        self.assertIn("if score Alice deaths matches 2", steps.command)
