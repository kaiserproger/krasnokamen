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


class LifecycleGuardTests(unittest.TestCase):
    def test_command_fence_uses_private_read_only_query(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            server = _harness(tmp)
            server.run_dir.mkdir(parents=True)
            server.log_path.write_text("")
            stdin: Any = SimpleNamespace(writes=[], flush=lambda: None)
            stdin.write = stdin.writes.append
            server._process = SimpleNamespace(poll=lambda: None, stdin=stdin)

            def fake_wait(path: Path, timeout: float, condition: object) -> str:
                # The fence marker is unique per call: read it back from the console write.
                marker = stdin.writes[0].decode().splitlines()[1].split("list ", 1)[1]
                path.write_text(f"fixture complete\nScores for {marker}: {{}}\n")
                matched, text = condition()
                self.assertTrue(matched)
                return text

            with mock.patch.object(lifecycle.runtime, "wait_for_path_condition", side_effect=fake_wait):
                result = server.command("time query daytime")
            sent = stdin.writes[0].decode()
            marker = sent.splitlines()[1].split("list ", 1)[1]
            self.assertTrue(marker.startswith("MatrixConsoleFence"))
            self.assertEqual(len(marker) - len("MatrixConsoleFence"), 32)
            self.assertEqual(sent, f"time query daytime\nscoreboard players list {marker}\n")
            # Only the output before the fence is the command's answer.
            self.assertEqual(result, "fixture complete\n")

    def test_kill9_restart_sends_sigkill_and_requires_minus9(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "vanilla").mkdir()
            server = _harness(tmp)
            server.run_dir.mkdir(parents=True)
            server.log_path.write_text("checkpoint committed tick=7\n")
            calls: list[str] = []
            (server.world_dir / "native").mkdir(parents=True)
            (server.world_dir / "native" / "state.redb").write_bytes(b"checkpoint-seven")
            journal = server.world_dir / "journal"
            journal.mkdir()
            (journal / "batch-8-9.bin").write_bytes(b"published-player-image")
            (journal / "batch-10-10.tmp").write_bytes(b"unpublished-tail")
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
            def start_after_snapshot(*args, **kwargs):
                snapshot, = (server.run_dir / "crashes").iterdir()
                self.assertEqual(
                    (snapshot / "world" / "native" / "state.redb").read_bytes(),
                    b"checkpoint-seven",
                )
                self.assertEqual(
                    (snapshot / "world" / "journal" / "batch-8-9.bin").read_bytes(),
                    b"published-player-image",
                )
                self.assertEqual(
                    (snapshot / "world" / "journal" / "batch-10-10.tmp").read_bytes(),
                    b"unpublished-tail",
                )
                self.assertEqual((snapshot / "server.log").read_text(), "checkpoint committed tick=7\n")
                # Model a later checkpoint pruning/replacing the original evidence.
                (journal / "batch-8-9.bin").unlink()
                (server.world_dir / "native" / "state.redb").write_bytes(b"later-checkpoint")
                return new

            with mock.patch.object(
                lifecycle.runtime, "wait_port"
            ) as waited, mock.patch.object(lifecycle.subprocess, "Popen", return_value=new) as popen:
                with mock.patch.object(lifecycle.runtime, "stop_process") as stopped:
                    popen.side_effect = start_after_snapshot
                    receipt = server.kill9_restart()
            stopped.assert_not_called()
            self.assertEqual(calls, ["kill", "wait"])
            popen.assert_called_once()
            waited.assert_called_once()
            snapshot = Path(receipt["crash_snapshot_dir"])
            self.assertEqual((snapshot / "world" / "journal" / "batch-8-9.bin").read_bytes(),
                             b"published-player-image")
            proof = json.loads((snapshot / "proof.json").read_text())
            self.assertEqual(proof, receipt)
            self.assertEqual(
                receipt,
                {
                    "old_pid": 11,
                    "new_pid": 13,
                    "old_exit": -9,
                    "published_prefix_before_kill": 9,
                    "crash_snapshot_dir": str(snapshot),
                    "log_offset_after_kill": len(b"checkpoint committed tick=7\n"),
                    "mutation_tick_bounds": None,
                    "same_world_dir": str(server.world_dir),
                    "same_config": str(server.config_path),
                    "same_binary": str(server.binary),
                },
            )
            server._close_handles()

    def test_kill9_snapshot_failure_refuses_successor(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            server = _harness(tmp)
            server.run_dir.mkdir(parents=True)
            server.log_path.write_text("")
            server.world_dir.mkdir()
            server._process = SimpleNamespace(
                pid=11, poll=lambda: None, kill=lambda: None, wait=lambda timeout: -9,
            )
            with mock.patch.object(lifecycle.shutil, "copytree", side_effect=OSError("disk full")), \
                    mock.patch.object(lifecycle.runtime, "start_server") as started:
                with self.assertRaisesRegex(OSError, "disk full"):
                    server.kill9_restart()
            started.assert_not_called()
            self.assertIsNone(server._process)

    def test_kill9_records_mutation_tick_bounds(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            server = _harness(tmp)
            server.run_dir.mkdir(parents=True)
            server.world_dir.mkdir()
            server.log_path.write_text("checkpoint committed \x1b[3mtick\x1b[0m\x1b[2m=\x1b[0m1598\n")
            server._argv = ["krasnokamen"]
            server._process = SimpleNamespace(
                pid=11, poll=lambda: None, kill=lambda: None, wait=lambda timeout: -9,
            )
            with mock.patch.object(lifecycle.runtime, "start_server",
                                   return_value=SimpleNamespace(pid=13)), \
                    mock.patch.object(lifecycle.runtime, "wait_port"):
                receipt = server.kill9_restart(mutation_tick_bounds=(1599, 1642))
            self.assertEqual(receipt["mutation_tick_bounds"], [1599, 1642])
            self.assertEqual(receipt["published_prefix_before_kill"], 1598)
            self.assertEqual(json.loads(
                (Path(receipt["crash_snapshot_dir"]) / "proof.json").read_text(),
            )["mutation_tick_bounds"], [1599, 1642])
            server._close_handles()


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


class WalProbeBoundsTests(unittest.TestCase):
    def test_grant_bounds_bracket_command_and_received_inventory(self) -> None:
        from .backends.protocol_matrix import capability_probe

        server = mock.Mock()
        actor = mock.Mock(name="actor")
        actor.name = "Proto0"
        actor.snapshot.return_value = {
            "inventory": [{"item_id": "minecraft:stone", "count": 16}],
        }
        actor.wait_until.side_effect = lambda predicate, **kwargs: self.assertTrue(predicate())
        server.dashboard_stats.side_effect = [
            {"simulation_tick": 1640}, {"simulation_tick": 1642},
        ]
        server.kill9_restart.return_value = {"mutation_tick_bounds": [1640, 1642]}
        events = mock.Mock()
        events.attach_mock(server, "server")
        events.attach_mock(actor, "actor")
        report = {}
        capability_probe("server_kill9_wal_replay", server, [actor], report)
        calls = events.mock_calls
        checkpoint = calls.index(mock.call.server.save_all())
        before = calls.index(mock.call.server.dashboard_stats())
        give = calls.index(mock.call.server.command("give Proto0 stone 16"))
        after = calls.index(mock.call.server.dashboard_stats(), before + 1)
        kill = calls.index(mock.call.server.kill9_restart(mutation_tick_bounds=(1640, 1642)))
        self.assertLess(checkpoint, before)
        self.assertLess(before, give)
        self.assertLess(give, after)
        self.assertLess(after, kill)
        self.assertTrue(any(call[0] == "actor.wait_until" for call in calls[give:after]))
        self.assertEqual(report["kill9"]["mutation_tick_bounds"], [1640, 1642])


if __name__ == "__main__":
    unittest.main()
