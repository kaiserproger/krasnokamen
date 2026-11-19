"""Loader launch regressions and retained immutable-candidate/isolation contracts."""
from __future__ import annotations

import json
import tempfile
import tomllib
import unittest
from pathlib import Path
from unittest import mock

from . import loader, profiles, runtime


def setUpModule() -> None:
    # .analysis/ is gitignored, so a fresh checkout or worktree lacks it.
    (runtime.REPO_ROOT / ".analysis").mkdir(exist_ok=True)


class LoaderV2Tests(unittest.TestCase):
    def test_java_clients_use_private_home_for_all_platforms(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for platform in (None, *runtime.PLATFORMS):
                with self.subTest(platform=platform), \
                     mock.patch.dict(runtime.os.environ, {"JAVA_TOOL_OPTIONS": "-Xmx512m -Duser.home=/owner"}), \
                     mock.patch.object(Path, "is_file", return_value=True), \
                     mock.patch.object(runtime.os, "access", return_value=True), \
                     mock.patch.object(runtime, "port_open", return_value=False), \
                     mock.patch.object(runtime, "wait_port"), \
                     mock.patch.object(runtime.subprocess, "Popen") as launch:
                    _, handle = runtime.start_client(
                        game_dir=root / "game", log_path=root / "client.log",
                        token="unit-test-only", mcp_port=12345,
                        username="HarnessUnit", display=":99", platform=platform,
                    )
                    handle.close()
                    env = launch.call_args.kwargs["env"]
                    home = root / "game" / ".home"
                    self.assertEqual(
                        env["JAVA_TOOL_OPTIONS"],
                        f'-Xmx512m -Duser.home=/owner -Duser.home="{home}"',
                    )
                    self.assertEqual(env["HOME"], str(home))

    def test_v2_profile_never_builds_or_replaces_deployment(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            ctx = profiles.ProfileContext(root, platform="fabric",
                server_bin=root / "frozen", plugins_dir=root / "candidate",
                scenario_id="loader-v2-settlements-smoke")
            with mock.patch.object(profiles, "_run_command") as build, \
                 mock.patch.object(loader, "run", return_value={"passed": True}) as run:
                profiles.run_loader_live(ctx)
            build.assert_not_called()
            self.assertEqual(run.call_args.kwargs, {
                "server_bin": ctx.server_bin, "plugins_dir": ctx.plugins_dir,
                "client_ready_timeout_seconds": profiles.PROFILES["loader-live"]["client_ready_timeout_seconds"]})
            self.assertEqual(json.loads((root / "ops.json").read_text())[0]["name"], "GateFabric")

    def test_shared_probe_uses_frozen_binary_without_building(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "krasnokamen"
            binary.write_text("#!/bin/false\n")
            binary.chmod(0o700)
            with mock.patch.dict("os.environ", {"KRASNOKAMEN_SERVER_BINARY": str(binary)}), \
                 mock.patch.object(profiles, "_run_command") as build:
                selected = profiles._server_binary(profiles.ProfileContext(Path(temporary)))
            self.assertEqual(selected, binary)
            build.assert_not_called()

    def test_v2_config_points_at_readonly_candidate_and_frozen_binary(self) -> None:
        with tempfile.TemporaryDirectory(dir=runtime.REPO_ROOT / ".analysis") as temporary:
            root = Path(temporary)
            binary = root / "frozen"
            binary.write_text("#!/bin/false\n")
            binary.chmod(0o700)
            plugins = root / "candidate"
            owner = plugins / "server-only"
            owner.mkdir(parents=True)
            (owner / "plugin.toml").write_text('id = "server-only"\n')
            artifact = root / "artifact"
            artifact.mkdir()
            deploy = mock.Mock()
            with mock.patch.object(runtime, "port_open", return_value=False), \
                 mock.patch.object(runtime, "start_xvfb", side_effect=RuntimeError("config fence")):
                with self.assertRaisesRegex(RuntimeError, "config fence"):
                    loader.run("fabric", 10, artifact, {"run": mock.Mock(), "deploy": deploy},
                        server_bin=binary, plugins_dir=plugins, client_ready_timeout_seconds=1800)
            config = tomllib.loads((artifact / "playable.toml").read_text())
            self.assertEqual(config["plugins"]["directory"], str(plugins))
            result = json.loads((artifact / "scenario.json").read_text())
            self.assertEqual(result["server_binary"], str(binary))
            self.assertEqual(result["expected_bundle_cache_files"], [])
            self.assertEqual((owner / "plugin.toml").read_text(), 'id = "server-only"\n')
            deploy.assert_not_called()

    def _configured_run(self, root: Path, name: str) -> dict:
        """One loader.run up to the Xvfb fence; returns the config port and scenario record."""
        binary = root / "frozen"
        binary.write_text("#!/bin/false\n")
        binary.chmod(0o700)
        plugins = root / "candidate"
        plugins.mkdir(exist_ok=True)
        artifact = root / name
        artifact.mkdir()
        with mock.patch.object(runtime, "start_xvfb", side_effect=RuntimeError("config fence")):
            with self.assertRaisesRegex(RuntimeError, "config fence"):
                loader.run("fabric", 10, artifact, {"run": mock.Mock()},
                           server_bin=binary, plugins_dir=plugins, client_ready_timeout_seconds=1800)
        config = tomllib.loads((artifact / "playable.toml").read_text())
        record = json.loads((artifact / "scenario.json").read_text())
        return {"config_port": config["network"]["port"], "record": record}

    def test_each_run_takes_its_own_free_port_and_the_prompt_follows_it(self) -> None:
        import socket

        with tempfile.TemporaryDirectory(dir=runtime.REPO_ROOT / ".analysis") as temporary:
            root = Path(temporary)
            first = self._configured_run(root, "first")
            # Another agent's gate is listening on the first run's port.
            with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as busy:
                busy.bind(("127.0.0.1", first["config_port"]))
                busy.listen()
                second = self._configured_run(root, "second")
            for run in (first, second):
                port = run["config_port"]
                self.assertNotEqual(port, loader.TEMPLATE_SERVER_PORT)
                self.assertEqual(run["record"]["server_address"], f"127.0.0.1:{port}")
                self.assertEqual(run["record"]["server_port"], port)
                self.assertNotEqual(run["record"]["mcp_port"], port)
            self.assertNotEqual(first["config_port"], second["config_port"])

    def test_permission_prompt_titles_name_the_runs_server_address(self) -> None:
        self.assertEqual(loader.confirmation_titles(31234), (
            "Allow Krasnokamen content from 127.0.0.1:31234?",
            "Allow Krasnokamen content from localhost:31234?",
        ))
        client = mock.Mock()
        titles = []
        def click(_tool: str, arguments: dict) -> None:
            titles.append(arguments["expected_title"])
            if len(titles) == 2:
                return None
            raise RuntimeError("no such dialog")
        client.call_tool.side_effect = click
        self.assertEqual(loader.confirm_loader_permission(client, 5.0, 31234),
                         "Allow Krasnokamen content from localhost:31234?")
        self.assertEqual(titles, list(loader.confirmation_titles(31234)))

    def test_loader_start_waits_for_mcp_with_the_profile_cold_build_allowance(self) -> None:
        with tempfile.TemporaryDirectory(dir=runtime.REPO_ROOT / ".analysis") as temporary:
            root = Path(temporary)
            binary = root / "frozen"
            binary.write_text("#!/bin/false\n")
            binary.chmod(0o700)
            plugins = root / "plugins"
            plugins.mkdir()
            artifact = root / "artifact"
            artifact.mkdir()
            allowance = profiles.PROFILES["loader-live"]["client_ready_timeout_seconds"]
            self.assertIsNone(allowance)
            with mock.patch.object(runtime, "start_xvfb", return_value=(mock.Mock(), ":99", None)), \
                 mock.patch.object(runtime, "sibling_root", return_value=root / "loader"), \
                 mock.patch.object(runtime, "ensure_loader_gradle_home"), \
                 mock.patch.object(runtime.subprocess, "Popen"), \
                 mock.patch.object(runtime, "wait_port") as ready, \
                 mock.patch.object(runtime, "port_open", return_value=False), \
                 mock.patch.object(runtime, "stop_process"), \
                 mock.patch.object(runtime.os, "access", return_value=True), \
                 mock.patch.object(Path, "is_file", return_value=True), \
                 mock.patch.object(loader, "McpClient", side_effect=RuntimeError("MCP fence")):
                with self.assertRaisesRegex(RuntimeError, "MCP fence"):
                    loader.run("neoforge", 10, artifact, {"run": mock.Mock()},
                               server_bin=binary, plugins_dir=plugins,
                               client_ready_timeout_seconds=allowance)
            client_wait = ready.call_args
            self.assertEqual(client_wait.args[1], allowance)
            self.assertEqual(client_wait.kwargs["phase_name"], "client_start")
            record = json.loads((artifact / "scenario.json").read_text())
            self.assertEqual(record["client_ready_timeout_seconds"], allowance)

    def test_cold_build_wait_has_no_clock_deadline_and_requires_owned_listener(self) -> None:
        process = mock.Mock()
        process.poll.return_value = None
        selector = mock.Mock()
        selector.select.return_value = [(mock.Mock(fd=7), 1)]
        with tempfile.TemporaryDirectory() as temporary, \
             mock.patch.object(runtime, "_require_event_fds", return_value=(selector, 7, 8)), \
             mock.patch.object(runtime, "_probe_port", return_value=True) as probe, \
             mock.patch.object(runtime, "_owns_listener", side_effect=[False, True]), \
             mock.patch.object(runtime, "_drain") as drain, \
             mock.patch.object(runtime.os, "close"), \
             mock.patch.object(runtime.time, "monotonic", side_effect=AssertionError("build clock deadline")):
            runtime.wait_port(12345, None, process, log_path=Path(temporary) / "client.log")
        selector.select.assert_called_once_with(None)
        self.assertEqual(probe.call_count, 2)
        probe.assert_called_with(12345, None)
        drain.assert_called_once_with(7)
        selector.close.assert_called_once()

    def test_cold_build_wait_fails_on_process_exit(self) -> None:
        process = mock.Mock(returncode=3)
        process.poll.return_value = 3
        selector = mock.Mock()
        with tempfile.TemporaryDirectory() as temporary, \
             mock.patch.object(runtime, "_require_event_fds", return_value=(selector, 7, 8)), \
             mock.patch.object(runtime.os, "close"):
            log = Path(temporary) / "client.log"
            log.write_text("createMinecraftArtifacts failed")
            with self.assertRaisesRegex(runtime.ChildExited, "createMinecraftArtifacts failed"):
                runtime.wait_port(12345, None, process, log_path=log)
        selector.select.assert_not_called()

    def test_cancelling_cold_start_stops_unreturned_client_process(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with mock.patch.object(runtime, "sibling_root", return_value=root), \
                 mock.patch.object(runtime, "ensure_loader_gradle_home", return_value=root), \
                 mock.patch.object(Path, "is_file", return_value=True), \
                 mock.patch.object(runtime.os, "access", return_value=True), \
                 mock.patch.object(runtime, "port_open", return_value=False), \
                 mock.patch.object(runtime.subprocess, "Popen") as launch, \
                 mock.patch.object(runtime, "wait_port", side_effect=KeyboardInterrupt("SIGTERM")), \
                 mock.patch.object(runtime, "stop_process") as stop:
                with self.assertRaises(KeyboardInterrupt):
                    runtime.start_client(game_dir=root / "game", log_path=root / "client.log",
                                         token="unit", mcp_port=12345, username="Test",
                                         display=":99", platform="neoforge", ready_timeout_seconds=None)
                stop.assert_called_once_with(launch.return_value)
                self.assertTrue(launch.call_args.kwargs["stdout"].closed)

    def test_loader_gradle_home_defaults_to_the_shared_work_cache(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            (work / "loader-gradle-home").mkdir()
            loader_root = work / "solaris-loader"
            with mock.patch.dict(runtime.os.environ, {}, clear=False):
                runtime.os.environ.pop("GRADLE_USER_HOME", None)
                self.assertEqual(runtime.ensure_loader_gradle_home(loader_root), work / "loader-gradle-home")
                self.assertEqual(runtime.os.environ["GRADLE_USER_HOME"], str(work / "loader-gradle-home"))
                # An explicit choice is never overridden.
                runtime.os.environ["GRADLE_USER_HOME"] = "/explicit"
                self.assertEqual(runtime.ensure_loader_gradle_home(loader_root), Path("/explicit"))
                runtime.os.environ.pop("GRADLE_USER_HOME")
            with mock.patch.dict(runtime.os.environ, {}, clear=False):
                runtime.os.environ.pop("GRADLE_USER_HOME", None)
                lonely = Path(temporary) / "other" / "solaris-loader"
                runtime.ensure_loader_gradle_home(lonely)
                self.assertNotIn("GRADLE_USER_HOME", runtime.os.environ)

    def test_client_launch_reuses_gradle_cache_despite_private_home(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            shared = root / "loader-gradle-home"
            shared.mkdir()
            for explicit in (None, str(root / "explicit-gradle")):
                with self.subTest(explicit=explicit), \
                     mock.patch.dict(runtime.os.environ), \
                     mock.patch.object(runtime, "sibling_root", return_value=root / "loader"), \
                     mock.patch.object(Path, "is_file", return_value=True), \
                     mock.patch.object(runtime.os, "access", return_value=True), \
                     mock.patch.object(runtime, "port_open", return_value=False), \
                     mock.patch.object(runtime, "wait_port") as ready, \
                     mock.patch.object(runtime.subprocess, "Popen") as launch:
                    runtime.os.environ.pop("GRADLE_USER_HOME", None)
                    if explicit is not None:
                        runtime.os.environ["GRADLE_USER_HOME"] = explicit
                    _, handle = runtime.start_client(
                        game_dir=root / "game", log_path=root / "client.log",
                        token="test-only", mcp_port=12345, username="GateFabric",
                        display=":99", platform="fabric", client_profile="loader")
                    handle.close()
                    env = launch.call_args.kwargs["env"]
                    self.assertEqual(env["HOME"], str(root / "game" / ".home"))
                    self.assertEqual(env.get("GRADLE_USER_HOME"), explicit or str(shared))
                    self.assertEqual(ready.call_args.args[1], 180.0)


if __name__ == "__main__":
    unittest.main()
