"""A world holding block entities the core cannot decode must not stop the server.

Each fixture block sits alone in its own chunk. After a clean stop the exported
Anvil world gets one field of each record set to a value the owner of that block
entity refuses, the world is imported again and the same player rejoins next to
those chunks. The server must refuse exactly those chunks (one error each), keep
ticking and keep serving the player; before the hydration owner, a corrupt
jukebox or shrieker panicked the tick thread. The refused chunks must not stop
checkpointing: the checkpoint after the refusal and the shutdown checkpoint both
commit.

Crafter and shelf records cannot take the Anvil route: the importer validates
them by their block and refuses the whole import. They arrive through the tick
WAL instead. After the first restart the scenario places a crafter and an oak shelf,
stops the server cleanly and publishes one forged WAL batch (wal_forge) whose
opaque block-entity images sit at those blocks under a foreign record id, so
replay's id-based validator lets them through; the next start must refuse
exactly those two chunks instead of panicking in their hydration owners. A conduit
record keeps a valid id and Target but repeats its `y` field: the importer reads the
first occurrence, the conduit owner demands exactly one.
"""
from __future__ import annotations

import re
from pathlib import Path
from typing import Any, Callable

from .anvil_corrupt import BYTE, COMPOUND, INT, LIST, LONG, STRING, Edit, Tag, field
from .native_v2 import _block, _echo, _point
from .wal_forge import committed_tick, network_compound, publish_opaque_batch
from .probe_steps import ProbeSteps

SCENARIO = "native-v2-corrupt-block-entities"


SIGN = (STRING, "minecraft:sign")
# A record the replay validator does not check by id, holding a slot index no 3x3 or 9-slot owner accepts.
BAD_ITEMS = (LIST, (COMPOUND, [[("Slot", (BYTE, 99)), ("id", (STRING, "minecraft:stone")), ("count", (INT, 1))]]))


def _edit(fields: dict[str, Tag]) -> Edit:
    def edit(compound: list[tuple[str, Tag]]) -> None:
        compound[:] = [(key, value) for key, value in compound if key not in fields]
        compound.extend(fields.items())
    return edit


def _repeat(name: str) -> Edit:
    """Append a second copy of an existing field: the record then repeats a durable key."""
    def edit(compound: list[tuple[str, Tag]]) -> None:
        compound.append((name, field(compound, name)))
    return edit


# block, setblock state (with data, so the chunk holds a record to corrupt), edit applied to the
# exported record, whether hydration must refuse the chunk, text the refusal must name.
# Crafter and shelf records are validated by the importer through their block (mc-world test
# block_entities_are_validated_by_their_block), so they take the WAL route (WAL_FIXTURES). A conduit
# keeps its id and Target, so the importer takes it, but the second `y` breaks the owner's exactly-one rule.
# A beacon record only has to be a compound; its undecodable-record panic cannot be reached, so the
# beacon fixture is the runtime-path control: an odd but decodable record is not refused and the
# beacon keeps running every 80 ticks.
FIXTURES: tuple[tuple[str, str, Edit, bool, str], ...] = (
    ("jukebox", 'minecraft:jukebox{RecordItem:{id:"minecraft:music_disc_13",count:1}}',
     _edit({"ticks_since_song_started": (LONG, -1)}), True, "jukebox"),
    ("sculk_shrieker", "minecraft:sculk_shrieker{warning_level:1}",
     _edit({"warning_level": (INT, 9)}), True, "sculk_shrieker"),
    ("beacon", 'minecraft:beacon{Levels:0}', _edit({"id": SIGN, "Levels": (STRING, "odd")}), False, "beacon"),
    ("conduit", "minecraft:conduit{Target:[I;1,2,3,4]}", _repeat("y"), True, "conduit"),
)

# Blocks that take the WAL route: block, text the refusal must name, the forged record's fields.
WAL_FIXTURES: tuple[tuple[str, str, list[tuple[str, Tag]]], ...] = (
    ("crafter", "crafter", [("id", SIGN), ("Items", BAD_ITEMS)]),
    ("oak_shelf", "shelf", [("id", SIGN), ("Items", BAD_ITEMS)]),
)

# Chunk offsets (x, z) from the player's chunk. The native client keeps its loading screen up until
# the chunks within about 12 blocks of the camera are known, so a refused chunk may not touch the
# player's chunk (a separate client finding); all stay inside the simulation distance of 4.
OFFSETS = ((2, 0), (-2, 0), (0, 2), (0, -2))
WAL_OFFSETS = ((2, 2), (-2, 2))


def _centres(origin: tuple[int, int, int], offsets: tuple[tuple[int, int], ...]) -> list[dict[str, int]]:
    chunk_x, chunk_z = origin[0] // 16, origin[2] // 16
    return [{"x": (chunk_x + dx) * 16 + 8, "y": origin[1], "z": (chunk_z + dz) * 16 + 8}
            for dx, dz in offsets]


