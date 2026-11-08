"""Unit contracts for public profile receipts; no server launch."""
import argparse
import unittest
import json
from pathlib import Path
import tempfile
from unittest.mock import patch

from tools.coreperf import profiles


class ProfilesTests(unittest.TestCase):
    def test_shared_heavy_fixture_exact_counts_and_loaded_regions(self):
        recipe = profiles.fixture(100000, 20000)
        self.assertLessEqual(len(recipe["regions"]), 100)
        commands = [c for r in recipe["regions"] for c in r["commands"]]
        for kind, count in recipe["expected"].items():
            self.assertEqual(sum(c.startswith(f"summon {kind} ") for c in commands), count)
        self.assertEqual(recipe["expected"]["minecraft:villager"], 20000)
        self.assertEqual(sum("minecraft:composter" in c for c in commands), 20000)
        self.assertFalse(any("NoAI" in c or "Invulnerable" in c for c in commands))
        self.assertIn("gamerule randomTickSpeed 3", commands)
        self.assertIn("gamerule doDaylightCycle true", commands)

    def test_native_fixture_has_eight_equal_mob_kinds_and_no_villagers(self):
        recipe = profiles.fixture(100000, 0)
        self.assertEqual(recipe["expected"]["minecraft:villager"], 0)
        self.assertEqual([n for k, n in recipe["expected"].items() if k != "minecraft:villager"], [12500] * 8)

    def test_idle_and_explore_do_not_summon_entities(self):
        recipe = profiles.fixture(0, 0)
        self.assertTrue(recipe["regions"])
        self.assertFalse(any(c.startswith("summon ") for r in recipe["regions"] for c in r["commands"]))
        for name, movement, edits in (("idle", 0, 0), ("explore-3", 20, 0),
                                      ("explore-20", 20, 0), ("native-100", 20, 2)):
            args = argparse.Namespace(profile=name, mobs=profiles.PROFILES[name][1], warmup_seconds=30, measure_seconds=120)
            plan = profiles.schedule(args)["windows"]
            self.assertEqual([w["duration_ms"] for w in plan], [30000, 120000])
            self.assertEqual(plan[-1]["movement_hz"], movement)
            self.assertEqual(plan[-1]["edit_hz"], edits)

    def test_half_open_window_tps_and_server_only_memory(self):
        samples = [{"wall_start_unix_ns": at, "wall_us": wall} for at, wall in
                   ((999, 999999), (1000, 1000), (2000, 2000), (3000, 3000), (4000, 999999))]
        processes = [{"captured_unix_ns": at, "processes": {
            "server": {"rss_bytes": rss, "peak_rss_bytes": 100},
            "bots": {"rss_bytes": 1000, "peak_rss_bytes": 2000}},
            "stats": {"entities": {"total": 3}, "chunks": {"loaded_total": 4}}}
            for at, rss in ((999, 999999), (1000, 10), (2000, 20), (3000, 30), (4000, 999999))]
        result = profiles.summarize(samples, processes, 1000, 4000)
        self.assertEqual(result["ticks"], 3)
        self.assertEqual(result["tps"], 1000000)
        self.assertEqual(result["mspt"], {"p50": 2, "p95": 3, "p99": 3, "max": 3})
        self.assertEqual(result["rss"]["server"]["steady_median_bytes"], 20)
        self.assertEqual(result["rss"]["server"]["peak_lifetime_bytes"], 100)
        self.assertIsNone(result["chunk_generation_per_second"])
        with self.assertRaises(ValueError):
            profiles.summarize([], processes, 1000, 4000)

    def test_missing_reset_overrun_and_noncontiguous_ticks_fail(self):
        initial = {"schema_version": 1, "instance_started_unix_ns": 1, "source_skipped_ticks_total": 0}
        metric = dict(initial, samples=[{"tick": 11, "wall_start_unix_ns": 100, "wall_us": 20}])
        self.assertEqual(profiles.validate_metrics(metric, initial, 10), 11)
        for changes in ({"overrun": True}, {"instance_started_unix_ns": 2},
                        {"source_skipped_ticks_total": 1}, {"schema_version": 2},
                        {"samples": [{"tick": 12, "wall_start_unix_ns": 100, "wall_us": 20}]},
                        {"samples": [{"tick": 11, "wall_start_unix_ns": 100, "wall_us": -1}]}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                profiles.validate_metrics(dict(metric, **changes), initial, 10)

    def test_prepared_recipe_and_population_and_saved_identities_are_required(self):
        recipe = {"expected": {"minecraft:cow": 1}, "variant": "example"}
        cohort = {"counts": {"minecraft:cow": 1},
                  "identities": {"uuid": {"kind": "minecraft:cow"}}}
        prepared = {"recipe_sha256": profiles.load.digest(recipe), "cohort": cohort}
        profiles.validate_prepared(recipe, prepared)
        for bad in (
            dict(prepared, recipe_sha256="wrong"),
            dict(prepared, cohort={"counts": cohort["counts"], "identities": {}}),
            dict(prepared, cohort={"counts": {"minecraft:cow": 2}, "identities": cohort["identities"]}),
            dict(prepared, cohort={"counts": cohort["counts"]}),
        ):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                profiles.validate_prepared(recipe, bad)
        with self.assertRaises(ValueError):
            profiles.validate_cohort(cohort, {"counts": cohort["counts"],
                                     "identities": {"other": {"kind": "minecraft:cow"}}},
                                     recipe["expected"])

    def test_pin_manifest_must_belong_to_supplied_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pin = root / "pin"
            pin.mkdir()
            binary = pin / "solaris-v2"
            binary.write_bytes(b"server")
            manifest = {"repo": "core", "kind": "server", "binary": str(binary),
                        "head": "abc", "diff_sha256": "def", "command": "cargo build",
                        "target": "test-target", "binary_sha256": profiles.load.scale.binary_sha256(binary)}
            (pin / "manifest.json").write_text(json.dumps(manifest))
            with patch.object(profiles, "PIN_ROOT", root):
                self.assertEqual(profiles.pinned_binary(binary, "server"), manifest)
                with self.assertRaises(ValueError):
                    profiles.pinned_binary(binary, "bots")
                binary.write_bytes(b"other")
                with self.assertRaises(ValueError):
                    profiles.pinned_binary(binary, "server")
            with self.assertRaises(ValueError):
                profiles.pinned_binary(binary, "server")

    def test_missing_source_samples_fail(self):
        initial = {"schema_version": 1, "instance_started_unix_ns": 1,
                   "source_skipped_ticks_total": 0}
        with self.assertRaisesRegex(ValueError, "samples missing"):
            profiles.validate_metrics(initial, initial, 0)


class InitialMetricsTests(unittest.IsolatedAsyncioTestCase):
    async def test_waits_for_first_pushed_tick_instead_of_sampling_an_empty_ring(self):
        class Session:
            async def http(self, url, timeout):
                self.url, self.timeout = url, timeout
                return {"schema_version": 1, "latest_tick": 75 if "wait=true" in url else None}

        session = Session()
        initial = await profiles.initial_metrics(session, "http://localhost")
        self.assertEqual(initial["latest_tick"], 75)
        self.assertEqual(session.url, "http://localhost/load-metrics?wait=true&min_samples=1")
        self.assertEqual(session.timeout, 12)

    async def test_missing_or_invalid_telemetry_still_fails_closed(self):
        class Session:
            async def http(self, _url, _timeout):
                return {"schema_version": 2, "latest_tick": None}

        with self.assertRaisesRegex(ValueError, "exact source tick telemetry unavailable"):
            await profiles.initial_metrics(Session(), "http://localhost")


if __name__ == "__main__":
    unittest.main()
