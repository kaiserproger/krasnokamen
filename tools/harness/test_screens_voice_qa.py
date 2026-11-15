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

    def test_voice_fixture_binds_the_group_key_it_presses(self):
        with tempfile.TemporaryDirectory() as directory, \
             mock.patch.object(qa, "Run") as run_type, \
             mock.patch.object(qa, "Microphone"), \
             mock.patch.object(qa, "SoundCapture"), \
             mock.patch.object(qa, "write_tone"), \
             mock.patch.object(qa.runtime, "reserve_port", return_value=24454):
            run = run_type.return_value
            run.__enter__.return_value = run
            run.root = Path(directory)
            run.server.return_value = ("127.0.0.1:25565", run.root / "server.log")
            run.java.side_effect = RuntimeError("captured-client-config")
            with self.assertRaisesRegex(RuntimeError, "captured-client-config"):
                qa.voice_leg(mock.Mock(), Path("/unused/server"), 60, {}, {})
            files = run.java.call_args.args[4]
            self.assertIn("key_key.voice_chat_group:key.keyboard.g",
                          files.get("options.txt", "").splitlines())

    def test_voice_reset_allows_fresh_capability_registration_only_after_reconnect(self):
        cleared = {"registered": False, "server_enabled": False, "listening": False}
        advertised = {**cleared, "registered": True}
        cases = [
            (cleared, advertised, [], [], True),
            (advertised, advertised, [], [], False),
            (cleared, {**advertised, "server_enabled": True}, [], [], False),
            (cleared, {**advertised, "listening": True}, [], [], False),
            (cleared, advertised, [{"id": "hud/voice_mic"}], [], False),
            (cleared, advertised, [], [{"id": "hud/voice_mic"}], False),
        ]
        for before, after, before_hud, after_hud, expected in cases:
            with self.subTest(before=before, after=after, expected=expected), \
                 mock.patch.object(qa, "wait_play"), mock.patch.object(qa, "register"), \
                 mock.patch.object(qa, "native_shot", return_value="quiet.png"):
                native = mock.Mock()
                native.observe.side_effect = lambda **kwargs: (
                    {"voice": before, "ui_nodes": before_hud, "in_play": False}
                    if kwargs.get("outside_play") else
                    {"voice": after, "ui_nodes": after_hud, "in_play": True})
                evidence = {}
                self.assertEqual(qa.voice_reconnect(native, "quiet:25565", evidence), expected)
                self.assertEqual(evidence["native_on_non_voice_server"]["voice"], after)

    def test_screen_manifest_records_missing_monitor_after_loader_acknowledgement(self):
        for blocks in ([], ["computer:monitor"]):
            with self.subTest(blocks=blocks), tempfile.TemporaryDirectory() as directory, \
                 mock.patch.object(qa, "Run") as run_type, \
                 mock.patch.object(qa, "write_tone"), mock.patch.object(qa, "SoundCapture"), \
                 mock.patch.object(qa, "register"), \
                 mock.patch.object(qa, "arena", side_effect=RuntimeError("past manifest")):
                state = {"loader": {"acknowledged": True, "transfer_pending": False,
                                    "registered_blocks": blocks}}
                def ready(predicate, label):
                    if not predicate(state):
                        raise RuntimeError("awaiting impossible post-ack content")
                    return state
                def check(label, condition):
                    if not condition:
                        raise RuntimeError(label)
                run = run_type.return_value
                run.__enter__.return_value = run
                run.__exit__.return_value = False
                run.root = Path(directory)
                run.evidence = {}
                run.server.return_value = ("fixture:25565", None)
                probe = run.native.return_value
                probe.wait.side_effect = ready
                probe.check.side_effect = check
                probe.report = {"id": "native"}
                verdicts = {}
                with self.assertRaises(RuntimeError):
                    qa.screens_leg(mock.Mock(), Path("/unused"), 60, verdicts, {})
                self.assertIs(verdicts.get("native_loads_computer_monitor"), bool(blocks))


if __name__ == "__main__":
    unittest.main()
