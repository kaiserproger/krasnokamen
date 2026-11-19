"""Mocked Java-client regressions and retained lifecycle/security contracts.

No test launches a real client: runtime lifecycle and MCP sessions are faked.
Observed launch/bootstrap regressions remain alongside conservative identity,
permission, authentication, receipt and environment-isolation contracts.
"""

from __future__ import annotations

import itertools
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from types import SimpleNamespace

from . import java_clients

from .backends import java_matrix


class FakeLogHandle:
    def __init__(self) -> None:
        self.closed = False

    def close(self) -> None:
        self.closed = True


class FakeMcpClient:
    """Stand-in for McpClient; per-test behaviour via ``handler``."""

    instances: list[FakeMcpClient] = []
    events: list[tuple[str, str]] = []
    handler = None

    def __init__(self, endpoint: str, token: str, request_timeout_seconds: float = 15.0) -> None:
        self.endpoint = endpoint
        self.token = token
        self.request_timeout_seconds = request_timeout_seconds
        self.calls: list[tuple[str, dict]] = []
        self.closed = False
        self.chat: list[str] = []
        self.version = 0
        FakeMcpClient.instances.append(self)

    def initialize(self) -> dict:
        self.calls.append(("initialize", {}))
        FakeMcpClient.events.append(("initialize", self.endpoint))
        assert FakeMcpClient.handler is not None
        return FakeMcpClient.handler(self, "initialize", {})

    def call_tool(self, name: str, arguments: dict | None = None) -> dict:
        args = dict(arguments or {})
        self.calls.append((name, args))
        FakeMcpClient.events.append((name, self.endpoint))
        assert FakeMcpClient.handler is not None
        return FakeMcpClient.handler(self, name, args)

    def close(self) -> None:
        self.closed = True


def _ready_handler(client: FakeMcpClient, name: str, args: dict) -> dict:
    if name == "initialize":
        return {"protocolVersion": "test"}
    if name == "minecraft_observe":
        client.version += 1
        return {"state_version": client.version, "in_play": True, "recent_chat": list(client.chat)}
    if name == "minecraft_wait_for_state_change":
        return {"state_version": client.version}
    if name == "minecraft_wait_for_play":
        return {"in_play": True, "endpoint": client.endpoint}
    if name == "minecraft_click_confirmation_button":
        raise AssertionError(f"vanilla client must never confirm: {args}")
    if name == "minecraft_send_chat":
        raise AssertionError(f"unauthenticated client must never chat: {args}")
    return {}


def _auth_handler(client: FakeMcpClient, name: str, args: dict) -> dict:
    if name == "initialize":
        return {"protocolVersion": "test"}
    if name == "minecraft_observe":
        client.version += 1
        return {"state_version": client.version, "in_play": True, "recent_chat": list(client.chat)}
    if name == "minecraft_wait_for_state_change":
        return {"state_version": client.version}
    if name == "minecraft_send_chat":
        assert args.get("command") is True, args
        parts = args["message"].split()
        assert len(parts) == 3 and parts[0] == "register" and parts[1] == parts[2], args
        client.chat.append("registered successfully")
        client.chat.append("logged in")
        return {"sent": True}
    if name == "minecraft_wait_for_play":
        return {"in_play": True, "endpoint": client.endpoint}
    return {}


