import math
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools.harness import java_launcher

from tools.harness.backends import screens_voice_qa as qa


class ScreensVoiceHelperTests(unittest.TestCase):
    def test_facing_puts_the_crosshair_on_the_target(self):
        yaw, pitch = qa.facing((1000.5, 160.0, 1000.5), (1000.5, 161.62, 1004.5))
        self.assertAlmostEqual(yaw, 0.0)
        self.assertAlmostEqual(pitch, 0.0)
        yaw, _ = qa.facing((999.5, 160.0, 1002.0), (1000.5, 161.62, 1002.0))
        self.assertAlmostEqual(yaw, -90.0)  # +x is yaw -90 in vanilla
        _, pitch = qa.facing((0.0, 160.0, 0.0), (0.0, 171.62, 10.0))
        self.assertAlmostEqual(pitch, -45.0)  # looking up is negative pitch

    def test_recording_measures_only_the_tone_that_was_heard(self):
        with tempfile.TemporaryDirectory() as directory:
            recording = qa.Recording(qa.write_tone(Path(directory) / "heard.wav", 440, 0.5))
            window = recording.segment(0, recording.size(), (440, 660))
            self.assertTrue(qa.heard(window, 440))
            self.assertTrue(qa.silent(window, 660))
            empty = recording.segment(recording.size(), recording.size(), (440,))
            self.assertEqual(empty["frames"], 0)
            self.assertTrue(qa.silent(empty, 440))

    def test_a_packetized_tone_is_heard_despite_phase_jumps(self):
        # A relayed stream restarts phase at packet boundaries; it is still the same audible tone.
        rate, packet = qa.RATE, qa.RATE // 50
        mono, phase = [], 0.0
        for index in range(3 * 50):
            phase += 0.37 * index
            mono.extend(0.05 * math.sin(phase + 2 * math.pi * 440 * n / rate) for n in range(packet))
        amplitude = qa.tone_amplitudes(mono, rate, (440, 660))
        self.assertTrue(qa.heard({"amplitude": amplitude}, 440), amplitude)
        self.assertTrue(qa.silent({"amplitude": amplitude}, 660), amplitude)

    def test_voice_endpoint_comes_from_svc_pin_and_does_not_leak_to_screens(self):
        with mock.patch.dict(os.environ):
            for key in ("KRASNOKAMEN_JAVA_VERSION", "KRASNOKAMEN_JAVA_PLATFORM"):
                os.environ.pop(key, None)
            for version in ("26.1.2", "26.3"):
                with mock.patch.object(qa.runtime, "_mod_artifact_path") as path, \
                     mock.patch.object(qa.runtime, "_select_mod_entry",
                                       return_value={"game_versions": [version]}):
                    path.return_value.read_text.return_value = (
                        '{"baseline":{"minecraft":"' + version + '"}}')
                    with qa.java_endpoint("voice"):
                        argv = java_launcher.modded_gradle_arguments(":loader-fabric:runClientMcp")
                        self.assertIn(f"-PminecraftVersion={version}", argv)
                        with self.assertRaisesRegex(Exception, "conflicts with task"):
                            java_launcher.modded_gradle_arguments(":loader-neoforge:runClientMcp")
                self.assertNotIn("KRASNOKAMEN_JAVA_VERSION", os.environ)
            with qa.java_endpoint("screens"):
                self.assertEqual(os.environ["KRASNOKAMEN_JAVA_VERSION"], "26.3")
            self.assertNotIn("KRASNOKAMEN_JAVA_PLATFORM", os.environ)

    def test_voice_endpoint_refuses_conflicting_selectors(self):
        for key, value in (("KRASNOKAMEN_JAVA_VERSION", "26.3"),
                           ("KRASNOKAMEN_JAVA_PLATFORM", "neoforge")):
            with mock.patch.dict(os.environ, {key: value}):
                with self.assertRaisesRegex(ValueError, f"voice requires {key}="):
                    with qa.java_endpoint("voice"):
                        self.fail("conflicting selector reached the voice leg")
                self.assertEqual(os.environ[key], value)

    def test_run_scopes_pinned_endpoints_to_each_leg(self):
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict(os.environ):
            root = Path(directory)
            binary = root / "server"
            binary.write_bytes(b"unit fixture")
            os.environ.update(KRASNOKAMEN_SERVER_BINARY=str(binary), KRASNOKAMEN_PW_ROOT="private")
            for key in ("KRASNOKAMEN_JAVA_VERSION", "KRASNOKAMEN_JAVA_PLATFORM", qa.LEG_ENV):
                os.environ.pop(key, None)
            commands = []
            def leg(*args):
                commands.append(java_launcher.modded_gradle_arguments(":loader-fabric:runClientMcp"))
            ctx = qa.ProfileContext(artifact_dir=root, client="native", windowed=True)
            with mock.patch.object(qa, "validate_args", return_value=60), \
                 mock.patch.object(qa.native_client, "resolve_binary"), \
                 mock.patch.object(qa, "screens_leg", side_effect=leg), \
                 mock.patch.object(qa, "voice_leg", side_effect=leg):
                qa.run(ctx)
            self.assertIn("-PminecraftVersion=26.3", commands[0])
            self.assertIn("-PminecraftVersion=26.1.2", commands[1])
            self.assertNotIn("KRASNOKAMEN_JAVA_VERSION", os.environ)


if __name__ == "__main__":
    unittest.main()