def fixture_points(origin: tuple[int, int, int]) -> list[dict[str, int]]:
    return _centres(origin, OFFSETS)


def wal_points(origin: tuple[int, int, int]) -> list[dict[str, int]]:
    return _centres(origin, WAL_OFFSETS)


def corruptions(origin: tuple[int, int, int]) -> dict[tuple[int, int, int], Edit]:
    """The record edit per fixture position; the hook applies them to the exported world."""
    return {
        (point["x"], point["y"], point["z"]): edit
        for point, (_, _, edit, _, _) in zip(fixture_points(origin), FIXTURES)
    }


def _server_log(steps: ProbeSteps) -> str:
    path = getattr(steps.client, "server_log_path", None)
    if not isinstance(path, Path):
        raise RuntimeError("corrupt block entities require the harness server log path")
    return path.read_text(errors="replace")


def _gametime(steps: ProbeSteps, label: str) -> int:
    marker = f"gametime {label} {abs(hash(label)) % 10**6}"
    steps.call("minecraft_send_chat", {"message": marker, "command": False})
    steps.wait(lambda state: marker in "\n".join(state["recent_chat"]), label + "_marker")
    steps.fixture("time query gametime")
    found: list[int] = []

    def answered(state: dict[str, Any]) -> bool:
        for line in state["recent_chat"]:
            if marker in line:
                return False
            if match := re.search(r"(\d+)\s*$", line):
                found.append(int(match.group(1)))
                return True
        return False

    steps.wait(answered, label + "_answer")
    return found[0]


def _commits(log: str) -> int:
    return log.count("checkpoint committed")


def _declined(log: str) -> list[str]:
    return [line for line in log.splitlines()
            if "checkpoint skipped; journal retained" in line or "checkpoint declined" in line]


