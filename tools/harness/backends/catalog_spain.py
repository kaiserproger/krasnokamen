"""Spain workload support using the existing catalog-load process lifecycle.

This module keeps causally relevant bot events until consumed. Periodic samples
are recorded by Session.read_events but are not put in the control queue.
"""
from __future__ import annotations

import asyncio
import hashlib
import json
import os
from pathlib import Path
import shutil
import time
import uuid
from typing import Any

from .catalog_load import ClassifiedFailure, Session
from .catalog_spain_oracle import check_bread_job, check_farmer_delivery, check_hauls, check_persisted_carry, check_persisted_job, check_wheat_pickup


class SpainCohort(Session):
    """One actor/observer process in a shared, independently owned server."""

    def __init__(self, args: Any, directory: Any):
        super().__init__(args, directory)
        self.pending: list[dict[str, Any]] = []
        self.epoch_unix_ns: int | None = None
        self.ready_image: dict[str, Any] = {}
        self.event_condition = asyncio.Condition()
        self.event_pump: asyncio.Task[None] | None = None
        self.command_locks: dict[int, asyncio.Lock] = {}

    def ready_villagers(self) -> list[str]:
        mechanics = self.ready_image.get("mechanics", {})
        entities = mechanics.get("entities", {}).get("minecraft:villager", {})
        samples = entities.get("samples")
        if (any(mechanics.get(key) != 0 for key in ("capacity_errors", "identity_errors", "position_errors"))
                or not isinstance(samples, list) or type(entities.get("active")) is not int):
            raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain initial villager observation is incomplete")
        try:
            villagers = sorted({str(uuid.UUID(row["uuid"])) for row in samples if row.get("active")})
        except (ValueError, KeyError, TypeError, AttributeError) as error:
            raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain invalid initial villager identity") from error
        if not villagers or len(villagers) != entities["active"]:
            raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain initial visible villager identities are incomplete")
        return villagers

    def push_event(self, event: dict[str, Any]) -> None:
        if not self.stopping and (event.get("event") == "failure" or event.get("disconnects", 0)):
            self.failure = event
        if event.get("event") == "item_collection_overflow" or event.get("retention_truncated", 0):
            self.failure = {"event": "failure", "error": "Spain item observation retention truncated"}
        if event.get("event") == "sample":
            if not self.stopping and (event.get("mechanics") or {}).get("deaths", 0):
                event = {"event": "failure", "error": "Spain observed bot death"}
                self.failure = event
            else:
                return
        if self.events.full():
            self.failure = {"event": "failure", "error": "Spain causal event queue overflow"}
            return
        self.events.put_nowait(event)

    async def retain_event(self, event: dict[str, Any]) -> None:
        async with self.event_condition:
            if len(self.pending) >= 4096:
                self.failure = {"event": "failure", "error": "Spain pending event retention overflow"}
            else:
                self.pending.append(event)
            self.event_condition.notify_all()

    async def pump_events(self) -> None:
        while True:
            event = await self.events.get()
            await self.retain_event(event)
            if self.failure is not None:
                return

    async def event(self, names: set[str], timeout: float, *, command_id: str | None = None, predicate: Any = None) -> dict[str, Any]:
        if self.event_pump is None:
            self.event_pump = asyncio.create_task(self.pump_events())
            self.readers.append(self.event_pump)
        deadline = asyncio.get_running_loop().time() + timeout
        async with self.event_condition:
            while True:
                self.alive()
                for index, event in enumerate(self.pending):
                    if event.get("event") in names and (command_id is None or event.get("id") == command_id) and (predicate is None or predicate(event)):
                        return self.pending.pop(index)
                remaining = deadline - asyncio.get_running_loop().time()
                if remaining <= 0:
                    raise ClassifiedFailure("CORRECTNESS_FAIL", f"Spain missing events {sorted(names)}")
                try:
                    await asyncio.wait_for(self.event_condition.wait(), remaining)
                except asyncio.TimeoutError as error:
                    raise ClassifiedFailure("CORRECTNESS_FAIL", f"Spain missing events {sorted(names)}") from error

    async def command_event(self, bot: int, ident: str, text: str, prefix: str, timeout: float) -> dict[str, Any]:
        lock = self.command_locks.setdefault(bot, asyncio.Lock())
        deadline = asyncio.get_running_loop().time() + timeout
        try:
            await asyncio.wait_for(lock.acquire(), timeout)
        except asyncio.TimeoutError as error:
            raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain command serialization deadline expired") from error
        try:
            remaining = deadline - asyncio.get_running_loop().time()
            if remaining <= 0:
                raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain command deadline expired")
            return await self._command_event(bot, ident, text, prefix, remaining)
        finally:
            lock.release()

    async def _command_event(self, bot: int, ident: str, text: str, prefix: str, timeout: float) -> dict[str, Any]:
        process = self.processes["bots"]
        if process.stdin is None:
            raise ClassifiedFailure("INFRA_BLOCKED", "Spain bot stdin unavailable")
        operation = {"event": "command", "id": ident, "bot": bot, "command": text, "expected_prefix": prefix}
        process.stdin.write((json.dumps(operation, separators=(",", ":")) + "\n").encode())
        await process.stdin.drain()
        event = await self.event({"runtime_command_reply", "runtime_command_rejected", "runtime_command_timeout"}, timeout, command_id=ident)
        if event.get("event") != "runtime_command_reply" or event.get("bot") != bot:
            raise ClassifiedFailure("CORRECTNESS_FAIL", f"Spain command failed: {event}")
        marker, written, received = (event.get(key) for key in ("marker_receive_seq", "write_receive_seq", "receive_seq"))
        if any(type(value) is not int for value in (marker, written, received)) or not marker <= written < received:
            raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain command reply lacks a fresh receive fence")
        reply = event.get("reply")
        if not isinstance(reply, str) or not reply.startswith(prefix):
            raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain command reply prefix mismatch")
        return event

    async def command(self, bot: int, ident: str, text: str, prefix: str, timeout: float) -> str:
        return (await self.command_event(bot, ident, text, prefix, timeout))["reply"]

    async def json_command(self, bot: int, ident: str, text: str, timeout: float) -> tuple[dict[str, Any], dict[str, Any]]:
        event = await self.command_event(bot, ident, text, "{", timeout)
        try:
            value = json.loads(event["reply"])
        except (ValueError, TypeError) as error:
            raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain state reply is not complete JSON") from error
        if not isinstance(value, dict):
            raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain state reply is not a JSON object")
        return value, event

    async def start_haul(self, bot: int, ident: str, timeout: float) -> dict[str, Any]:
        process = self.processes["bots"]
        if process.stdin is None:
            raise ClassifiedFailure("INFRA_BLOCKED", "Spain bot stdin unavailable")
        process.stdin.write((json.dumps({"event": "start_haul", "id": ident, "bot": bot}, separators=(",", ":")) + "\n").encode())
        await process.stdin.drain()
        event = await self.event({"haul_start_accepted", "haul_start_rejected"}, timeout, command_id=ident)
        if event.get("event") != "haul_start_accepted" or event.get("bot") != bot:
            raise ClassifiedFailure("CORRECTNESS_FAIL", f"Spain haul start rejected: {event}")
        event["harness_received_unix_ns"] = time.time_ns()
        event["harness_time_scope"] = "wall-clock time after consuming acceptance event; not server tick or packet timestamp"
        return event

    async def pickup(self, bot: int, haul_id: str, player_uuid: str, farmer_uuid: str,
                     eligible_workers: set[str], count: int, timeout: float) -> tuple[dict[str, Any], dict[str, Any]]:
        """Join an observed player drop and farmer collection in either event order."""
        if type(count) is not int or count <= 0:
            raise ValueError("Spain pickup count must be positive")
        deadline = asyncio.get_running_loop().time() + timeout
        drop = await self.event({"farmer_drop_observed", "haul_finished"}, timeout, command_id=haul_id)
        if drop.get("event") == "haul_finished":
            raise ClassifiedFailure("CORRECTNESS_FAIL", f"Spain haul ended before farmer drop: {drop}")
        if drop.get("bot") != bot or drop.get("player_uuid") != player_uuid or drop.get("farmer_uuid") != farmer_uuid:
            raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain unexpected farmer drop identity")
        if drop.get("item") != "minecraft:wheat" or drop.get("count") != count or drop.get("inventory_before") != count or drop.get("inventory_after") != 0 or not drop.get("item_uuid"):
            raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain drop lacks exact observed wheat debit")
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain missing matching farmer pickup")
        terminal = None
        while True:
            names = {"item_collection_observed"} if terminal is not None else {"item_collection_observed", "haul_finished"}
            event = await self.event(
                names, remaining,
                predicate=lambda row: (row.get("event") == "haul_finished" and row.get("id") == haul_id)
                    or (row.get("event") == "item_collection_observed" and row.get("item_uuid") == drop["item_uuid"]),
            )
            if event.get("event") == "item_collection_observed":
                collector = event.get("collector_uuid")
                if collector is not None and collector not in eligible_workers:
                    raise ClassifiedFailure("CORRECTNESS_FAIL", f"Spain tracked wheat collected by an unbaselined entity: {event}")
                if event.get("amount") != count:
                    raise ClassifiedFailure("CORRECTNESS_FAIL", f"Spain tracked wheat collected in a different quantity: {event}")
                if collector is None:
                    remaining = deadline - asyncio.get_running_loop().time()
                    if remaining <= 0:
                        raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain missing matching farmer pickup")
                    continue
                if terminal is not None:
                    await self.retain_event(terminal)
                return drop, event
            if event.get("bot") != bot or event.get("outcome") != "finished":
                raise ClassifiedFailure("CORRECTNESS_FAIL", f"Spain haul failed before farmer pickup: {event}")
            terminal = event
            remaining = deadline - asyncio.get_running_loop().time()
            if remaining <= 0:
                raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain missing matching farmer pickup")

    async def haul_finished(self, bot: int, ident: str, timeout: float) -> dict[str, Any]:
        event = await self.event({"haul_finished"}, timeout, command_id=ident)
        if event.get("bot") != bot or event.get("outcome") != "finished" or not isinstance(event.get("receipt"), dict):
            raise ClassifiedFailure("CORRECTNESS_FAIL", f"Spain haul did not finish: {event}")
        return event["receipt"]

    def observed_ns(self, event: dict[str, Any]) -> int:
        elapsed = event.get("elapsed_us")
        if type(self.epoch_unix_ns) is not int or type(elapsed) is not int or elapsed < 0:
            raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain event lacks run-epoch timing")
        return self.epoch_unix_ns + elapsed * 1000

    @staticmethod
    def verify_haul(spec: dict[str, Any], receipt: dict[str, Any]) -> None:
        failures = check_hauls([spec], {"hauls": [receipt]})
        if failures:
            raise ClassifiedFailure("CORRECTNESS_FAIL", f"Spain haul evidence: {failures}")

    @staticmethod
    def verify_farmer_delivery(spec: dict[str, Any], receipt: dict[str, Any],
                               drop: dict[str, Any], collection: dict[str, Any], collector: str) -> None:
        failures = check_farmer_delivery(spec, receipt, drop, collection, collector)
        if failures:
            raise ClassifiedFailure("CORRECTNESS_FAIL", f"Spain delivery evidence: {failures}")

    @staticmethod
    def verify_pickup(before: dict[str, Any], after: dict[str, Any], worker: str, item_id: int, count: int) -> None:
        failures = check_wheat_pickup(before, after, worker, item_id, count)
        if failures:
            raise ClassifiedFailure("CORRECTNESS_FAIL", f"Spain pickup evidence: {failures}")

    @staticmethod
    def verify_production(before: dict[str, Any], after: dict[str, Any], job: dict[str, Any],
                          worker: str, job_id: str, wheat_id: int, bread_id: int) -> None:
        failures = check_bread_job(before, after, job, worker, job_id, wheat_id, bread_id)
        if failures:
            raise ClassifiedFailure("CORRECTNESS_FAIL", f"Spain production evidence: {failures}")


