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

    def test_official_recipe_bytes_remain_unchanged(self):
        self.assertEqual(profiles.load.digest(profiles.fixture(100000, 0)),
                         "92db3da297768ff28ba35e9e5d6da9513b8bed3dd2b3d397fe117bebe2984368")
        self.assertEqual(profiles.load.digest(profiles.fixture(100000, 20000)),
                         "0084b89a8aac94b7194964c15ec2f1559d131b7c73f0ea6545de7ff9072ae374")

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

    def test_remote_host_accepts_relocated_pin_and_checks_manifest_and_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            store = root / "absent-owner-host" / "pins"
            pin = root / "remote-artifacts" / "pin"
            pin.mkdir(parents=True)
            binary = pin / "solaris-v2"
            binary.write_bytes(b"server")
            manifest = {"repo": "core", "kind": "server",
                        "binary": str(store / "original-pin" / binary.name),
                        "head": "abc", "diff_sha256": "def", "command": "cargo build --release",
                        "target": "test-target", "binary_sha256": profiles.load.scale.binary_sha256(binary)}
            receipt = pin / "manifest.json"
            receipt.write_text(json.dumps(manifest))
            with patch.object(profiles, "PIN_ROOT", store):
                self.assertEqual(profiles.pinned_binary(binary, "server"), manifest)
                with self.assertRaises(ValueError):
                    profiles.pinned_binary(binary, "bots")
                for path in (str(store / "original-pin" / "other-binary"), None):
                    with self.subTest(path=path):
                        receipt.write_text(json.dumps(dict(manifest, binary=path)))
                        with self.assertRaises(ValueError):
                            profiles.pinned_binary(binary, "server")
                receipt.write_text(json.dumps(manifest))
                binary.write_bytes(b"other")
                with self.assertRaises(ValueError):
                    profiles.pinned_binary(binary, "server")

    def test_incremental_pin_is_refused_but_pre_field_and_full_pins_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            store = root / "absent-owner-host" / "pins"
            pin = root / "remote-artifacts" / "pin"
            pin.mkdir(parents=True)
            binary = pin / "solaris-v2"
            binary.write_bytes(b"server")
            manifest = {"repo": "core", "kind": "server", "binary": str(binary),
                        "head": "abc", "diff_sha256": "def", "command": "cargo build --release",
                        "target": "test-target", "binary_sha256": profiles.load.scale.binary_sha256(binary)}
            receipt = pin / "manifest.json"
            with patch.object(profiles, "PIN_ROOT", store):
                for recorded, accepted in ((None, True), (False, True), (True, False)):
                    with self.subTest(incremental=recorded):
                        receipt.write_text(json.dumps(manifest if recorded is None else dict(manifest, incremental=recorded)))
                        if accepted:
                            profiles.pinned_binary(binary, "server")
                        else:
                            with self.assertRaisesRegex(ValueError, "incremental"):
                                profiles.pinned_binary(binary, "server")

    def test_capacity_ladder_refuses_incremental_or_mismatched_pins_before_any_rung(self):
        import asyncio
        from tools.coreperf import capacity_ladder
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            store = root / "absent-owner-host" / "pins"
            def pin(kind, name, *, head="abc", **extra):
                folder = root / kind
                folder.mkdir()
                binary = folder / name
                binary.write_bytes(kind.encode())
                binary.chmod(0o755)
                (folder / "manifest.json").write_text(json.dumps(
                    {"repo": "core", "kind": kind, "binary": str(binary), "head": head, "diff_sha256": "def",
                     "command": "cargo build --release", "target": "t",
                     "binary_sha256": profiles.load.scale.binary_sha256(binary), **extra}))
                return binary
            server = pin("server", "solaris-v2")
            bots = pin("bots", "player-scale-bots")
            args = argparse.Namespace(out=root / "out", server_bin=server, bots_bin=bots)
            with patch.object(profiles, "PIN_ROOT", store), patch.object(capacity_ladder, "rung") as rung:
                (server.parent / "manifest.json").write_text(json.dumps(
                    dict(json.loads((server.parent / "manifest.json").read_text()), incremental=True)))
                args.out = root / "out1"
                with self.assertRaisesRegex(SystemExit, "incremental"):
                    asyncio.run(capacity_ladder.main_async(args))
                (server.parent / "manifest.json").write_text(json.dumps(
                    dict(json.loads((server.parent / "manifest.json").read_text()), incremental=False, head="other")))
                args.out = root / "out2"
                with self.assertRaisesRegex(SystemExit, "different source trees"):
                    asyncio.run(capacity_ladder.main_async(args))
                rung.assert_not_called()

    def test_unpinned_binary_without_manifest_passes_the_soft_check(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "solaris-v2"
            binary.write_bytes(b"x")
            profiles.load.scale.refuse_incremental_pin(binary, "server")
            (Path(directory) / "manifest.json").write_text(json.dumps({"incremental": True}))
            with self.assertRaisesRegex(ValueError, "incremental"):
                profiles.load.scale.refuse_incremental_pin(binary, "server")

    def test_owner_host_requires_pin_store_and_exact_manifest_path(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            store = root / "owner-host" / "pins"
            runner = store.parent / "solaris" / "tools" / "dev" / "mem-run.sh"
            runner.parent.mkdir(parents=True)
            runner.touch()
            pin = store / "pin"
            pin.mkdir(parents=True)
            binary = pin / "solaris-v2"
            binary.write_bytes(b"server")
            manifest = {"repo": "core", "kind": "server", "binary": str(binary),
                        "head": "abc", "diff_sha256": "def", "command": "cargo build --release",
                        "target": "test-target", "binary_sha256": profiles.load.scale.binary_sha256(binary)}
            receipt = pin / "manifest.json"
            receipt.write_text(json.dumps(manifest))
            with patch.object(profiles, "PIN_ROOT", store):
                self.assertEqual(profiles.pinned_binary(binary, "server"), manifest)
                receipt.write_text(json.dumps(dict(manifest, binary=str(store / "other-pin" / binary.name))))
                with self.assertRaises(ValueError):
                    profiles.pinned_binary(binary, "server")
                outside = root / "outside-pin" / binary.name
                outside.parent.mkdir()
                outside.write_bytes(b"server")
                outside.with_name("manifest.json").write_text(json.dumps(dict(manifest, binary=str(outside))))
                with self.assertRaisesRegex(ValueError, "not in the pin-release store"):
                    profiles.pinned_binary(outside, "server")

    def test_missing_source_samples_fail(self):
        initial = {"schema_version": 1, "instance_started_unix_ns": 1,
                   "source_skipped_ticks_total": 0}
        with self.assertRaisesRegex(ValueError, "samples missing"):
            profiles.validate_metrics(initial, initial, 0)

    def test_scale_variants_have_exact_small_cohorts_and_disjoint_regions(self):
        self.assertEqual(len(profiles.SCALE_PROFILES), 9)
        for mix in ("all-active", "mostly-resting", "sparse-active"):
            recipe = profiles.scale_fixture(80, mix)
            self.assertEqual(sum(recipe["expected"].values()), 80)
            self.assertEqual(sum(row["resident"] for row in recipe["coverage"]), 80)
            self.assertEqual(sum(row["resident"] for row in recipe["coverage"] if row["active"]),
                             8 if mix == "sparse-active" else 80)
            chunks = [tuple(chunk) for row in recipe["coverage"] for chunk in row["chunks"]]
            self.assertEqual(len(chunks), len(set(chunks)))
            for kind, count in recipe["expected"].items():
                self.assertEqual(sum(c.startswith(f"summon {kind} ")
                                     for r in recipe["regions"] for c in r["commands"]), count)
            self.assertFalse(any("NoAI" in c or "Invulnerable" in c
                                 for r in recipe["regions"] for c in r["commands"]))

    def test_scale_spawn_bodies_clear_pen_walls_at_full_region_capacity(self):
        recipe = profiles.scale_fixture(8192, "all-active")
        for region in recipe["regions"]:
            x, z = (v - 32 for v in region["anchor"])
            summons = [command.split() for command in region["commands"] if command.startswith("summon ")]
            self.assertEqual(len(summons), 1024)
            for command in summons:
                # Spider is the widest cohort member at 1.4 blocks.
                self.assertGreaterEqual(float(command[2]) - x, 0.7)
                self.assertLessEqual(float(command[2]) - x, 64 - 0.7)
                self.assertGreaterEqual(float(command[4]) - z, 0.7)
                self.assertLessEqual(float(command[4]) - z, 64 - 0.7)

    def test_cold_forceload_centres_cover_pen_without_activating_it(self):
        recipe = profiles.scale_fixture(80, "sparse-active")
        for region, coverage in zip(recipe["regions"], recipe["coverage"]):
            if coverage["active"]:
                continue
            tickets = [c.split() for c in region["commands"] if c.startswith("forceload add ")]
            centres = [(int(c[2]) // 16, int(c[3]) // 16) for c in tickets]
            self.assertEqual(len(centres), 2)
            for x, z in coverage["chunks"]:
                self.assertNotIn((x, z), centres)
                self.assertTrue(any(max(abs(x-cx), abs(z-cz)) <= 2 for cx, cz in centres))

    def test_projectile_bank_uses_real_dispenser_owner_and_fixed_control_geometry(self):
        armed = profiles.scale_fixture(80, "all-active")
        control = profiles.scale_fixture(80, "all-active")
        profiles.add_projectile_fixture(armed, 2, smoke=True)
        profiles.add_projectile_fixture(control, 0, smoke=True)
        for recipe in (armed, control):
            commands = recipe["regions"][-1]["commands"]
            self.assertEqual(sum("minecraft:dispenser[" in c for c in commands), 4)
            self.assertEqual(recipe["stress"]["pulse_interval_ms"], 3000)
        self.assertEqual(sum("item replace block" in c for c in armed["regions"][-1]["commands"]), 2)
        self.assertFalse(any("item replace block" in c for c in control["regions"][-1]["commands"]))
        args = argparse.Namespace(profile="scale-100k-all-active", mobs=80, warmup_seconds=30, measure_seconds=120)
        plan = profiles.schedule(args, armed)
        self.assertEqual(plan["windows"][0]["commands"], plan["windows"][1]["commands"])
        rows = [{"region": i, "active_chunks": 16, "motion_admitted": row["resident"],
                 "motion_full": 0, "motion_resting": row["resident"], "projectile_candidates": 0}
                for i, row in enumerate(armed["coverage"])]
        rows[-1]["projectile_candidates"] = 2
        sample = {"tick": 3, "coreperf": {"tick": 3, "resident_entities": 90, "regions": rows}}
        self.assertEqual(profiles.validate_scale_sample(armed, sample)["projectiles"], 2)
        rows[-1]["projectile_candidates"] = True
        with self.assertRaisesRegex(ValueError, "projectile owner"):
            profiles.validate_scale_sample(armed, sample)
        del rows[-1]["projectile_candidates"]
        with self.assertRaisesRegex(ValueError, "projectile owner"):
            profiles.validate_scale_sample(armed, sample)

    def test_xp_bottles_observe_real_orb_owner_not_offered_commands(self):
        recipe = profiles.scale_fixture(80, "all-active")
        profiles.add_xp_fixture(recipe, 2, smoke=True)
        self.assertEqual(recipe["stress"]["kind"], "xp-orbs")
        self.assertEqual(sum("minecraft:experience_bottle 64" in c
                             for c in recipe["regions"][-1]["commands"]), 2)
        rows = [{"region": i, "active_chunks": 16, "motion_admitted": row["resident"],
                 "motion_full": 0, "motion_resting": row["resident"], "projectile_candidates": 0}
                for i, row in enumerate(recipe["coverage"])]
        sample = {"tick": 3, "coreperf": {"tick": 3, "resident_entities": 90,
                  "xp_orbs_processed": 9, "xp_orbs_by_region": {str(recipe["stress"]["regions"][0]): 7},
                  "regions": rows}}
        self.assertEqual(profiles.validate_scale_sample(recipe, sample)["xp_orbs"], 7)
        del sample["coreperf"]["xp_orbs_processed"]
        with self.assertRaisesRegex(ValueError, "XP-orb owner"):
            profiles.validate_scale_sample(recipe, sample)

    def test_zero_xp_control_has_only_an_observer_region(self):
        recipe = profiles.scale_fixture(10, "all-active", active_override=0)
        before = len(recipe["regions"])
        profiles.add_xp_fixture(recipe, 0, smoke=False)
        self.assertEqual(len(recipe["regions"]), before + 1)
        self.assertEqual(recipe["stress"]["dispenser_capacity"], 0)
        self.assertEqual(recipe["stress"]["regions"], [])
        self.assertEqual(recipe["stress"]["commands"], [])
        self.assertFalse(recipe.get("expected_blocks"))
        index = recipe["stress"]["observer_region"]
        row = {"region": index, "active_chunks": 16, "motion_admitted": 0,
               "motion_full": 0, "motion_resting": 0}
        sample = {"tick": 1, "coreperf": {"tick": 1, "resident_entities": 11,
                  "regions": [row], "xp_orbs_processed": 0, "xp_orbs_by_region": {}}}
        observed = profiles.validate_scale_sample(recipe, sample)
        self.assertEqual((observed["xp_orbs"], observed["active"]), (0, 0))
        row["active_chunks"] = 15
        with self.assertRaisesRegex(ValueError, "active chunk coverage"):
            profiles.validate_scale_sample(recipe, sample)
        row["active_chunks"] = 16
        sample["coreperf"]["xp_orbs_processed"] = 1
        with self.assertRaisesRegex(ValueError, "zero XP control"):
            profiles.validate_scale_sample(recipe, sample)
        del sample["coreperf"]["xp_orbs_by_region"]
        with self.assertRaisesRegex(ValueError, "XP-orb owner"):
            profiles.validate_scale_sample(recipe, sample)

    def test_brewing_fixture_uses_chests_and_public_item_placement(self):
        recipe = profiles.scale_fixture(80, "all-active")
        profiles.add_brewing_fixture(recipe, 6, stands=2)
        commands = recipe["regions"][-1]["commands"]
        self.assertEqual(sum("minecraft:chest" in c for c in commands), 6)
        self.assertEqual(sum("minecraft:brewing_stand" in c for c in commands), 2)
        self.assertEqual(sum("minecraft:nether_wart" in c for c in commands), 2)
        self.assertEqual(sum("minecraft:blaze_powder" in c for c in commands), 2)
        self.assertEqual(len(recipe["stress"]["commands"]), 2)
        self.assertTrue(all("minecraft:water" in c for c in recipe["stress"]["commands"]))
        rows = [{"region": i, "active_chunks": 16, "motion_admitted": row["resident"],
                 "motion_full": 0, "motion_resting": row["resident"], "projectile_candidates": 0}
                for i, row in enumerate(recipe["coverage"])]
        region = str(recipe["stress"]["regions"][0])
        sample = {"tick": 3, "coreperf": {"tick": 3, "resident_entities": 90, "regions": rows,
                  "brewing_discovery_by_region": {region: 8},
                  "brewing_active_by_region": {region: 2},
                  "brewing_progressing_by_region": {region: 2}}}
        self.assertEqual(profiles.validate_scale_sample(recipe, sample)["brewing_progressing"], 2)
        sample["coreperf"]["brewing_progressing_by_region"][region] = True
        with self.assertRaisesRegex(ValueError, "invalid brewing owner counters"):
            profiles.validate_scale_sample(recipe, sample)
        sample["coreperf"]["brewing_progressing_by_region"][region] = 2
        sample["coreperf"]["brewing_discovery_by_region"][region] = 7
        with self.assertRaisesRegex(ValueError, "below requested containers"):
            profiles.validate_scale_sample(recipe, sample)

    def test_productive_workers_require_real_distinct_farm_cell_changes(self):
        recipe = profiles.scale_fixture(78, "all-active")
        profiles.add_productive_fixture(recipe, 2, smoke=True)
        self.assertEqual(recipe["population"]["resident_requested"], 80)
        self.assertEqual(recipe["population"]["active_requested"], 80)
        self.assertEqual(recipe["expected"]["minecraft:villager"], 2)
        self.assertEqual(recipe["variant"], "noncanonical-scale-80-all-active")
        self.assertEqual(len(recipe["stress"]["worker_cells"]), 2)
        rows = [{"region": i, "active_chunks": 16,
                 "motion_admitted": 0 if row["kind"] == "minecraft:villager" else row["resident"],
                 "motion_full": 0,
                 "motion_resting": 0 if row["kind"] == "minecraft:villager" else row["resident"],
                 "projectile_candidates": 0} for i, row in enumerate(recipe["coverage"])]
        cell = recipe["stress"]["worker_cells"][0]
        region = str(recipe["stress"]["regions"][0])
        sample = {"tick": 3, "coreperf": {"tick": 3, "resident_entities": 90, "regions": rows,
                  "brain_candidates_by_region": {region: 2}, "brain_admitted_by_region": {region: 1},
                  "productive_harvested_cells": [cell]}}
        self.assertEqual(profiles.validate_scale_sample(recipe, sample)["harvested_cells"], [cell])
        self.assertEqual(profiles.validate_scale_sample(recipe, sample)["admitted"], 79)
        sample["coreperf"]["productive_harvested_cells"] = [[0, 0, 0]]
        with self.assertRaisesRegex(ValueError, "outside declared worker cells"):
            profiles.validate_scale_sample(recipe, sample)

    def test_damage_fixture_requires_accepted_distinct_owner_victims(self):
        recipe = profiles.scale_fixture(78, "all-active", excluded_kinds=("cow",))
        profiles.add_damage_fixture(recipe, 2, smoke=True)
        self.assertEqual(recipe["population"]["resident_requested"], 80)
        self.assertEqual(recipe["expected"]["minecraft:cow"], 2)
        self.assertTrue(all("execute as" in c and "nbt={CustomName:" in c
                            for c in recipe["stress"]["commands"]))
        rows = [{"region": i, "active_chunks": 16, "motion_admitted": row["resident"],
                 "motion_full": 0, "motion_resting": row["resident"], "projectile_candidates": 0}
                for i, row in enumerate(recipe["coverage"])]
        region = str(recipe["stress"]["regions"][0])
        sample = {"tick": 3, "coreperf": {"tick": 3, "resident_entities": 90, "regions": rows,
                  "damage_victims_by_region": {region: 2}, "damage_reactive_by_region": {}}}
        self.assertEqual(profiles.validate_scale_sample(recipe, sample)["damage_victims"], 2)
        del sample["coreperf"]["damage_victims_by_region"]
        with self.assertRaisesRegex(ValueError, "damage owner"):
            profiles.validate_scale_sample(recipe, sample)

    def test_all_residents_can_be_damage_targets_without_empty_world_setup(self):
        recipe = profiles.scale_fixture(0, "all-active", excluded_kinds=("cow",))
        profiles.add_damage_fixture(recipe, 2, smoke=True)
        self.assertEqual(recipe["population"]["resident_requested"], 2)
        self.assertTrue(any(command.startswith("gamerule doMobSpawning")
                            for command in recipe["regions"][0]["commands"]))

    def test_map_frames_require_saved_item_and_real_map_record(self):
        recipe = profiles.scale_fixture(78, "all-active")
        profiles.add_map_frame_fixture(recipe, 2, smoke=True)
        self.assertEqual(recipe["population"]["resident_requested"], 80)
        self.assertEqual(len(recipe["regions"][-1]["frame_positions"]), 2)
        positions = recipe["regions"][-1]["frame_positions"]
        self.assertEqual(len({tuple(position) for position in positions}), 2)
        item = {"id": "minecraft:filled_map", "count": 1, "components": {"minecraft:map_id": 7}}
        rows = [{"uuid": str(i), "active": True,
                 "payload": {"CustomName": profiles.compare.COHORT_NAME, "Item": dict(item)}}
                for i in range(2)]
        snapshot = {"entities": {"minecraft:item_frame": {"samples": rows}},
                    "globals": {"map/7": {"version": 1, "payload": {"colors": [0]}}}}
        self.assertEqual(profiles.validate_map_frames(snapshot, 2), {"0": 7, "1": 7})
        snapshot["globals"]["map/7"] = {}
        with self.assertRaisesRegex(ValueError, "map record"):
            profiles.validate_map_frames(snapshot, 2)
        del snapshot["globals"]["map/7"]
        with self.assertRaisesRegex(ValueError, "map record"):
            profiles.validate_map_frames(snapshot, 2)

    def test_map_frames_keep_257_distinct_identities_across_real_map_batches(self):
        recipe = profiles.scale_fixture(10, "all-active")
        profiles.add_map_frame_fixture(recipe, 257, smoke=True)
        positions = recipe["regions"][-1]["frame_positions"]
        self.assertEqual(len(positions), 257)
        self.assertEqual(len({tuple(position) for position in positions}), 257)
        rows = [{"uuid": str(i), "active": True, "payload": {
            "CustomName": profiles.compare.COHORT_NAME, "Item": {
                "id": "minecraft:filled_map", "count": 1,
                "components": {"minecraft:map_id": 7 if i < 256 else 8}}}} for i in range(257)]
        snapshot = {"entities": {"minecraft:item_frame": {"samples": rows}},
                    "globals": {f"map/{i}": {"version": 1, "payload": {"colors": [0]}} for i in (7, 8)}}
        identities = profiles.validate_map_frames(snapshot, 257)
        self.assertEqual((identities["0"], identities["256"]), (7, 8))
        rows[-1]["payload"]["Item"]["components"]["minecraft:map_id"] = 7
        with self.assertRaisesRegex(ValueError, "more than 256"):
            profiles.validate_map_frames(snapshot, 257)
        rows[-1]["payload"]["Item"]["components"]["minecraft:map_id"] = 8
        rows[-1]["uuid"] = "0"
        with self.assertRaisesRegex(ValueError, "identity"):
            profiles.validate_map_frames(snapshot, 257)

    def test_scale_sample_rejects_missing_inactive_or_corrupt_coverage(self):
        recipe = profiles.scale_fixture(80, "sparse-active")
        active = next(i for i, row in enumerate(recipe["coverage"]) if row["active"])
        expected = recipe["coverage"][active]
        sample = {"tick": 42, "coreperf": {"tick": 42, "resident_entities": 81, "regions": [
            {"region": active, "active_chunks": len(expected["chunks"]),
             "motion_admitted": expected["resident"], "motion_full": 1,
             "motion_resting": expected["resident"] - 1}]}}
        self.assertEqual(profiles.validate_scale_sample(recipe, sample)["active"], 8)
        for bad in (
            {"tick": 42},
            {"tick": 42, "coreperf": {"tick": 41, "resident_entities": 81, "regions": sample["coreperf"]["regions"]}},
            {"tick": 42, "coreperf": {"tick": 42, "resident_entities": 79, "regions": sample["coreperf"]["regions"]}},
            {"tick": 42, "coreperf": {"tick": 42, "resident_entities": 81, "regions": []}},
            {"tick": 42, "coreperf": {"tick": 42, "resident_entities": 81, "regions": [
                dict(sample["coreperf"]["regions"][0], motion_admitted=7)]}},
            {"tick": 42, "coreperf": {"tick": 42, "resident_entities": 81, "regions": [
                dict(sample["coreperf"]["regions"][0], active_chunks=15)]}},
        ):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                profiles.validate_scale_sample(recipe, bad)

    def test_motion_partition_requires_explicit_integer_full_and_resting(self):
        recipe = profiles.scale_fixture(80, "sparse-active")
        index = next(i for i, row in enumerate(recipe["coverage"]) if row["active"])
        expected = recipe["coverage"][index]
        row = {"region": index, "active_chunks": len(expected["chunks"]),
               "motion_admitted": expected["resident"], "motion_full": 3,
               "motion_resting": expected["resident"] - 3}
        sample = {"tick": 5, "coreperf": {"tick": 5, "resident_entities": 80, "regions": [row]}}
        self.assertEqual(profiles.validate_scale_sample(recipe, sample)["admitted"], expected["resident"])
        for key in ("motion_full", "motion_resting"):
            saved = row.pop(key)
            with self.subTest(missing=key), self.assertRaises(ValueError):
                profiles.validate_scale_sample(recipe, sample)
            row[key] = True
            with self.subTest(boolean=key), self.assertRaises(ValueError):
                profiles.validate_scale_sample(recipe, sample)
            row[key] = saved
        row["motion_resting"] -= 1
        with self.assertRaisesRegex(ValueError, "partition"):
            profiles.validate_scale_sample(recipe, sample)

    def test_million_resident_does_not_become_million_active(self):
        recipe = {"population": {"resident_requested": 1_000_000, "active_requested": 100_000},
                  "coverage": [{"resident": 100_000, "active": True, "chunks": [[0, 0]]},
                               {"resident": 900_000, "active": False, "chunks": [[2, 2]]}]}
        sample = {"tick": 10, "coreperf": {"tick": 10, "resident_entities": 1_000_100, "regions": [
            {"region": 0, "active_chunks": 1, "motion_admitted": 100_000,
             "motion_full": 50_000, "motion_resting": 50_000}]}}
        result = profiles.validate_scale_sample(recipe, sample)
        self.assertEqual(result["active"], 100_000)
        self.assertNotEqual(result["active"], 1_000_000)

    def test_visibility_requires_matching_measured_source_tick(self):
        samples = [{"tick": 8, "wall_start_unix_ns": 100},
                   {"tick": 9, "wall_start_unix_ns": 200}]
        rows = [{"stats": {"coreperf_visibility": {
            "source_tick": 8, "tracked_entity_viewer_pairs": 3}}}]
        visible = profiles.visibility_summary(samples, rows, 100, 200)
        self.assertEqual(visible["tracked_entity_viewer_pairs_min"], 3)
        self.assertEqual(visible["source_ticks"], [8])
        with self.assertRaises(ValueError):
            profiles.visibility_summary(samples, rows, 200, 300)


    def test_xp_players_and_locality_are_independent_of_cold_residents(self):
        for players in (1, 100, 1000):
            for locality in ("compact", "spread"):
                recipe = profiles.scale_fixture(80, "all-active", active_override=0)
                profiles.add_xp_fixture(recipe, 1, smoke=True, players=players, locality=locality)
                stress = recipe["stress"]
                self.assertEqual(len(stress["anchors"]), players)
                centers = {(x//16, z//16) for x, z in stress["anchors"]}
                self.assertEqual(len(centers), 1 if locality == "compact" else min(16, players))
                self.assertEqual(recipe["population"]["active_requested"], 0)
                self.assertEqual(len(stress["regions"]), 1)
                self.assertEqual(len(recipe["expected_blocks"]), 4)
                rows = []
                for x, z in stress["anchors"]:
                    center = [x//16, z//16]
                    rows.append({"initial_chunk_center": center, "final_chunk_center": center,
                                 "chunk_center_transitions": 0})
                proof = {"mechanics": {"players": rows}}
                self.assertEqual(profiles.validate_xp_observers(stress, proof)["players"], players)
                rows[0]["final_chunk_center"] = [999, 999]
                with self.assertRaisesRegex(ValueError, "observer"):
                    profiles.validate_xp_observers(stress, proof)
                rows.pop()
                with self.assertRaisesRegex(ValueError, "locality evidence"):
                    profiles.validate_xp_observers(stress, proof)

    def test_xp_zero_observers_requires_cold_zero_orb_control(self):
        recipe = profiles.scale_fixture(10, "all-active", active_override=0)
        profiles.add_xp_fixture(recipe, 0, smoke=True, players=0)
        self.assertFalse(any(row["active"] for row in recipe["coverage"]))
        self.assertEqual(recipe["stress"]["anchors"], [])
        sample = {"tick": 2, "coreperf": {"tick": 2, "resident_entities": 10,
                  "regions": [], "xp_orbs_processed": 0, "xp_orbs_by_region": {}}}
        self.assertEqual(profiles.validate_scale_sample(recipe, sample)["active"], 0)
        with self.assertRaisesRegex(ValueError, "invalid XP workload"):
            profiles.add_xp_fixture(profiles.scale_fixture(10, "all-active", active_override=0),
                                    1, smoke=True, players=0)

    def test_full_xp_banks_are_compact_and_preserve_offered_counts(self):
        for count in (10000, 100000):
            recipe = profiles.scale_fixture(10, "all-active", active_override=0)
            profiles.add_xp_fixture(recipe, count, smoke=False)
            stress = recipe["stress"]
            self.assertEqual(stress["dispenser_capacity"], 100000)
            self.assertEqual(len(stress["regions"]), 1)
            self.assertEqual(len(stress["commands"]), 52)
            self.assertEqual(len(recipe["expected_blocks"]), 100000)
            self.assertEqual(sum(row["item"] == "minecraft:experience_bottle"
                                 for row in recipe["expected_blocks"]), count)
            positions = [tuple(row["position"]) for row in recipe["expected_blocks"]]
            self.assertEqual(len(set(positions)), 100000)
            self.assertLess(max(position[1] for position in positions), 320)
            self.assertEqual(sum("minecraft:experience_bottle 64" in command
                                 for command in recipe["regions"][-1]["commands"]), count)

    def test_exact_cohort_tick_rejects_missing_and_wrong_counts(self):
        samples = [{"tick": 10, "xp_orbs": 2}, {"tick": 11, "xp_orbs": 1}]
        self.assertEqual(profiles.exact_cohort_tick(samples, "xp_orbs", 1), 11)
        for bad, count in ((samples, 3), (samples, 0), ([], 0), ([{"tick": 1}], 0)):
            with self.subTest(bad=bad, count=count), self.assertRaises(ValueError):
                profiles.exact_cohort_tick(bad, "xp_orbs", count)
        self.assertEqual(profiles.exact_cohort_tick([{"tick": 12, "xp_orbs": 0}], "xp_orbs", 0), 12)

    def test_saved_fixture_block_census_rejects_wrong_or_missing_blocks(self):
        recipe = {"expected_blocks": [{"position": [1, 2, 3], "block": "minecraft:chest"}]}
        snapshot = {"blocks": {"fixture-0": {"position": [1, 2, 3],
                    "final": {"name": "minecraft:chest"}}}}
        self.assertEqual(profiles.validate_fixture_blocks(recipe, snapshot),
                         {"blocks": {"minecraft:chest": 1}, "loaded_sources": {}})
        snapshot["blocks"]["fixture-0"]["final"]["name"] = "minecraft:stone"
        with self.assertRaisesRegex(ValueError, "differs"):
            profiles.validate_fixture_blocks(recipe, snapshot)
        snapshot["blocks"].clear()
        with self.assertRaisesRegex(ValueError, "count"):
            profiles.validate_fixture_blocks(recipe, snapshot)
        recipe["expected_blocks"].append(recipe["expected_blocks"][0])
        with self.assertRaisesRegex(ValueError, "duplicated"):
            profiles.fixture_block_spec(recipe)
        recipe = {"expected_blocks": [{"position": [1, 2, 3], "block": "minecraft:dispenser",
                                       "item": "minecraft:arrow", "count": 64}]}
        slots = [{"name": "minecraft:arrow", "count": 64}] + [{"name": "minecraft:air", "count": 0} for _ in range(8)]
        snapshot = {"blocks": {"fixture-0": {"position": [1, 2, 3], "final": {"name": "minecraft:dispenser"}}},
                    "containers": {"fixture-container-0": {"position": [1, 2, 3], "final": slots}}}
        self.assertEqual(profiles.validate_fixture_blocks(recipe, snapshot),
                         {"blocks": {"minecraft:dispenser": 1}, "loaded_sources": {"minecraft:arrow": 1}})
        slots[0]["count"] = 63
        with self.assertRaisesRegex(ValueError, "ammunition"):
            profiles.validate_fixture_blocks(recipe, snapshot)

    def test_saved_spawn_rule_state_rejects_missing_or_wrong_values(self):
        snapshot = {"globals": {"checkpoint": {"payload": {"gamerules": {
            "spawn_mobs": "true", "spawn_monsters": "false"}}}}}
        expected = {"spawn_mobs": True, "spawn_monsters": False}
        self.assertEqual(profiles.validate_spawn_rules(snapshot, expected), expected)
        snapshot["globals"]["checkpoint"]["payload"]["gamerules"]["spawn_mobs"] = "false"
        with self.assertRaisesRegex(ValueError, "differs"):
            profiles.validate_spawn_rules(snapshot, expected)
        del snapshot["globals"]["checkpoint"]["payload"]["gamerules"]
        with self.assertRaisesRegex(ValueError, "missing saved spawn-rule"):
            profiles.validate_spawn_rules(snapshot, expected)

    def test_natural_cohort_requires_saved_sheared_and_burning_states(self):
        for kind, animal, field, before in (("wool-sheep", "sheep", "Sheared", True),
                                             ("daylight-zombies", "zombie", "Fire", False)):
            stress = {"kind": kind, "requested_count": 1}
            row = {"active": True, "payload": {"CustomName": profiles.compare.COHORT_NAME, field: 1}}
            snapshot = {"entities": {f"minecraft:{animal}": {"samples": [row]}}}
            self.assertEqual(profiles.validate_natural_snapshot(stress, snapshot, before=before)["positive_state"], 1)
            row["payload"][field] = 0
            with self.assertRaisesRegex(ValueError, "state differs"):
                profiles.validate_natural_snapshot(stress, snapshot, before=before)
            row["active"] = False
            with self.assertRaisesRegex(ValueError, "count"):
                profiles.validate_natural_snapshot(stress, snapshot, before=before)

    def test_reactive_pairs_require_saved_thorns_and_source_tick_work(self):
        recipe = profiles.scale_fixture(10, "all-active", excluded_kinds=("sheep", "zombie"))
        profiles.add_reactive_fixture(recipe, 1)
        self.assertEqual(recipe["population"]["resident_requested"], 12)
        self.assertEqual(recipe["expected"]["minecraft:iron_golem"], 1)
        armor = {"id": "minecraft:diamond_chestplate", "components": {
            "minecraft:enchantments": {"minecraft:thorns": 10}}}
        row = {"active": True, "payload": {"CustomName": profiles.compare.COHORT_NAME,
               "equipment": {"chest": armor}}}
        snapshot = {"entities": {"minecraft:zombie": {"samples": [row]}}}
        self.assertEqual(profiles.validate_reactive_equipment(snapshot, 1), 1)
        self.assertEqual(profiles.validate_reactive_samples(recipe["stress"], [{"damage_reactive": 1}]), 1)
        rows = [{"region": i, "active_chunks": len(coverage["chunks"]),
                 "motion_admitted": coverage["resident"], "motion_full": coverage["resident"],
                 "motion_resting": 0, "projectile_candidates": 0}
                for i, coverage in enumerate(recipe["coverage"])]
        region = str(recipe["stress"]["regions"][0])
        sample = {"tick": 1, "coreperf": {"tick": 1, "resident_entities": 12, "regions": rows,
                  "damage_victims_by_region": {region: 1}, "damage_reactive_by_region": {region: 1}}}
        self.assertEqual(profiles.validate_scale_sample(recipe, sample)["damage_reactive"], 1)
        del sample["coreperf"]["damage_reactive_by_region"]
        with self.assertRaisesRegex(ValueError, "damage owner"):
            profiles.validate_scale_sample(recipe, sample)
        armor["components"]["minecraft:enchantments"]["minecraft:thorns"] = 0
        with self.assertRaisesRegex(ValueError, "thorns"):
            profiles.validate_reactive_equipment(snapshot, 1)
        with self.assertRaisesRegex(ValueError, "below"):
            profiles.validate_reactive_samples(recipe["stress"], [{"damage_reactive": 0}])
        with self.assertRaisesRegex(ValueError, "missing"):
            profiles.validate_reactive_samples(recipe["stress"], [])

    def test_churn_bounds_and_saved_identity_turnover(self):
        recipe = profiles.scale_fixture(99, "all-active", excluded_kinds=("cow", "pig"))
        profiles.add_churn_fixture(recipe, 1, 1)
        before = {"counts": recipe["expected"], "identities": {}}
        snapshot = {"entities": {}}
        for kind, count in recipe["expected"].items():
            for index in range(count):
                identity = f"{kind}-{index}"
                before["identities"][identity] = {"kind": kind}
                final_kind = "minecraft:pig" if kind == "minecraft:cow" else kind
                final_id = "replacement" if kind == "minecraft:cow" else identity
                snapshot["entities"].setdefault(final_kind, {"samples": []})["samples"].append({
                    "uuid": final_id, "active": True, "final_position": [0, 64, 0],
                    "payload": {"CustomName": profiles.compare.COHORT_NAME}})
        _, proof = profiles.validate_churn_cohort(recipe, before, snapshot)
        self.assertEqual(proof["replaced_min"], 1)
        snapshot["entities"]["minecraft:pig"]["samples"][0]["uuid"] = "minecraft:cow-0"
        with self.assertRaisesRegex(ValueError, "turn over"):
            profiles.validate_churn_cohort(recipe, before, snapshot)
        rows = [{"region": i, "active_chunks": len(row["chunks"]),
                 "motion_admitted": row["resident"], "motion_full": row["resident"],
                 "motion_resting": 0, "projectile_candidates": 0}
                for i, row in enumerate(recipe["coverage"])]
        rows[-1]["motion_admitted"] = rows[-1]["motion_full"] = 3
        with self.assertRaisesRegex(ValueError, "transition bounds"):
            profiles.validate_scale_sample(recipe, {"tick": 1, "coreperf": {
                "tick": 1, "resident_entities": 102, "regions": rows}})


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

    async def test_cold_visibility_proof_rejects_cap_and_counts_overlap_once(self):
        class Session:
            def __init__(self, unique):
                self.unique = unique
                self.args = argparse.Namespace(metrics_bearer_token="secret")

            async def http(self, url, timeout, *, bearer_token):
                return {"source_tick": 11, "viewers": 2,
                        "tracked_entity_viewer_pairs": 4 if self.unique is not None else 5,
                        "unique_tracked_entities": self.unique, "pair_limit": 4}

        session = Session(3)
        proof = await profiles.cold_visibility_proof(session, "http://localhost", 2, 10, 1)
        self.assertEqual(proof["unique_tracked_entities"], 3)
        with self.assertRaisesRegex(profiles.ColdVisibilityUnavailable, "requires 5 pair visits") as caught:
            await profiles.cold_visibility_proof(Session(None), "http://localhost", 2, 10, 1)
        self.assertIsNone(caught.exception.proof["unique_tracked_entities"])
        self.assertEqual(caught.exception.proof["tracked_entity_viewer_pairs"], 5)

    async def test_cold_visibility_capture_does_not_retry_stale_or_partial_proof(self):
        class Session:
            def __init__(self):
                self.requests = 0
                self.args = argparse.Namespace(metrics_bearer_token="secret")

            async def http(self, _url, _timeout, *, bearer_token):
                self.requests += 1
                return {"source_tick": 10, "viewers": 1}

        session = Session()
        with self.assertRaisesRegex(ValueError, "wrong source tick or incomplete viewers"):
            await profiles.cold_visibility_proof(session, "http://localhost", 2, 10, 1)
        self.assertEqual(session.requests, 1)

    async def test_productive_selector_rejected_before_preparation_including_zero(self):
        with tempfile.TemporaryDirectory() as directory:
            for count in (0, 1, 1000):
                args = argparse.Namespace(out=Path(directory) / str(count), profile="scale-100k-all-active",
                                          productive_npcs=count)
                with patch.object(profiles, "host_receipt", side_effect=AssertionError("must not prepare")):
                    self.assertEqual(await profiles.run(args), 1)
                result = json.loads((args.out / "result.json").read_text())
                self.assertEqual(result["status"], "failed")
                self.assertIn("productive-npcs unavailable", result["error"])


    async def test_public_prepare_receipt_rejects_wrong_saved_fixture_blocks(self):
        async def prepared(_args, _directory, recipe, _key, _hashes):
            identities = {f"{kind}-{i}": {"kind": kind} for kind, count in recipe["expected"].items()
                          for i in range(count)}
            return Path("/unused"), {"recipe_sha256": profiles.load.digest(recipe),
                "cohort": {"counts": recipe["expected"], "identities": identities},
                "fixture_cache": {"status": "disabled"},
                "snapshot": {"entities": {kind: {"active": count}
                                           for kind, count in recipe["expected"].items()}}}
        async def observed(_args, _directory, _world, _config, spec):
            if corrupt == "block":
                return {"blocks": {}}
            result = {"blocks": {row["name"]: {"position": row["position"],
                      "final": {"name": "minecraft:dispenser"}} for row in spec["blocks"]},
                      "containers": {row["name"]: {"position": row["position"],
                      "final": [{"name": "minecraft:air", "count": 0} for _ in range(9)]}
                      for row in spec["containers"]}}
            if corrupt == "ammo":
                result["containers"]["fixture-container-0"]["final"][0] = {"name": "minecraft:arrow", "count": 1}
            return result
        manifest = {"head": "base", "diff_sha256": "diff", "binary_sha256": "sha"}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for corrupt in ("none", "block", "ammo"):
                args = argparse.Namespace(out=root / str(corrupt), profile="scale-100k-all-active",
                    productive_npcs=None, server_bin=root / "server" / "solaris-v2",
                    bots_bin=root / "bots" / "player-scale-bots", build_receipt=root / "server" / "manifest.json",
                    vanilla_data_dir=root, mobs=10, villagers=0, damage_victims=None, map_frames=None,
                    churn_percent=None, daylight_zombies=None, wool_sheep=None, reactive_pairs=None, xp_orbs=None,
                    spawn_mobs=None, spawn_monsters=None,
                    xp_players=None, xp_locality=None, projectiles=0, containers=None, smoke=True,
                    prepare_only=True, readiness_seconds=1800)
                with patch.object(profiles, "host_receipt", return_value={}), \
                     patch.object(profiles, "pinned_binary", return_value=manifest), \
                     patch.object(profiles.load, "tree_hash", return_value={"sha256": "registry"}), \
                     patch.object(profiles.compare, "fixture_cache_key", return_value="fixture"), \
                     patch.object(profiles.compare, "prepare", new=prepared), \
                     patch.object(profiles.load, "native_snapshot", new=observed):
                    status = await profiles.run(args)
                result = json.loads((args.out / "result.json").read_text())
                self.assertEqual(status, int(corrupt != "none"))
                self.assertEqual(result["status"], "prepared" if corrupt == "none" else "failed")
                if corrupt == "block":
                    self.assertIn("saved fixture block count", result["error"])
                if corrupt == "ammo":
                    self.assertIn("saved fixture ammunition", result["error"])


if __name__ == "__main__":
    unittest.main()
