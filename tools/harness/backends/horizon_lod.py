"""Protocol-level acceptance of the `horizon:lod` far-terrain service (owner D64).

Written from the owner decision and Horizon's own client source
(`tools/harness/horizon_lod_wire.py` ports `lod_net.rs`), never from the
server implementation. The probe is a Horizon-like client (protocol 777, brand
`vanilla`, Horizon's request discipline) against a real server on a world
pregenerated out past the tested distance, because the service answers from
STORED chunks only.

Server runs, each on the same pregenerated world:

* config validation: `[horizon_lod] enabled = true` with `distance_chunks`
  outside its range must refuse to start with a message naming the key;
* ON (view distance 4, `distance_chunks` 16): one startup WARN about the seed;
  the Offer arrives in configuration, decodes and carries the configured
  distance; a vanilla-like client that ignores the Offer stays connected;
  tiles beyond the view distance decode with Horizon's checks and carry the
  stored terrain (compared with the chunks a second connection receives from
  the same stored world, exact top-block state per column); a tile over
  never-generated ground carries no invented terrain; tiles past
  `distance_chunks`, misaligned tiles and non-power-of-two strides are never
  served;
* OFF twice (section absent, and `enabled = false`): no Offer, no seed WARN,
  requests answered with nothing.

Every check is recorded; the profile fails closed when any fails or a step
raises. On a server without the service the first failing checks are the
missing Offer and the missing tiles.
"""

from __future__ import annotations

import argparse
import uuid
import json
import re
import subprocess
from pathlib import Path
from typing import Any

from .. import horizon_lod_wire as hz
from .. import runtime, wire
from ..profiles import ProfileContext

SEED_DEFAULT = 42
MIN_Y = -64  # overworld; the chunk sections' lowest block
WORLD_SPAWN_CHUNK_RADIUS = 14  # the oracle connection's view distance (a circle, and its spawn may differ by a few blocks)


class Checks:
    """Named checks that all run; the profile fails when any failed."""

    def __init__(self) -> None:
        self.rows: dict[str, dict[str, Any]] = {}

    def __call__(self, name: str, passed: bool, detail: Any = None) -> bool:
        self.rows[name] = {"passed": bool(passed), "detail": detail}
        return bool(passed)

    def guard(self, name: str, step) -> Any:
        """Run `step`; an exception is a failed check, not an abort of the later ones."""
        try:
            return step()
        except Exception as error:  # noqa: BLE001 - every failure is evidence
            self(name, False, f"{type(error).__name__}: {error}")
            return None

    def failed(self) -> list[str]:
        return [name for name, row in self.rows.items() if not row["passed"]]


def validate_args(ctx: ProfileContext) -> tuple[argparse.Namespace, float]:
    parser = argparse.ArgumentParser(prog="horizon-lod")
    parser.add_argument("--seed", type=int, default=SEED_DEFAULT)
    parser.add_argument("--distance-chunks", type=int, default=16)
    parser.add_argument("--view-distance", type=int, default=4)
    parser.add_argument("--pregenerate-radius", type=int, default=176,
                        help="block radius of the stored area around spawn")
    try:
        args = parser.parse_args(ctx.extra_args)
    except SystemExit as error:
        raise ValueError("invalid horizon-lod arguments") from error
    # Tiles 5..8 chunks out are compared; 15 chunks out must be unstored; 30 is past the distance.
    if not 1 <= args.view_distance <= 4:
        raise ValueError("--view-distance must be 1..4 (tiles are compared 5..8 chunks out)")
    if args.distance_chunks < 16 or args.distance_chunks > 20:
        raise ValueError("--distance-chunks must be 16..20 (the unstored tile sits 15 chunks out, "
                         "the refused one 30 out)")
    if args.pregenerate_radius < 8 * 16 + 16 or args.pregenerate_radius > 13 * 16:
        raise ValueError("--pregenerate-radius must store 8 chunks out and leave 15 unstored")
    if ctx.platform is not None or ctx.plugins_dir is not None:
        raise ValueError("horizon-lod runs the bare server without Loader/plugins")
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else 900.0
    if not timeout > 0:
        raise ValueError("timeout must be positive")
    return args, timeout


