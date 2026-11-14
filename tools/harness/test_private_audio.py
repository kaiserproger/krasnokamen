"""SVC may never enumerate or open the owner's microphone graph."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

from . import audio
from .matrix_api import MissingCapability


class PrivateAudioTest(unittest.TestCase):
    def test_host_environment_rejected_before_any_audio_command(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {}, clear=True):
            with patch.object(audio.subprocess, "run") as run:
                with self.assertRaisesRegex(MissingCapability, "private_audio"):
                    audio.private_audio_preflight(Path(directory))
                run.assert_not_called()

    def environment(self, root):
        return {"KRASNOKAMEN_PW_ROOT": str(root), "PIPEWIRE_RUNTIME_DIR": str(root),
                "XDG_RUNTIME_DIR": str(root), "PULSE_SERVER": "unix:" + str(root / "pulse/native"),
                "PULSE_SOURCE": "agents-silent-input", "ALSOFT_DRIVERS": "pulse"}

    def test_hardware_node_rejected_even_inside_private_runtime(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            graph = [{"info": {"props": {"node.name": "alsa_input.real_mic", "media.class": "Audio/Source"}}}]
            with patch.dict(os.environ, self.environment(root)), patch.object(Path, "is_socket", return_value=True), patch.object(audio.subprocess, "run", return_value=Mock(stdout=json.dumps(graph))):
                with self.assertRaisesRegex(MissingCapability, "hardware_audio"):
                    audio.private_audio_preflight(root)

    def test_only_virtual_source_allows_svc_capture(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            graph = [{"type": "PipeWire:Interface:Node", "info": {"props": {
                "node.name": "agents-silent-input", "media.class": "Audio/Source/Virtual"}}},
                {"info":{"props":{"node.name":"agents-null","media.class":"Audio/Sink"}}},
                {"type": "PipeWire:Interface:Link", "info": {"output-node-id": 8, "input-node-id": 9}}]
            with patch.dict(os.environ, self.environment(root)), patch.object(Path, "is_socket", return_value=True), patch.object(audio.subprocess, "run", return_value=Mock(stdout=json.dumps(graph))):
                result = audio.private_audio_preflight(root)
            self.assertEqual(result["capture_source"], "agents-silent-input")
            self.assertEqual(result["graph"], graph)
            self.assertTrue((root / "private-audio-preflight.json").is_file())

    def test_no_virtual_source_does_not_fallback_to_default_capture(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.dict(os.environ, self.environment(root)), patch.object(Path, "is_socket", return_value=True), patch.object(audio.subprocess, "run", return_value=Mock(stdout="[]")):
                with self.assertRaisesRegex(MissingCapability, "silent_input"):
                    audio.private_audio_preflight(root)

    def test_native_alsa_requires_private_config_before_graph_query(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            with patch.dict(os.environ,self.environment(root),clear=True), \
                 patch.object(Path,"is_socket",return_value=True), \
                 patch.object(audio.subprocess,"run") as run:
                with self.assertRaisesRegex(MissingCapability,"private_alsa"):
                    audio.private_audio_preflight(root,require_alsa=True)
                run.assert_not_called()

    def test_native_private_alsa_bytes_are_checked_and_recorded(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            config=root/"alsa.conf"
            config.write_text(audio.private_alsa_config(root))
            environment={**self.environment(root),"ALSA_CONFIG_PATH":str(config)}
            graph=[{"info":{"props":{"node.name":"agents-silent-input",
                                     "media.class":"Audio/Source/Virtual"}}},
                   {"info":{"props":{"node.name":"agents-null","media.class":"Audio/Sink"}}}]
            with patch.dict(os.environ,environment,clear=True), \
                 patch.object(Path,"is_socket",return_value=True), \
                 patch.object(audio.subprocess,"run",return_value=Mock(stdout=json.dumps(graph))):
                result=audio.private_audio_preflight(root/"proof",require_alsa=True)
                self.assertEqual(result["alsa_config"],config.read_text())
                config.write_text(config.read_text()+"\n</usr/share/alsa/alsa.conf>\n")
                with self.assertRaisesRegex(MissingCapability,"private_alsa"):
                    audio.private_audio_preflight(root/"bad",require_alsa=True)

    def test_missing_virtual_sink_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            graph=[{"info":{"props":{"node.name":"agents-silent-input",
                                    "media.class":"Audio/Source/Virtual"}}}]
            with patch.dict(os.environ,self.environment(root)), \
                 patch.object(Path,"is_socket",return_value=True), \
                 patch.object(audio.subprocess,"run",return_value=Mock(stdout=json.dumps(graph))):
                with self.assertRaisesRegex(MissingCapability,"virtual_null_output"):
                    audio.private_audio_preflight(root)


class JavaAudioIsolationTest(unittest.TestCase):
    """Owner rule: an agent Java client never reaches the owner's speakers or microphone."""

    PARENT = {"ALSOFT_DRIVERS": "pulse", "PULSE_SERVER": "unix:/run/user/1000/pulse/native",
              "PULSE_SINK": "agents-null", "PIPEWIRE_NODE": "agents-null",
              "XDG_RUNTIME_DIR": "/run/user/1000"}

    def forced(self, game):
        env = dict(self.PARENT)
        audio.force_java_audio_isolation(env, game)
        return env

    def test_forcing_overrides_a_pulse_parent_and_receipt_is_recorded(self):
        with tempfile.TemporaryDirectory() as directory:
            game = Path(directory) / "game"
            env = self.forced(game)
            self.assertEqual(env["ALSOFT_DRIVERS"], "null")
            self.assertEqual(env["PULSE_SERVER"], "unix:/nonexistent")
            self.assertEqual(env["PIPEWIRE_REMOTE"], "/nonexistent")
            self.assertEqual(env["XDG_RUNTIME_DIR"], str(game / ".audio/runtime"))
            self.assertEqual(Path(env["ALSA_CONFIG_PATH"]).read_text(), audio.JAVA_NULL_ALSA_CONFIG)
            self.assertNotIn("PULSE_SINK", env)
            self.assertNotIn("PIPEWIRE_NODE", env)
            result = audio.require_java_audio_isolation(env, game)
            self.assertEqual(result["openal_backend"], "null")
            self.assertEqual(json.loads((game / audio.JAVA_AUDIO_RECEIPT).read_text()), result)

    def test_any_other_driver_is_refused_without_a_receipt(self):
        for driver in ("pulse", "alsa", "pipewire", "wave", None):
            with self.subTest(driver=driver), tempfile.TemporaryDirectory() as directory:
                game = Path(directory) / "game"
                env = self.forced(game)
                if driver is None:
                    del env["ALSOFT_DRIVERS"]
                else:
                    env["ALSOFT_DRIVERS"] = driver
                with self.assertRaisesRegex(MissingCapability, "java_audio:ALSOFT_DRIVERS"):
                    audio.require_java_audio_isolation(env, game)
                self.assertFalse((game / audio.JAVA_AUDIO_RECEIPT).exists())

    def test_reachable_audio_server_or_foreign_alsa_config_is_refused(self):
        cases = {
            "PULSE_SERVER": "unix:/run/user/1000/pulse/native",
            "PIPEWIRE_REMOTE": "pipewire-0",
            "XDG_RUNTIME_DIR": "/run/user/1000",
            "ALSA_CONFIG_PATH": "/etc/alsa.conf",
            "PULSE_SINK": "agents-null",
        }
        for name, value in cases.items():
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                game = Path(directory) / "game"
                env = self.forced(game)
                env[name] = value
                with self.assertRaisesRegex(MissingCapability, "java_audio"):
                    audio.require_java_audio_isolation(env, game)
        with tempfile.TemporaryDirectory() as directory:
            game = Path(directory) / "game"
            env = self.forced(game)
            Path(env["ALSA_CONFIG_PATH"]).write_text("pcm.!default { type pulse }\n")
            with self.assertRaisesRegex(MissingCapability, "private_null_alsa_config"):
                audio.require_java_audio_isolation(env, game)


if __name__ == "__main__":
    unittest.main()