class JavaClientsTests(unittest.TestCase):
    def setUp(self) -> None:
        FakeMcpClient.instances.clear()
        FakeMcpClient.events.clear()
        FakeMcpClient.handler = _ready_handler
        self._tmp = tempfile.TemporaryDirectory(prefix="java-clients-test-")
        self.addCleanup(self._tmp.cleanup)
        self.artifact = Path(self._tmp.name) / "artifacts"
        self.start_kwargs: list[dict] = []
        self.handles: list[FakeLogHandle] = []
        self.stopped: list = []
        self.xvfb_proc = mock.Mock(pid=42)
        self.xvfb_log = FakeLogHandle()
        self._port_counter = itertools.count(50100)
        self.fail_usernames: set[str] = set([])

    def _fake_start_client(self, **kwargs):
        self.start_kwargs.append(kwargs)
        kwargs["env_snapshot"] = dict(os.environ)
        if kwargs["username"] in self.fail_usernames:
            raise RuntimeError(f"injected launch failure for {kwargs['username']}")
        handle = FakeLogHandle()
        self.handles.append(handle)
        return mock.Mock(pid=1000 + len(self.start_kwargs)), handle

    def _fake_ready(self, client, timeout_seconds: float) -> dict:
        FakeMcpClient.events.append(("ready", client.endpoint))
        return {"in_play": False, "screen": {"class": "TitleScreen"}}

    def _fake_stop(self, process, **kwargs) -> None:
        self.stopped.append(process)

    def _start_patches(self) -> None:
        patchers = [
            mock.patch.object(
                java_clients.runtime, "start_xvfb",
                return_value=(self.xvfb_proc, ":99", self.xvfb_log),
            ),
            mock.patch.object(
                java_clients.runtime, "reserve_port",
                side_effect=lambda: next(self._port_counter),
            ),
            mock.patch.object(
                java_clients.runtime, "start_client", side_effect=self._fake_start_client,
            ),
            mock.patch.object(
                java_clients.runtime, "wait_client_ready_for_connect",
                side_effect=self._fake_ready,
            ),
            mock.patch.object(
                java_clients.runtime, "stop_process", side_effect=self._fake_stop,
            ),
            mock.patch.object(java_clients, "McpClient", FakeMcpClient),
        ]
        for patcher in patchers:
            patcher.start()
            self.addCleanup(patcher.stop)

    def _group_text(self) -> str:
        return (self.artifact / "group.json").read_text()

    def test_matrix_fabric_group_uses_only_private_runtime_launches(self) -> None:
        self._start_patches()
        args = SimpleNamespace(timeout_seconds=20)
        server = SimpleNamespace(port=25567, enabled_plugins=[])
        with mock.patch("subprocess.Popen", side_effect=AssertionError("unexpected prelaunch process")), \
                mock.patch.object(java_clients.runtime, "sibling_root", return_value=Path("/loader")), \
                mock.patch.object(java_clients, "_confirm_exact_permission", return_value="Allowed"):
            with java_matrix.group(args, self.artifact, 2, server, "fabric_krasnokamen_loader") as clients:
                self.assertEqual(len(clients), 2)
                self.assertTrue(all(not client.closed for client in clients))
        self.assertEqual(len(self.start_kwargs), 2)
        java_clients.runtime.start_xvfb.assert_called_once()
        for kwargs in self.start_kwargs:
            self.assertEqual(kwargs["display"], ":99")
            self.assertEqual(kwargs["platform"], "fabric")
            self.assertEqual(kwargs["client_profile"], "loader")
            self.assertNotIn("launch_manifest", kwargs)
        self.assertTrue(self.xvfb_log.closed)
        self.assertIn(self.xvfb_proc, self.stopped)

    def test_group_uses_distinct_identities_dirs_ports_tokens(self) -> None:
        self._start_patches()
        address = "127.0.0.1:25567"
        with java_clients.client_group(self.artifact, 3, address) as clients:
            self.assertEqual(len(clients), 3)
            self.assertEqual(len(FakeMcpClient.instances), 3)
        usernames = [kwargs["username"] for kwargs in self.start_kwargs]
        self.assertEqual(sorted(usernames), ["JMatrix00", "JMatrix01", "JMatrix02"])
        game_dirs = [kwargs["game_dir"] for kwargs in self.start_kwargs]
        self.assertEqual(len({str(path) for path in game_dirs}), 3)
        for index, game_dir in enumerate(sorted(game_dirs, key=str)):
            self.assertEqual(game_dir, self.artifact / f"client-{index:02d}" / "game")
            options = game_dir / "options.txt"
            self.assertTrue(options.is_file())
            self.assertIn("onboardAccessibility:false", options.read_text())
        log_paths = [kwargs["log_path"] for kwargs in self.start_kwargs]
        self.assertEqual(len({str(path) for path in log_paths}), 3)
        ports = [kwargs["mcp_port"] for kwargs in self.start_kwargs]
        self.assertEqual(len(set(ports)), 3)
        tokens = [kwargs["token"] for kwargs in self.start_kwargs]
        self.assertEqual(len(set(tokens)), 3)
        for kwargs in self.start_kwargs:
            self.assertNotIn("platform", kwargs)
            self.assertEqual(kwargs["display"], ":99")
            self.assertEqual(kwargs["client_profile"], "vanilla")
        for instance in FakeMcpClient.instances:
            names = [name for name, _ in instance.calls]
            self.assertEqual(names, ["initialize", "minecraft_connect", "minecraft_wait_for_play"])
            connect_args = instance.calls[1][1]
            self.assertEqual(connect_args, {"server_addr": address})
        by_endpoint = {instance.endpoint: instance for instance in FakeMcpClient.instances}
        self.assertEqual(len(by_endpoint), 3)
        for endpoint in by_endpoint:
            ordered = [name for name, seen in FakeMcpClient.events if seen == endpoint]
            self.assertEqual(
                ordered, ["initialize", "ready", "minecraft_connect", "minecraft_wait_for_play"],
            )

    def test_group_json_records_usernames_ports_profiles_and_play(self) -> None:
        self._start_patches()
        address = "127.0.0.1:25567"
        with java_clients.client_group(self.artifact, 2, address):
            pass
        payload = json.loads(self._group_text())
        self.assertTrue(payload["passed"])
        self.assertFalse(payload["authenticate"])
        self.assertEqual(payload["address"], address)
        self.assertIsNone(payload["platform"])
        self.assertEqual(payload["client_profile"], "vanilla")
        self.assertEqual(payload["display"], ":99")
        entries = sorted(payload["clients"], key=lambda entry: entry["index"])
        self.assertEqual([entry["username"] for entry in entries], ["JMatrix00", "JMatrix01"])
        by_token = {kwargs["token"]: kwargs for kwargs in self.start_kwargs}
        for entry in entries:
            self.assertEqual(entry["client_profile"], "vanilla")
            self.assertIsNone(entry["platform"])
            self.assertFalse(entry["authenticated"])
            self.assertEqual(entry["stage"], "in-play")
            self.assertIsNone(entry["permission_title"])
            self.assertIn(entry["mcp_port"], [kwargs["mcp_port"] for kwargs in self.start_kwargs])
            instance = next(
                inst for inst in FakeMcpClient.instances
                if inst.endpoint == f"http://127.0.0.1:{entry['mcp_port']}/mcp"
            )
            self.assertEqual(instance.token, by_token[instance.token]["token"])
            self.assertEqual(entry["play"], {"in_play": True, "endpoint": instance.endpoint})
        text = self._group_text()
        self.assertNotIn("token", text)
        for kwargs in self.start_kwargs:
            self.assertNotIn(kwargs["token"], text)

    def test_loader_confirms_exact_server_address(self) -> None:
        self._start_patches()
        address = "10.9.8.7:25567"
        expected_title = f"Allow Krasnokamen content from {address}?"

        def handler(client: FakeMcpClient, name: str, args: dict) -> dict:
            if name == "initialize":
                return {"protocolVersion": "test"}
            if name == "minecraft_click_confirmation_button":
                self.assertEqual(
                    args, {"expected_title": expected_title, "button_label": "Allow"},
                )
                return {"confirmed": True}
            if name == "minecraft_wait_for_play":
                return {"in_play": True}
            return {}

        FakeMcpClient.handler = handler
        with java_clients.client_group(
            self.artifact, 2, address, platform="fabric", client_profile="loader",
        ):
            pass
        for kwargs in self.start_kwargs:
            self.assertEqual(kwargs["platform"], "fabric")
            self.assertEqual(kwargs["client_profile"], "loader")
        for instance in FakeMcpClient.instances:
            ordered = [
                name for name, seen in FakeMcpClient.events if seen == instance.endpoint
            ]
            self.assertEqual(
                ordered,
                ["initialize", "ready", "minecraft_connect",
                 "minecraft_click_confirmation_button", "minecraft_wait_for_play"],
            )
        payload = json.loads(self._group_text())
        self.assertTrue(payload["passed"])
        self.assertEqual(payload["platform"], "fabric")
        self.assertEqual(payload["client_profile"], "loader")
        for entry in payload["clients"]:
            self.assertEqual(entry["platform"], "fabric")
            self.assertEqual(entry["permission_title"], expected_title)
            self.assertEqual(entry["stage"], "in-play")

    def test_group_never_touches_the_private_audio_graph_on_scenario_and_startup_failure(self) -> None:
        self._start_patches()
        with mock.patch("tools.harness.audio.private_audio_preflight") as preflight:
            with self.assertRaisesRegex(RuntimeError, "scenario oracle failed"):
                with java_clients.client_group(self.artifact, 2, "127.0.0.1:25567"):
                    raise RuntimeError("scenario oracle failed")
            self.fail_usernames = {"JMatrix01"}
            with self.assertRaisesRegex(RuntimeError, "injected launch failure"):
                with java_clients.client_group(self.artifact, 2, "127.0.0.1:25567"):
                    self.fail("partial startup cannot yield")
            preflight.assert_not_called()
        for handle in self.handles:
            self.assertTrue(handle.closed)

    def test_authenticate_registers_each_client_with_fresh_acks(self) -> None:
        self._start_patches()
        FakeMcpClient.handler = _auth_handler
        with java_clients.client_group(
            self.artifact, 2, "127.0.0.1:25567", authenticate=True,
        ) as clients:
            self.assertEqual(len(clients), 2)
        passwords: set[str] = set()
        for instance in FakeMcpClient.instances:
            sends = [args for name, args in instance.calls if name == "minecraft_send_chat"]
            self.assertEqual(len(sends), 1)
            self.assertTrue(sends[0]["command"])
            parts = sends[0]["message"].split()
            self.assertEqual(len(parts), 3)
            self.assertEqual(parts[0], "register")
            self.assertEqual(parts[1], parts[2])
            passwords.add(parts[1])
        self.assertEqual(len(passwords), 2)
        payload = json.loads(self._group_text())
        self.assertTrue(payload["passed"])
        self.assertTrue(payload["authenticate"])
        for entry in payload["clients"]:
            self.assertTrue(entry["authenticated"])
            self.assertEqual(entry["stage"], "authenticated")
        text = self._group_text()
        self.assertNotIn("password", text.lower())
        self.assertNotIn("token", text)
        for password in passwords:
            self.assertNotIn(password, text)

    def test_authenticate_missing_ack_fails_closed_without_secret(self) -> None:
        self._start_patches()

        def handler(client: FakeMcpClient, name: str, args: dict) -> dict:
            if name == "initialize":
                return {"protocolVersion": "test"}
            if name == "minecraft_observe":
                client.version += 1
                return {"state_version": client.version, "in_play": True, "recent_chat": []}
            if name == "minecraft_wait_for_state_change":
                return {"state_version": client.version}
            if name == "minecraft_send_chat":
                return {"sent": True}  # server stays silent: no ack ever arrives
            if name == "minecraft_wait_for_play":
                return {"in_play": True}
            return {}

        FakeMcpClient.handler = handler
        sent_password = "SilentServer9"
        with mock.patch.object(
            java_clients.secrets, "token_urlsafe", return_value=sent_password,
        ):
            with self.assertRaisesRegex(RuntimeError, "authentication failed"):
                with java_clients.client_group(
                    self.artifact, 1, "127.0.0.1:25567",
                    authenticate=True, timeout_seconds=1,
                ):
                    self.fail("must not yield when auth ack is missing")
        payload = json.loads(self._group_text())
        self.assertFalse(payload["passed"])
        self.assertIn("authentication failed", payload["error"])
        self.assertNotIn(sent_password, self._group_text())
        for instance in FakeMcpClient.instances:
            self.assertTrue(instance.closed)

    def test_group_does_not_cap_gradle_and_compiler_jvms(self) -> None:
        self._start_patches()
        saved = dict(os.environ)
        os.environ.pop("JDK_JAVA_OPTIONS", None)
        self.addCleanup(lambda: os.environ.clear() or os.environ.update(saved))
        snapshots: list[dict] = []
        original = self._fake_start_client

        def capturing(**kwargs):
            snapshots.append(dict(os.environ))
            return original(**kwargs)

        with mock.patch.object(java_clients.runtime, "start_client", side_effect=capturing):
            with java_clients.client_group(self.artifact, 1, "127.0.0.1:25567"):
                pass
        self.assertEqual(len(snapshots), 1)
        self.assertNotIn("JDK_JAVA_OPTIONS", snapshots[0])
        self.assertNotIn("JDK_JAVA_OPTIONS", os.environ)

    def test_heap_cap_respects_operator_preset(self) -> None:
        self._start_patches()
        saved = dict(os.environ)
        os.environ["JDK_JAVA_OPTIONS"] = "-Xmx4G"
        self.addCleanup(lambda: os.environ.clear() or os.environ.update(saved))
        snapshots: list[dict] = []
        original = self._fake_start_client

        def capturing(**kwargs):
            snapshots.append(dict(os.environ))
            return original(**kwargs)

        with mock.patch.object(java_clients.runtime, "start_client", side_effect=capturing):
            with java_clients.client_group(self.artifact, 1, "127.0.0.1:25567"):
                pass
        self.assertEqual(snapshots[0]["JDK_JAVA_OPTIONS"], "-Xmx4G")
        self.assertEqual(os.environ["JDK_JAVA_OPTIONS"], "-Xmx4G")

    def test_headless_env_guard_applies_during_launch_and_restores(self) -> None:
        self._start_patches()
        saved = dict(os.environ)
        os.environ["WAYLAND_DISPLAY"] = "wayland-9"
        os.environ["PULSE_SINK"] = "owner-sink"
        os.environ.pop("PIPEWIRE_NODE", None)
        self.addCleanup(lambda: os.environ.clear() or os.environ.update(saved))
        snapshots: list[dict] = []
        original = self._fake_start_client

        def capturing(**kwargs):
            snapshots.append(dict(os.environ))
            return original(**kwargs)

        with mock.patch.object(java_clients.runtime, "start_client", side_effect=capturing):
            with java_clients.client_group(self.artifact, 1, "127.0.0.1:25567"):
                pass
        self.assertEqual(len(snapshots), 1)
        self.assertNotIn("WAYLAND_DISPLAY", snapshots[0])
        self.assertNotIn("PIPEWIRE_NODE", snapshots[0])
        self.assertEqual(snapshots[0]["PULSE_SINK"], "owner-sink")  # start_client strips it per child env
        self.assertEqual(os.environ.get("WAYLAND_DISPLAY"), "wayland-9")
        self.assertEqual(os.environ.get("PULSE_SINK"), "owner-sink")
        self.assertNotIn("PIPEWIRE_NODE", os.environ)

    def test_reconnect_auth_logs_in_without_registering_again(self) -> None:
        self._start_patches()
        password = "ReconnectPass9"

        def handler(client, name, args):
            if name == "minecraft_send_chat":
                self.assertEqual(args, {"message": f"login {password}", "command": True})
                client.chat.append("logged in")
                return {"sent": True}
            return _ready_handler(client, name, args)

        FakeMcpClient.handler = handler
        client = FakeMcpClient("http://127.0.0.1:50100/mcp", "tok")
        result = java_clients.authenticate_client(client, password, register=False)
        self.assertEqual(result, {"authenticated": True})

    def test_reconnect_auth_accepts_new_identical_login_line(self) -> None:
        self._start_patches()

        def handler(client, name, args):
            if name == "minecraft_send_chat":
                client.chat.append("logged in")
                return {"sent": True}
            return _ready_handler(client, name, args)

        FakeMcpClient.handler = handler
        client = FakeMcpClient("http://127.0.0.1:50100/mcp", "tok")
        client.chat = ["logged in"]
        result = java_clients.authenticate_client(client, "ReconnectPass9", register=False)
        self.assertEqual(result, {"authenticated": True})




if __name__ == "__main__":
    unittest.main()
