"""Unit contracts for the per-item cost table; synthetic server logs, no server launch."""
import json
from pathlib import Path
import tempfile
import unittest

from tools.coreperf import items_table


def tick_log(tick: int, items: int, *, merge: bool, entity_ms: float, plan_ms: float) -> str:
    lines = [
        f"items census: tick={tick} dimension=Overworld items={items} alive={items} ground={items // 2} "
        f"moving={items // 4} mature={items} stack_count={items} step_items={items} "
        f"step_cpu_ms={items * 0.0004:.3f} clock_ns=40 census_ms={items * 0.00002:.3f}",
    ]
    if merge:
        lines.append(f"items merge: tick={tick} dimension=Overworld examined={items} gather_sort_ms=1.000 "
                     f"scan_ms={items * 0.0005:.3f} candidates={items * 3} overlapping={items} merged=0")
    lines += [
        f"items views: pending={items // 4} items_pending={items // 4} changed={items // 4} "
        f"items_changed={items // 4} probe_ms={items * 0.00001:.3f}",
        "  entity phase detail ms: pre=0.01 step_scope=2.50",
        f"phase ms: tick={tick} input=1.0 chunks=0.5 entity={entity_ms:.2f} merge=0.00 block=2.0 commit=1.0 "
        "snapshot=0.1 views=0.7 intents=0 dimension=Overworld entity_tail=0.0 block_prepare=0.0 "
        "publish_tail=0.0 boundary_gaps=0.0 before_return=6.0 allocator=0.1 entity_life=0.0 entity_species=0.0",
        f"phase ms: tick={tick} input=0 chunks=0 entity=0 merge=0 block=0 commit=0 snapshot=0 views=0 "
        "intents=0 dimension=Nether entity_tail=0 block_prepare=0 publish_tail=0 boundary_gaps=0 "
        "before_return=0 allocator=0 entity_life=0 entity_species=0",
        f"edge items: tick={tick} viewers=10 packets={items // 2} bytes={items * 9} item_pairs={items * 10} "
        f"all_pairs={items * 10 + 500} item_visits={items * 10} all_visits={items * 10 + 500} "
        "item_spawn_pkts=0 item_spawn_bytes=0 "
        f"item_move_pkts={items // 4} item_move_bytes={items * 3} item_meta_pkts=0 item_meta_bytes=0 "
        f"prelude_ms=0.1 index_ms=0.2 plan_ms={plan_ms:.3f} fill_ms=0.1 total_ms={plan_ms + 0.4:.3f} probe_ms=0.05",
    ]
    return "\n".join(lines)


def leg(root: Path, items: int, ticks: int = 40, offset: int = 1) -> Path:
    run = root / f"leg-{items}" / "run"
    run.mkdir(parents=True)
    samples, log = [], []
    for sample_tick in range(100, 100 + ticks):
        entity_ms = 1.0 + items * 0.001
        samples.append({"tick": sample_tick, "wall_start_unix_ns": 1_000 + sample_tick, "wall_us": 8000 + items,
                        "phase_us": {"entity_us": round(entity_ms * 1000)}})
        log.append(tick_log(sample_tick + offset, items, merge=(sample_tick + offset) % 20 == 0,
                            entity_ms=entity_ms, plan_ms=0.5 + items * 0.0002))
        log.append(f"tick outer ms: tick={sample_tick + offset} pre=0.1 run=5.0 journal=0.5 seal_send_generated=0.2 post=0.3")
        if (sample_tick + offset) % 20 == 0:
            log.append(f"checkpoint capture ms: tick={sample_tick + offset} data={items * 0.002:.2f} owners=1.0 "
                       "scheduler=0.1 chunks=2.0 absent=0.0 dimensions=0.0 job=0.1")
    (run / "samples.jsonl").write_text(json.dumps({"metrics": {"samples": samples}}) + "\n")
    (run / "server.log").write_text("\n".join(log) + "\n")
    (run / "result.json").write_text(json.dumps({
        "status": "measured", "start_unix_ns": 1_000, "end_unix_ns": 1_000_000,
        "summary": {"tps": 19.5, "mspt": {"p50": 8.0, "p95": 9.0, "p99": 9.5, "max": 10.0}},
        "entity_counts_before": {"minecraft:item": items}, "entity_counts_after": {"minecraft:item": items}}))
    return run.parent


