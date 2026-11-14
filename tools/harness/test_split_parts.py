"""Split/merge contract for long real-client gates.

A split gate runs N independent parts, each its own `tools.harness run` child
(own server, client and scene slot), and the combined verdict fails when any
part fails or is missing. These are adapter-contract tests with a fake child;
they are not gate coverage.
"""
import io
import json
import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from . import profiles


class FakeChild:
    """Stands in for subprocess.Popen: writes one child receipt, then exits."""

    def __init__(self, receipt, exit_code=0):
        self.receipt = receipt
        self.exit_code = exit_code
        self.waits = []

    def wait(self, timeout=None):
        self.waits.append(timeout)
        return self.exit_code


def fake_spawn(by_label, calls, *, duplicate=False, receipt_writer=None):
    """`_spawn_split` seam: one receipt per part root, as the real child writes it."""
    def spawn(argv, env, log_path):
        label = env[profiles.SPLIT_PART_ENV]
        calls.append({"argv": list(argv), "env": dict(env), "log": str(log_path)})
        root = Path(env["KRASNOKAMEN_VALIDATION_ROOT"])
        artifact = root / ("20260101T000000-" + label)
        artifact.mkdir(parents=True, exist_ok=True)
        receipt = by_label.get(label)
        if receipt is not None:
            (artifact / "result.json").write_text(json.dumps(receipt))
            if duplicate:
                (artifact / "result.json").write_text(json.dumps(receipt))
                other = root / ("20260101T000001-" + label)
                other.mkdir(parents=True, exist_ok=True)
                (other / "result.json").write_text(json.dumps(receipt))
        log_path.write_bytes(b"child log\n")
        return FakeChild(receipt, 0 if receipt is None else receipt.get("__exit__", 0))
    return spawn


def context(directory, parts=2, **overrides):
    fields = dict(artifact_dir=Path(directory), extra_args=[], release=False,
                  timeout_seconds=600.0, platform=None, client="native",
                  windowed=True, plugins_dir=None, server_bin=None, parts=parts)
    fields.update(overrides)
    return SimpleNamespace(**fields)


def passed(details=None):
    return {"status": "passed", "error": None, "details": details or {}}


