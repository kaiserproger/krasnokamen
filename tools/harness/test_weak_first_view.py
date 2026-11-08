"""The first-view benchmark cannot silently escape its requested envelope."""
import json
import os
import tempfile
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from tools.harness import __main__ as cli, preflight, profiles
from tools.harness.profiles import ProfileContext
from tools.harness.backends import weak_first_view


class WeakFirstViewTests(unittest.TestCase):
    def context(self, **overrides):
        values = dict(artifact_dir=Path('/unused'), client='native', windowed=True,
                      extra_args=['--server-cpus', '0', '--client-cpus', '1-3', '--memory', '2G'])
        values.update(overrides)
        return ProfileContext(**values)

    def test_cli_dispatches_windowed_native_first_view(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = Mock(return_value={"first_view_seconds": 1.0})
            with patch.object(cli, "ARTIFACT_ROOT", Path(tmp)), \
                    patch.object(preflight, "check", return_value=set()), \
                    patch.dict(profiles.PROFILES["weak-first-view"], {"run": run}):
                code = cli.cmd_run(["weak-first-view", "--client", "native", "--windowed"])
            receipt = json.loads(next(Path(tmp).glob("*/result.json")).read_text())
        self.assertEqual(code, 0)
        self.assertEqual(receipt["status"], "passed")
        self.assertEqual(receipt["details"]["first_view_seconds"], 1.0)
        self.assertEqual(run.call_args.args[0].client, "native")
        self.assertTrue(run.call_args.args[0].windowed)

    def test_client_failure_keeps_private_data_root_and_server_exit_slot(self):
        with tempfile.TemporaryDirectory() as tmp:
            artifact = Path(tmp) / "run"
            artifact.mkdir()
            source = Path(tmp) / "source"
            (source / "facts").mkdir(parents=True)
            ctx = self.context(artifact_dir=artifact)
            args = SimpleNamespace(server_cpus="0", client_cpus="1-3", memory="2G")
            server = Mock(returncode=0)

            def fail_client(context, *_args, **_kwargs):
                root = artifact / "client-data" / "solaris-client"
                self.assertEqual(os.environ["XDG_DATA_HOME"], str(root.parent))
                self.assertEqual((root / "facts").resolve(), source / "facts")
                context.commands.append(["client"])
                context.exits.append(None)
                context.exits[len(context.commands) - 1] = 7
                raise RuntimeError("client startup failed")

            with patch.dict(os.environ, {"SOLARIS_SERVER_BINARY": "/bin/true"}), \
                    patch.object(weak_first_view, "validate_args", return_value=args), \
                    patch.object(weak_first_view.native_client, "resolve_binary", return_value=Path("/bin/true")), \
                    patch.object(weak_first_view.native_client, "_client_root", return_value=source), \
                    patch.object(weak_first_view.native_client, "start", side_effect=fail_client), \
                    patch.object(weak_first_view.catalog_load, "config", return_value=(artifact / "server.toml", None, 1234)), \
                    patch.object(weak_first_view.common, "scope_prefix", return_value=[]), \
                    patch.object(weak_first_view.os, "sched_setaffinity"), \
                    patch.object(weak_first_view.runtime, "start_server", return_value=server), \
                    patch.object(weak_first_view.runtime, "wait_port"), \
                    patch.object(weak_first_view.runtime, "start_xvfb", return_value=(None, ":99", None)), \
                    patch.object(weak_first_view.runtime, "stop_process"):
                with self.assertRaisesRegex(RuntimeError, "client startup failed"):
                    weak_first_view.run(ctx)
            self.assertEqual(ctx.exits, [0, 7])

    def test_server_and_observer_sets_stay_inside_the_runner_affinity(self):
        with patch.object(weak_first_view.os, 'sched_getaffinity', return_value={0, 1, 2, 3}):
            args = weak_first_view.validate_args(self.context())
            self.assertEqual(args.server_cpus, '0')
            self.assertEqual(args.client_cpus, '1-3')
            self.assertEqual(args.memory, '2G')
            with self.assertRaisesRegex(ValueError, 'outside the measurement envelope'):
                weak_first_view.validate_args(self.context(extra_args=[
                    '--server-cpus', '4', '--client-cpus', '1-3', '--memory', '2G']))

    def test_headless_or_java_does_not_substitute_for_a_usable_view(self):
        for override in ({'windowed': False}, {'client': 'java'}, {'platform': 'fabric'}):
            with self.subTest(override=override), self.assertRaisesRegex(ValueError, 'requires'):
                weak_first_view.validate_args(self.context(**override))

    def test_default_envelope_uses_one_allowed_cpu_and_real_ram_cap(self):
        with patch.object(weak_first_view.os, 'sched_getaffinity', return_value={8, 9, 10, 11}):
            args = weak_first_view.validate_args(self.context(extra_args=[]))
            self.assertEqual(args.server_cpus, '8')
            observer = set(weak_first_view.common.parse_cpu_list(args.client_cpus))
            self.assertTrue(observer and observer <= {9, 10, 11})
            self.assertEqual(args.memory, '2G')

    def test_malformed_envelope_fails_as_a_receiptable_error(self):
        with self.assertRaisesRegex(ValueError, 'invalid envelope arguments'):
            weak_first_view.validate_args(self.context(extra_args=['--server-cpus']))


if __name__ == '__main__':
    unittest.main()