class ItemsTableTests(unittest.TestCase):
    def test_parse_attaches_item_lines_to_the_next_overworld_phase_line_only(self):
        ticks, edge, loop = items_table.parse_log(tick_log(7, 1000, merge=True, entity_ms=2.0, plan_ms=1.0))
        self.assertEqual(list(ticks), [7])
        row = ticks[7]
        self.assertEqual(row["census"]["items"], 1000)
        self.assertEqual(row["merge"]["examined"], 1000)
        self.assertEqual(row["views"]["items_pending"], 250)
        self.assertEqual(row["laps"]["entity phase"]["step_scope"], 2.5)
        self.assertEqual(edge[7]["item_pairs"], 10000)
        self.assertEqual(loop, {"outer": [], "checkpoint": []})

    def test_loop_level_lines_are_collected_apart_from_dimension_ticks(self):
        _, _, loop = items_table.parse_log(
            "tick outer ms: tick=9 pre=0.10 run=5.00 journal=0.50 seal_send_generated=0.20 post=0.30\n"
            "checkpoint capture ms: tick=9 data=12.50 owners=1.00 scheduler=0.10 chunks=3.00 absent=0.00 "
            "dimensions=0.20 job=0.10\n")
        self.assertEqual(loop["outer"][0]["run"], 5.0)
        self.assertEqual(loop["checkpoint"][0]["data"], 12.5)

    def test_expiry_lines_collect_per_pass(self):
        ticks, _, _ = items_table.parse_log(
            "items expiry: tick=5 dimension=Overworld pass=age items=40 removed=40 remove_ms=2.000\n"
            "phase ms: tick=5 input=0 chunks=0 entity=0 merge=0 block=0 commit=0 snapshot=0 views=0 intents=0 "
            "dimension=Overworld entity_tail=0 block_prepare=0 publish_tail=0 boundary_gaps=0 before_return=0 "
            "allocator=0 entity_life=0 entity_species=0\n")
        self.assertEqual(ticks[5]["expiry"], [{"tick": 5.0, "dimension": "Overworld", "pass": "age",
                                               "items": 40.0, "removed": 40.0, "remove_ms": 2.0}])

    def test_offset_between_sample_and_phase_ticks_is_found(self):
        with tempfile.TemporaryDirectory() as directory:
            for offset in (-2, 0, 3):
                root = leg(Path(directory) / str(offset), 2000, offset=offset)
                result = items_table.analyze_leg(root, "n2k")
                self.assertEqual(result["offset"], offset)
                self.assertEqual(result["matched_ticks"], 40)

    def test_ladder_recovers_per_item_slopes_per_tick(self):
        with tempfile.TemporaryDirectory() as directory:
            legs = [items_table.analyze_leg(leg(Path(directory), size), f"n{size}") for size in (0, 10000, 30000)]
            ladder = items_table.ladder_table(legs)
            self.assertEqual(ladder["sizes"], [0, 10000, 30000])
            entity = ladder["columns"]["phase.entity"]
            self.assertAlmostEqual(entity["per_item_per_tick"], 0.001, places=6)  # ms per item
            self.assertAlmostEqual(entity["base"], 1.0, places=3)
            plan = ladder["columns"]["edge.plan_ms"]
            self.assertAlmostEqual(plan["per_item_per_tick"], 0.0002, places=7)
            self.assertAlmostEqual(ladder["columns"]["edge.bytes"]["per_item_per_tick"], 9.0, places=6)
            text = items_table.markdown(legs, ladder)
            self.assertIn("phase.entity", text)
            self.assertIn("| phase.entity | 1.00, 11.00, 31.00 | 1.000 |", text)  # 0.001 ms = 1.000 µs per item per tick
            self.assertIn("| edge.bytes | 0.00, 90000.00, 270000.00 | 9.000 |", text)  # counts are not scaled

    def test_attribution_overhead_is_subtracted_from_the_timed_phases(self):
        with tempfile.TemporaryDirectory() as directory:
            result = items_table.analyze_leg(leg(Path(directory), 10000), "n10k")
            self.assertAlmostEqual(result["overhead_ms"]["census"], 0.2, places=6)
            self.assertAlmostEqual(result["overhead_ms"]["views_probe"], 0.1, places=6)
            self.assertAlmostEqual(result["phases_net_ms"]["entity"], result["phases_ms"]["entity"] - 0.2, places=6)
            self.assertAlmostEqual(result["phases_net_ms"]["views"], 0.7 - 0.1, places=6)
            self.assertAlmostEqual(result["edge"]["probe_ms"], 0.05, places=6)

    def test_checkpoint_capture_is_amortized_over_window_ticks(self):
        with tempfile.TemporaryDirectory() as directory:
            result = items_table.analyze_leg(leg(Path(directory), 1000, ticks=40), "n1k")
            self.assertEqual(result["checkpoint"]["captures"], 2)
            self.assertAlmostEqual(result["checkpoint"]["data_ms"], 2.0, places=3)
            self.assertAlmostEqual(result["checkpoint"]["ms_per_tick"], 2 * (2.0 + 1.0 + 0.1 + 2.0 + 0.1) / 40, places=3)
            self.assertAlmostEqual(result["outer_ms"]["run"], 5.0)

    def test_step_cost_subtracts_the_clock_pair(self):
        with tempfile.TemporaryDirectory() as directory:
            result = items_table.analyze_leg(leg(Path(directory), 5000), "n5k")
            # 0.0004 ms = 400 ns per item as logged, minus the 40 ns clock pair.
            self.assertAlmostEqual(result["step"]["ns_per_item"], 360.0, places=1)
            self.assertEqual(result["items"]["mean"], 5000)

    def test_merge_cost_is_per_examined_item_on_cadence_ticks_only(self):
        with tempfile.TemporaryDirectory() as directory:
            result = items_table.analyze_leg(leg(Path(directory), 4000, ticks=60), "n4k")
            self.assertEqual(result["merge"]["passes"], 3)
            self.assertAlmostEqual(result["merge"]["candidates_per_item"], 3.0)
            self.assertAlmostEqual(result["merge"]["ms_per_pass"], 1.0 + 4000 * 0.0005, places=3)


if __name__ == "__main__":
    unittest.main()
