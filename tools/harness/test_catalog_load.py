"""Catalog configuration and preserved fixture/CLI regression coverage."""
import asyncio
import io
import json
import tempfile
from contextlib import redirect_stdout
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch
import unittest

from .backends import catalog_load as load
from . import __main__ as cli


class CatalogLoadTests(unittest.TestCase):
    def test_canonical_cli_forwards_backend_arguments_with_or_without_separator(self):
        for separator in ([], ["--"]):
            with self.subTest(separator=separator), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                out = root / "classified"
                with patch.object(cli, "ARTIFACT_ROOT", root / "canonical"), redirect_stdout(io.StringIO()):
                    result = cli.cmd_run(["catalog-load", *separator,
                                          "--smoke", "--scenario", "plugin-heavy", "--out", str(out)])
                self.assertEqual(result, 0)
                receipt = json.loads((out / "plugin-heavy/result.json").read_text())
                self.assertEqual(receipt["outcome"], "INFRA_BLOCKED")
                canonical = json.loads(next((root / "canonical").glob("*/result.json")).read_text())
                self.assertEqual(canonical["status"], "prepared")


    def test_controls_use_real_recipes_and_mix_capabilities_do_not_alias(self):
        mapping = json.loads(load.MAPPING.read_text())
        entries = {row["id"]: row for row in mapping["scenarios"]}
        for name in load.catalog_controls.CONTROL_IDS:
            recipe = load.catalog_controls.recipe(name, 10, True)
            self.assertEqual(recipe["id"], name)
            self.assertTrue(recipe["preparation_regions"])
            self.assertIsNone(entries[name]["capability"]["outcome"])
        for name in ("MIX-01", "MIX-02", "MIX-04", "MIX-08", "MIX-18", "MIX-21",
                     "mixed-survival-long"):
            with self.subTest(scenario=name):
                row = entries[name]
                self.assertIsNone(row["capability"]["outcome"])
                self.assertTrue(row["work_proof"]["required"])
                self.assertEqual(row["oracle"]["required"], row["invariants"])
                self.assertTrue(row["catalog_closure"])


class CatalogEventTests(unittest.IsolatedAsyncioTestCase):

    async def test_public_region_uses_explicit_supported_save_barrier(self):
        region = {"name": "shared", "view_distance": 4, "anchor": [32, 64, 32],
                  "commands": ["time set 2000"]}
        for save_command in ("save-all", "save-all flush"):
            with self.subTest(save_command=save_command), tempfile.TemporaryDirectory() as directory:
                phase = SimpleNamespace(spawn=AsyncMock(), ready=AsyncMock(return_value={"clients": 1}),
                                        stop=AsyncMock(), shutdown={}, process_samples=lambda: {})
                root = Path(directory) / "region"
                with patch.object(load, "Session", return_value=phase), \
                     patch.object(load, "bot_command", return_value=["bots"]) as bot_command, \
                     patch.object(load, "_await_fixture_checkpoint", new=AsyncMock(return_value={})):
                    await load.public_region(SimpleNamespace(readiness_seconds=1, view_distance=8),
                                             root, 25565, region, save_command=save_command)
                self.assertEqual(bot_command.call_args.args[0].view_distance, 8)
                command = phase.spawn.call_args.args[1]
                self.assertIn("--open-loop", command)
                schedule = Path(command[command.index("--schedule-file") + 1])
                self.assertEqual(schedule, root / "schedule.json")
                self.assertEqual(json.loads(schedule.read_text()),
                                 {"windows": [{"name": "steady", "duration_ms": 1000}]})
                commands = json.loads((root / "commands.json").read_text())
                self.assertEqual(commands["setup"], ["time set 2000", save_command])
                self.assertEqual(set(commands), {"setup"})
                phase.stop.assert_awaited_once()

    async def test_frame_setup_saves_after_observed_interactions(self):
        region = {"name": "frames", "anchor": [32, 32], "commands": ["summon item_frame 0 65 0"],
                  "frame_positions": [[0, 65, 0]]}
        for observed in (1, 0):
            with self.subTest(observed=observed), tempfile.TemporaryDirectory() as directory:
                phase = SimpleNamespace(spawn=AsyncMock(), ready=AsyncMock(return_value={"clients": 1}),
                                        stop=AsyncMock(), shutdown={}, process_samples=lambda: {},
                                        fixture_map_frames={"count": observed})
                root = Path(directory) / "region"
                with patch.object(load, "Session", return_value=phase), \
                     patch.object(load, "bot_command", return_value=["bots"]), \
                     patch.object(load, "_await_fixture_checkpoint", new=AsyncMock(return_value={})):
                    if observed:
                        receipt = await load.public_region(SimpleNamespace(readiness_seconds=1, view_distance=8),
                                                           root, 25565, region, save_command="save-all")
                        self.assertEqual(receipt["map_frames_observed"]["count"], 1)
                    else:
                        with self.assertRaisesRegex(load.ClassifiedFailure, "metadata evidence incomplete"):
                            await load.public_region(SimpleNamespace(readiness_seconds=1, view_distance=8),
                                                     root, 25565, region, save_command="save-all")
                commands = json.loads((root / "commands.json").read_text())
                self.assertEqual(commands["setup"], region["commands"])
                self.assertEqual(commands["save_after_frames"], "save-all")
                self.assertEqual(commands["frame_positions"], region["frame_positions"])


    async def test_checkpoint_ack_survives_consumed_ready_and_queue_eviction(self):
        reader = asyncio.StreamReader(limit=64 * 1024 * 1024)
        events = [{"event": "fixture_commands_sent", "commands_sent": 1},
                  {"event": "fixture_checkpoint_saved", "bot": 0}, {"event": "ready", "clients": 1}]
        reader.feed_data(b"".join(json.dumps(row).encode() + b"\n" for row in events))
        reader.feed_eof()
        session = load.Session(None, Path("."))
        await session.read_events(SimpleNamespace(stdout=reader), io.BytesIO())
        self.assertEqual((await session.ready(1, 1))["clients"], 1)
        for index in range(200):
            session.push_event({"event": "sample", "sample": index})
        receipt = await load._await_fixture_checkpoint(session, 1)
        self.assertEqual(receipt["commands_sent_event"]["commands_sent"], 1)
        self.assertEqual(receipt["checkpoint_event"]["event"], "fixture_checkpoint_saved")
        self.assertGreater(receipt["checkpoint_event"]["captured_unix_ns"], 0)


if __name__ == "__main__":
    unittest.main()
