import unittest

from tools.harness.backends.integrated_scene import wait_ui


class IntegratedSceneTests(unittest.TestCase):
    def test_title_wait_samples_after_missing_state_event(self):
        class Probe:
            def __init__(self):
                self.states = iter([
                    {"screen": {"class": "old"}, "state_version": 0},
                    {"screen": {"class": "new"}, "state_version": 0},
                ])
                self.checked = None
            def observe(self):
                return next(self.states)
            def remaining(self):
                return 20.0
            def call(self, name, args):
                assert name == "minecraft_wait_for_state_change"
                assert args["timeout_seconds"] == 1.0
                raise RuntimeError("client state event did not arrive")
            def check(self, label, condition):
                self.checked = (label, condition)
        probe = Probe()
        state = wait_ui(probe, lambda s: s["screen"]["class"] == "new", "new_screen")
        self.assertEqual(state["screen"]["class"], "new")
        self.assertEqual(probe.checked, ("new_screen", True))

    def test_title_wait_does_not_hide_other_errors(self):
        class Probe:
            def observe(self):
                return {"state_version": 0}
            def remaining(self):
                return 20.0
            def call(self, name, args):
                raise RuntimeError("connection lost")
        with self.assertRaisesRegex(RuntimeError, "connection lost"):
            wait_ui(Probe(), lambda _: False, "screen")
