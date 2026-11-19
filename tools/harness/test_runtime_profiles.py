#!/usr/bin/env python3
"""Java-profile regressions and conservative public/safety contracts.

These tests fake process spawn, Gradle discovery and readiness; they are not
evidence that a real client or audio lane works. Observed regressions cover
exact endpoint selectors, game-only heap caps, private build/project caches,
and verified named private audio sinks. Null-audio admission, profile validation
and prerequisite-report contracts remain conservatively retained.

Durable usage lives in ``docs/AGENT_TOOLING.md`` ("Java client profiles").
"""

from __future__ import annotations

from contextlib import contextmanager
import itertools
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from . import audio, runtime
from .matrix_api import MissingCapability


def _valid_kwargs(game_dir: Path, log_path: Path, **overrides):
    kwargs = {
        "game_dir": game_dir,
        "log_path": log_path,
        "token": "unit-test-only",
        "mcp_port": 39111,
        "username": "SvcProfile",
        "display": ":99",
    }
    kwargs.update(overrides)
    return kwargs

@contextmanager
def _gradlew_present():
    """Fake Gradle/sibling discovery only; real game-dir/mod files stay real."""
    real_is_file = Path.is_file

    def fake_is_file(self: Path) -> bool:
        if self.name == "gradlew":
            return True
        return real_is_file(self)

    with mock.patch.object(Path, "is_file", fake_is_file), \
            mock.patch.object(runtime, "sibling_root", return_value=Path("/test/loader")):
        yield



