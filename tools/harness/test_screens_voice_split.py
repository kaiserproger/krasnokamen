"""Split contract for screens-voice-qa: one part per leg, one private audio graph each.

A part re-enters `screen_audio_graph.sh` itself, so two parts never share a
PipeWire graph or its null sink; a split started inside a graph is refused
instead. Adapter-contract tests with a fake child, not gate coverage.
"""
import os
import unittest
from types import SimpleNamespace
from unittest import mock

from tools.harness import profiles
from tools.harness.backends import screens_voice_qa as qa


def context(parts=2, **overrides):
    fields = dict(artifact_dir=None, client="native", windowed=True, extra_args=[],
                  platform=None, timeout_seconds=600.0, parts=parts, client_evidence={},
                  commands=[], cwds=[], exits=[], logs=[], plugins_dir=None,
                  release=False, server_bin=None)
    fields.update(overrides)
    return SimpleNamespace(**fields)


def block(label, verdicts, status="passed"):
    return {"label": label, "status": status, "artifact": f"/a/{label}",
            "log": f"/a/{label}.log", "details": {"verdicts": verdicts}}


class SplitRunTests(unittest.TestCase):
    def test_a_split_inside_a_private_graph_is_refused_for_its_shared_sink(self):
        with mock.patch.dict(os.environ, {"KRASNOKAMEN_PW_ROOT": "/tmp/graph"}):
            with self.assertRaisesRegex(ValueError, "own graph and sink"):
                qa._split_run(context(), 600.0)

    def test_each_part_carries_its_own_leg(self):
        with mock.patch.dict(os.environ, {"KRASNOKAMEN_PW_ROOT": ""}), \
                mock.patch.dict(os.environ, {}, clear=False), \
                mock.patch.object(qa, "run_split", return_value=[
                    block("screens", {"share_allow_streams": True}),
                    block("voice", {"proximity_heard": True})]) as split:
            os.environ.pop(qa.LEG_ENV, None)
            os.environ.pop("KRASNOKAMEN_PW_ROOT", None)
            result = qa._split_run(context(), 600.0)
        parts = split.call_args.args[2]
        self.assertEqual(split.call_args.args[1], "screens-voice-qa")
        self.assertEqual([(part.label, part.env[qa.LEG_ENV]) for part in parts],
                         [("screens", "screens"), ("voice", "voice")])
        self.assertEqual(result["verdicts"],
                         {"share_allow_streams": True, "proximity_heard": True, "all": True})
        self.assertEqual([part["leg"] for part in result["parts"]], ["screens", "voice"])

    def test_run_splits_before_entering_any_graph(self):
        with mock.patch.dict(os.environ, {}, clear=False), \
                mock.patch.object(qa, "validate_args", return_value=600.0), \
                mock.patch.object(qa, "_split_run", return_value={"verdicts": {"all": True}}) as split, \
                mock.patch.object(qa, "_run_in_private_graph") as graph:
            os.environ.pop("KRASNOKAMEN_PW_ROOT", None)
            os.environ.pop(qa.LEG_ENV, None)
            self.assertEqual(qa.run(context()), {"verdicts": {"all": True}})
        split.assert_called_once()
        graph.assert_not_called()

    def test_a_non_split_run_still_enters_the_private_graph(self):
        with mock.patch.dict(os.environ, {}, clear=False), \
                mock.patch.object(qa, "validate_args", return_value=600.0), \
                mock.patch.object(qa, "_split_run") as split, \
                mock.patch.object(qa, "_run_in_private_graph", return_value={"verdicts": {}}) as graph:
            os.environ.pop("KRASNOKAMEN_PW_ROOT", None)
            self.assertEqual(qa.run(context(parts=1)), {"verdicts": {}})
        split.assert_not_called()
        graph.assert_called_once()


    def test_each_part_starts_outside_any_graph_so_it_builds_its_own(self):
        # The child env carries no KRASNOKAMEN_PW_ROOT: the part re-enters
        # screen_audio_graph.sh itself, so two legs never share a sink.
        import tempfile
        from pathlib import Path
        children = []

        def spawn(argv, env, log_path):
            children.append(dict(env))
            root = Path(env["KRASNOKAMEN_VALIDATION_ROOT"])
            artifact = root / "run"
            artifact.mkdir(parents=True, exist_ok=True)
            (artifact / "result.json").write_text(
                '{"status": "passed", "details": {"verdicts": {"'
                + env[qa.LEG_ENV] + '_ok": true}}}')
            log_path.write_bytes(b"")
            return FakeChild()

        class FakeChild:
            def wait(self, timeout=None):
                return 0

        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        ctx = context(artifact_dir=Path(directory.name))
        with mock.patch.dict(os.environ, {}, clear=False), \
                mock.patch.object(profiles, "_spawn_split", spawn):
            os.environ.pop("KRASNOKAMEN_PW_ROOT", None)
            os.environ.pop(qa.LEG_ENV, None)
            qa._split_run(ctx, 600.0)
        self.assertEqual(len(children), 2)
        for env in children:
            self.assertNotIn("KRASNOKAMEN_PW_ROOT", env)
        self.assertEqual([env[qa.LEG_ENV] for env in children], ["screens", "voice"])


class MergeTests(unittest.TestCase):
    def test_a_part_that_published_no_verdicts_fails_the_merge(self):
        with self.assertRaisesRegex(RuntimeError, "screens part published no verdicts"):
            qa.merge_leg_parts([block("screens", {}), block("voice", {"proximity_heard": True})],
                               qa.LEGS)

    def test_one_verdict_reported_by_two_legs_fails_the_merge(self):
        with self.assertRaisesRegex(RuntimeError, "verdict shared reported by parts screens and voice"):
            qa.merge_leg_parts([block("screens", {"shared": True}),
                                block("voice", {"shared": True})], qa.LEGS)

    def test_a_false_verdict_fails_the_merge_even_when_the_child_passed(self):
        with self.assertRaisesRegex(RuntimeError, "verdicts failed: \\['proximity_heard'\\]"):
            qa.merge_leg_parts([block("screens", {"share_allow_streams": True}),
                                block("voice", {"proximity_heard": False})], qa.LEGS)


if __name__ == "__main__":
    unittest.main()
