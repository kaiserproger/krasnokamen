"""Focused checks for singleplayer regression registration and event setup."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from tools.harness import regressions, scenarios
from tools.harness.backends import singleplayer_regressions as sp


class SingleplayerRegressionTests(unittest.TestCase):
    def test_scenarios_are_graphical_native_ledger_checks(self):
        for name in (sp.FIRST, sp.LEASE, sp.SLOW, sp.WEAK):
            with self.subTest(name=name):
                entry = scenarios.SCENARIOS[name]
                self.assertEqual(entry.evidence, "graphical")
                self.assertEqual(entry.scenario_id, name)
                command = regressions._client_command(entry.profile, name)
                self.assertIn("--windowed", command)
                self.assertIn("native", command)

    def test_slow_wrapper_runs_real_server_with_cgroup_quota_receipt(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            binary = root / "real server"
            binary.touch()
            pidfile, unit = sp.singleplayer_slow.stage_server(binary, root, root)
            wrapper = (root / "solaris-v2").read_text()
            self.assertTrue(unit.startswith("solaris-sp-slow-"))
            self.assertEqual(pidfile, root / "slow-server.pid")
            self.assertIn("CPUQuota=1%", wrapper)
            self.assertIn("/proc/self/cgroup", wrapper)
            self.assertIn("cpu.max", wrapper)
            self.assertIn("--expand-environment=no", wrapper)
            self.assertIn("server-process.pid", wrapper)
            self.assertIn(str(binary), wrapper)

    def test_laptop_wrapper_does_not_artificially_throttle_the_server(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            binary = root / "real server"
            pidfile, unit = sp.singleplayer_slow.stage_server(binary, root, root, slow=False)
            wrapper = (root / "solaris-v2").read_text()
            self.assertIsNone(unit)
            self.assertEqual(pidfile, root / "slow-server.pid")
            self.assertNotIn("CPUQuota", wrapper)
            self.assertNotIn("systemd-run", wrapper)
            self.assertIn(str(binary), wrapper)
            self.assertIn("server-process.pid", wrapper)

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
                patch.dict(sp.os.environ, {"SOLARIS_SERVER_BINARY": str(binary)}),
                patch.object(sp.runtime, "REPO_ROOT", root / "repo"),
                patch.object(sp.native_client, "_client_root", return_value=template),
                patch.object(sp.native_client, "resolve_binary", return_value=binary),
                patch.object(sp.runtime, "start_xvfb", side_effect=RuntimeError("fixture staged")),
            ):
                with self.assertRaisesRegex(RuntimeError, "fixture staged"):
                    sp.run(ctx)
            staged = artifact / "data/solaris-client/servers/content/26.3"
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

    def test_owned_server_is_matched_by_binary_not_by_file_name(self):
        # The ledger tooling hard-links the server it built as `solaris-v2-<sha256>`.
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            server = root / "solaris-v2-0123abcd"
            other = root / "unrelated-binary"
            shutil.copy2("/usr/bin/sleep", server)
            shutil.copy2("/usr/bin/sleep", other)
            owner = SimpleNamespace(pid=os.getpid())
            child = subprocess.Popen([str(server), "30"])
            try:
                self.assertEqual(sp._server_child(owner, server.resolve()), child.pid)
                with self.assertRaisesRegex(RuntimeError, "expected one owned singleplayer server"):
                    sp._server_child(owner, other.resolve())
            finally:
                child.kill()
                child.wait()


if __name__ == "__main__":
    unittest.main()
