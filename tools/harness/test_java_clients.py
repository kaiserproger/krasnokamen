"""Mocked lifecycle tests for the multi Java-client helper.

These tests never launch a real client: ``runtime`` lifecycle functions and
``McpClient`` are faked. They pin the contract the matrix runner relies on:
N distinct identities/game dirs, play readiness before yield, exact-address
Loader permission confirmation, optional krasnokamen-auth registration with
secret-free receipts, N8 heap caps, failure group.json evidence, and full
cleanup on any failure.
"""

from __future__ import annotations

import itertools
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from . import java_clients


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

    def _enter_group(self, *args, **kwargs):
        return java_clients.client_group(*args, **kwargs)

    def _group_text(self) -> str:
        return (self.artifact / "group.json").read_text()

    def test_rejects_bad_count_before_any_lifecycle(self) -> None:
        self._start_patches()
        for bad in (0, 9, -2, "3", True, None):
            with self.assertRaises(ValueError, msg=f"count={bad!r}"):
                with self._enter_group(self.artifact, bad, "127.0.0.1:25567"):
                    self.fail("must not yield")
        java_clients.runtime.start_xvfb.assert_not_called()
        java_clients.runtime.start_client.assert_not_called()

    def test_rejects_bad_address_profile_platform_timeout_authenticate(self) -> None:
        self._start_patches()
        with self.assertRaises(ValueError):
            with self._enter_group(self.artifact, 2, ""):
                pass
        with self.assertRaises(ValueError):
            with self._enter_group(self.artifact, 2, "127.0.0.1:25567", platform="bogus"):
                pass
        with self.assertRaises(ValueError):
            with self._enter_group(self.artifact, 2, "127.0.0.1:25567", client_profile=""):
                pass
        with self.assertRaises(ValueError):
            with self._enter_group(self.artifact, 2, "127.0.0.1:25567", timeout_seconds=0):
                pass
        with self.assertRaises(ValueError):
            with self._enter_group(self.artifact, 2, "127.0.0.1:25567", authenticate="yes"):
                pass
        java_clients.runtime.start_xvfb.assert_not_called()

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

    def test_not_in_play_writes_failure_receipt_and_cleans_everything(self) -> None:
        self._start_patches()

        def handler(client: FakeMcpClient, name: str, args: dict) -> dict:
            if name == "initialize":
                return {"protocolVersion": "test"}
            if name == "minecraft_wait_for_play":
                return {"in_play": False, "endpoint": client.endpoint}
            return {}

        FakeMcpClient.handler = handler
        entered = False
        with self.assertRaisesRegex(RuntimeError, "did not enter Play"):
            with java_clients.client_group(self.artifact, 2, "127.0.0.1:25567"):
                entered = True
        self.assertFalse(entered)
        payload = json.loads(self._group_text())
        self.assertFalse(payload["passed"])
        self.assertIn("did not enter Play", payload["error"])
        self.assertGreaterEqual(len(payload["clients"]), 1)
        self.assertLessEqual(len(payload["clients"]), 2)  # parallel workers may already be launched
        for entry in payload["clients"]:
            self.assertIsNone(entry["play"])
        self.assertNotIn("token", self._group_text())
        for instance in FakeMcpClient.instances:
            self.assertTrue(instance.closed)
        for handle in self.handles:
            self.assertTrue(handle.closed)
        self.assertTrue(self.xvfb_log.closed)
        self.assertIn(self.xvfb_proc, self.stopped)
        self.assertEqual(len([proc for proc in self.stopped if proc is not self.xvfb_proc]),
                         len(payload["clients"]))

    def test_partial_launch_failure_writes_receipt_and_cleans_siblings(self) -> None:
        self._start_patches()
        self.fail_usernames = {"JMatrix01"}
        with self.assertRaisesRegex(RuntimeError, "injected launch failure"):
            with java_clients.client_group(self.artifact, 3, "127.0.0.1:25567"):
                self.fail("must not yield on partial failure")
        payload = json.loads(self._group_text())
        self.assertFalse(payload["passed"])
        self.assertIn("injected launch failure", payload["error"])
        launched_usernames = {kwargs["username"] for kwargs in self.start_kwargs}
        self.assertIn("JMatrix00", launched_usernames)
        self.assertIn("JMatrix01", launched_usernames)
        for instance in FakeMcpClient.instances:
            self.assertTrue(instance.closed)
        for handle in self.handles:
            self.assertTrue(handle.closed)
        self.assertTrue(self.xvfb_log.closed)
        self.assertIn(self.xvfb_proc, self.stopped)
        stopped_ids = {id(proc) for proc in self.stopped}
        self.assertIn(id(self.xvfb_proc), stopped_ids)
        self.assertNotIn("token", self._group_text())

    def test_initialize_failure_still_closes_session(self) -> None:
        self._start_patches()

        def handler(client: FakeMcpClient, name: str, args: dict) -> dict:
            if name == "initialize":
                raise RuntimeError("injected init failure")
            return _ready_handler(client, name, args)

        FakeMcpClient.handler = handler
        with self.assertRaisesRegex(RuntimeError, "injected init failure"):
            with java_clients.client_group(self.artifact, 2, "127.0.0.1:25567"):
                self.fail("must not yield on init failure")
        # A sibling may already have initialized before failure cancellation.
        self.assertGreaterEqual(len(FakeMcpClient.instances), 1)
        self.assertLessEqual(len(FakeMcpClient.instances), 2)
        for instance in FakeMcpClient.instances:
            self.assertTrue(instance.closed)
        payload = json.loads(self._group_text())
        self.assertFalse(payload["passed"])
        self.assertIn("injected init failure", payload["error"])
        for handle in self.handles:
            self.assertTrue(handle.closed)
        self.assertTrue(self.xvfb_log.closed)
        self.assertIn(self.xvfb_proc, self.stopped)

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

    def test_exit_after_yield_cleans_everything(self) -> None:
        self._start_patches()
        with java_clients.client_group(self.artifact, 2, "127.0.0.1:25567"):
            pass
        for instance in FakeMcpClient.instances:
            self.assertTrue(instance.closed)
        for handle in self.handles:
            self.assertTrue(handle.closed)
        self.assertTrue(self.xvfb_log.closed)
        self.assertIn(self.xvfb_proc, self.stopped)

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

    def test_authenticate_client_rejects_stale_baseline_match(self) -> None:
        self._start_patches()
        client = FakeMcpClient("http://127.0.0.1:50100/mcp", "tok", 5.0)
        client.chat = ["older session: registered ok"]
        FakeMcpClient.handler = _auth_handler
        result = java_clients.authenticate_client(client, "FreshPass9", timeout_seconds=10)
        self.assertEqual(result, {"authenticated": True})
        sends = [args for name, args in client.calls if name == "minecraft_send_chat"]
        self.assertEqual(len(sends), 1)
        self.assertEqual(sends[0], {"message": "register FreshPass9 FreshPass9", "command": True})

    def test_authenticate_client_validates_password_shape(self) -> None:
        self._start_patches()
        client = FakeMcpClient("http://127.0.0.1:50100/mcp", "tok", 5.0)
        for bad in ("", "abc", "has space", "tab\there", "x" * 129):
            with self.assertRaises(ValueError, msg=f"password={bad!r}"):
                java_clients.authenticate_client(client, bad)
        self.assertEqual(client.calls, [])

    def test_authenticate_client_missing_ack_error_hides_password(self) -> None:
        self._start_patches()
        password = "SecretMissingAck4"

        def handler(client: FakeMcpClient, name: str, args: dict) -> dict:
            if name == "minecraft_observe":
                client.version += 1
                return {"state_version": client.version, "in_play": True, "recent_chat": []}
            if name == "minecraft_wait_for_state_change":
                return {"state_version": client.version}
            if name == "minecraft_send_chat":
                return {"sent": True}
            return {}

        FakeMcpClient.handler = handler
        client = FakeMcpClient("http://127.0.0.1:50100/mcp", "tok", 5.0)
        with self.assertRaisesRegex(RuntimeError, "authentication acknowledgement missing"):
            java_clients.authenticate_client(client, password, timeout_seconds=1)
        try:
            java_clients.authenticate_client(client, password, timeout_seconds=1)
        except RuntimeError as error:
            self.assertNotIn(password, str(error))
        else:
            self.fail("expected RuntimeError")

    def test_heap_cap_applies_during_launch_and_restores(self) -> None:
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
        self.assertEqual(snapshots[0]["JDK_JAVA_OPTIONS"], "-Xmx512M")
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

    def test_no_direct_process_spawning(self) -> None:
        self.assertNotIn("subprocess", dir(java_clients))
        self._start_patches()
        with mock.patch("subprocess.Popen", side_effect=AssertionError("direct spawn")):
            with java_clients.client_group(self.artifact, 2, "127.0.0.1:25567"):
                pass
        java_clients.runtime.start_xvfb.assert_called_once_with(self.artifact)
        self.assertEqual(java_clients.runtime.start_client.call_count, 2)

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