def _prepare(steps: ProbeSteps, origin: tuple[int, int, int],
             fixtures: tuple[tuple[str, str, Edit, bool, str], ...],
             points: list[dict[str, int]]) -> None:
    if not callable(getattr(steps.client, "p4_restart_server", None)):
        raise RuntimeError("corrupt block entities require the clean p4_restart_server hook")
    for point, (block, state, *_rest) in zip(points, fixtures):
        steps.fixture(f"setblock {point['x']} {point['y']} {point['z']} {state}")
        _block(steps, point, f"minecraft:{block}")
    _echo(steps, "fixtures_ordered")
    steps.report["corrupt_fixtures"] = [{**point, "block": block}
                                         for point, (block, *_rest) in zip(points, fixtures)]
    steps.client.corrupt_fixture = {
        (point["x"], point["y"], point["z"]): edit
        for point, (_, _, edit, _, _) in zip(points, fixtures)
    }


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    from .native_p4 import _restart

    points = fixture_points(origin)
    _prepare(steps, origin, FIXTURES, points)
    _restart(steps)
    start = _gametime(steps, "after_import_first")
    # Long enough for the beacon's 80-tick runtime pass over its odd record.
    steps.call("minecraft_wait_ticks", {"ticks": 100})
    steps.check("server_keeps_ticking_after_corrupt_chunks_load", _gametime(steps, "after_import_second") > start + 80)
    log = _server_log(steps)
    steps.check("server_never_panicked", "panicked" not in log)
    refusals = [line for line in log.splitlines() if "stored chunk unavailable" in line]
    steps.report["refusals"] = refusals
    refused_chunks = []
    for point, (block, _, _, refused, named) in zip(points, FIXTURES):
        chunk = f"chunk ({point['x'] // 16}, {point['z'] // 16})"
        mine = [line for line in refusals if chunk in line]
        if refused:
            refused_chunks.append(chunk)
            steps.check(f"{block}_chunk_refused_once_with_its_own_error",
                        len(mine) == 1 and named in mine[0].lower())
        else:
            steps.check(f"{block}_decodable_record_is_not_refused", not mine)
    steps.check("only_the_undecodable_chunks_are_refused", len(refusals) == len(refused_chunks))
    # A refused chunk never stops checkpointing: an explicit save after the refusal commits, is not
    # declined, and its one error names every refused chunk.
    before = _commits(log)
    steps.fixture("save-all")
    _echo(steps, "save_after_refusal_ordered")
    steps.wait(lambda _: _commits(_server_log(steps)) > before, "checkpoint_commits_after_refusal")
    log = _server_log(steps)
    steps.check("checkpoint_after_refusal_was_not_declined", not _declined(log))
    loud = [line for line in log.splitlines() if "checkpoint skipped quarantined chunks" in line]
    steps.report["checkpoint_refusal_errors"] = loud
    steps.check("checkpoint_error_lists_every_refused_chunk",
                bool(loud) and all(chunk in loud[-1] for chunk in refused_chunks)
                and loud[-1].count("unsaved entries") == len(refused_chunks))
    # The player's own chunk is intact: the platform under the player is still received.
    _block(steps, _point(origin, dy=-1), "minecraft:obsidian")
    # Crafter and shelf records cannot come through the importer. Place the blocks, then stop cleanly:
    # the hook publishes a forged WAL batch over the shutdown checkpoint (see wal_forge).
    wal = wal_points(origin)
    for point, (block, _, _) in zip(wal, WAL_FIXTURES):
        steps.fixture(f"setblock {point['x']} {point['y']} {point['z']} minecraft:{block}")
        _block(steps, point, f"minecraft:{block}")
    _echo(steps, "wal_fixtures_ordered")
    steps.report["wal_fixtures"] = [{**point, "block": block} for point, (block, _, _) in zip(wal, WAL_FIXTURES)]

    def forge(world: Path) -> Path:
        tick = committed_tick(_server_log(steps))
        records = {
            (point["x"], point["y"], point["z"]): network_compound(
                [*fields, ("x", (INT, point["x"])), ("y", (INT, point["y"])), ("z", (INT, point["z"]))])
            for point, (_, _, fields) in zip(wal, WAL_FIXTURES)
        }
        steps.report["forged_wal"] = {"after_checkpoint_tick": tick, "positions": [list(key) for key in records]}
        return publish_opaque_batch(world, tick, records)

    steps.client.wal_forge = forge
    # Clean shutdown: its checkpoint commits too (the log between the stop and the next start).
    stopped_at = len(_server_log(steps))
    steps.client.corrupt_fixture = None
    steps.call("minecraft_disconnect")
    address = steps.client.p4_restart_server(steps.remaining())
    steps.client.wal_forge = None
    steps.call("minecraft_connect", {"server_addr": address})
    play = steps.call("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})
    steps.check("second_restart_client_entered_play", play["in_play"] is True)
    shutdown, _, restarted = _server_log(steps)[stopped_at:].partition("Solaris is listening")
    steps.check("shutdown_checkpoint_committed_beside_refused_chunks", _commits(shutdown) >= 1)
    steps.check("shutdown_checkpoint_was_not_declined", not _declined(shutdown))

    def after_restart() -> str:
        return _server_log(steps)[stopped_at:].partition("Solaris is listening")[2]

    # No re-import happened: the restarted server meets the same stored records and refuses them again.
    wal_chunks = [f"chunk ({point['x'] // 16}, {point['z'] // 16})" for point in wal]
    steps.wait(lambda _: all(chunk in after_restart() for chunk in refused_chunks + wal_chunks),
               "refused_chunks_stay_refused_after_restart")
    refusals = [line for line in after_restart().splitlines() if "stored chunk unavailable" in line]
    steps.report["refusals_after_wal_replay"] = refusals
    for chunk, (block, named, _) in zip(wal_chunks, WAL_FIXTURES):
        mine = [line for line in refusals if chunk in line]
        steps.check(f"{block}_wal_record_refused_once_with_its_own_error",
                    len(mine) == 1 and named in mine[0].lower())
    steps.check("wal_replay_never_panicked", "panicked" not in after_restart())
    resumed = _gametime(steps, "after_wal_replay_first")
    steps.call("minecraft_wait_ticks", {"ticks": 100})
    steps.check("server_keeps_ticking_after_wal_refusals", _gametime(steps, "after_wal_replay_second") > resumed + 80)
    steps.report["limits"] = [
        "jukebox, shrieker and conduit records corrupted in an exported Anvil copy and re-imported; crafter "
        "and shelf records cannot take this route because the importer validates them by their block, so they "
        "reach the server as forged opaque images in a tick-WAL batch (wal_forge) published over a clean "
        "shutdown checkpoint",
        "the beacon record is decodable by construction: that fixture proves its runtime pass survives an "
        "odd record, not a refusal",
        "the WAL-refused chunks are refused after the second start; whether a later checkpoint declines "
        "beside them is not asserted here",
        "byte identity of a refused record is covered by the mc-core chunk_loader test, not here",
    ]


def run_near_camera(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    """Known-red client finding: a refused chunk under the spawn keeps the loading screen up.

    A vanilla client proceeds with a hole in the world; the native client never reaches in_play
    while a refused or missing chunk lies within about 12 blocks of the camera
    (play_world_presentation_ready needs the biome neighbourhood). The restart helper's own
    restart_client_entered_play check is the failing assertion.
    """
    from .native_p4 import _restart

    fixtures = FIXTURES[:1]
    # The fixture sits in the player's own chunk, six blocks from the player.
    chunk_x = origin[0] // 16
    points = [{"x": chunk_x * 16 + (origin[0] % 16 + 6) % 16, "y": origin[1], "z": origin[2]}]
    _prepare(steps, origin, fixtures, points)
    _restart(steps)
    steps.check("server_never_panicked", "panicked" not in _server_log(steps))