class SpainPopulation:
    """Keep one catalog-owned server and its 2+2+4 bot cohorts alive together."""

    def __init__(self, args: Any, directory: Any):
        self.args = args
        self.directory = directory
        self.server = Session(args, directory)
        self.cohorts: list[SpainCohort] = []
        self.sizes: list[int] = []

    async def start_server(self, command: list[str], dashboard_url: str, timeout: float) -> None:
        from .catalog_load import wait_dashboard

        await self.server.spawn("server", command)
        await wait_dashboard(self.server, dashboard_url, timeout)

    async def add_cohort(self, command: list[str], size: int, timeout: float) -> SpainCohort:
        expected = (2, 2, 4)
        if len(self.sizes) >= len(expected) or size != expected[len(self.sizes)]:
            raise ValueError("Spain cohorts must be admitted as 2, 2, then 4")
        self.server.alive("SETUP_FAILED")
        for cohort in self.cohorts:
            cohort.alive("SETUP_FAILED")
        directory = self.directory / f"cohort-{len(self.sizes) + 1}"
        directory.mkdir(exist_ok=True)
        cohort = SpainCohort(self.args, directory)
        self.cohorts.append(cohort)
        await cohort.spawn("bots", command, events=True)
        cohort.ready_image = await cohort.ready(size, timeout)
        process = cohort.processes["bots"]
        if process.stdin is None:
            raise ClassifiedFailure("INFRA_BLOCKED", "Spain bot stdin unavailable")
        process.stdin.write(b'{"event":"run"}\n')
        await process.stdin.drain()
        running = await cohort.event({"running"}, timeout)
        cohort.epoch_unix_ns = running.get("open_loop", {}).get("epoch_unix_ns")
        if type(cohort.epoch_unix_ns) is not int or cohort.epoch_unix_ns <= 0:
            raise ClassifiedFailure("INFRA_BLOCKED", "Spain cohort lacks a run epoch")
        self.sizes.append(size)
        return cohort

    async def wait_tick(self, dashboard_url: str, tick: int, timeout: float) -> dict[str, Any]:
        """Wait for a native tick boundary using the dashboard's pushed sample stream."""
        if type(tick) is not int or tick < 1:
            raise ValueError("Spain tick fence must be positive")
        deadline = asyncio.get_running_loop().time() + timeout
        while True:
            remaining = deadline - asyncio.get_running_loop().time()
            if remaining <= 0:
                raise ClassifiedFailure("CORRECTNESS_FAIL", f"Spain tick {tick} was not observed")
            self.server.alive()
            for cohort in self.cohorts:
                cohort.alive()
            metrics = await self.server.http(
                f"{dashboard_url}/load-metrics?after_tick={tick - 1}&wait=true&min_samples=1",
                min(remaining, 12),
            )
            if metrics.get("overrun"):
                raise ClassifiedFailure("INFRA_BLOCKED", "Spain tick fence was overwritten")
            if any(sample.get("tick") == tick for sample in metrics.get("samples", [])):
                return metrics

    async def bread_cycle(self, actor: SpainCohort, owner: SpainCohort, spec: dict[str, Any],
                          haul_id: str, owner_bot: int, settlement: str, building: int,
                          station: list[int], stand: list[int], wheat_id: int, bread_id: int,
                          baselines: dict[str, tuple[dict[str, Any], dict[str, Any]]], claimed_workers: set[str],
                          dashboard_url: str, tick_budget: int, timeout: float) -> dict[str, Any]:
        """Fence one observed haul, actual collector pickup and native bread production."""
        from . import catalog_support as scale

        target = spec.get("farmer_delivery", {}).get("farmer_uuid")
        if not isinstance(target, str) or spec.get("item") != "minecraft:wheat" or spec.get("count") != 3:
            raise ValueError("Spain bread cycle requires three wheat and a named pursuit target")
        if not isinstance(settlement, str) or not settlement or any(c.isspace() for c in settlement):
            raise ValueError("Spain settlement name must be a single command token")
        if type(building) is not int or building <= 0 or type(tick_budget) is not int or tick_budget <= 0:
            raise ValueError("Spain building and tick budget must be positive")
        if any(not isinstance(point, list) or len(point) != 3 or any(type(c) is not int for c in point)
               for point in (station, stand)):
            raise ValueError("Spain station and stand must be integer coordinates")
        deadline = asyncio.get_running_loop().time() + timeout
        def remaining() -> float:
            value = deadline - asyncio.get_running_loop().time()
            if value <= 0:
                raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain bread cycle deadline expired")
            return value

        drop, collection = await actor.pickup(spec["bot"], haul_id, spec["player_uuid"], target, set(baselines), 3, remaining())
        worker = collection["collector_uuid"]
        if worker in claimed_workers:
            raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain concurrent hauls share a collector")
        claimed_workers.add(worker)
        initial, initial_event = baselines[worker]
        receipt = await actor.haul_finished(spec["bot"], haul_id, remaining())
        actor.verify_haul(spec, receipt)
        actor.verify_farmer_delivery(spec, receipt, drop, collection, worker)
        if owner.observed_ns(initial_event) >= min(actor.observed_ns(drop), actor.observed_ns(collection)):
            raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain initial worker carry was not observed before drop and pickup")
        loaded, loaded_event = await owner.json_command(
            owner_bot, f"{haul_id}-loaded", f"settlement production {settlement} carry {worker}", remaining())
        if owner.observed_ns(loaded_event) <= actor.observed_ns(collection):
            raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain loaded worker carry predates pickup")
        owner.verify_pickup(initial, loaded, worker, wheat_id, 3)
        evidence = {"haul_id": haul_id, "target_farmer": target, "collector_worker": worker,
                    "haul": receipt, "drop": drop, "collection": collection,
                    "initial_carry": initial, "initial_reply": initial_event,
                    "loaded_carry": loaded, "loaded_reply": loaded_event}
        evidence_path = self.directory / f"{haul_id}-evidence.json"
        scale.dump(evidence_path, evidence)
        start = (f"settlement production {settlement} start {haul_id} {worker} "
                 f"solaris-settlements:bread 1 {building} "
                 f"{','.join(map(str, station))} {','.join(map(str, stand))}")
        accepted = await owner.command(owner_bot, f"{haul_id}-start", start, "Production operation processed", remaining())
        evidence["accepted"] = accepted
        scale.dump(evidence_path, evidence)
        tick_start = loaded.get("tick")
        if type(tick_start) is not int:
            raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain loaded carry has no native tick")
        await self.wait_tick(dashboard_url, tick_start + tick_budget, remaining())
        job, job_event = await owner.json_command(
            owner_bot, f"{haul_id}-job", f"settlement production {settlement} job {haul_id}", remaining())
        final, final_event = await owner.json_command(
            owner_bot, f"{haul_id}-final", f"settlement production {settlement} carry {worker}", remaining())
        evidence.update(job=job, job_reply=job_event, final_carry=final, final_reply=final_event)
        scale.dump(evidence_path, evidence)
        owner.verify_production(loaded, final, job, worker, haul_id, wheat_id, bread_id)
        return {"haul_id": haul_id, "target_farmer": target, "collector_worker": worker,
                "haul": receipt, "drop": drop, "collection": collection,
                "initial_carry": initial, "initial_reply": initial_event, "loaded_carry": loaded,
                "final_carry": final, "job": job, "job_reply": job_event, "final_reply": final_event, "accepted": accepted,
                "ticks_observed": final["tick"] - initial["tick"]}

    async def stop(self) -> dict[str, Any]:
        for cohort in reversed(self.cohorts):
            await cohort.stop()
        await self.server.stop(server_timeout=90)
        return {"server": self.server.shutdown, "cohorts": [cohort.shutdown for cohort in self.cohorts]}


