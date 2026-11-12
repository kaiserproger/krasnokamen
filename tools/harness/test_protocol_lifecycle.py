#!/usr/bin/env python3
"""Hermetic unit regression tests for protocol_lifecycle.ProtocolServer.

No server is started here: process spawn, readiness waits, log fences and
the offline restore CLI are faked; the real release binary is exercised by
the parent matrix via cmd_run. Run with:
python3 -m unittest tools.harness.test_protocol_lifecycle
"""

from __future__ import annotations

import json
import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import Any
from unittest import mock

from .backends import protocol_lifecycle as lifecycle
from .backends.protocol_lifecycle import Blocked, PortReservation, ProtocolServer


def _harness(tmp: str, **overrides: object) -> ProtocolServer:
    root = Path(tmp)
    kwargs: dict[str, object] = {
        "binary": root / "krasnokamen",
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
            with mock.patch.object(lifecycle, "PORT_LOCK_DIR", Path(tmp) / "locks"):
                server.write_config()
                assert server.dashboard_port is not None
                self.assertTrue(26100 <= server.dashboard_port <= 26199)
                self.assertNotEqual(server.dashboard_port, server.port)
                server._close_handles()


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
            (Path(tmp) / "krasnokamen").touch()
            server = _harness(tmp)
            with self.assertRaises(Blocked):
                server.start()

    def test_start_blocked_without_plugins_dir(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "krasnokamen").touch()
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
            server.run_dir.mkdir(parents=True)
            server.log_path.write_text("checkpoint committed tick=7\n")
            calls: list[str] = []
            old = SimpleNamespace(
                pid=11,
                poll=lambda: None,
                kill=lambda: calls.append("kill"),
                wait=lambda timeout: calls.append("wait") or -9,
            )
            new = SimpleNamespace(pid=13, poll=lambda: None)
            server._process = old
            server._argv = ["krasnokamen", "--config", "cfg"]
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
                    "published_prefix_before_kill": 7,
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


class PortLeaseTests(unittest.TestCase):
    def test_concurrent_reservations_get_distinct_ports(self) -> None:
        # Regression (red on ef508fc7): reserve_matrix_port used to bind-probe
        # and close the socket before the caller bound it, so N concurrent
        # callers could all be handed the same port. The flock lease must hand
        # out N distinct ports to N simultaneous callers.
        import threading
        from . import runtime
        count = 8
        with tempfile.TemporaryDirectory() as tmp:
            base = runtime.reserve_port()
            port_range = (base, base + 63)
            with mock.patch.object(lifecycle, "PORT_LOCK_DIR", Path(tmp) / "locks"):
                barrier = threading.Barrier(count)
                results: list[int] = []
                errors: list[BaseException] = []
                held: list = []

                def grab() -> None:
                    try:
                        barrier.wait(timeout=15)
                        reservation = lifecycle.reserve_matrix_port(port_range=port_range)
                        held.append(reservation)
                        results.append(int(reservation))
                        barrier.wait(timeout=15)  # hold every lease until all reserved
                    except BaseException as error:  # noqa: BLE001 - collected below
                        errors.append(error)

                threads = [threading.Thread(target=grab) for _ in range(count)]
                for thread in threads:
                    thread.start()
                for thread in threads:
                    thread.join(timeout=30)
                for reservation in held:
                    release = getattr(reservation, "release", None)
                    if callable(release):
                        release()
            self.assertEqual(errors, [])
            self.assertEqual(len(results), count)
            self.assertEqual(len(set(results)), count,
                             f"concurrent reservations collided: {sorted(results)}")

    def test_held_lease_blocks_the_same_port_until_released(self) -> None:
        from . import runtime
        with tempfile.TemporaryDirectory() as tmp:
            base = runtime.reserve_port()
            with mock.patch.object(lifecycle, "PORT_LOCK_DIR", Path(tmp) / "locks"):
                first = lifecycle.reserve_matrix_port(port_range=(base, base))
                with self.assertRaises(Blocked):
                    lifecycle.reserve_matrix_port(port_range=(base, base))
                release = getattr(first, "release", None)
                if callable(release):
                    release()
                second = lifecycle.reserve_matrix_port(port_range=(base, base))
                self.assertEqual(int(second), base)
                release = getattr(second, "release", None)
                if callable(release):
                    release()

    def test_leases_survive_stop_and_release_on_final_close(self) -> None:
        # Restarts (stop/start, kill9) reuse the same fixed port, so the lease
        # must survive _close_handles (stop) and drop only on close().
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "vanilla").mkdir()
            server = _harness(tmp)
            with mock.patch.object(lifecycle, "PORT_LOCK_DIR", Path(tmp) / "locks"):
                server.write_config()
                assert server.dashboard_port is not None
                reservation = server._reservations[-1]
                self.assertIsInstance(reservation, PortReservation)
                self.assertEqual(int(reservation), server.dashboard_port)
                server._close_handles()  # what stop() does
                with self.assertRaises(Blocked):
                    lifecycle.reserve_matrix_port(port_range=(int(reservation), int(reservation)))
                server.close()
                again = lifecycle.reserve_matrix_port(port_range=(int(reservation), int(reservation)))
                self.assertEqual(int(again), int(reservation))
                release = getattr(again, "release", None)
                if callable(release):
                    release()

    def test_lock_dir_is_one_fixed_path_regardless_of_env(self) -> None:
        # Main decision: the lock namespace must not depend on ANY env var.
        # Every process on the host uses the same literal directory, created
        # sticky 1777 with one lock file per port.
        import stat
        import subprocess
        import sys
        self.assertEqual(lifecycle.PORT_LOCK_DIR, Path("/tmp/solaris-port-locks"))
        root = str(Path(__file__).resolve().parents[2])
        for extra in ({"KRASNOKAMEN_WORK_ROOT": "/w", "TMPDIR": "/t",
                       "KRASNOKAMEN_PORT_LOCK_DIR": "/ignored"}, {}):
            env = {k: v for k, v in os.environ.items()
                   if k not in ("KRASNOKAMEN_WORK_ROOT", "TMPDIR", "KRASNOKAMEN_PORT_LOCK_DIR")}
            env.update(extra)
            out = subprocess.run(
                [sys.executable, "-c",
                 f"import sys; sys.path.insert(0, {root!r});"
                 "from tools.harness.backends import protocol_lifecycle as lc;"
                 "print(lc.PORT_LOCK_DIR)"],
                env=env, capture_output=True, text=True, check=True).stdout.strip()
            self.assertEqual(out, "/tmp/solaris-port-locks")
        from . import runtime
        base = runtime.reserve_port()
        reservation = lifecycle.reserve_matrix_port(port_range=(base, base))
        try:
            mode = stat.S_IMODE(os.stat(lifecycle.PORT_LOCK_DIR).st_mode)
            self.assertEqual(mode, 0o1777)
            self.assertTrue((lifecycle.PORT_LOCK_DIR / f"{base}.lock").is_file())
        finally:
            reservation.release()

    def test_reservations_coordinate_across_processes_with_different_env(self) -> None:
        # Regression (red on 207b7097): the lock dir used to be derived from
        # env, so two OS processes with different KRASNOKAMEN_WORK_ROOT/TMPDIR
        # used different lock dirs and both leased the same port. Separate
        # processes with deliberately different env holding live leases must
        # get distinct ports from the one machine-wide namespace.
        import subprocess
        import sys
        from . import runtime
        base = runtime.reserve_port()
        root = str(Path(__file__).resolve().parents[2])
        child = (
            "import sys, time; sys.path.insert(0, {root!r});"
            "from tools.harness.backends import protocol_lifecycle as lc;"
            "r = lc.reserve_matrix_port(port_range=({base}, {base} + 3));"
            "print(int(r), flush=True); time.sleep(30)"  # hold the live lease
        ).format(root=root, base=base)
        envs = [
            {"KRASNOKAMEN_WORK_ROOT": "/tmp/fake-root-a", "TMPDIR": "/tmp/fake-tmp-a"},  # set
            {},  # unset
        ]
        procs = []
        for extra in envs:
            env = {k: v for k, v in os.environ.items()
                   if k not in ("KRASNOKAMEN_WORK_ROOT", "TMPDIR", "KRASNOKAMEN_PORT_LOCK_DIR")}
            env.update(extra)
            procs.append(subprocess.Popen(
                [sys.executable, "-c", child], env=env,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True))
        try:
            ports = [int(procs[i].stdout.readline().strip()) for i in range(2)]
            for proc in procs:
                proc.stdout.close()
                proc.stderr.close()
        finally:
            for proc in procs:
                proc.kill()
                proc.wait(timeout=10)
        self.assertEqual(len(set(ports)), 2,
                         f"cross-process leases collided despite different env: {ports}")


if __name__ == "__main__":
    unittest.main()
