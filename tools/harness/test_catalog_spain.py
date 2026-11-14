import asyncio
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock

from tools.harness.backends.catalog_load import ClassifiedFailure
from tools.harness.backends.catalog_spain import SpainCohort, SpainPopulation, run_concurrent


class SpainCohortTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.cohort = SpainCohort(SimpleNamespace(), Path(self.tmp.name))

    async def test_concurrent_waiters_receive_crossed_events(self):
        first = asyncio.create_task(self.cohort.event({"reply"}, 1, command_id="first"))
        second = asyncio.create_task(self.cohort.event({"reply"}, 1, command_id="second"))
        loop = asyncio.get_running_loop()
        loop.call_soon(self.cohort.push_event, {"event": "reply", "id": "second"})
        loop.call_soon(self.cohort.push_event, {"event": "reply", "id": "first"})
        one, two = await asyncio.gather(first, second)
        self.assertEqual((one["id"], two["id"]), ("first", "second"))

    async def test_same_bot_commands_are_serialized(self):
        entered, release = asyncio.Event(), asyncio.Event()
        active = 0
        peak = 0
        async def fake_command(_bot, ident, _text, _prefix, _timeout):
            nonlocal active, peak
            active += 1
            peak = max(peak, active)
            entered.set()
            await release.wait()
            active -= 1
            return {"id": ident}
        self.cohort._command_event = fake_command
        first = asyncio.create_task(self.cohort.command_event(1, "first", "x", "x", 1))
        await entered.wait()
        second = asyncio.create_task(self.cohort.command_event(1, "second", "x", "x", 1))
        asyncio.get_running_loop().call_soon(release.set)
        rows = await asyncio.gather(first, second)
        self.assertEqual([row["id"] for row in rows], ["first", "second"])
        self.assertEqual(peak, 1)

    def test_visible_villager_roster_rejects_truncation(self):
        identity = "7db6bca5-aa5d-c3b0-465a-7529fa63650d"
        self.cohort.ready_image = {"mechanics": {"capacity_errors": 0, "identity_errors": 0,
            "position_errors": 0, "entities": {"minecraft:villager": {
                "active": 1, "samples": [{"uuid": identity, "active": True}]}}}}
        self.assertEqual(self.cohort.ready_villagers(), [identity])
        self.cohort.ready_image["mechanics"]["entities"]["minecraft:villager"]["active"] = 2
        with self.assertRaises(ClassifiedFailure):
            self.cohort.ready_villagers()

    async def test_sampled_death_interrupts_pickup_wait(self):
        self.cohort.push_event({"event": "sample", "mechanics": {"deaths": 1}})
        with self.assertRaisesRegex(ClassifiedFailure, "observed bot death"):
            await self.cohort.pickup(0, "haul", "player", "farmer", {"farmer"}, 3, 1)

    async def test_pickup_correlates_collection_before_drop(self):
        self.cohort.push_event({"event": "item_collection_observed", "item_uuid": "item", "collector_uuid": "farmer", "amount": 3, "retention_truncated": 0})
        self.cohort.push_event({"event": "farmer_drop_observed", "id": "haul", "bot": 0, "player_uuid": "player", "farmer_uuid": "farmer", "item_uuid": "item", "item": "minecraft:wheat", "count": 3, "inventory_before": 3, "inventory_after": 0, "retention_truncated": 0})
        drop, pickup = await self.cohort.pickup(0, "haul", "player", "farmer", {"farmer"}, 3, 1)
        self.assertEqual(drop["item_uuid"], pickup["item_uuid"])

    async def test_truncated_event_fails_closed(self):
        self.cohort.push_event({"event": "item_collection_overflow", "retention_truncated": 1})
        with self.assertRaises(ClassifiedFailure):
            await self.cohort.pickup(0, "haul", "player", "farmer", {"farmer"}, 3, 1)

    async def test_wrong_collector_is_not_pickup(self):
        self.cohort.push_event({"event": "farmer_drop_observed", "id": "haul", "bot": 0, "player_uuid": "player", "farmer_uuid": "farmer", "item_uuid": "item", "item": "minecraft:wheat", "count": 3, "inventory_before": 3, "inventory_after": 0})
        self.cohort.push_event({"event": "item_collection_observed", "item_uuid": "item", "collector_uuid": "other", "amount": 3})
        with self.assertRaisesRegex(ClassifiedFailure, "tracked wheat collected by an unbaselined entity"):
            await self.cohort.pickup(0, "haul", "player", "farmer", {"farmer"}, 3, 1)

    async def test_partial_pickup_cannot_prove_exact_transfer(self):
        self.cohort.push_event({"event": "farmer_drop_observed", "id": "haul", "bot": 0, "player_uuid": "player", "farmer_uuid": "farmer", "item_uuid": "item", "item": "minecraft:wheat", "count": 3, "inventory_before": 3, "inventory_after": 0})
        self.cohort.push_event({"event": "item_collection_observed", "item_uuid": "item", "collector_uuid": "farmer", "amount": 2})
        with self.assertRaisesRegex(ClassifiedFailure, "tracked wheat collected in a different quantity"):
            await self.cohort.pickup(0, "haul", "player", "farmer", {"farmer"}, 3, 1)

    async def test_unresolved_collector_can_be_resolved_by_another_observer(self):
        self.cohort.push_event({"event": "farmer_drop_observed", "id": "haul", "bot": 0, "player_uuid": "player", "farmer_uuid": "farmer", "item_uuid": "item", "item": "minecraft:wheat", "count": 3, "inventory_before": 3, "inventory_after": 0})
        self.cohort.push_event({"event": "item_collection_observed", "item_uuid": "item", "collector_uuid": None, "amount": 3})
        self.cohort.push_event({"event": "item_collection_observed", "item_uuid": "item", "collector_uuid": "farmer", "amount": 3})
        _, pickup = await self.cohort.pickup(0, "haul", "player", "farmer", {"farmer"}, 3, 1)
        self.assertEqual(pickup["collector_uuid"], "farmer")

    async def test_baselined_other_collector_can_be_attributed(self):
        self.cohort.push_event({"event": "farmer_drop_observed", "id": "haul", "bot": 0, "player_uuid": "player", "farmer_uuid": "farmer", "item_uuid": "item", "item": "minecraft:wheat", "count": 3, "inventory_before": 3, "inventory_after": 0})
        self.cohort.push_event({"event": "item_collection_observed", "item_uuid": "item", "collector_uuid": "other", "amount": 3})
        _, pickup = await self.cohort.pickup(0, "haul", "player", "farmer", {"farmer", "other"}, 3, 1)
        self.assertEqual(pickup["collector_uuid"], "other")

    async def test_failed_haul_stops_pickup_wait(self):
        self.cohort.push_event({"event": "haul_finished", "id": "haul", "bot": 0,
                                "outcome": "failed", "receipt": {"error": "deadline exceeded"}})
        with self.assertRaisesRegex(ClassifiedFailure, "haul ended before farmer drop"):
            await self.cohort.pickup(0, "haul", "player", "farmer", {"farmer"}, 3, 1)

    async def test_failed_haul_after_drop_stops_collection_wait(self):
        self.cohort.push_event({"event": "farmer_drop_observed", "id": "haul", "bot": 0,
                                "player_uuid": "player", "farmer_uuid": "farmer", "item_uuid": "item",
                                "item": "minecraft:wheat", "count": 3, "inventory_before": 3,
                                "inventory_after": 0})
        self.cohort.push_event({"event": "haul_finished", "id": "haul", "bot": 0,
                                "outcome": "failed", "receipt": {"error": "actor died"}})
        with self.assertRaisesRegex(ClassifiedFailure, "haul failed before farmer pickup"):
            await self.cohort.pickup(0, "haul", "player", "farmer", {"farmer"}, 3, 1)

    async def test_command_rejects_stale_reply_fence(self):
        class Input:
            def write(self, data):
                self.data = data

            async def drain(self):
                pass

        self.cohort.processes["bots"] = SimpleNamespace(stdin=Input(), returncode=None)
        self.cohort.push_event({"event": "runtime_command_reply", "id": "one", "bot": 0, "reply": "ok",
                                "marker_receive_seq": 2, "write_receive_seq": 3, "receive_seq": 3})
        with self.assertRaises(ClassifiedFailure):
            await self.cohort.command(0, "one", "settlement list", "ok", 1)


