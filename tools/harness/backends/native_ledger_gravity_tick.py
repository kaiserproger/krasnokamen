"""Ledger HS-3: an imported due gravity tick starts the fall without a nearby write.

The fixture is the state the row was reported from and that only a save can
carry: an unsupported sand block whose chunk `block_ticks` list holds a due tick
for it. A native write never leaves that behind - `/setblock` converts a gravity
cell into a falling entity inside the placing write (`falling::start`), and no
engine path schedules a gravity block tick - so the harness plants the saved
record into the world `world export-anvil` just wrote
(`anvil_scheduled_tick.plant_due_gravity_tick`: the sand's support cell becomes
air and the chunk gains the sand's own tick entry), then `world import-anvil`
reads it back through the product's restore path
(`mc-world-conversion/src/import.rs:726-745`).

Before the export the sand sits on its support and the client reads it there,
so a mis-placed fixture cannot pass on an empty assertion. After the restart
nothing writes near the block: the scene only reads it, and the due tick alone
must start the fall. A frozen world is deliberately *not* used: `/tick freeze`
holds the falling *entity* such a placement already spawned, which would prove
the entity lane instead of the restored block tick.

Assertions:
* `saved_tick_moved_the_sand` - the block leaves its cell inside the tick budget
  the planted entry names, with no command writing near it (the row's symptom);
* `sand_landed_on_its_support` - it lands one block down on the kept support.
The read of the sand in its cell right after the rejoin is recorded, not
asserted: the tick is due after the rejoin, and a slower rejoin would only make
the block fall earlier.
"""
from __future__ import annotations

from typing import Any

from .anvil_scheduled_tick import plant_due_gravity_tick
from .native_v2 import _block, _echo
from .probe_steps import ProbeSteps

SCENARIO = "native-v2-ledger-gravity-tick"
SAND = "minecraft:sand"
STONE = "minecraft:stone"
AIR = "minecraft:air"
# The planted tick is due 600 ticks (30 s) after the restart: the measured
# restart-plus-join cost of this harness is ~187 ticks (freeze-export gate), so
# the client reads the sand in its cell first and the wait below still ends
# inside the scenario budget.
DELAY_TICKS = 600
TICKS_PER_STEP = 20
STEPS = 45


def _block_id(steps: ProbeSteps, point: dict[str, int]) -> str:
    answer = steps.call("minecraft_read_block", point)
    if isinstance(answer, dict) and isinstance(answer.get("block"), dict):
        return str(answer["block"].get("block_id", ""))
    return str(answer.get("block_id", "")) if isinstance(answer, dict) else ""


def _point(x: int, y: int, z: int) -> dict[str, int]:
    return {"x": x, "y": y, "z": z}


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    if not callable(getattr(steps.client, "p4_restart_server", None)):
        raise RuntimeError("HS-3 requires the p4_restart_server export/import lifecycle hook")
    x, y, z = origin
    # Open sky above the pad: the kept support is the landing floor, the cell
    # between it and the sand is the one the export edit turns into air.
    support = _point(x + 3, y + 7, z + 3)
    gap = _point(x + 3, y + 8, z + 3)
    placed = _point(x + 3, y + 9, z + 3)
    for point, block in ((support, STONE), (gap, STONE), (placed, SAND)):
        steps.fixture(f"setblock {point['x']} {point['y']} {point['z']} {block}")
    _block(steps, support, STONE)
    _block(steps, gap, STONE)
    held = _block(steps, placed, SAND)
    steps.check("sand_placed_on_its_support", held.get("status") == "matched")
    steps.report["fixture"] = {"support": support, "gap": gap, "sand": placed}
    _echo(steps, "gravity_tick_fixture_placed")

    steps.fixture("save-all")
    player = steps.observe()["player"]["entity_uuid"]
    steps.call("minecraft_disconnect", {})
    steps.check("gravity_tick_client_disconnected",
                steps.call("minecraft_observe")["in_play"] is False)

    # The harness hook stops the server cleanly, exports the private world,
    # lets the fixture rewrite the exported copy and imports it again before the
    # restart, so the due tick reaches the chunk as saved Anvil data.
    edits: dict[str, Any] = {}

    def plant(world: Any) -> dict[str, Any]:
        edits.update(plant_due_gravity_tick(
            world, clear=(gap["x"], gap["y"], gap["z"]),
            tick=(placed["x"], placed["y"], placed["z"]),
            block=SAND, clear_block=STONE, delay=DELAY_TICKS))
        return edits

    steps.client.export_edit = plant
    address = steps.client.p4_restart_server(steps.remaining())
    steps.report["export_edit"] = edits
    steps.check("export_carried_the_saved_tick",
                edits.get("tick", {}).get("t") == DELAY_TICKS
                and edits.get("cleared", {}).get("now") == AIR
                and edits.get("tick", {}).get("i") == SAND)
    steps.call("minecraft_connect", {"server_addr": address})
    play = steps.call("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})
    steps.check("reimport_entered_play", play["in_play"] is True)
    state = steps.wait(lambda s: s["player"]["on_ground"], "reimport_grounded")
    steps.check("reimport_same_player", state["player"]["entity_uuid"] == player)
    steps.call("minecraft_wait_for_loaded_block", {
        **placed, "timeout_seconds": steps.remaining()})

    reads: list[str] = []
    for _ in range(STEPS):
        reads.append(_block_id(steps, placed))
        if reads[-1] != SAND:
            break
        steps.call("minecraft_wait_ticks", {"ticks": TICKS_PER_STEP})
    steps.report["reads_after_reimport"] = reads
    steps.report["first_read_after_reimport"] = reads[0] if reads else None
    steps.report["landing"] = {**steps.call("minecraft_read_block", gap)}
    steps.report["support"] = {**steps.call("minecraft_read_block", support)}
    # The row's symptom: with the fix reversed the restored tick is never
    # activated, so the sand holds its cell for the whole budget.
    steps.check("saved_tick_moved_the_sand", reads[-1] != SAND)
    steps.check("sand_landed_on_its_support", _block_id(steps, gap) == SAND)
    steps.check("landing_support_survived", _block_id(steps, support) == STONE)
    _echo(steps, "gravity_tick_landed_ordered")

    steps.report["limits"] = [
        "One saved sand tick, one conversion round trip and one support cell cleared in the "
        "exported copy; the fall is read through the client's own chunk model and nothing writes "
        "near the block after the restart. No gravel, no other falling kind and no second saved "
        "tick; the sand's own supported placement before the export is the only world edit.",
    ]
