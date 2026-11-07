#!/usr/bin/env python3
"""Hermetic unit regression tests for protocol_lifecycle.ProtocolServer.

No server is started here: process spawn, readiness waits, log fences and
the offline restore CLI are faked; the real release binary is exercised by
the parent matrix via cmd_run. Run with:
python3 -m unittest tools.harness.test_protocol_lifecycle
"""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import Any
from unittest import mock

from .backends import protocol_lifecycle as lifecycle
from .backends.protocol_lifecycle import Blocked, ProtocolServer


def _harness(tmp: str, **overrides: object) -> ProtocolServer:
    root = Path(tmp)
    kwargs: dict[str, object] = {
        "binary": root / "solaris-v2",
        "run_dir": root / "run",
        "port": 26111,
        "vanilla_data": root / "vanilla",
        "seed": 81,
    }
    kwargs.update(overrides)
    return ProtocolServer(**kwargs)  # type: ignore[arg-type]


class ConfigTests(unittest.TestCase):
    def test_rejects_port_outside_261xx(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(ValueError):
                _harness(tmp, port=25565)
            with self.assertRaises(ValueError):
                _harness(tmp, dashboard_port=8080)

    def test_rejects_dashboard_port_equal_to_game_port(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(ValueError):
                _harness(tmp, dashboard_port=26111)

    def test_config_paths_ports_and_plugins_forwarding(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "vanilla").mkdir()
            plugins = root / "live-plugins"
            plugins.mkdir()
            server = _harness(
                tmp,
                plugins_dir=plugins,
                dashboard_port=26112,
                server_name="slice-a",
            )
            self.assertEqual(server.config_path, root / "run" / "runproto.toml")
            self.assertEqual(server.log_path, root / "run" / "server.log")
            self.assertEqual(server.world_dir, root / "run" / "world")
            self.assertEqual(server.backup_dir, root / "run" / "backups")
            path = server.write_config()
            text = path.read_text()
            self.assertIn("port = 26111", text)
            self.assertIn("port = 26112", text)
            self.assertIn(f"directory = {json.dumps(str(plugins))}", text)
            self.assertIn("seed = 81", text)
            self.assertIn(f"world_dir = {json.dumps(str(root / 'run' / 'world'))}", text)

    def test_config_omits_plugins_section_without_plugins_dir(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "vanilla").mkdir()
            server = _harness(tmp)
            with mock.patch.object(
                lifecycle, "reserve_matrix_port", return_value=26177
            ) as reserved:
                server.write_config()
            reserved.assert_called_once_with(exclude=26111, port_range=(26100, 26199))
            text = server.config_path.read_text()
            self.assertNotIn("[plugins]", text)
            self.assertEqual(server.dashboard_port, 26177)

    def test_dashboard_auto_port_stays_in_matrix_range(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "vanilla").mkdir()
            server = _harness(tmp)
            server.write_config()
            assert server.dashboard_port is not None
            self.assertTrue(26100 <= server.dashboard_port <= 26199)
            self.assertNotEqual(server.dashboard_port, server.port)


class LifecycleGuardTests(unittest.TestCase):
    def test_start_refuses_double_start(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            server = _harness(tmp)
            server._process = SimpleNamespace(poll=lambda: None)
            with self.assertRaises(RuntimeError):
                server.start()

    def test_start_blocked_without_binary(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "vanilla").mkdir()
            server = _harness(tmp)
            with self.assertRaises(Blocked):
                server.start()

    def test_start_blocked_without_vanilla_data(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "solaris-v2").touch()
            server = _harness(tmp)
            with self.assertRaises(Blocked):
                server.start()

    def test_start_blocked_without_plugins_dir(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "solaris-v2").touch()
            (root / "vanilla").mkdir()
            server = _harness(tmp, plugins_dir=root / "missing-plugins")
            with self.assertRaises(Blocked):
                server.start()

    def test_console_requires_running_server(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            server = _harness(tmp)
            with self.assertRaises(RuntimeError):
                server.console("save-all", "Saved the game")

    def test_console_waits_exact_fence_not_prefix(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "vanilla").mkdir()
            server = _harness(tmp)
            server.run_dir.mkdir(parents=True)
            server.log_path.write_text("Saving the game (this may take a moment!)\n")
            stdin: Any = SimpleNamespace(writes=[], flush=lambda: None)
            stdin.write = stdin.writes.append  # type: ignore[method-assign]
            server._process = SimpleNamespace(poll=lambda: None, stdin=stdin)
            seen: list[str] = []

            def fake_wait(path: Path, timeout: float, condition: object) -> str:
                assert path == server.log_path
                server.log_path.write_text(
                    "Saving the game (this may take a moment!)\nSaved the game\n"
                )
                matched, text = condition()  # type: ignore[operator]
                self.assertTrue(matched)
                seen.append(text)
                return text

            with mock.patch.object(lifecycle.runtime, "wait_for_path_condition", side_effect=fake_wait):
                line = server.save_all()
            self.assertEqual(line, "Saved the game")
            self.assertEqual(stdin.writes, [b"save-all\n"])

    def test_stop_fails_closed_on_nonzero_exit(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "vanilla").mkdir()
            server = _harness(tmp)
            server.run_dir.mkdir(parents=True)
            server.log_path.write_text("")
            stdin = SimpleNamespace(write=lambda b: None, flush=lambda: None)
            proc = SimpleNamespace(pid=7, poll=lambda: None, stdin=stdin, wait=lambda timeout: 1)
            server._process = proc
            with mock.patch.object(
                lifecycle.runtime, "wait_for_path_condition", return_value="Stopping the server\n"
            ):
                with self.assertRaises(RuntimeError):
                    server.stop()

    def test_kill9_restart_sends_sigkill_and_requires_minus9(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "vanilla").mkdir()
            server = _harness(tmp)
            calls: list[str] = []
            old = SimpleNamespace(
                pid=11,
                poll=lambda: None,
                kill=lambda: calls.append("kill"),
                wait=lambda timeout: calls.append("wait") or -9,
            )
            new = SimpleNamespace(pid=13, poll=lambda: None)
            server._process = old
            server._argv = ["solaris-v2", "--config", "cfg"]
            server._log_handle = SimpleNamespace(close=lambda: None)
            with mock.patch.object(
                lifecycle.runtime, "wait_port"
            ) as waited, mock.patch.object(lifecycle.subprocess, "Popen", return_value=new) as popen:
                with mock.patch.object(lifecycle.runtime, "stop_process") as stopped:
                    receipt = server.kill9_restart()
            stopped.assert_not_called()
            self.assertEqual(calls, ["kill", "wait"])
            popen.assert_called_once()
            waited.assert_called_once()
            self.assertEqual(
                receipt,
                {
                    "old_pid": 11,
                    "new_pid": 13,
                    "old_exit": -9,
                    "same_world_dir": str(server.world_dir),
                    "same_config": str(server.config_path),
                    "same_binary": str(server.binary),
                },
            )
            server._close_handles()

    def test_kill9_restart_fails_closed_without_minus9(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "vanilla").mkdir()
            server = _harness(tmp)
            old = SimpleNamespace(
                pid=11, poll=lambda: None, kill=lambda: None, wait=lambda timeout: 0
            )
            server._process = old
            with self.assertRaises(RuntimeError):
                server.kill9_restart()


class BackupProofTests(unittest.TestCase):
    def _running(self, tmp: str) -> ProtocolServer:
        root = Path(tmp)
        (root / "vanilla").mkdir()
        server = _harness(tmp)
        server.run_dir.mkdir(parents=True)
        server.log_path.write_text("")
        stdin = SimpleNamespace(write=lambda b: None, flush=lambda: None)
        server._process = SimpleNamespace(poll=lambda: None, stdin=stdin, pid=21)
        return server

    def test_hot_backup_invocation_and_manifest_proof(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            server = self._running(tmp)
            backup = Path(tmp) / "run" / "backups" / "backup-stamp-tick9"
            backup.mkdir(parents=True)
            (backup / "manifest.json").write_text("{}")
            done = (
                f"Backup {backup} complete: checkpoint tick 8, recoverable to tick 9, "
                "3 files, 4 ms\n"
            )
            with mock.patch.object(
                lifecycle.runtime, "wait_for_path_condition", return_value=done
            ):
                found = server.hot_backup()
            self.assertEqual(found, backup)
            stdin = server._process.stdin
            self.assertIsNotNone(stdin)

    def test_hot_backup_refusal_is_not_a_pass(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            server = self._running(tmp)
            with mock.patch.object(
                lifecycle.runtime,
                "wait_for_path_condition",
                return_value="Backup failed: boom\n",
            ):
                with self.assertRaises(RuntimeError):
                    server.hot_backup()

    def test_hot_backup_without_manifest_proof_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            server = self._running(tmp)
            ghost = Path(tmp) / "run" / "backups" / "backup-ghost-tick1"
            done = (
                f"Backup {ghost} complete: checkpoint tick 1, recoverable to tick 1, "
                "1 files, 1 ms\n"
            )
            with mock.patch.object(
                lifecycle.runtime, "wait_for_path_condition", return_value=done
            ):
                with self.assertRaises(RuntimeError):
                    server.hot_backup()

    def test_restore_proof_invokes_offline_cli_and_checks_state(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "vanilla").mkdir()
            server = _harness(tmp)
            server.run_dir.mkdir(parents=True)
            server.config_path.write_text("# cfg\n")
            backup = root / "backup-ok"
            backup.mkdir()
            (backup / "manifest.json").write_text("{}")
            target = root / "run" / "restore-proof"
            state = target / "native" / "state.redb"
            body = "restore of b into t complete: 12 files verified\n"

            def fake_run(*args: object, **kwargs: object) -> SimpleNamespace:
                argv = args[0]
                assert argv[:4] == [str(server.binary), "--config", str(server.config_path), "restore"]
                assert "--from" in argv and "--world-dir" in argv
                state.parent.mkdir(parents=True, exist_ok=True)
                state.write_bytes(b"db")
                return SimpleNamespace(returncode=0, stdout=body.encode())

            with mock.patch.object(lifecycle.subprocess, "run", side_effect=fake_run):
                receipt = server.restore_proof(backup)
            self.assertEqual(
                receipt,
                {
                    "backup": str(backup),
                    "target": str(target),
                    "files_verified": 12,
                    "world_state": str(state),
                    "argv": receipt["argv"],
                },
            )

    def test_restore_proof_rejects_nonempty_target(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "vanilla").mkdir()
            server = _harness(tmp)
            backup = root / "backup-ok"
            backup.mkdir()
            (backup / "manifest.json").write_text("{}")
            target = root / "target"
            target.mkdir()
            (target / "junk").write_text("x")
            with self.assertRaises(RuntimeError):
                server.restore_proof(backup, target)

    def test_restore_proof_rejects_backup_without_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "vanilla").mkdir()
            server = _harness(tmp)
            with self.assertRaises(Blocked):
                server.restore_proof(root / "nope")


class PauseProbeTests(unittest.TestCase):
    def test_pause_probe_blocked_without_pause_config(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            server = _harness(tmp)
            with self.assertRaises(Blocked):
                server.pause_empty_probe()


if __name__ == "__main__":
    unittest.main()
