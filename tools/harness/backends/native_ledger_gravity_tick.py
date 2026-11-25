"""Ledger HS-3: an imported due gravity tick starts the fall without a nearby write.

The row's fixture is an Anvil world that *saves* a due `block_ticks` entry for a floating
sand block. `/setblock` can never leave that behind on a running server (the tick loop
consumes it), but a frozen world can: `/tick freeze` stops the tick loop while the
scheduled tick stays in the store, the clean stop exports it as the chunk's own
`block_ticks` list (`mc-world-conversion/src/export.rs:110-199`), and the import reads it
back (`import.rs:714-745`). After the restart nothing but the restored tick can move the
sand, so the assertions are "the block left its position" and "it landed one block down".

WHY this is a control: with the row's fix (`d3e8d4686`) reversed the restored tick is
never activated and the sand stays; the cargo ref
`falling::tests::restored_due_gravity_tick_starts_the_fall_without_nearby_writes` is the
same claim in core.
"""
from __future__ import annotations

from typing import Any

from .probe_steps import ProbeSteps

SAND = "minecraft:sand"
AIR = "minecraft:air"
STEPS = 10
TICKS_PER_STEP = 10


def _read(steps: ProbeSteps, pos: tuple[int, int, int]) -> str:
    block = steps.call("minecraft_read_block", {"x": pos[0], "y": pos[1], "z": pos[2]})
    return block.get("block", {}).get("block_id", "")


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    if not callable(getattr(steps.client, "promotion_mount_export_reimport", None)):
        raise RuntimeError("HS-3 requires the offline clean export/reimport lifecycle hook")
    # Six blocks above the pad floor: floating, well inside the client's view.
    spot = (x + 2, y + 6, z + 2)
    below = (spot[0], spot[1] - 1, spot[2])

    # A frozen world keeps the scheduled tick in the store instead of running it.
    steps.fixture("tick freeze")
    steps.fixture(f"setblock {below[0]} {below[1]} {below[2]} minecraft:air")
    steps.fixture(f"setblock {spot[0]} {spot[1]} {spot[2]} {SAND}")
    steps.call("minecraft_wait_for_loaded_block", {
        "x": spot[0], "y": spot[1], "z": spot[2], "timeout_seconds": steps.remaining()})
    steps.check("frozen_sand_placed", _read(steps, spot) == SAND)
    steps.check("no_support_below", _read(steps, below) == AIR)
    steps.call("minecraft_wait_ticks", {"ticks": 20})
    steps.check("frozen_sand_stays_while_the_tick_is_pending", _read(steps, spot) == SAND)
    steps.report["fixture"] = {"sand": list(spot), "below": list(below)}

    # Clean stop -> export -> import -> restart: the due tick is now saved Anvil data.
    steps.fixture("save-all")
    player = steps.observe()["player"]["entity_uuid"]
    steps.call("minecraft_disconnect")
    receipt = steps.client.promotion_mount_export_reimport(steps.remaining())
    if not isinstance(receipt, dict) or not isinstance(receipt.get("address"), str) or not receipt["address"]:
        raise RuntimeError("export/reimport hook must return an address and conversion receipts")
    if not all(isinstance(receipt.get(key), str) and receipt[key] for key in ("export_receipt", "import_receipt")):
        raise RuntimeError("export/reimport hook omitted conversion receipt paths")
    steps.report["world_conversion"] = receipt
    steps.call("minecraft_connect", {"server_addr": receipt["address"]})
    steps.check("reimport_entered_play", steps.call("minecraft_wait_for_play", {
        "timeout_seconds": steps.remaining()})["in_play"] is True)
    state = steps.wait(lambda s: s["player"]["on_ground"], "reimport_grounded")
    steps.check("reimport_same_player", state["player"]["entity_uuid"] == player)
    steps.call("minecraft_wait_for_loaded_block", {
        "x": spot[0], "y": spot[1], "z": spot[2], "timeout_seconds": steps.remaining()})

    # No command writes near the spot after the restart: only the restored tick can move it.
    reads: list[str] = []
    for _ in range(STEPS):
        steps.call("minecraft_wait_ticks", {"ticks": TICKS_PER_STEP})
        reads.append(_read(steps, spot))
        if reads[-1] != SAND:
            break
    steps.report["reads_after_restart"] = reads
    steps.check("restored_due_tick_starts_the_fall", reads and reads[-1] != SAND)
    steps.check("sand_landed_one_block_down", _read(steps, below) == SAND)

    steps.report["limits"] = [
        "One frozen-world sand block at six blocks above the pad, one clean export/import round trip; "
        "the fall is read through the client's own chunk model. No gravel, no unsupported-direction "
        "block, no second tick and no neighbour write other than the fixture's own air setblock.",
    ]