def server_config(*, port: int, world: Path, seed: int, view_distance: int,
                  horizon_lod: str | None) -> str:
    text = f"""[server]
name = "horizon-lod-acceptance"
view_distance = {view_distance}
simulation_distance = {min(view_distance, 4)}

[network]
bind_address = "127.0.0.1"
port = {port}

[auth]
online_mode = false

[data]
world_dir = {json.dumps(str(world))}
vanilla_data_dir = "data/vanilla"
seed = {seed}
"""
    return text + (horizon_lod or "")


class Server:
    """One server process on the shared world, with its own log."""

    def __init__(self, ctx: ProfileContext, binary: Path, artifact: Path, label: str,
                 config: str, port: int):
        self.ctx, self.label, self.port = ctx, label, port
        self.config_path = artifact / f"{label}.toml"
        self.log_path = artifact / f"{label}.log"
        self.config_path.write_text(config)
        self.argv = [str(binary), "--config", str(self.config_path)]
        self.process = None
        self.log = None

    def start(self) -> None:
        self.log = self.log_path.open("wb")
        self.ctx.commands.append(self.argv)
        self.ctx.cwds.append(str(runtime.REPO_ROOT))
        self.ctx.exits.append(None)
        self.ctx.logs.append(str(self.log_path.relative_to(runtime.REPO_ROOT))
                             if self.log_path.is_relative_to(runtime.REPO_ROOT) else str(self.log_path))
        self.process = runtime.start_server(
            self.argv, cwd=runtime.REPO_ROOT, stdin=subprocess.DEVNULL, stdout=self.log,
            stderr=subprocess.STDOUT, start_new_session=True)

    def wait_ready(self, timeout: float) -> None:
        runtime.wait_port(self.port, timeout, self.process, log_path=self.log_path)

    def stop(self) -> int | None:
        runtime.stop_process(self.process)
        if self.log is not None:
            self.log.close()
        if self.process is not None:
            self.ctx.exits[self.ctx.commands.index(self.argv)] = self.process.returncode
            return self.process.returncode
        return None

    def text(self) -> str:
        return self.log_path.read_text(errors="replace")


def seed_warnings(text: str) -> list[str]:
    """WARN lines about the seed that name Horizon (the contract: one startup WARN that far tiles
    help recover the world seed)."""
    return [line for line in text.splitlines()
            if re.search(r"warn", line, re.I) and "seed" in line.lower() and "horizon" in line.lower()]


