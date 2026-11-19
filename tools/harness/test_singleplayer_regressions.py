"""Focused checks for singleplayer regression registration and event setup."""
import os
from pathlib import Path
import shutil
import signal
import sys
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from tools.harness import runtime
from tools.harness.backends import singleplayer_regressions as sp


def setUpModule() -> None:
    # .analysis/ is gitignored, so a fresh checkout or worktree lacks it.
    (runtime.REPO_ROOT / ".analysis").mkdir(exist_ok=True)


class SingleplayerRegressionTests(unittest.TestCase):
    def test_startup_quota_release_targets_only_the_owned_scope(self):
        with patch.object(sp.singleplayer_slow.subprocess, "run") as run:
            sp.singleplayer_slow.release_startup_quota("solaris-sp-slow-test")
        run.assert_called_once_with(
            ["systemctl", "--user", "set-property", "--runtime",
             "solaris-sp-slow-test.scope", "CPUQuota="],
            check=True, capture_output=True, text=True,
        )

    @unittest.skipUnless(hasattr(os, "pidfd_open"), "requires Linux pidfd")
    def test_server_progress_is_observed_before_listening_without_a_timer(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            pidfile = root / "server.pid"
            pidfile.write_text(str(os.getpid()))
            log = root / "server.log"
            text = 'Loading game data stage="registries"\nis listening addr=127.0.0.1:1\n'
            log.write_text(text)
            progress = []
            self.assertEqual(
                sp.singleplayer_slow.wait_server(pidfile, log, 5, on_progress=progress.append),
                "listening",
            )
            self.assertEqual(progress, [text])

    def test_staged_server_content_contains_no_directory_symlink(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            content = root / "repo" / "data" / "vanilla"
            (content / "tags").mkdir(parents=True)
            (content / "tags" / "blocks.json").write_text("{}")
            binary = root / "binary"
            binary.touch()
            template = root / "template"
            template.mkdir()
            artifact = root / "artifact"
            artifact.mkdir()
            ctx = SimpleNamespace(client="native", windowed=True, scenario_id=sp.FIRST,
                                  extra_args=[], platform=None, timeout_seconds=30,
                                  artifact_dir=artifact, client_evidence={})
            with (
                patch.dict(sp.os.environ, {"KRASNOKAMEN_SERVER_BINARY": str(binary), "KRASNOKAMEN_CLIENT_ROOT": str(template)}),
                patch.object(sp.runtime, "REPO_ROOT", root / "repo"),
                patch.object(sp.native_client, "resolve_binary", return_value=binary),
                patch.object(sp.runtime, "start_xvfb", side_effect=RuntimeError("fixture staged")),
            ):
                with self.assertRaisesRegex(RuntimeError, "fixture staged"):
                    sp.run(ctx)
            staged = artifact / "data/krasnokamen-client/servers/content/26.3"
            self.assertFalse(staged.is_symlink())
            self.assertFalse((staged / "tags").is_symlink())
            self.assertEqual((staged / "tags/blocks.json").read_text(), "{}")

    def test_wait_path_does_not_create_unstarted_world(self):
        with tempfile.TemporaryDirectory() as temp:
            world = Path(temp) / "world"
            database = world / "native" / "state.redb"
            seen = []

            def appeared(path, timeout, condition):
                seen.append(path)
                self.assertFalse(path.exists())
                if path == database:
                    path.touch()
                else:
                    path.mkdir()
                self.assertTrue(condition()[0])

            with patch.object(sp.runtime, "wait_for_path_condition", appeared):
                sp._wait_path(database, 30)
            self.assertEqual(seen, [world, world / "native", database])

    def exec_child(self, binary: Path) -> int:
        """Fork and exec `binary 30`; return its pid once the exec has completed.

        Fork and exec `binary` (a copy of this interpreter) with a 30 s sleep: the pid is
        returned once the exec has completed.

        `write_fd` is close-on-exec, so the parent's read returns EOF exactly when the child
        execs (or dies trying): the exact event, no sleep and no polling. Reading /proc/<pid>/exe
        right after `subprocess.Popen` is the race this replaces — a forked-but-not-yet-execed
        child still points at the test process's own image."""
        read_fd, write_fd = os.pipe()  # the write end closes at exec: the parent reads EOF then
        os.set_inheritable(write_fd, False)
        pid = os.fork()
        if pid == 0:  # pragma: no cover - the child never returns here
            try:
                os.close(read_fd)
                # A copy of this interpreter under the server's name: it stays alive for 30 s
                # whatever its path is called (this host's /usr/bin/sleep is a uutils multi-call
                # that intermittently dispatches on the exe name and exits instead of sleeping).
                os.execv(str(binary), ["python3", "-c", "import time; time.sleep(30)"])
            finally:
                os._exit(127)
        os.close(write_fd)
        try:
            os.read(read_fd, 1)
        finally:
            os.close(read_fd)
        self.assertIsNone(os.waitid(os.P_PID, pid, os.WEXITED | os.WNOHANG),
                          "the child exited instead of exec'ing the binary")
        return pid

    def test_owned_server_is_matched_by_binary_not_by_file_name(self):
        # The ledger tooling hard-links the server it built as `krasnokamen-<sha256>`.
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            server = root / "krasnokamen-0123abcd"
            other = root / "unrelated-binary"
            shutil.copy2(sys.executable, server)
            shutil.copy2(sys.executable, other)
            owner = SimpleNamespace(pid=os.getpid())
            spawned = []
            try:
                spawned.append(self.exec_child(server))
                self.assertEqual(sp._server_child(owner, server.resolve()), spawned[0])
                # `other` is copied but never started: asking for it must find no owned server.
                with self.assertRaisesRegex(RuntimeError, "expected one owned singleplayer server"):
                    sp._server_child(owner, other.resolve())
            finally:
                for pid in spawned:
                    try:
                        os.kill(pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                for pid in spawned:
                    try:
                        os.waitpid(pid, 0)
                    except ChildProcessError:
                        pass


if __name__ == "__main__":
    unittest.main()
