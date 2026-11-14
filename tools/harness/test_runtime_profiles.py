#!/usr/bin/env python3
"""Mock-focused regression tests for client_profile launch in ``runtime``.

These tests never launch a real client: Gradle, process spawn, port wait and
mod-jar bytes are faked. They pin the contract the java-matrix runner relies
on:

- ``None`` keeps the legacy no-Loader ``:fabric-agent:runClientMcp`` task;
- explicit ``vanilla`` runs Mojang Main with the java-agent, without a modloader;
- ``loader`` requires an explicit platform and uses the matching
  ``:loader-<platform>:runClientMcp`` task;
- ``svc``/``dh`` reuse the real ``:loader-fabric:runClientMcp`` Fabric launch
  with the pinned publisher jar verified (size/sha512/sha1) and staged into
  the run's isolated ``game_dir/mods`` — fail-closed before Gradle starts on
  missing jars or hash/size mismatch;
- mod presence is never treated as working voice/DH support;
- child env always drops ``WAYLAND_DISPLAY`` and pins null audio, defaults
  the N8-safe ``JDK_JAVA_OPTIONS=-Xmx512M`` heap without overriding an
  explicit operator value;
- missing published artifacts for the MC/platform cell block with the exact
  prerequisite instead of launching.

Durable usage lives in ``docs/AGENT_TOOLING.md`` ("Java client profiles").
"""

from __future__ import annotations

import hashlib
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

def _gradlew_present() -> mock._patch:
    """Fake only the Gradle wrapper check; real game-dir/mod files stay real."""
    real_is_file = Path.is_file

    def fake_is_file(self: Path) -> bool:
        if self.name == "gradlew":
            return True
        return real_is_file(self)

    return mock.patch.object(Path, "is_file", fake_is_file)