def _spawn_chunk(position: list[float]) -> tuple[int, int]:
    return int(position[0] // 16), int(position[2] // 16)


def _tile_origin_chunks(tile_chunk: tuple[int, int]) -> tuple[int, int]:
    return tile_chunk[0] * 16, tile_chunk[1] * 16


def beyond_chunks(first: tuple[int, int], stride: int, spawn: tuple[int, int], view: int) -> int:
    """How many of a stride-wide tile's chunks lie beyond the view distance of the spawn chunk."""
    return sum(max(abs(first[0] + i - spawn[0]), abs(first[1] + j - spawn[1])) > view
               for i in range(stride) for j in range(stride))


def aligned_tile(stride: int, spawn: tuple[int, int], view: int, reach: int) -> tuple[int, int]:
    """The aligned tile of `stride` (a tile is `stride` chunks wide) with the most chunks beyond
    the view distance, among those wholly within `reach` chunks of the spawn chunk (the oracle's
    coverage) and at least half beyond."""
    best = None
    for index_x in range(-64, 64):
        for index_z in range(-64, 64):
            first = (index_x * stride, index_z * stride)
            far = max(max(abs(first[0] + i - spawn[0]), abs(first[1] + j - spawn[1]))
                      for i in range(stride) for j in range(stride))
            beyond = beyond_chunks(first, stride, spawn, view)
            if far <= reach and beyond * 2 >= stride * stride and (best is None or beyond > best[0]):
                best = (beyond, first)
    if best is None:
        raise ValueError(f"no stride-{stride} tile fits between view {view} and reach {reach}")
    return best[1]


class Oracle:
    """The stored world as a plain protocol client receives it (protocol 777 block-state ids)."""

    def __init__(self, observer):
        self.observer = observer

    def state(self, x: int, y: int, z: int) -> int:
        from ..protocol_chunks import block_state
        return block_state(self.observer.chunk_sections[(x >> 4, z >> 4)], (x, y, z), MIN_Y)

    def top(self, x: int, z: int, skip: set[int]) -> int | None:
        """One above the highest block that is neither air nor in `skip`."""
        sections = self.observer.chunk_sections[(x >> 4, z >> 4)]
        for y in range(MIN_Y + 16 * len(sections) - 1, MIN_Y - 1, -1):
            if self.state(x, y, z) not in skip:
                return y + 1
        return None


def compare_tile(reply: hz.Reply, request: hz.Request, oracle: Oracle,
                 water_states: set[int]) -> dict[str, Any]:
    """Per-column comparison of a stored tile with the received chunks."""
    skip = {0, *water_states}
    columns = len(reply.columns)
    with_surface = exact_state = near_top = loose = watered = water_match = 0
    heights = set()
    stride = request.stride
    for index, column in enumerate(reply.columns):
        wx = request.x + (index % hz.SIDE) * stride
        wz = request.z + (index // hz.SIDE) * stride
        if column.surface is None:
            continue
        with_surface += 1
        surface = column.surface
        heights.add(surface.height)
        footprint = [oracle.top(wx + dx, wz + dz, skip) for dx in range(stride) for dz in range(stride)]
        footprint = [h for h in footprint if h is not None]
        if stride == 1:
            exact_state += oracle.state(wx, surface.height - 1, wz) == surface.state
            near_top += bool(footprint) and abs(surface.height - footprint[0]) <= 2
            if surface.water is not None:
                watered += 1
                water_match += oracle.state(wx, surface.water[0] - 1, wz) == surface.water[1]
        else:
            loose += bool(footprint) and min(footprint) - 1 <= surface.height <= max(footprint) + 1
    return {"tile": [request.x, request.z, request.stride], "columns": columns,
            "with_surface": with_surface, "distinct_heights": len(heights),
            "exact_top_state": exact_state, "top_within_2": near_top,
            "watered": watered, "water_state_matches": water_match, "footprint_loose": loose}


def _fraction(part: int, whole: int) -> float:
    return part / whole if whole else 0.0


def run(ctx: ProfileContext) -> dict[str, Any]:
    args, timeout = validate_args(ctx)
    from ..profiles import _server_binary

    artifact = ctx.artifact_dir
    artifact.mkdir(parents=True, exist_ok=True)
    binary = _server_binary(ctx, "server-build.log")
    world = artifact / "world"
    world.mkdir(parents=True, exist_ok=True)
    checks = Checks()
    report: dict[str, Any] = {
        "scope": "horizon:lod protocol: Offer, stored-terrain tiles beyond view distance, refusals, off switch",
        "server_binary": str(binary), "seed": args.seed, "distance_chunks": args.distance_chunks,
        "view_distance": args.view_distance, "pregenerate_radius": args.pregenerate_radius,
        "checks": checks.rows,
    }
    ctx.client_evidence["horizon_lod"] = report
    enabled_toml = (f"\n[horizon_lod]\nenabled = true\ndistance_chunks = {args.distance_chunks}\n")

    def server_for(label: str, view: int, horizon_lod: str | None) -> Server:
        port = runtime.reserve_port()
        return Server(ctx, binary, artifact, label,
                      server_config(port=port, world=world, seed=args.seed, view_distance=view,
                                    horizon_lod=horizon_lod), port=port)

    # -- the stored world: pregenerate out past everything compared (stopped world, own lease)
    with runtime.phase("server_start"):
        pregen = server_for("pregenerate", args.view_distance, None)
        argv = [*pregen.argv, "world", "pregenerate", "--radius", str(args.pregenerate_radius)]
        ctx.commands.append(argv)
        ctx.cwds.append(str(runtime.REPO_ROOT))
        ctx.exits.append(None)
        ctx.logs.append(str(pregen.log_path))
        with pregen.log_path.open("wb") as log:
            result = subprocess.run(argv, cwd=runtime.REPO_ROOT, stdout=log,
                                    stderr=subprocess.STDOUT, timeout=timeout)
        ctx.exits[-1] = result.returncode
        text = pregen.text()
        checks("world_pregenerated", result.returncode == 0 and "pregenerate complete" in text,
               {"exit": result.returncode, "log": str(pregen.log_path)})
        if not checks.rows["world_pregenerated"]["passed"]:
            raise RuntimeError("pregeneration failed; the stored world is the fixture")

    # -- configuration validation
    for bad in (0, 2_000_000):
        label = f"invalid-distance-{bad}"
        server = server_for(label, args.view_distance,
                            f"\n[horizon_lod]\nenabled = true\ndistance_chunks = {bad}\n")
        server.start()
        try:
            try:
                server.wait_ready(120.0)
                checks(f"invalid_distance_{bad}_rejected", False,
                       "server started with an out-of-range distance_chunks")
            except runtime.ChildExited as error:
                text = server.text().lower()
                named = "distance_chunks" in text or "horizon_lod" in text
                checks(f"invalid_distance_{bad}_rejected", error.returncode != 0 and named,
                       {"exit": error.returncode, "names_the_key": named,
                        "log_tail": text[-400:]})
        finally:
            server.stop()

    # -- ON: the protocol probe, then a plain client for the stored-world oracle
    on = server_for("on", args.view_distance, enabled_toml)
    on.start()
    tiles: dict[str, Any] = {}
    spawn_chunk = None
    spawn_position = None
    water_states: set[int] = set()
    requests: list[hz.Request] = []
    replies: list[hz.Reply] = []
    try:
        on.wait_ready(timeout)
        warns = seed_warnings(on.text())
        checks("startup_warns_about_the_seed", len(warns) == 1, warns)

        client = hz.HorizonLikeClient(("127.0.0.1", on.port), "HorizonProbe",
                                      view_distance=args.view_distance)
        try:
            with runtime.phase("join"):
                client.join()
                client.wait_ready()
            payloads = client.offer_payloads()
            offer = hz.decode_offer(payloads[0]) if len(payloads) == 1 else None
            checks("offer_in_configuration", offer is not None,
                   {"horizon_lod_payloads": len(payloads), "channels": client.configuration_channels()})
            if offer is not None:
                checks("offer_matches_configuration",
                       offer == hz.Offer(args.distance_chunks, hz.IN_FLIGHT, hz.PER_SECOND),
                       {"offer": offer.__dict__, "configured_distance_chunks": args.distance_chunks})
            spawn_position = list(client.position)
            spawn_chunk = _spawn_chunk(spawn_position)
            report["spawn"] = {"position": spawn_position, "chunk": list(spawn_chunk)}

            sx, sz = spawn_chunk
            view = args.view_distance
            reach = 8
            stride1 = [(sx + 5, sz), (sx + 6, sz - 3), (sx - 7, sz + 2), (sx + 1, sz + 7),
                       (sx - 4, sz - 8), (sx - 8, sz - 6)]
            stride2 = aligned_tile(2, spawn_chunk, view, reach)
            stride4 = aligned_tile(4, spawn_chunk, view, reach)
            wanted = ([(cx * 16, cz * 16, 1) for cx, cz in stride1]
                      + [(stride2[0] * 16, stride2[1] * 16, 2),
                         (stride4[0] * 16, stride4[1] * 16, 4)])
            report["tiles_requested"] = wanted
            checks("tiles_are_beyond_the_view_distance",
                   all(max(abs(cx - sx), abs(cz - sz)) > view for cx, cz in stride1)
                   and all(beyond_chunks(first, size, spawn_chunk, view) * 2 >= size * size
                           for first, size in ((stride2, 2), (stride4, 4))),
                   {"view_distance": view,
                    "rule": "stride-1 tiles wholly beyond; stride-2/4 tiles at least half beyond"})
            if offer is not None:
                answers = checks.guard("tiles_answered_and_decode", lambda: client.fetch(wanted))
                if answers is not None:
                    decoded = list(answers.values())
                    refused = [i for i, r in enumerate(decoded) if r is None or r.refused]
                    checks("tiles_answered_and_decode", not refused,
                           {"requested": len(wanted), "refused_indexes": refused})
                    for (x, z, stride), reply in zip(wanted, decoded):
                        requests.append(hz.Request(0, x, z, stride))
                        replies.append(reply)
                        water_states.update(c.surface.water[1] for c in reply.columns
                                            if c.surface is not None and c.surface.water)
                    report["reply_shapes"] = [
                        {"tile": [r.x, r.z, r.stride], "refused": rep.refused,
                         "columns_with_surface": sum(c.surface is not None for c in rep.columns)}
                        for r, rep in zip(requests, replies)]

                # never-generated ground: 15 chunks out, inside distance_chunks, outside the stored area
                unstored = checks.guard(
                    "unstored_tile_answered",
                    lambda: client.fetch([((sx + 15) * 16, sz * 16, 1)]))
                if unstored is not None:
                    reply = next(iter(unstored.values()))
                    invented = sum(not c.empty for c in reply.columns) if not reply.refused else 0
                    checks("unstored_tile_carries_no_invented_terrain", invented == 0,
                           {"refused": reply.refused, "columns_with_terrain": invented})

                # past distance_chunks (and its slack), misaligned, and non-power-of-two strides
                def refused_or_silent(label: str, x: int, z: int, stride: int) -> None:
                    bad = client.request(x, z, stride)
                    good = client.request((sx + 5) * 16, sz * 16, 1)
                    client.pump_until(lambda: good in client.replies)
                    served = hz.decode_reply(client.replies.pop(good))
                    answer = client.replies.pop(bad, None)
                    carries = (answer is not None and not hz.decode_reply(answer).refused
                               and any(not c.empty for c in hz.decode_reply(answer).columns))
                    checks(label, not served.refused and not carries,
                           {"good_tile_served": not served.refused, "bad_tile_answered": answer is not None,
                            "bad_tile_carries_terrain": carries})

                checks.guard("tile_past_distance_not_served",
                             lambda: refused_or_silent("tile_past_distance_not_served",
                                                       (sx + 30) * 16, sz * 16, 1))
                checks.guard("misaligned_tile_not_served",
                             lambda: refused_or_silent("misaligned_tile_not_served",
                                                       (sx + 5) * 16 + 1, sz * 16, 1))
                checks.guard("non_power_of_two_stride_not_served",
                             lambda: refused_or_silent("non_power_of_two_stride_not_served",
                                                       (sx + 5) * 16, sz * 16, 3))
        except Exception as error:  # noqa: BLE001
            checks("horizon_like_probe", False, f"{type(error).__name__}: {error}")
        finally:
            client.close()

        # A vanilla-like client ignores the Offer and must stay connected through configuration and play.
        def vanilla_like() -> None:
            sock, joined = wire.join_offline_socket(("127.0.0.1", on.port), "VanillaProbe", timeout=120.0)
            try:
                wire.wait_play_loop(sock, timeout=120.0)
            finally:
                sock.close()
            checks("vanilla_like_client_stays_connected", True, {"entity_id": joined["entity_id"]})

        checks.guard("vanilla_like_client_stays_connected", vanilla_like)
    finally:
        on_exit = on.stop()
    checks("on_server_clean_exit", on_exit == 0, on_exit)

    # -- the oracle: the same stored world, view distance 12 (a circle that covers every chunk within
    # 8 of spawn), no service, a plain protocol-777 client that keeps the chunks it is sent
    if spawn_chunk is not None and replies:
        oracle_server = server_for("oracle", WORLD_SPAWN_CHUNK_RADIUS, None)
        oracle_server.start()
        try:
            oracle_server.wait_ready(timeout)
            observer = hz.HorizonLikeClient(("127.0.0.1", oracle_server.port), "OracleProbe",
                                            view_distance=WORLD_SPAWN_CHUNK_RADIUS, store_chunks=True,
                                            timeout=240.0)
            try:
                observer.join()
                observer.wait_ready()
                needed = {(cx, cz) for r in requests
                          for cx in range(r.x >> 4, (r.x + 16 * r.stride - 1 >> 4) + 1)
                          for cz in range(r.z >> 4, (r.z + 16 * r.stride - 1 >> 4) + 1)}
                observer.pump_until(lambda: needed <= observer.chunk_sections.keys())
                checks("oracle_received_every_tile_chunk", needed <= observer.chunk_sections.keys(),
                       {"chunks": len(needed), "oracle_spawn": observer.position, "probe_spawn": spawn_position,
                        "note": "the server spreads spawns, so the two joins start at different points; "
                                "tiles are compared by absolute coordinates"})
                oracle = Oracle(observer)
                stats = [compare_tile(reply, request, oracle, water_states)
                         for request, reply in zip(requests, replies) if not reply.refused]
                report["tile_comparison"] = stats
                fine = [s for s in stats if s["tile"][2] == 1]
                coarse = [s for s in stats if s["tile"][2] != 1]
                columns = sum(s["columns"] for s in fine)
                checks("stored_tiles_have_terrain",
                       _fraction(sum(s["with_surface"] for s in fine), columns) >= 0.99,
                       {"columns": columns, "with_surface": sum(s["with_surface"] for s in fine)})
                checks("stored_top_block_matches_the_world",
                       _fraction(sum(s["exact_top_state"] for s in fine), columns) >= 0.95,
                       {"columns": columns, "exact": sum(s["exact_top_state"] for s in fine),
                        "rule": ">=95% of columns: the block below the reported top face is exactly the "
                                "reported state in the stored chunk"})
                checks("stored_surface_height_matches_the_world",
                       _fraction(sum(s["top_within_2"] for s in fine), columns) >= 0.80,
                       {"columns": columns, "within_2": sum(s["top_within_2"] for s in fine),
                        "rule": ">=80% of columns within 2 blocks of the highest non-air, non-water block "
                                "(foliage and plants account for the slack)"})
                watered = sum(s["watered"] for s in fine)
                if watered:
                    checks("stored_water_matches_the_world",
                           _fraction(sum(s["water_state_matches"] for s in fine), watered) >= 0.90,
                           {"watered": watered, "match": sum(s["water_state_matches"] for s in fine)})
                checks("stored_terrain_is_not_flat",
                       len(set().union(*[{r.surface.height for r in rep.columns if r.surface}
                                         for rep in replies if not rep.refused])) >= 4,
                       "a constant or flat fake cannot pass; pick a seed with relief if this fails")
                checks("coarse_tiles_stay_inside_the_footprint",
                       bool(coarse) and all(_fraction(s["footprint_loose"], s["columns"]) >= 0.90
                                            for s in coarse), coarse)
            except Exception as error:  # noqa: BLE001
                checks("oracle_comparison", False, f"{type(error).__name__}: {error}")
            finally:
                observer.close()
        except Exception as error:  # noqa: BLE001
            checks("oracle_server", False, f"{type(error).__name__}: {error}")
        finally:
            oracle_server.stop()

    # Fail closed: a check that never ran is a failed check, not an absent one.
    for name in ("tiles_answered_and_decode", "unstored_tile_carries_no_invented_terrain",
                 "tile_past_distance_not_served", "misaligned_tile_not_served",
                 "non_power_of_two_stride_not_served", "stored_tiles_have_terrain",
                 "stored_top_block_matches_the_world", "stored_surface_height_matches_the_world",
                 "stored_terrain_is_not_flat", "coarse_tiles_stay_inside_the_footprint"):
        if name not in checks.rows:
            checks(name, False, "not reached: no Offer, or no tile was served")

    # -- OFF: absent section, then explicit enabled = false
    for label, toml in (("off-absent", None),
                        ("off-explicit",
                         f"\n[horizon_lod]\nenabled = false\ndistance_chunks = {args.distance_chunks}\n")):
        server = server_for(label, args.view_distance, toml)
        server.start()
        try:
            server.wait_ready(timeout)
            checks(f"{label}_no_seed_warning", not seed_warnings(server.text()), seed_warnings(server.text()))
            client = hz.HorizonLikeClient(("127.0.0.1", server.port), "HorizonProbe",
                                          view_distance=args.view_distance)
            try:
                client.join()
                client.wait_ready()
                checks(f"{label}_no_offer", not client.offer_payloads(),
                       {"channels": client.configuration_channels()})
                sx, sz = _spawn_chunk(client.position)
                client.request((sx + 5) * 16, sz * 16, 1)
                client.next_keepalive()  # everything the server wrote before its next KeepAlive was read
                checks(f"{label}_requests_unanswered", not client.replies,
                       {"replies": sorted(client.replies)})
            finally:
                client.close()
        except Exception as error:  # noqa: BLE001
            checks(f"{label}_probe", False, f"{type(error).__name__}: {error}")
        finally:
            server.stop()

    failed = checks.failed()
    report["failed_checks"] = failed
    if failed:
        raise RuntimeError("horizon-lod acceptance failed: " + ", ".join(failed))
    return report