async def run_concurrent(operations: list[Any]) -> list[Any]:
    """Cancel sibling work on failure; never leave unowned actor tasks behind."""
    tasks = [asyncio.create_task(operation) for operation in operations]
    try:
        return await asyncio.gather(*tasks)
    except BaseException:
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        raise


def _offline_uuid(username: str) -> str:
    return str(uuid.UUID(bytes=hashlib.md5(("OfflinePlayer:" + username).encode()).digest(), version=3))


async def run_spain(args: Any) -> int:
    """Run four observed production chains, including a concurrent pair, in one 2→4→8 world.

    Operator-provisioned stock and a workstation are disclosed in the supplied
    fixture. This is an assisted diagnostic, not an industrial-capacity pass.
    """
    from .catalog_load import bot_command, config
    from . import catalog_support as scale
    from .catalog_spain_disk import WorldDiskGuard

    args.out.mkdir(parents=True)
    started = time.monotonic()
    result: dict[str, Any] = {
        "scenario": "spain-industrial", "outcome": "INFRA_BLOCKED",
        "scope": "assisted wheat hauling and bread production, with two concurrent final-stage chains; not autonomous extraction or industrial capacity",
        "performance_evidence": False, "cohorts": [], "cycles": [],
    }
    population = None
    guard = None
    try:
        if args.spain_fixture is None:
            raise ClassifiedFailure("SETUP_FAILED", "Spain requires --spain-fixture")
        fixture = json.loads(args.spain_fixture.read_text())
        world_source = Path(fixture["world"]).resolve()
        if not world_source.is_dir():
            raise ClassifiedFailure("SETUP_FAILED", f"Spain staged world missing: {world_source}")
        if any(binary is None or not binary.is_file() or not os.access(binary, os.X_OK)
               for binary in (args.server_bin, args.bots_bin)):
            raise ClassifiedFailure("INFRA_BLOCKED", "Spain requires explicit executable server and bot binaries")
        if shutil.disk_usage(args.out).free < 60 * 1024**3:
            raise ClassifiedFailure("INFRA_BLOCKED", "Spain launch requires at least 60 GiB free")
        registry_path = Path(fixture["embedded_registry"]).resolve()
        registry = json.loads(registry_path.read_text())
        reports_path = args.vanilla_data_dir / "reports" / "registries.json"
        reports = json.loads(reports_path.read_text())
        ids = {}
        for item in ("minecraft:wheat", "minecraft:bread"):
            embedded = registry["minecraft:item"]["entries"][item]["protocol_id"]
            reported = reports["minecraft:item"]["entries"][item]["protocol_id"]
            if type(embedded) is not int or embedded != reported:
                raise ClassifiedFailure("INFRA_BLOCKED", f"Spain embedded and runtime item identity disagree for {item}")
            ids[item] = embedded
        version = json.loads((args.vanilla_data_dir / "version.json").read_text())
        if version.get("id") != "26.3" or version.get("protocol_version") != 777:
            raise ClassifiedFailure("INFRA_BLOCKED", "Spain expects the explicitly pinned 26.3/777 content")
        route_sources = (fixture["source_route"], fixture["destination_route"])
        routes = [json.loads(Path(path).read_text())["waypoints"] for path in route_sources]
        if any(not isinstance(route, list) or len(route) < 2 for route in routes):
            raise ClassifiedFailure("SETUP_FAILED", "Spain waypoint fixture is incomplete")
        if routes[0][-1] != routes[1][0]:
            raise ClassifiedFailure("SETUP_FAILED", "Spain proposed route legs do not connect")
        world = args.out / "world"
        shutil.copytree(world_source, world)
        if shutil.disk_usage(args.out).free < 60 * 1024**3:
            raise ClassifiedFailure("INFRA_BLOCKED", "Spain world copy breached the 60 GiB reserve")
        task = asyncio.current_task()
        if task is None:
            raise ClassifiedFailure("INFRA_BLOCKED", "Spain disk guard requires an asyncio task")
        guard = WorldDiskGuard(world, 60 * 1024**3, task)
        result.update(seed=100, world=str(world), staged_world=str(world_source), fixture=fixture,
                      registry_ids=ids, data_version=version,
                      binaries={"server": {"path": str(args.server_bin), "sha256": scale.binary_sha256(args.server_bin)},
                                "bots": {"path": str(args.bots_bin), "sha256": scale.binary_sha256(args.bots_bin)}},
                      registry_sha256=scale.binary_sha256(registry_path),
                      report_registry_sha256=scale.binary_sha256(reports_path))
        args.seed = 100
        config_path, dashboard, game = config(args, args.out, world, 8, operators=False)
        population = SpainPopulation(args, args.out)
        await population.start_server([str(args.server_bin), "--config", str(config_path), "--no-console"],
                                      dashboard, args.readiness_seconds)
        owner = None
        worker = fixture["farmer_uuid"]
        second_worker = fixture["second_farmer_uuid"]
        try:
            worker = str(uuid.UUID(worker))
            second_worker = str(uuid.UUID(second_worker))
        except (ValueError, TypeError, AttributeError) as error:
            raise ClassifiedFailure("SETUP_FAILED", "Spain requires two valid farmer identities") from error
        if worker == second_worker:
            raise ClassifiedFailure("SETUP_FAILED", "Spain concurrent farmer targets must differ")
        for stage, (prefix, size, actors) in enumerate((("spain1", 2, (0,)), ("spain2", 2, (0,)), ("spain3", 4, (0, 2))), 1):
            if shutil.disk_usage(args.out).free < 60 * 1024**3:
                raise ClassifiedFailure("INFRA_BLOCKED", "Spain disk reserve reached before cohort admission")
            cohort_dir = args.out / f"cohort-{stage}"
            cohort_dir.mkdir()
            containers, hauls = [], []
            for bot in actors:
                ident = f"bread-{stage}-{bot}"
                source, destination = f"source-{bot}", f"destination-{bot}"
                containers.extend([{"name": source, "position": fixture["source_chest"], "bot": bot, "slots": 27},
                                   {"name": destination, "position": fixture["destination_chest"], "bot": bot, "slots": 27}])
                hauls.append({"id": ident, "defer_start": True, "bot": bot, "observer_bot": bot + 1,
                              "player_uuid": _offline_uuid(f"{prefix}_{bot:04}"), "source": source,
                              "destination": destination, "item": "minecraft:wheat", "count": 3,
                              "source_route": routes[0], "destination_route": routes[1],
                              "farmer_delivery": {"farmer_uuid": second_worker if stage == 3 and bot == 2 else worker},
                              "timeout_ms": 300000})
            # These probes only expose attacker identities actually received by a bot.
            # An unprobed, unobserved or evicted entity remains unknown.
            attack_kinds = ("bee", "bogged", "cave_spider", "creeper", "drowned", "enderman",
                            "husk", "iron_golem", "phantom", "pillager", "skeleton", "spider",
                            "stray", "vindicator", "witch", "wither_skeleton", "wolf", "zombie",
                            "zombie_villager")
            entities = [{"kind": "minecraft:player", "count": 8, "reader_images": True},
                        {"kind": "minecraft:villager", "count": 16, "reader_images": True},
                        {"kind": "minecraft:item", "count": 128, "reader_images": True}]
            entities.extend({"kind": f"minecraft:{kind}", "count": 32, "reader_images": True}
                            for kind in attack_kinds)
            spec = {"schema_version": 1, "entities": entities, "containers": containers, "hauls": hauls}
            spec_path, schedule_path = cohort_dir / "mechanic-spec.json", cohort_dir / "schedule.json"
            scale.dump(spec_path, spec)
            scale.dump(schedule_path, {"windows": [{"name": "steady", "duration_ms": 7200000, "movement_hz": 20}]})
            command = bot_command(args, cohort_dir, game, size) + ["--name-prefix", prefix, "--unassisted-login",
                       "--open-loop", "--schedule-file", str(schedule_path), "--mechanic-spec", str(spec_path)]
            cohort = await population.add_cohort(command, size, args.readiness_seconds)
            if owner is None:
                owner = cohort
                await owner.command(1, "claim", "settlement claim", f"Claimed {fixture['settlement']}", args.readiness_seconds)
            stats = await population.server.http(dashboard + "/stats", 5)
            connected = stats.get("players", {}).get("count")
            if connected != sum(population.sizes):
                raise ClassifiedFailure("CORRECTNESS_FAIL", f"Spain connected roster mismatch: {connected}")
            result["cohorts"].append({"stage": stage, "connected": connected, "actors": len(actors), "command": command})
            scale.dump(args.out / "result.json", result)
            candidates = cohort.ready_villagers()
            if any(haul["farmer_delivery"]["farmer_uuid"] not in candidates for haul in hauls):
                raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain pursuit target was not in the initial observed roster")
            baselines = {}
            for index, candidate in enumerate(candidates):
                baseline, event = await owner.json_command(
                    1, f"stage-{stage}-baseline-{index}",
                    f"settlement production {fixture['settlement']} carry {candidate}", min(args.readiness_seconds, 15))
                if baseline.get("worker") != candidate:
                    raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain candidate carry identity mismatch")
                baselines[candidate] = (baseline, event)
            result.setdefault("pre_drop_baselines", []).append({
                "stage": stage, "scope": "all villagers in this cohort's ready image; later unseen collectors fail closed",
                "candidates": {candidate: {"carry": baseline, "reply": event}
                               for candidate, (baseline, event) in baselines.items()},
            })
            scale.dump(args.out / "result.json", result)
            starts = await run_concurrent([
                cohort.start_haul(haul["bot"], haul["id"], args.readiness_seconds) for haul in hauls
            ])
            result["cohorts"][-1]["haul_starts"] = starts
            claimed_workers: set[str] = set()
            cycles = await run_concurrent([
                population.bread_cycle(
                    cohort, owner, haul, haul["id"], 1, fixture["settlement"], fixture["building"],
                    fixture["station"], fixture["stand"], ids["minecraft:wheat"], ids["minecraft:bread"],
                    baselines, claimed_workers, dashboard, 1200, 900)
                for haul in hauls
            ])
            if len(cycles) > 1 and max(start["harness_received_unix_ns"] for start in starts) >= min(
                    cohort.observed_ns(cycle["drop"]) for cycle in cycles):
                raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain concurrent hauls lacked overlapping admitted work")
            result["cycles"].extend(cycles)
            scale.dump(args.out / "result.json", result)
        pre_restart_carry = {}
        for index, worker in enumerate(sorted({cycle["collector_worker"] for cycle in result["cycles"]})):
            carry, event = await owner.json_command(
                1, f"pre-restart-carry-{index}", f"settlement production {fixture['settlement']} carry {worker}",
                args.readiness_seconds)
            if carry.get("worker") != worker:
                raise ClassifiedFailure("CORRECTNESS_FAIL", "Spain pre-restart carry identity mismatch")
            pre_restart_carry[worker] = {"carry": carry, "reply": event}
        result["pre_restart_carry"] = pre_restart_carry
        scale.dump(args.out / "result.json", result)
        before_restart = await population.stop()
        result["pre_restart_shutdown"] = before_restart
        population = None
        shutdown_rows = [before_restart["server"].get("server", {})]
        shutdown_rows.extend(row.get("bots", {}) for row in before_restart["cohorts"])
        if any(row.get("forced") or row.get("returncode") != 0 for row in shutdown_rows):
            raise ClassifiedFailure("INFRA_BLOCKED", "Spain pre-restart shutdown was not clean")
        restart_dir = args.out / "restart"
        restart_dir.mkdir()
        restart_config, restart_dashboard, restart_game = config(args, restart_dir, world, 2, operators=False)
        population = SpainPopulation(args, restart_dir)
        await population.start_server([str(args.server_bin), "--config", str(restart_config), "--no-console"],
                                      restart_dashboard, args.readiness_seconds)
        restart_cohort_dir = restart_dir / "cohort-1"
        restart_cohort_dir.mkdir()
        restart_spec = restart_cohort_dir / "mechanic-spec.json"
        restart_schedule = restart_cohort_dir / "schedule.json"
        scale.dump(restart_spec, {"schema_version": 1})
        scale.dump(restart_schedule, {"windows": [{"name": "steady", "duration_ms": 7200000}]})
        restart_command = bot_command(args, restart_cohort_dir, restart_game, 2) + [
            "--name-prefix", "spain1", "--unassisted-login", "--open-loop",
            "--schedule-file", str(restart_schedule), "--mechanic-spec", str(restart_spec)]
        restart_owner = await population.add_cohort(restart_command, 2, args.readiness_seconds)
        result["restart"] = {"scope": "same world and owner identity after clean server shutdown",
                             "command": restart_command, "jobs": []}
        for cycle in result["cycles"]:
            job_id = cycle["haul_id"]
            persisted, event = await restart_owner.json_command(
                1, f"{job_id}-restart-job", f"settlement production {fixture['settlement']} job {job_id}",
                args.readiness_seconds)
            failures = check_persisted_job(cycle["job"], persisted)
            result["restart"]["jobs"].append({"job_id": job_id, "job": persisted, "reply": event,
                                               "failures": failures})
            scale.dump(args.out / "result.json", result)
            if failures:
                raise ClassifiedFailure("CORRECTNESS_FAIL", f"Spain restart evidence: {failures}")
        result["restart"]["carries"] = []
        for index, (worker, original) in enumerate(pre_restart_carry.items()):
            carry, event = await restart_owner.json_command(
                1, f"restart-carry-{index}", f"settlement production {fixture['settlement']} carry {worker}",
                args.readiness_seconds)
            failures = check_persisted_carry(original["carry"], carry)
            result["restart"]["carries"].append({"worker": worker, "carry": carry, "reply": event,
                                                  "failures": failures})
            scale.dump(args.out / "result.json", result)
            if failures:
                raise ClassifiedFailure("CORRECTNESS_FAIL", f"Spain restart carry evidence: {failures}")
        result["outcome"] = "ASSISTED_CHAIN_ONLY"
        result["reason"] = "Four assisted chains, including one concurrent pair, are not autonomous extraction, industrial swarm, or a performance measurement."
    except asyncio.CancelledError:
        if guard is None or guard.reason is None:
            raise
        result["outcome"], result["reason"] = "INFRA_BLOCKED", guard.reason
    except (ClassifiedFailure, OSError, RuntimeError, ValueError, KeyError, TypeError, asyncio.TimeoutError) as error:
        result["outcome"] = error.outcome if isinstance(error, ClassifiedFailure) else "INFRA_BLOCKED"
        result["reason"] = f"{type(error).__name__}: {error}"
    finally:
        if population is not None:
            result["shutdown"] = await population.stop()
        if guard is not None:
            result["disk_guard"] = guard.close()
        result["wall_seconds"] = time.monotonic() - started
        scale.dump(args.out / "result.json", result)
    print(json.dumps({"event": "spain-final", "outcome": result["outcome"], "receipt": str(args.out / "result.json")}), flush=True)
    return 1
