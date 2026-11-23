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

    def spawn_recording(self, events, receipts, waits=None):
        def spawn(_argv, env, log_path):
            label = env[profiles.SPLIT_PART_ENV]
            events.append(f"spawn:{label}")
            root = Path(env["KRASNOKAMEN_VALIDATION_ROOT"])
            artifact = root / ("20260101T000000-" + label)
            artifact.mkdir(parents=True, exist_ok=True)
            (artifact / "result.json").write_text(json.dumps(receipts[label]))
            log_path.write_bytes(b"child log\n")
            def wait(timeout=None):
                events.append(f"wait:{label}")
                if waits is not None:
                    waits.append((label, timeout))
                return 0
            return SimpleNamespace(wait=wait)
        return spawn

    def test_parts_run_one_at_a_time_so_two_full_legs_never_share_a_scope(self):
        """Negative control for HARNESS-SCREENS-VOICE-PARTS-OOM: overlap means OOM, not a verdict."""
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        events: list[str] = []
        receipts = {"a": passed(), "b": passed()}
        with patch.object(profiles, "_spawn_split", self.spawn_recording(events, receipts)):
            blocks = profiles.run_split(context(directory.name), "screens-voice-qa",
                                        [profiles.SplitPart("a"), profiles.SplitPart("b")],
                                        sequential=True)
        # Concurrent code would start both legs first: spawn:a, spawn:b, wait:a, wait:b.
        self.assertEqual(events, ["spawn:a", "wait:a", "spawn:b", "wait:b"])
        self.assertEqual([block["status"] for block in blocks], ["passed", "passed"])

    def test_other_split_gates_keep_the_concurrent_start(self):
        """buildings-qa and civilization-seed-qa parts fit together; only screens is sequential."""
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        events: list[str] = []
        receipts = {"a": passed(), "b": passed()}
        with patch.object(profiles, "_spawn_split", self.spawn_recording(events, receipts)):
            profiles.run_split(context(directory.name), "buildings-qa",
                               [profiles.SplitPart("a"), profiles.SplitPart("b")])
        self.assertEqual(events, ["spawn:a", "spawn:b", "wait:a", "wait:b"])

    def test_each_sequential_leg_gets_its_own_full_timeout(self):
        """A slow earlier leg must not shorten the next one's budget (shared deadline = false timeout)."""
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        events: list[str] = []
        waits: list[tuple[str, float | None]] = []
        receipts = {"a": passed(), "b": passed()}
        # wall clock: part a starts at 0 and ends 1500 s later; part b then starts at 1500.
        ticks = iter([0.0, 0.0, 1500.0, 1500.0])
        with patch.object(profiles, "_spawn_split", self.spawn_recording(events, receipts, waits)), \
                patch.object(profiles.time, "monotonic", side_effect=lambda: next(ticks, 3000.0)):
            profiles.run_split(context(directory.name, timeout_seconds=1500.0),
                               "screens-voice-qa",
                               [profiles.SplitPart("a"), profiles.SplitPart("b")], sequential=True)
        self.assertEqual([label for label, _ in waits], ["a", "b"])
        self.assertEqual([round(timeout) for _, timeout in waits], [1500, 1500])

    def test_the_cli_refuses_parts_for_a_profile_without_independent_parts(self):
        from tools.harness import __main__ as cli
        with patch("sys.stderr", new_callable=io.StringIO) as error, \
                patch("sys.stdout", new_callable=io.StringIO):
            self.assertNotEqual(cli.cmd_run(["scene-shot", "--parts", "2"]), 0)
        message = error.getvalue()
        self.assertIn("has no independent parts", message)
        for profile in ("buildings-qa", "civilization-seed-qa", "screens-voice-qa"):
            self.assertIn(profile, message)


if __name__ == "__main__":
    unittest.main()