class ProfileTaskTests(unittest.TestCase):
    def setUp(self):
        selectors = mock.patch.dict(os.environ)
        selectors.start()
        self.addCleanup(selectors.stop)
        os.environ.pop("KRASNOKAMEN_JAVA_VERSION", None)
        os.environ.pop("KRASNOKAMEN_JAVA_PLATFORM", None)
        os.environ.pop("JDK_JAVA_OPTIONS", None)

    def test_exact_selectors_reach_spawned_gradle_command(self):
        endpoints = (("26.1.2", "fabric"), ("26.1.2", "neoforge"), ("26.2", "neoforge"),
                     ("26.3", "fabric"), ("26.3", "neoforge"))
        for version, platform in endpoints:
            with self.subTest(version=version, platform=platform), \
                    tempfile.TemporaryDirectory() as directory, _gradlew_present(), \
                    mock.patch.dict(os.environ, {"KRASNOKAMEN_JAVA_VERSION": version,
                                                 "KRASNOKAMEN_JAVA_PLATFORM": platform,
                                                 "KRASNOKAMEN_CLIENT_JAR": "/test/client.jar"}), \
                    mock.patch.object(runtime.os, "access", return_value=True), \
                    mock.patch.object(runtime, "port_open", return_value=False), \
                    mock.patch.object(runtime, "wait_port"), \
                    mock.patch.object(runtime.subprocess, "Popen") as launch:
                root = Path(directory)
                _, handle = runtime.start_client(**_valid_kwargs(
                    root / "game", root / "client.log", client_profile="loader", platform=platform))
                handle.close()
                argv = launch.call_args.args[0]
                self.assertIn(f"-PminecraftVersion={version}", argv)
                self.assertIn("-Pkrasnokamen.clientJar=/test/client.jar", argv)
                self.assertEqual(argv[-1], runtime._resolve_profile_task("loader", platform)["task"])
                self.assertEqual(runtime.profile_prerequisites("loader", platform)["composition"],
                                 [f"krasnokamen-loader-{platform}"])
                if version == "26.1.2":
                    self.assertIn("-PneoVersion=26.1.2.76", argv)
                    self.assertIn("-PfabricApiVersion=0.155.2+26.1.2", argv)
                elif version == "26.2":
                    self.assertIn("-PneoVersion=26.2.0.93", argv)
                else:
                    self.assertIn("-PneoVersion=26.3.0.83", argv)
                    self.assertIn("-PfabricApiVersion=0.163.0+26.3", argv)

    def test_neoforge_client_heap_does_not_limit_gradle(self):
        with tempfile.TemporaryDirectory() as directory, _gradlew_present(), \
                mock.patch.object(runtime.os, "access", return_value=True), \
                mock.patch.object(runtime, "port_open", return_value=False), \
                mock.patch.object(runtime, "wait_port"), \
                mock.patch.object(runtime.subprocess, "Popen") as launch:
            root = Path(directory)
            _, handle = runtime.start_client(**_valid_kwargs(
                root / "game", root / "client.log", client_profile="loader", platform="neoforge"))
            handle.close()
            self.assertNotIn("JDK_JAVA_OPTIONS", launch.call_args.kwargs["env"])
            argv = launch.call_args.args[0]
            script = Path(argv[argv.index("--init-script") + 1]).read_text()
            self.assertIn('path == ":loader-neoforge:runClientMcp"', script)
            self.assertIn("maxHeapSize = '512m'", script)
            self.assertEqual(argv[-1], ":loader-neoforge:runClientMcp")

    def test_legacy_selectors_refuse_composition_change_before_spawn(self):
        with tempfile.TemporaryDirectory() as directory, _gradlew_present(), \
                mock.patch.dict(os.environ, {"KRASNOKAMEN_JAVA_VERSION": "26.1.2",
                                             "KRASNOKAMEN_JAVA_PLATFORM": "fabric"}), \
                mock.patch.object(runtime.os, "access", return_value=True), \
                mock.patch.object(runtime, "port_open", return_value=False), \
                mock.patch.object(runtime, "wait_port"), \
                mock.patch.object(runtime.subprocess, "Popen") as launch:
            root = Path(directory)
            with self.assertRaisesRegex(MissingCapability, "conflicts with task :fabric-agent"):
                runtime.start_client(**_valid_kwargs(root / "game", root / "client.log"))
            launch.assert_not_called()
            self.assertEqual(runtime._resolve_profile_task(None, None)["composition"],
                             ["krasnokamen-agent-neoforge-mod"])

    def _force_driver(self, driver: str):
        """Run the real forcing, then leave ``driver`` in the env (a broken forcing)."""
        real = audio.force_java_audio_isolation

        def broken(env, game_dir):
            real(env, game_dir)
            env["ALSOFT_DRIVERS"] = driver

        return mock.patch("tools.harness.audio.force_java_audio_isolation", side_effect=broken)

    def test_every_lane_refuses_launch_when_child_env_would_carry_another_driver(self) -> None:
        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.object(runtime, "_stage_profile_mods"), \
                mock.patch.object(runtime, "port_open", return_value=False), \
                mock.patch.object(runtime.subprocess, "Popen") as launch, \
                self._force_driver("pulse"):
            root = Path(directory)
            for profile, platform in ((None, None), ("vanilla", None), ("loader", "fabric"),
                                      ("loader", "neoforge"), ("loader", "forge"),
                                      ("svc", None), ("dh", None)):
                game = root / str(profile) / str(platform)
                with self.subTest(profile=profile, platform=platform), \
                        self.assertRaisesRegex(MissingCapability, "java_audio:ALSOFT_DRIVERS"):
                    runtime.start_client(**_valid_kwargs(
                        game, root / "client.log", client_profile=profile, platform=platform))
                self.assertFalse((game / audio.JAVA_AUDIO_RECEIPT).exists())
            launch.assert_not_called()

    def test_child_env_forces_null_audio_over_a_pulse_wrapper(self) -> None:
        wrapper_env = {
            "ALSOFT_DRIVERS": "pulse", "PULSE_SERVER": "unix:/run/user/1000/pulse/native",
            "PULSE_SINK": "agents-null", "PULSE_SOURCE": "agents-silent-input",
            "PIPEWIRE_NODE": "agents-null", "PIPEWIRE_RUNTIME_DIR": "/run/user/1000",
            "XDG_RUNTIME_DIR": "/run/user/1000", "ALSA_CONFIG_PATH": "/etc/alsa.conf",
        }
        for profile in (None, "vanilla"):
            with self.subTest(profile=profile), tempfile.TemporaryDirectory() as directory, \
                    _gradlew_present(), \
                    mock.patch.dict(os.environ, wrapper_env), \
                    mock.patch.object(runtime.os, "access", return_value=True), \
                    mock.patch.object(runtime, "port_open", return_value=False), \
                    mock.patch.object(runtime, "wait_port"), \
                    mock.patch.object(runtime, "wait_for_path_condition"), \
                    mock.patch("tools.harness.java_launcher.vanilla_command",
                               return_value=["java", "Main"]), \
                    mock.patch.object(runtime, "_agent_jar_path", return_value=Path(directory) / "a.jar"), \
                    mock.patch.object(runtime.subprocess, "Popen") as launch:
                root = Path(directory)
                launch.return_value = mock.Mock()
                _, handle = runtime.start_client(**_valid_kwargs(
                    root / "game", root / "client.log", client_profile=profile))
                handle.close()
                env = launch.call_args.kwargs["env"]
                private = root / "game" / ".audio"
                self.assertEqual(env["ALSOFT_DRIVERS"], "null")
                self.assertEqual(env["ALSA_CONFIG_PATH"], str(private / "alsa-null.conf"))
                self.assertEqual((private / "alsa-null.conf").read_text(),
                                 "pcm.!default { type null }\nctl.!default { type null }\n")
                self.assertEqual(env["PIPEWIRE_REMOTE"], "/nonexistent")
                self.assertEqual(env["PULSE_SERVER"], "unix:/nonexistent")
                self.assertEqual(env["XDG_RUNTIME_DIR"], str(private / "runtime"))
                for name in ("PULSE_SINK", "PULSE_SOURCE", "PIPEWIRE_NODE", "PIPEWIRE_RUNTIME_DIR"):
                    self.assertNotIn(name, env)
                receipt = json.loads((root / "game" / audio.JAVA_AUDIO_RECEIPT).read_text())
                self.assertEqual(receipt["openal_backend"], "null")
                self.assertEqual(receipt["isolation"]["ALSOFT_DRIVERS"], "null")
                self.assertEqual(os.environ["ALSOFT_DRIVERS"], "pulse")

    def test_launch_needs_no_private_pulse_graph(self) -> None:
        with tempfile.TemporaryDirectory() as directory, _gradlew_present(), \
                mock.patch.dict(os.environ, {}), \
                mock.patch("tools.harness.audio.private_audio_preflight") as graph, \
                mock.patch.object(runtime.os, "access", return_value=True), \
                mock.patch.object(runtime, "port_open", return_value=False), \
                mock.patch.object(runtime, "wait_port"), \
                mock.patch.object(runtime.subprocess, "Popen") as launch:
            os.environ.pop("KRASNOKAMEN_PW_ROOT", None)
            root = Path(directory)
            launch.return_value = mock.Mock()
            _, handle = runtime.start_client(**_valid_kwargs(root / "game", root / "client.log"))
            handle.close()
            graph.assert_not_called()
            self.assertEqual(launch.call_args.kwargs["env"]["ALSOFT_DRIVERS"], "null")

    def test_legacy_default_keeps_legacy_task(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            seen: dict = {}
            with _gradlew_present(), \
                    mock.patch.object(runtime.os, "access", return_value=True), \
                    mock.patch.object(runtime, "port_open", return_value=False), \
                    mock.patch.object(runtime, "wait_port") as ready, \
                    mock.patch.object(runtime.subprocess, "Popen") as launch:
                launch.return_value = mock.Mock()
                process, handle = runtime.start_client(
                    **_valid_kwargs(root / "game", root / "client.log"))
                try:
                    self.assertIs(process, launch.return_value)
                    self.assertEqual(ready.call_args.args[1], 180.0)
                    argv = launch.call_args.args[0]
                    self.assertEqual(argv[-1], ":fabric-agent:runClientMcp")
                    self.assertIn("--no-daemon", argv)
                    self.assertIn("--no-configuration-cache", argv)
                    env = launch.call_args.kwargs["env"]
                    self.assertEqual(env["DISPLAY"], ":99")
                    self.assertNotIn("WAYLAND_DISPLAY", env)
                    self.assertEqual(env["ALSOFT_DRIVERS"], "null")
                    self.assertNotIn("PULSE_SINK", env)
                    self.assertNotIn("JDK_JAVA_OPTIONS", env)
                    self.assertIn("--init-script", argv)
                    script = Path(argv[argv.index("--init-script") + 1]).read_text()
                    self.assertIn('path == ":fabric-agent:runClientMcp"', script)
                    self.assertIn("maxHeapSize = '512m'", script)
                    seen["env"] = env
                finally:
                    handle.close()
            self.assertEqual(seen["env"]["KRASNOKAMEN_CLIENT_MCP_USERNAME"], "SvcProfile")

    def test_rejects_unknown_profile_and_bad_combos(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaisesRegex(ValueError, "client_profile"):
                runtime.start_client(
                    **_valid_kwargs(root / "game", root / "client.log",
                                    client_profile="quilt"))
            for bad_profile in ("vanilla",):
                with self.assertRaisesRegex(ValueError, "no platform"):
                    runtime.start_client(
                        **_valid_kwargs(root / "game", root / "client.log",
                                        client_profile=bad_profile, platform="fabric"))
            with self.assertRaisesRegex(ValueError, "Fabric-only"):
                runtime.start_client(
                    **_valid_kwargs(root / "game", root / "client.log",
                                    client_profile="svc", platform="neoforge"))

    def test_explicit_heap_is_respected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with _gradlew_present(), \
                    mock.patch.object(runtime.os, "access", return_value=True), \
                    mock.patch.object(runtime, "port_open", return_value=False), \
                    mock.patch.object(runtime, "wait_port"), \
                    mock.patch.object(runtime.subprocess, "Popen") as launch, \
                    mock.patch.dict(os.environ, {"JDK_JAVA_OPTIONS": "-Xmx1G"}):
                launch.return_value = mock.Mock()
                _, handle = runtime.start_client(
                    **_valid_kwargs(root / "game", root / "client.log"))
                try:
                    self.assertEqual(launch.call_args.kwargs["env"]["JDK_JAVA_OPTIONS"], "-Xmx1G")
                    argv = launch.call_args.args[0]
                    script = Path(argv[argv.index("--init-script") + 1]).read_text()
                    self.assertNotIn("maxHeapSize", script)
                    self.assertIn("project.layout.buildDirectory.set", script)
                finally:
                    handle.close()

    def test_audible_client_uses_only_verified_named_private_sink(self):
        with tempfile.TemporaryDirectory() as directory, _gradlew_present(), \
                mock.patch.object(runtime.os, "access", return_value=True), \
                mock.patch.object(runtime, "port_open", return_value=False), \
                mock.patch.object(runtime, "wait_port"), \
                mock.patch.object(runtime.subprocess, "Popen") as launch, \
                mock.patch.dict(os.environ, {"ALSOFT_DRIVERS": "pulse",
                                            "PULSE_SERVER": "unix:/private/pulse/native"}), \
                mock.patch.object(audio, "private_audio_preflight") as preflight:
            root = Path(directory)
            preflight.return_value = {"graph": [{"info": {"props": {
                "node.name": "viewer", "media.class": "Audio/Sink", "node.description": "viewer"}}}],
                "openal_backend": "pulse"}
            _, handle = runtime.start_client(**_valid_kwargs(
                root / "game", root / "client.log", client_profile="loader",
                platform="fabric", sound_sink="viewer"))
            handle.close()
            env = launch.call_args.kwargs["env"]
            self.assertEqual(env["ALSOFT_DRIVERS"], "pulse")
            self.assertEqual(env["PULSE_SERVER"], "unix:/private/pulse/native")
            self.assertEqual(env["PULSE_SINK"], "viewer")
            preflight.assert_called_once_with(root / "game" / ".audio", require_alsa=True)
            options = (root / "game" / "options.txt").read_text()
            self.assertIn("soundDevice:viewer\n", options)
            self.assertIn("soundCategory_master:1.0\n", options)

    def test_audible_client_without_private_graph_is_refused_before_spawn(self):
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict(os.environ), \
                mock.patch.object(runtime, "port_open", return_value=False), \
                mock.patch.object(runtime.subprocess, "Popen") as launch:
            os.environ.pop("KRASNOKAMEN_PW_ROOT", None)
            root = Path(directory)
            with self.assertRaisesRegex(MissingCapability, "screen_audio_graph_wrapper_required"):
                runtime.start_client(**_valid_kwargs(
                    root / "game", root / "client.log", client_profile="loader",
                    platform="fabric", sound_sink="host-speakers"))
            launch.assert_not_called()

    def test_every_gradle_launch_isolates_endpoint_builds_even_with_explicit_heap(self):
        for platform in ("fabric", "neoforge"):
            with self.subTest(platform=platform), tempfile.TemporaryDirectory() as directory, \
                    _gradlew_present(), mock.patch.dict(os.environ, {"JDK_JAVA_OPTIONS": "-Xmx1G"}), \
                    mock.patch.object(runtime.os, "access", return_value=True), \
                    mock.patch.object(runtime, "port_open", return_value=False), \
                    mock.patch.object(runtime, "wait_port"), \
                    mock.patch.object(runtime.subprocess, "Popen") as launch:
                root = Path(directory)
                _, handle = runtime.start_client(**_valid_kwargs(
                    root / "game", root / "client.log", client_profile="loader", platform=platform))
                handle.close()
                argv = launch.call_args.args[0]
                self.assertIn("--init-script", argv)
                self.assertIn("--project-cache-dir", argv)
                cache = Path(argv[argv.index("--project-cache-dir") + 1])
                self.assertTrue(cache.is_absolute())
                self.assertEqual(cache.parent.name, f"loader-{platform}")
                self.assertEqual(cache.name, "project-cache")
                script = Path(argv[argv.index("--init-script") + 1]).read_text()
                self.assertIn("gradleProperty('minecraftVersion').get()", script)
                self.assertIn("gradle.startParameter.projectCacheDir.parentFile", script)
                self.assertIn("project.layout.buildDirectory.set(new File(endpoint, project.name))", script)
                self.assertNotIn("maxHeapSize", script)


def _mod_entry(profile: str, platform: str) -> dict:
    return runtime._select_mod_entry(profile, platform)


class PrerequisiteReportTests(unittest.TestCase):
    def test_vanilla_and_loader_have_no_jar_section(self) -> None:
        info = runtime.profile_prerequisites("vanilla", None)
        self.assertTrue(info["task"].startswith("direct-java:"))
        self.assertIn("java-agent", info["composition"])
        self.assertNotIn("jar", info)
        loader = runtime.profile_prerequisites("loader", "neoforge")
        self.assertEqual(loader["task"], ":loader-neoforge:runClientMcp")
        self.assertNotIn("jar", loader)

    def test_prerequisites_reject_unknown_profile(self) -> None:
        with self.assertRaisesRegex(ValueError, "client_profile"):
            runtime.profile_prerequisites("quilt", None)

    def test_mod_presence_is_not_voice_support(self) -> None:
        """The receipt records a staged jar, never a working voice/DH claim."""
        entry = _mod_entry("svc", "fabric")
        self.assertNotIn("voice_connected", entry)
        self.assertNotIn("negotiated", entry)
        info = runtime.profile_prerequisites("svc", None)
        self.assertNotIn("voice_connected", json.dumps(info))


if __name__ == "__main__":
    raise SystemExit(unittest.main())
