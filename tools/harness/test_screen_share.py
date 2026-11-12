import tempfile
import unittest
from pathlib import Path

from tools.harness import regressions, scenarios
from tools.harness.backends import screen_share

MANIFEST = '''id = "computer"
api = "0.9.0"

[[client.bundles]]
id = "computer-content"
loaders = ["fabric", "neoforge", "forge"]
required = false
'''


class ScreenShareTests(unittest.TestCase):
    def test_staged_computer_plugin_offers_its_bundle_to_native(self):
        staged = screen_share.with_native_loader(MANIFEST)
        self.assertIn('loaders = ["native", "fabric", "neoforge", "forge"]\n', staged)
        self.assertEqual(screen_share.with_native_loader(staged), staged)
        with self.assertRaises(RuntimeError):
            screen_share.with_native_loader(MANIFEST.replace("loaders", "platforms"))
        with tempfile.TemporaryDirectory() as root:
            source, plugins = Path(root, "computer"), Path(root, "plugins")
            source.mkdir()
            plugins.mkdir()
            (source / "plugin.toml").write_text(MANIFEST)
            screen_share.stage_computer_plugin(source, plugins)
            self.assertEqual((source / "plugin.toml").read_text(), MANIFEST, "the shared pack is never edited")
            self.assertIn('"native"', (plugins / "computer" / "plugin.toml").read_text())

    def test_a_cancelled_request_grants_nothing_only_when_no_answer_or_frame_arrived(self):
        state = {"screen_sharing": {"permission_answers": {"allowed": 1, "denied": 0},
                                    "cancelled_grants_left": 0, "received_frames": 7, "decoded_frames": 5}}
        before = screen_share.viewer_counters(state)
        self.assertTrue(screen_share.granted_nothing(before, dict(before)))
        for key in ("allowed", "cancelled_grants_left", "received_frames", "decoded_frames"):
            self.assertFalse(screen_share.granted_nothing(before, {**before, key: before[key] + 1}), key)
        self.assertEqual(screen_share.viewer_counters({})["allowed"], 0)

    def test_the_ledger_runs_the_scenario_on_the_native_windowed_client(self):
        entry = scenarios.SCENARIOS[screen_share.SCENARIO]
        self.assertEqual((entry.profile, entry.evidence), ("screen-share", "graphical"))
        command, is_profile = regressions._check_command("screen-share", screen_share.SCENARIO)
        self.assertFalse(is_profile)
        self.assertEqual(command[-5:], ["scenario", screen_share.SCENARIO, "--client", "native", "--windowed"])
        self.assertTrue(regressions.is_graphical_ref(f"harness:screen-share:{screen_share.SCENARIO}"))


if __name__ == "__main__":
    unittest.main()