class SplitPartsTests(unittest.TestCase):
    def run_split(self, receipts, parts, **ctx):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        calls = []
        with patch.object(profiles, "_spawn_split",
                          fake_spawn(receipts, calls, duplicate=ctx.pop("duplicate", False))):
            blocks = profiles.run_split(context(directory.name, **ctx), "buildings-qa",
                                        [profiles.SplitPart(label, env)
                                         for label, env in parts])
        return blocks, calls, Path(directory.name)

    def test_every_part_must_pass_before_the_split_gate_passes(self):
        blocks, calls, root = self.run_split(
            {"a": passed({"n": 1}), "b": passed({"n": 2})},
            [("a", {"BUILDINGS_QA_SELECT": "house"}), ("b", {"BUILDINGS_QA_SELECT": "storage"})])
        self.assertEqual([b["label"] for b in blocks], ["a", "b"])
        self.assertEqual([b["details"]["n"] for b in blocks], [1, 2])
        self.assertEqual([b["status"] for b in blocks], ["passed", "passed"])
        # Each part gets its own artifact root and the part label, so it cannot
        # inherit the parent's profile selection and split again.
        self.assertEqual(calls[0]["env"][profiles.SPLIT_PART_ENV], "a")
        self.assertEqual(calls[1]["env"][profiles.SPLIT_PART_ENV], "b")
        self.assertNotEqual(calls[0]["env"]["KRASNOKAMEN_VALIDATION_ROOT"],
                            calls[1]["env"]["KRASNOKAMEN_VALIDATION_ROOT"])
        self.assertEqual(calls[0]["env"]["BUILDINGS_QA_SELECT"], "house")
        self.assertEqual(calls[0]["argv"][:5],
                         [os.sys.executable, "-m", "tools.harness", "run", "buildings-qa"])
        self.assertTrue((root / "parts").is_dir())

    def test_a_failed_part_fails_the_combined_gate_and_names_it(self):
        receipts = {"a": passed(), "b": {"status": "failed", "error": "house L2 blockmatch",
                                          "details": {}}}
        with self.assertRaisesRegex(RuntimeError, r"split part\(s\) failed: b: failed: house L2 blockmatch"):
            self.run_split(receipts, [("a", {}), ("b", {})])

    def test_a_missing_part_receipt_fails_the_combined_gate(self):
        with self.assertRaisesRegex(RuntimeError, r"split part\(s\) failed: b: no result\.json"):
            self.run_split({"a": passed()}, [("a", {}), ("b", {})])

    def test_a_duplicate_part_receipt_fails_the_combined_gate(self):
        with self.assertRaisesRegex(RuntimeError, r"b: 2 result\.json"):
            self.run_split({"a": passed(), "b": passed()}, [("a", {}), ("b", {})],
                           duplicate=True)

    def test_a_nonzero_child_exit_fails_the_combined_gate(self):
        receipts = {"a": passed(), "b": {**passed(), "__exit__": 3}}
        with self.assertRaisesRegex(RuntimeError, r"b: exit 3"):
            self.run_split(receipts, [("a", {}), ("b", {})])

    def test_children_start_before_any_is_awaited(self):
        # Concurrency is the point of a split: all children are spawned first,
        # then awaited, never spawned one after another.
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        order = []

        def spawn(argv, env, log_path):
            order.append(("spawn", env[profiles.SPLIT_PART_ENV]))
            root = Path(env["KRASNOKAMEN_VALIDATION_ROOT"])
            artifact = root / "run"
            artifact.mkdir(parents=True, exist_ok=True)
            (artifact / "result.json").write_text(json.dumps(passed()))

            class Child(FakeChild):
                def wait(self, timeout=None):
                    order.append(("wait", env[profiles.SPLIT_PART_ENV]))
                    return super().wait(timeout)
            log_path.write_bytes(b"")
            return Child(passed())

        with patch.object(profiles, "_spawn_split", spawn):
            profiles.run_split(context(directory.name), "buildings-qa",
                               [profiles.SplitPart(label) for label in ("a", "b", "c")])
        self.assertEqual(order, [("spawn", "a"), ("spawn", "b"), ("spawn", "c"),
                                 ("wait", "a"), ("wait", "b"), ("wait", "c")])

    def test_child_argv_carries_the_parent_profile_options(self):
        ctx = context(tempfile.gettempdir(), client="native", windowed=True,
                      timeout_seconds=900.0, plugins_dir=Path("/tmp/plugins"),
                      extra_args=[], release=True)
        self.assertEqual(profiles.profile_argv(ctx),
                         ["--release", "--client", "native", "--windowed",
                          "--timeout-seconds", "900.0", "--plugins-dir", "/tmp/plugins"])

    def test_child_omits_options_the_parent_did_not_set(self):
        ctx = context(tempfile.gettempdir(), client="java", windowed=False,
                      timeout_seconds=None, plugins_dir=None, server_bin=Path("/tmp/server"))
        self.assertEqual(profiles.profile_argv(ctx),
                         ["--client", "java", "--server-bin", "/tmp/server"])

    def test_a_part_child_never_splits_again(self):
        with patch.dict(os.environ, {profiles.SPLIT_PART_ENV: "b"}):
            self.assertEqual(profiles.split_part_label(), "b")
        with patch.dict(os.environ, {}, clear=True):
            self.assertIsNone(profiles.split_part_label())

    def test_the_cli_refuses_parts_for_a_profile_without_independent_parts(self):
        from tools.harness import __main__ as cli
        with patch("sys.stderr", new_callable=io.StringIO) as error, \
                patch("sys.stdout", new_callable=io.StringIO):
            self.assertNotEqual(cli.cmd_run(["scene-shot", "--parts", "2"]), 0)
        message = error.getvalue()
        self.assertIn("has no independent parts", message)
        for profile in ("buildings-qa", "civilization-seed-qa", "screens-voice-qa"):
            self.assertIn(profile, message)

    def test_the_cli_refuses_zero_parts(self):
        from tools.harness import __main__ as cli
        with patch("sys.stderr", new_callable=io.StringIO) as error, \
                patch("sys.stdout", new_callable=io.StringIO):
            self.assertNotEqual(cli.cmd_run(["buildings-qa", "--parts", "0"]), 0)
        self.assertIn("--parts must be at least 1", error.getvalue())


if __name__ == "__main__":
    unittest.main()
