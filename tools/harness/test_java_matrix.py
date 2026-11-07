"""Java profile uses shared executable domain adapters, never admission as oracle."""
from contextlib import contextmanager
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

from .backends import java_matrix as matrix
from .matrix_api import MissingCapability


def row(subject="commands.list", **context):
    ctx = dict(client_kind="vanilla_java_26_1_2", game_mode="survival",
               dimension="overworld", difficulty="normal", player_count="1",
               network_condition="normal", server_lifecycle="steady", permission="non_op_0")
    ctx.update(context)
    return dict(id="bl:test/cell", subject=subject, context=ctx,
                steps=["exact action"], oracle=["exact oracle"], driver=dict(kind="java_mcp"))


class MatrixTest(unittest.TestCase):
    def test_unknown_subject_is_blocked_with_exact_shared_missing_capability(self):
        with patch.object(matrix, "load_adapter", side_effect=MissingCapability("domain_adapter:effects.Speed")):
            self.assertEqual(matrix.blockers(row("effects.Speed")), ["domain_adapter:effects.Speed"])

    def test_preflight_does_not_downgrade_context(self):
        cell = row(game_mode="adventure", dimension="the_end", permission="op_3")
        with patch.object(matrix, "load_adapter") as load:
            self.assertEqual(matrix.blockers(cell), [])
        load.assert_called_once_with("commands.list")
        self.assertEqual(cell["context"]["dimension"], "the_end")

    def test_ledger_preserves_driver_binary_and_stable_id(self):
        with tempfile.TemporaryDirectory() as directory:
            ledger = Path(directory) / "ledger.jsonl"
            matrix.append_ledger(ledger, row(), "blocked", "a" * 64, Path(directory), "missing")
            entry = json.loads(ledger.read_text())
            self.assertEqual(set(entry), {"id", "status", "driver", "server_sha", "run_dir", "ts", "note"})
            self.assertEqual(entry["driver"], "java_mcp")
            self.assertEqual(entry["server_sha"], "a" * 64)
            self.assertEqual(entry["id"], "bl:test/cell")
            self.assertTrue(entry["ts"].endswith("Z"))

    @contextmanager
    def fake_server(self, *args, **kwargs):
        yield SimpleNamespace(port=26200, command=Mock(), enabled_plugins=[], plugins_dir=Path(args[1]))

    @contextmanager
    def fake_group(self, *args, **kwargs):
        yield [Mock()]

    def execute(self, side_effect=None, checks=None):
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(matrix, "server_instance", self.fake_server), patch.object(matrix, "group", self.fake_group), patch.object(matrix, "load_adapter", return_value=SimpleNamespace(exclude_plugins=())), patch.object(matrix, "plugin_set_fingerprint", return_value="a" * 64):
                with patch.object(matrix, "run_subject", side_effect=side_effect, return_value=checks) as run:
                    result = matrix.run_cell(SimpleNamespace(timeout_seconds=30, plugin_profiles={}), row(), Path(directory))
                    return result, run

    def test_shared_domain_checks_are_the_only_pass_oracle(self):
        result, run = self.execute(checks=[{"name": "exact", "passed": True, "detail": "actual state"}])
        self.assertEqual(result["status"], "pass")
        ctx = run.call_args.args[0]
        self.assertEqual(ctx.row["subject"], "commands.list")
        self.assertIsInstance(ctx.clients[0], matrix.JavaMatrixClient)

    def test_failed_oracle_is_fail(self):
        result, _ = self.execute(checks=[{"name": "exact", "passed": False}])
        self.assertEqual(result["status"], "fail")

    def test_missing_verb_is_blocked_not_passed(self):
        result, _ = self.execute(side_effect=MissingCapability("client:select_trade"))
        self.assertEqual(result["status"], "blocked")
        self.assertIn("client:select_trade", result["note"])

    def test_stable_id_paths_are_bounded_and_contained(self):
        with tempfile.TemporaryDirectory() as directory:
            path = matrix.receipt_dir(Path(directory), "tr.some_origin:" + "x/" * 500)
            self.assertEqual(path.parent, Path(directory))
            self.assertLess(len(path.name), 255)


if __name__ == "__main__":
    unittest.main()