class SpainPopulationTests(unittest.IsolatedAsyncioTestCase):
    async def test_rejects_wrong_cohort_size_before_spawning(self):
        with tempfile.TemporaryDirectory() as directory:
            population = SpainPopulation(SimpleNamespace(), Path(directory))
            with self.assertRaises(ValueError):
                await population.add_cohort(["unused"], 4, 1)
            self.assertEqual(population.cohorts, [])

    async def test_failed_concurrent_work_cancels_sibling(self):
        started, cancelled = asyncio.Event(), asyncio.Event()
        async def sibling():
            try:
                started.set()
                await asyncio.Event().wait()
            finally:
                cancelled.set()
        async def fail():
            await started.wait()
            raise ValueError("failure")
        with self.assertRaisesRegex(ValueError, "failure"):
            await run_concurrent([sibling(), fail()])
        self.assertTrue(cancelled.is_set())

    async def test_failed_production_preserves_causal_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            population = SpainPopulation(SimpleNamespace(), Path(directory))
            population.wait_tick = AsyncMock()
            actor = SimpleNamespace(
                pickup=AsyncMock(return_value=({"elapsed_us": 2}, {"elapsed_us": 3, "collector_uuid": "worker"})),
                haul_finished=AsyncMock(return_value={"completed": True}),
                verify_haul=lambda *_: None, verify_farmer_delivery=lambda *_: None,
                observed_ns=lambda event: event["elapsed_us"])
            def reject(*_):
                raise ClassifiedFailure("CORRECTNESS_FAIL", "missing native production")
            owner = SimpleNamespace(
                observed_ns=lambda event: event["elapsed_us"], verify_pickup=lambda *_: None,
                verify_production=reject, command=AsyncMock(return_value="accepted"),
                json_command=AsyncMock(side_effect=[
                    ({"tick": 10}, {"elapsed_us": 4}),
                    ({"done": 0, "paused": "missing input"}, {"elapsed_us": 5}),
                    ({"tick": 20}, {"elapsed_us": 6})]))
            spec = {"bot": 0, "player_uuid": "actor", "farmer_delivery": {"farmer_uuid": "worker"},
                    "item": "minecraft:wheat", "count": 3}
            with self.assertRaisesRegex(ClassifiedFailure, "missing native production"):
                await population.bread_cycle(actor, owner, spec, "haul", 1, "village", 1,
                    [0, 1, 2], [0, 1, 3], 1, 2, {"worker": ({}, {"elapsed_us": 1})}, set(), "url", 10, 1)
            evidence = json.loads((Path(directory) / "haul-evidence.json").read_text())
            self.assertEqual(evidence["collector_worker"], "worker")
            self.assertEqual(evidence["job"]["paused"], "missing input")
            self.assertEqual(evidence["loaded_carry"]["tick"], 10)
            self.assertEqual(evidence["final_carry"]["tick"], 20)

    async def test_concurrent_cycles_reject_duplicate_collector(self):
        with tempfile.TemporaryDirectory() as directory:
            population = SpainPopulation(SimpleNamespace(), Path(directory))
            actor = SimpleNamespace(pickup=AsyncMock(return_value=({}, {"collector_uuid": "worker"})))
            spec = {"bot": 0, "player_uuid": "actor", "farmer_delivery": {"farmer_uuid": "worker"},
                    "item": "minecraft:wheat", "count": 3}
            with self.assertRaisesRegex(ClassifiedFailure, "concurrent hauls share a collector"):
                await population.bread_cycle(actor, SimpleNamespace(), spec, "haul", 1, "village", 1,
                    [0, 1, 2], [0, 1, 3], 1, 2, {"worker": ({}, {})}, {"worker"}, "url", 10, 1)