class ProfileTaskTests(unittest.TestCase):
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
                    mock.patch.object(runtime, "wait_port"), \
                    mock.patch.object(runtime.subprocess, "Popen") as launch:
                launch.return_value = mock.Mock()
                process, handle = runtime.start_client(
                    **_valid_kwargs(root / "game", root / "client.log"))
                try:
                    self.assertIs(process, launch.return_value)
                    argv = launch.call_args.args[0]
                    self.assertEqual(argv[-1], ":fabric-agent:runClientMcp")
                    self.assertIn("--no-daemon", argv)
                    self.assertIn("--no-configuration-cache", argv)
                    env = launch.call_args.kwargs["env"]
                    self.assertEqual(env["DISPLAY"], ":99")
                    self.assertNotIn("WAYLAND_DISPLAY", env)
                    self.assertEqual(env["ALSOFT_DRIVERS"], "null")
                    self.assertNotIn("PULSE_SINK", env)
                    self.assertEqual(env["JDK_JAVA_OPTIONS"], "-Xmx512M")
                    seen["env"] = env
                finally:
                    handle.close()
            self.assertEqual(seen["env"]["KRASNOKAMEN_CLIENT_MCP_USERNAME"], "SvcProfile")

    def test_vanilla_explicit_uses_direct_mojang_command(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with mock.patch.object(runtime, "port_open", return_value=False), \
                    mock.patch.object(runtime, "wait_port"), \
                    mock.patch.object(runtime, "wait_for_path_condition") as initialized, \
                    mock.patch.object(runtime, "subprocess", wraps=runtime.subprocess) as fake_subprocess, \
                    mock.patch("tools.harness.java_launcher.vanilla_command",
                               return_value=["java", "net.minecraft.client.main.Main"]) as vanilla_cmd, \
                    mock.patch.object(runtime, "_agent_jar_path",
                                      return_value=root / "agent.jar"):
                fake_subprocess.Popen.return_value = mock.Mock()
                fake_subprocess.Popen.return_value.poll.return_value = None
                _, handle = runtime.start_client(
                    **_valid_kwargs(root / "game", root / "client.log",
                                    client_profile="vanilla"))
                try:
                    vanilla_cmd.assert_called_once()
                    initialized.assert_called_once()
                    predicate = initialized.call_args.args[2]
                    self.assertFalse(predicate()[0])
                    (root / "client.log").write_text("Setting user: SvcProfile\n")
                    self.assertTrue(predicate()[0])
                    argv = fake_subprocess.Popen.call_args.args[0]
                    self.assertEqual(argv[:2], ["java", "net.minecraft.client.main.Main"])
                finally:
                    handle.close()

    def test_vanilla_missing_agent_jar_blocks_before_launch(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with mock.patch.object(runtime, "port_open", return_value=False), \
                    mock.patch.object(runtime.subprocess, "Popen") as launch, \
                    mock.patch.object(runtime, "_agent_jar_path",
                                      side_effect=RuntimeError("java-agent jar missing")):
                with self.assertRaisesRegex(RuntimeError, "java-agent jar missing"):
                    runtime.start_client(
                        **_valid_kwargs(root / "game", root / "client.log",
                                        client_profile="vanilla"))
                launch.assert_not_called()

    def test_loader_requires_platform_and_selects_task(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaisesRegex(ValueError, "requires platform"):
                runtime.start_client(
                    **_valid_kwargs(root / "game", root / "client.log",
                                    client_profile="loader"))
            for platform in ("fabric", "neoforge", "forge"):
                with _gradlew_present(), \
                        mock.patch.object(runtime.os, "access", return_value=True), \
                        mock.patch.object(runtime, "port_open", return_value=False), \
                        mock.patch.object(runtime, "wait_port"), \
                        mock.patch.object(runtime.subprocess, "Popen") as launch:
                    launch.return_value = mock.Mock()
                    _, handle = runtime.start_client(
                        **_valid_kwargs(root / "game", root / "client.log",
                                        client_profile="loader", platform=platform))
                    try:
                        self.assertEqual(
                            launch.call_args.args[0][-1], f":loader-{platform}:runClientMcp")
                    finally:
                        handle.close()

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
                finally:
                    handle.close()


def _mod_entry(profile: str, platform: str) -> dict:
    return runtime._select_mod_entry(profile, platform)


class ModProvisioningTests(unittest.TestCase):
    def _write_jar(self, directory: Path, entry: dict, payload: bytes) -> Path:
        jar = directory / entry["filename"]
        jar.write_bytes(payload)
        return jar

    def test_svc_stages_verified_jar_into_run_mods(self) -> None:
        entry = _mod_entry("svc", "fabric")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source_dir = root / "source"
            source_dir.mkdir()
            payload = b"svc-bytes-" + entry["filename"].encode()
            with mock.patch.object(runtime, "_select_mod_entry", return_value=dict(entry)), \
                    mock.patch.object(runtime, "_profile_jar_path",
                                      return_value=self._write_jar(source_dir, entry, payload)), \
                    mock.patch.object(runtime, "_verify_mod_jar") as verify, \
                    _gradlew_present(), \
                    mock.patch.object(runtime.os, "access", return_value=True), \
                    mock.patch.object(runtime, "port_open", return_value=False), \
                    mock.patch.object(runtime, "wait_port"), \
                    mock.patch.object(runtime.subprocess, "Popen") as launch, \
                    mock.patch("shutil.copy2") as copy:
                launch.return_value = mock.Mock()
                game = root / "run-game"
                _, handle = runtime.start_client(
                    **_valid_kwargs(game, root / "client.log", client_profile="svc"))
                try:
                    self.assertEqual(launch.call_args.args[0][-1], ":loader-fabric:runClientMcp")
                    copy.assert_called_once()
                    staged_target = Path(copy.call_args.args[1])
                    self.assertEqual(staged_target.parent, game / "mods")
                    # Fail-closed verification runs before Gradle starts.
                    self.assertGreaterEqual(verify.call_count, 1)
                    marker = json.loads((game / "krasnokamen-profile.json").read_text())
                    self.assertEqual(marker["client_profile"], "svc")
                    self.assertEqual(marker["platform"], "fabric")
                finally:
                    handle.close()

    def test_dh_stages_verified_jar_into_run_mods(self) -> None:
        entry = _mod_entry("dh", "fabric")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source_dir = root / "source"
            source_dir.mkdir()
            with mock.patch.object(runtime, "_select_mod_entry", return_value=dict(entry)), \
                    mock.patch.object(runtime, "_profile_jar_path",
                                      return_value=self._write_jar(
                                          source_dir, entry, b"dh-bytes")), \
                    mock.patch.object(runtime, "_verify_mod_jar") as verify, \
                    _gradlew_present(), \
                    mock.patch.object(runtime.os, "access", return_value=True), \
                    mock.patch.object(runtime, "port_open", return_value=False), \
                    mock.patch.object(runtime, "wait_port"), \
                    mock.patch.object(runtime.subprocess, "Popen") as launch, \
                    mock.patch("shutil.copy2"):
                launch.return_value = mock.Mock()
                _, handle = runtime.start_client(
                    **_valid_kwargs(root / "game", root / "client.log", client_profile="dh"))
                try:
                    self.assertEqual(launch.call_args.args[0][-1], ":loader-fabric:runClientMcp")
                    self.assertGreaterEqual(verify.call_count, 1)
                finally:
                    handle.close()

    def test_missing_jar_blocks_before_gradle(self) -> None:
        entry = _mod_entry("svc", "fabric")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with mock.patch.object(runtime, "_select_mod_entry", return_value=dict(entry)), \
                    mock.patch.object(runtime, "_profile_jar_path",
                                      return_value=root / "absent.jar"), \
                    _gradlew_present(), \
                    mock.patch.object(runtime.subprocess, "Popen") as launch:
                with self.assertRaisesRegex(RuntimeError, "svc jar missing"):
                    runtime.start_client(
                        **_valid_kwargs(root / "game", root / "client.log",
                                        client_profile="svc"))
                launch.assert_not_called()

    def test_hash_mismatch_blocks_before_gradle(self) -> None:
        entry = _mod_entry("dh", "fabric")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bad = root / entry["filename"]
            bad.write_bytes(b"tampered-bytes")
            with mock.patch.object(runtime, "_select_mod_entry", return_value=dict(entry)), \
                    mock.patch.object(runtime, "_profile_jar_path", return_value=bad), \
                    _gradlew_present(), \
                    mock.patch.object(runtime.subprocess, "Popen") as launch:
                with self.assertRaisesRegex(RuntimeError, "mismatch"):
                    runtime.start_client(
                        **_valid_kwargs(root / "run", root / "client.log",
                                        client_profile="dh"))
                launch.assert_not_called()

    def test_size_mismatch_blocks_before_gradle(self) -> None:
        entry = dict(_mod_entry("svc", "fabric"))
        entry["size"] = entry["size"] + 1
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            jar = root / entry["filename"]
            jar.write_bytes(b"x")
            with mock.patch.object(runtime, "_select_mod_entry", return_value=entry), \
                    mock.patch.object(runtime, "_profile_jar_path", return_value=jar), \
                    _gradlew_present(), \
                    mock.patch.object(runtime.subprocess, "Popen") as launch:
                with self.assertRaisesRegex(RuntimeError, "size mismatch"):
                    runtime.start_client(
                        **_valid_kwargs(root / "run", root / "client.log",
                                        client_profile="svc"))
                launch.assert_not_called()

    def test_missing_neoforge_svc_cell_blocks_with_prerequisite(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with mock.patch.object(runtime, "_resolve_profile_task",
                                   return_value={"platform": "fabric",
                                                 "task": ":loader-fabric:runClientMcp",
                                                 "label": "Fabric + svc"}), \
                    mock.patch.object(runtime, "_select_mod_entry",
                                      side_effect=RuntimeError(
                                          "no published svc artifact covers MC 26.1.2 on neoforge; "
                                          "the profile is blocked until the publisher ships one")), \
                    _gradlew_present(), \
                    mock.patch.object(runtime.subprocess, "Popen") as launch:
                with self.assertRaisesRegex(RuntimeError, "blocked until the publisher ships one"):
                    runtime.start_client(
                        **_valid_kwargs(root / "run", root / "client.log",
                                        client_profile="svc", platform="neoforge"))
                launch.assert_not_called()


class PrerequisiteReportTests(unittest.TestCase):
    def test_vanilla_and_loader_have_no_jar_section(self) -> None:
        info = runtime.profile_prerequisites("vanilla", None)
        self.assertTrue(info["task"].startswith("direct-java:"))
        self.assertIn("java-agent", info["composition"])
        self.assertNotIn("jar", info)
        loader = runtime.profile_prerequisites("loader", "neoforge")
        self.assertEqual(loader["task"], ":loader-neoforge:runClientMcp")
        self.assertNotIn("jar", loader)

    def test_svc_reports_present_verified_jar(self) -> None:
        entry = _mod_entry("svc", "fabric")
        payload = b"s" * 64
        digest512 = hashlib.sha512(payload).hexdigest()
        digest1 = hashlib.sha1(payload).hexdigest()
        with tempfile.TemporaryDirectory() as directory:
            jar = Path(directory) / entry["filename"]
            jar.write_bytes(payload)
            fake_entry = dict(entry, size=len(payload), sha512=digest512, sha1=digest1)
            with mock.patch.object(runtime, "_select_mod_entry", return_value=fake_entry), \
                    mock.patch.object(runtime, "_profile_jar_path", return_value=jar):
                info = runtime.profile_prerequisites("svc", None)
                self.assertEqual(info["task"], ":loader-fabric:runClientMcp")
                self.assertTrue(info["jar"]["present"])
                self.assertTrue(info["jar"]["size_ok"])
                self.assertTrue(info["jar"]["sha512_ok"])
                self.assertTrue(info["jar"]["sha1_ok"])

    def test_dh_reports_missing_jar_with_fetch_hint(self) -> None:
        entry = _mod_entry("dh", "fabric")
        with tempfile.TemporaryDirectory() as directory:
            missing = Path(directory) / entry["filename"]
            with mock.patch.object(runtime, "_select_mod_entry", return_value=dict(entry)), \
                    mock.patch.object(runtime, "_profile_jar_path", return_value=missing):
                info = runtime.profile_prerequisites("dh", None)
                self.assertFalse(info["jar"]["present"])
                self.assertIn(entry["url"], info["jar"]["detail"])
                self.assertIn("No download happens", info["jar"]["detail"])

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
